//! Golden values and all population/layout policy remain in external specs.
use formatkit_core::{bytes_at, Error, Pcm, Result};
use formatkit_corpus_test::{hash_bytes, run_dir, Args, Digest};
use serde_json::{json, Value};
use std::cell::Cell;
use ws_rom::BankState;
use ws_sound::{
    drivers::{
        byte::{self, Dialect},
        cell, sdk, RomView,
    },
    instrument,
    render::{self, Action, Event},
    WaveBank,
};

fn number(args: &Args, name: &str) -> Result<usize> {
    args.get(name)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| Error::Malformed(format!("sound spec lacks {name}")))
}
fn small(args: &Args, name: &str) -> Result<u16> {
    u16::try_from(number(args, name)?)
        .map_err(|_| Error::Malformed(format!("sound spec {name} exceeds word")))
}
fn values<'a>(args: &'a Args, name: &str) -> Result<&'a [Value]> {
    args.get(name)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| Error::Malformed(format!("sound spec lacks array {name}")))
}
fn row_args(value: &Value) -> Result<Args> {
    Ok(Args(
        value
            .as_object()
            .ok_or_else(|| Error::Malformed("sound row must be object".into()))?
            .clone(),
    ))
}
fn pcm_hash(pcm: &Pcm) -> String {
    let bytes: Vec<_> = pcm.samples().iter().flat_map(|s| s.to_le_bytes()).collect();
    hash_bytes(&bytes)
}

#[derive(Default)]
struct SoundDigest {
    headers: Cell<usize>,
    tracks: Cell<usize>,
    waves: Cell<usize>,
    cells: Cell<usize>,
    pcm_bytes: Cell<usize>,
    preview_frames: Cell<usize>,
    se_records: Cell<usize>,
    envelopes: Cell<usize>,
}

impl Digest for SoundDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let mut result = match args.str("kind") {
            Some("byte-table") => self.byte_table(bytes, args)?,
            Some("cell-directory") => self.cell_directory(bytes, args)?,
            Some("sdk-tracks") => self.sdk_tracks(bytes, args)?,
            Some("preview") => self.preview(bytes, args)?,
            Some("byte-banks") => self.byte_banks(bytes, args)?,
            Some("captured-preview") => self.captured_preview(bytes, args)?,
            _ => return Err(Error::Malformed("unknown sound corpus kind".into())),
        };
        result
            .as_object_mut()
            .ok_or_else(|| Error::Malformed("sound digest must be object".into()))?
            .insert("sha256".into(), json!(hash_bytes(bytes)));
        let references = values(args, "references")?;
        if references.is_empty()
            || references.iter().any(|r| {
                r.get("sha256")
                    .and_then(Value::as_str)
                    .is_none_or(|h| h.len() != 64)
            })
        {
            return Err(Error::Malformed(
                "sound corpus requires independent reference provenance".into(),
            ));
        }
        Ok(result)
    }
}

impl SoundDigest {
    fn captured_preview(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let frames = number(args, "frames")?;
        let tick_frames = number(args, "tick_frames")?;
        if tick_frames == 0 {
            return Err(Error::Malformed(
                "capture tick duration must be positive".into(),
            ));
        }
        let mut events = vec![Event {
            frame: 0,
            action: Action::Write {
                port: 0x91,
                value: 8,
            },
        }];
        let text = std::str::from_utf8(bytes)
            .map_err(|_| Error::Malformed("capture text is not UTF-8".into()))?;
        for line in text
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
        {
            let fields: Vec<_> = line.split('\t').collect();
            if fields.len() != 4 {
                return Err(Error::Malformed("capture row must have four fields".into()));
            }
            let tick: i32 = fields[0]
                .parse()
                .map_err(|_| Error::Malformed("capture tick is not signed integer".into()))?;
            if tick < -1 {
                return Err(Error::Malformed(
                    "capture tick precedes initialization".into(),
                ));
            }
            let frame = (tick.max(0) as usize)
                .checked_mul(tick_frames)
                .ok_or_else(|| Error::Malformed("capture timeline overflow".into()))?;
            let action = match fields[1] {
                "out" => {
                    let port = u8::from_str_radix(fields[2], 16)
                        .map_err(|_| Error::Malformed("capture port is not byte".into()))?;
                    let value = u8::from_str_radix(fields[3], 16)
                        .map_err(|_| Error::Malformed("capture value is not byte".into()))?;
                    if port == 0x52 && value & 128 != 0 {
                        return Err(Error::Unsupported(
                            "capture parity excerpt requires no sound DMA".into(),
                        ));
                    }
                    if !(0x80..=0x94).contains(&port) || matches!(port, 0x91..=0x93) {
                        continue;
                    }
                    Action::Write { port, value }
                }
                "wave" => {
                    let hex = fields[3];
                    if hex.len() != 128 || !hex.is_ascii() {
                        return Err(Error::Malformed(
                            "capture wave must contain 64 hex bytes".into(),
                        ));
                    }
                    let data: Result<Vec<_>> = hex
                        .as_bytes()
                        .chunks_exact(2)
                        .map(|pair| {
                            let text = std::str::from_utf8(pair)
                                .map_err(|_| Error::Malformed("wave hex is not ASCII".into()))?;
                            u8::from_str_radix(text, 16)
                                .map_err(|_| Error::Malformed("invalid wave hex byte".into()))
                        })
                        .collect();
                    // RAM captured after the tick is applied on the next tick
                    // by the existing Java listening preview.
                    let wave_frame = tick
                        .checked_add(1)
                        .ok_or_else(|| Error::Malformed("capture tick overflow".into()))?
                        as usize;
                    let wave_frame = wave_frame
                        .checked_mul(tick_frames)
                        .ok_or_else(|| Error::Malformed("capture wave timeline overflow".into()))?;
                    if wave_frame < frames {
                        events.push(Event {
                            frame: wave_frame,
                            action: Action::Waves(WaveBank::decode(&data?)?),
                        });
                    }
                    continue;
                }
                "pos" | "dma" => continue,
                _ => return Err(Error::Unsupported("unknown capture row kind".into())),
            };
            if frame < frames {
                events.push(Event { frame, action });
            }
        }
        events.sort_by_key(|event| event.frame);
        let pcm = render::render_coupled(&events, frames)?;
        self.preview_frames.set(self.preview_frames.get() + frames);
        Ok(
            json!({"frames":frames,"channels":pcm.channels(),"sample_rate":pcm.sample_rate(),"pcm_sha256":pcm_hash(&pcm)}),
        )
    }

    fn byte_banks(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let mut directories = vec![];
        for value in values(args, "pcm_directories")? {
            let row = row_args(value)?;
            let offset = number(&row, "offset")?;
            let count = number(&row, "count")?;
            if count > byte_limit() {
                return Err(Error::Malformed(
                    "PCM directory count exceeds policy".into(),
                ));
            }
            let size = count
                .checked_mul(11)
                .ok_or_else(|| Error::Malformed("PCM directory overflow".into()))?;
            let mut records = vec![];
            for data in bytes_at(bytes, offset, size)?.chunks_exact(11) {
                let command = byte::pcm_se(data)?;
                records.push(
                    serde_json::to_value(command)
                        .map_err(|error| Error::Malformed(error.to_string()))?,
                );
                self.se_records.set(self.se_records.get() + 1);
            }
            directories.push(json!({"offset":offset,"records":records}));
        }
        let mut envelopes = vec![];
        for value in values(args, "pitch_spans")? {
            let row = row_args(value)?;
            let offset = number(&row, "offset")?;
            let length = number(&row, "length")?;
            let mut reader = formatkit_core::Reader::new(bytes_at(bytes, offset, length)?);
            let mut records = vec![];
            while reader.remaining() > 0 {
                let envelope = instrument::PitchEnvelope::parse(&mut reader, 256)?;
                records.push(json!({"delay":envelope.delay,"steps":envelope.steps}));
                self.envelopes.set(self.envelopes.get() + 1);
            }
            envelopes.push(json!({"offset":offset,"records":records}));
        }
        Ok(json!({"directories":directories,"pitch_envelopes":envelopes}))
    }

    fn byte_table(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let view = RomView::new(bytes, BankState::RESET)?;
        let dialect = match args.str("dialect") {
            Some("a") => Dialect::A,
            Some("b") => Dialect::B,
            _ => return Err(Error::Malformed("sound corpus table dialect".into())),
        };
        let headers = byte::table(
            view,
            number(args, "table")?,
            number(args, "count")?,
            small(args, "header_segment")?,
            dialect,
            small(args, "fixed_segment")?,
        )?;
        self.headers.set(self.headers.get() + headers.len());
        let mut out = vec![];
        for header in headers {
            self.tracks.set(self.tracks.get() + header.tracks.len());
            let tracks: Result<Vec<_>> = header.tracks.iter().map(|p| Ok(json!({"slot":p.slot,"channel":p.channel(dialect),"offset":p.offset,"rom":view.offset(p.segment,p.offset)?}))).collect();
            out.push(json!({"mask":header.mask,"priority":header.priority,"segment":header.segment,"tracks":tracks?}));
        }
        let mut waves = vec![];
        for row in values(args, "wave_spans")? {
            let r = row_args(row)?;
            let offset = number(&r, "offset")?;
            let count = number(&r, "count")?;
            let len = count
                .checked_mul(16)
                .ok_or_else(|| Error::Malformed("wave extent overflow".into()))?;
            let decoded = instrument::wave_bank(view.file_span(offset, len)?, count)?;
            self.waves.set(self.waves.get() + count);
            let samples: Vec<_> = decoded
                .iter()
                .flat_map(|wave| wave.samples().iter().copied())
                .collect();
            waves
                .push(json!({"offset":offset,"count":count,"samples_sha256":hash_bytes(&samples)}));
        }
        let mut samples = vec![];
        for row in values(args, "samples")? {
            let r = row_args(row)?;
            let offset = number(&r, "offset")?;
            let len = number(&r, "length")?;
            let rate = u32::try_from(number(&r, "rate")?)
                .map_err(|_| Error::Malformed("PCM rate overflow".into()))?;
            let pcm = render::unsigned_pcm(view.file_span(offset, len)?, rate, render::MAX_FRAMES)?;
            samples.push(
                json!({"offset":offset,"length":len,"rate":rate,"pcm_sha256":pcm_hash(&pcm)}),
            );
            self.pcm_bytes.set(self.pcm_bytes.get() + len);
        }
        Ok(json!({"headers":out,"waves":waves,"samples":samples}))
    }

    fn cell_directory(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let view = RomView::new(bytes, BankState::RESET)?;
        let group_count = number(args, "group_count")?;
        let directory_size = group_count
            .checked_mul(4)
            .ok_or_else(|| Error::Malformed("group extent overflow".into()))?;
        let groups = cell::groups(
            view.file_span(number(args, "directory")?, directory_size)?,
            group_count,
        )?;
        let wave_count = number(args, "wave_count")?;
        let wave_limit = number(args, "wave_limit")?;
        let mut songs = vec![];
        let mut cells = vec![];
        let mut samples = vec![];
        let mut accepted = 0;
        for (group_index, group) in groups.iter().enumerate() {
            let end = groups
                .get(group_index + 1)
                .map_or(256, |next| usize::from(next.first_id));
            for id in usize::from(group.first_id)..end {
                let entry = u16::try_from((id - usize::from(group.first_id)) * 13)
                    .map_err(|_| Error::Malformed("song entry offset overflow".into()))?;
                let song = cell::Song::parse(view.span(group.segment, entry, 13)?)?;
                let pcm = song.pcm(view, group.segment);
                let mut valid = false;
                let mut is_pcm = false;
                match pcm {
                    Ok(Some(pcm)) => {
                        is_pcm = true;
                        valid = true;
                        let data = view.span(group.segment, pcm.offset, usize::from(pcm.length))?;
                        // Nominal listening rate supplied explicitly by the external spec.
                        let output = render::unsigned_pcm(
                            data,
                            number(args, "nominal_pcm_rate")? as u32,
                            render::MAX_FRAMES,
                        )?;
                        samples.push(
                            json!({"id":id,"length":pcm.length,"pcm_sha256":pcm_hash(&output)}),
                        );
                        self.pcm_bytes.set(self.pcm_bytes.get() + data.len());
                    }
                    Ok(None) => {
                        let wave_ids = song.waves(view, group.segment);
                        let mut parsed = vec![];
                        if let Ok(wave_ids) = wave_ids {
                            valid = wave_ids.iter().all(|&id| usize::from(id) < wave_limit);
                            for (channel, &offset) in song.tracks.iter().enumerate() {
                                match cell::track(
                                    view,
                                    group.segment,
                                    offset,
                                    number(args, "track_limit")?,
                                ) {
                                    Ok(track) => {
                                        for (index, data) in track.cells.iter().enumerate() {
                                            if let cell::Cell::Command {
                                                opcode: 10,
                                                operand,
                                            } = cell::Cell::parse(data)?
                                            {
                                                valid &= usize::from(operand & 255) < wave_limit;
                                            }
                                            let off = usize::from(offset) + index * 4;
                                            parsed.push((channel, off, *data));
                                        }
                                    }
                                    Err(_) => valid = false,
                                }
                            }
                        }
                        if valid {
                            accepted += 1;
                            self.tracks.set(self.tracks.get() + 4);
                            for (channel, offset, data) in parsed {
                                cells.push(id as u8);
                                cells.push(channel as u8);
                                cells.extend((offset as u16).to_le_bytes());
                                cells.extend(data);
                                self.cells.set(self.cells.get() + 1);
                            }
                        }
                    }
                    Err(_) => {}
                }
                songs.push(json!({"id":id,"segment":group.segment,"entry":entry,"tracks":song.tracks,"tempo":song.tempo,"wave_table":song.wave_table,"flag":song.flag,"pcm":is_pcm,"valid":valid}));
            }
        }
        let wave_size = wave_count
            .checked_mul(16)
            .ok_or_else(|| Error::Malformed("wave extent overflow".into()))?;
        let waves = instrument::wave_bank(
            view.file_span(number(args, "wave_base")?, wave_size)?,
            wave_count,
        )?;
        let samples_wave: Vec<_> = waves
            .iter()
            .flat_map(|wave| wave.samples().iter().copied())
            .collect();
        self.waves.set(self.waves.get() + wave_count);
        self.headers.set(self.headers.get() + songs.len());
        Ok(
            json!({"songs":songs,"accepted_sequences":accepted,"cells_sha256":hash_bytes(&cells),"wave_samples_sha256":hash_bytes(&samples_wave),"samples":samples}),
        )
    }

    fn sdk_tracks(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let count = number(args, "count")?;
        let size = count
            .checked_mul(6)
            .ok_or_else(|| Error::Malformed("SDK table overflow".into()))?;
        let tracks = sdk::tracks(bytes_at(bytes, number(args, "table")?, size)?, count)?;
        let view = RomView::new(bytes, BankState::RESET)?;
        let segment = small(args, "segment")?;
        let mut rows = vec![];
        for track in tracks {
            let data = view.span(
                segment,
                track.sequence,
                0x10000 - usize::from(track.sequence),
            )?;
            let sequence = sdk::sequence(data, number(args, "cell_limit")?)?;
            self.cells.set(self.cells.get() + sequence.len());
            rows.push(json!({"slot_offset":track.slot_offset,"channel":track.channel,"sequence":track.sequence,"cells":sequence.len(),"cells_sha256":hash_bytes(&data[..sequence.len()*2])}));
        }
        self.tracks.set(self.tracks.get() + count);
        Ok(json!({"tracks":rows}))
    }

    fn preview(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let frames = number(args, "frames")?;
        let mut events = vec![Event {
            frame: 0,
            action: Action::Waves(WaveBank::decode(bytes)?),
        }];
        for value in values(args, "writes")? {
            let row = row_args(value)?;
            let port = u8::try_from(number(&row, "port")?)
                .map_err(|_| Error::Malformed("preview port overflow".into()))?;
            let value = u8::try_from(number(&row, "value")?)
                .map_err(|_| Error::Malformed("preview value overflow".into()))?;
            events.push(Event {
                frame: number(&row, "frame")?,
                action: Action::Write { port, value },
            });
        }
        let pcm = render::render_coupled(&events, frames)?;
        self.preview_frames.set(self.preview_frames.get() + frames);
        Ok(
            json!({"frames":pcm.frame_count(),"channels":pcm.channels(),"sample_rate":pcm.sample_rate(),"pcm_sha256":pcm_hash(&pcm)}),
        )
    }
}

fn byte_limit() -> usize {
    ws_sound::drivers::MAX_RECORDS
}

#[test]
fn captured_audio_parity() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen sound specs"
    );
    let handler = SoundDigest::default();
    let inspected = run_dir("ws-sound-captures", &handler);
    if inspected > 0 {
        eprintln!(
            "ws-sound capture parity: {inspected} excerpts; {} frames; exact PCM hashes passed",
            handler.preview_frames.get()
        );
    }
}

#[test]
fn sound_bank_parity() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen sound specs"
    );
    let handler = SoundDigest::default();
    let inspected = run_dir("ws-sound-extra", &handler);
    if inspected > 0 {
        eprintln!("ws-sound bank parity: {inspected} samples; {} PCM SE records, {} pitch envelopes; all frozen digests passed",handler.se_records.get(),handler.envelopes.get());
    }
}

#[test]
fn sound_parity() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen sound specs"
    );
    let handler = SoundDigest::default();
    let inspected = run_dir("ws-sound", &handler);
    if inspected > 0 {
        eprintln!("ws-sound parity: {inspected} samples; {} headers, {} tracks, {} waves, {} sequence cells, {} PCM source bytes, {} preview frames; all frozen digests passed",handler.headers.get(),handler.tracks.get(),handler.waves.get(),handler.cells.get(),handler.pcm_bytes.get(),handler.preview_frames.get());
    }
}

// Include the driver-derived real headers in the workspace corpus acceptance gate.
#[path = "review_byte_c.rs"]
mod byte_c;
