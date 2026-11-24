use crate::{args, io, Args, Result, MAX_FILE};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::path::Path;
use swankit::{
    extract,
    rom::{BankState, RomGeometry},
    sound::{
        drivers::{
            sdk_discover::discover_sdk_link,
            sdk_driver::{field, SdkDriver, SdkLink, Target, VoiceTrigger},
        },
        render::{render_cycles, Action, CounterReload, CycleEvent},
        WaveBank,
    },
};

/// Default WonderSwan VBlank frame length in CPU cycles (159 scanlines * 256 cycles).
const DEFAULT_CYCLES_PER_TICK: u64 = 40_704;
/// WonderSwan scanline length in CPU cycles (WSdev Timing).
const CYCLES_PER_SCANLINE: u64 = 256;
/// Maximum chained voice descriptors followed per voice trigger.
const MAX_VOICE_CHAIN: usize = 64;
/// Default number of consecutive track-table entries started per song.
const DEFAULT_TRACKS_PER_SONG: u16 = 4;
/// Hard upper bound on simulated driver ticks per CLI invocation.
const MAX_TICKS: u32 = 100_000;

pub const HELP: &str =
    "swankit sound sdk-discover --rom ROM [--out-json JSON] [--force] [--json]
swankit sound sdk-render --rom ROM (--link LINK.json | --auto-link) --song N --ticks N --out-json JSON
  [--out-wav WAV] [--out-events FILE] [--reload-mode next-period|immediate]
  [--tracks N] [--initial-phase N] [--force] [--json]
Discover linked SDK sound-driver tables from ROM bytes or simulate the standard
SDK two-byte-cell sound driver and emit metadata, optional per-tick register
events (.json or .tsv), and an optional 24 kHz cycle-timed stereo WAV.
Integers accept decimal or 0x hexadecimal.";

#[derive(Debug, Deserialize)]
struct SongEntry {
    first_track: u16,
    count: u16,
}

#[derive(Debug, Deserialize)]
struct SdkLinkConfig {
    segment_file_offset: usize,
    link: SdkLink,
    #[serde(default = "default_headphones")]
    headphones: bool,
    #[serde(default = "default_tracks_per_song")]
    tracks_per_song: u16,
    #[serde(default = "default_cycles_per_tick")]
    cycles_per_tick: u64,
    #[serde(default)]
    song_count: Option<u16>,
    #[serde(default)]
    songs: Option<Vec<SongEntry>>,
}

fn default_headphones() -> bool {
    true
}

fn default_tracks_per_song() -> u16 {
    DEFAULT_TRACKS_PER_SONG
}

fn default_cycles_per_tick() -> u64 {
    DEFAULT_CYCLES_PER_TICK
}

#[derive(Debug)]
struct TrackMetadata {
    track: u16,
    slot: u16,
    channel: u8,
    flags: u8,
    sequence_offset: u16,
}

pub fn run(a: Args, json_mode: bool) -> Result<()> {
    let action = a.positional.get(1).cloned().ok_or(
        "sound requires a subcommand (sdk-discover, sdk-render); see swankit sound --help",
    )?;
    match action.as_str() {
        "sdk-discover" => sdk_discover(a, json_mode),
        "sdk-render" => sdk_render(a, json_mode),
        _ => Err(format!("unknown sound subcommand: {action}; see swankit sound --help").into()),
    }
}

fn sdk_discover(mut a: Args, json_mode: bool) -> Result<()> {
    let rom_path = a.require("--rom")?;
    let out_json = a.value("--out-json");
    let force = a.flag("--force");
    a.finish(2)?;

    let rom = io::read(&rom_path, MAX_FILE)?;
    let disc = discover_sdk_link(&rom)?;
    let output = json!({
        "schema": "swankit-sdk-link/1",
        "segment_file_offset": disc.segment_file_offset,
        "data_segment": disc.data_segment,
        "headphones": true,
        "fetch_variant": disc.fetch_variant,
        "tick_offset": disc.tick_offset,
        "fetch_offset": disc.fetch_offset,
        "clear_voice_offset": disc.clear_voice_offset,
        "start_voice_offset": disc.start_voice_offset,
        "link": disc.link,
    });
    let bytes = serde_json::to_vec_pretty(&output)?;
    if let Some(path) = &out_json {
        io::write(path, &bytes, force)?;
    }
    if json_mode || out_json.is_none() {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if let Some(path) = &out_json {
        println!(
            "Discovered SDK driver at segment 0x{:x} (track_table 0x{:04x}) -> {path}",
            disc.segment_file_offset, disc.link.track_table
        );
    }
    Ok(())
}

fn sdk_render(mut a: Args, json_mode: bool) -> Result<()> {
    let rom_path = a.require("--rom")?;
    let link_path = a.value("--link");
    let auto_link = a.flag("--auto-link");
    let song_raw = a.require("--song")?;
    let ticks_raw = a.require("--ticks")?;
    let out_json = a.require("--out-json")?;
    let out_wav = a.value("--out-wav");
    let out_events = a.value("--out-events");
    let reload_raw = a
        .value("--reload-mode")
        .unwrap_or_else(|| "next-period".to_owned());
    let tracks_override = a.value("--tracks");
    let initial_phase_raw = a.value("--initial-phase");
    let force = a.flag("--force");
    a.finish(2)?;

    if link_path.is_some() == auto_link {
        return Err("specify exactly one of --link LINK.json or --auto-link".into());
    }

    let initial_phase = match initial_phase_raw {
        Some(raw) => u8::try_from(args::number(&raw)?)
            .map_err(|_| format!("initial-phase must be in 0..=255: {raw}"))?,
        None => 0,
    };

    let song = u16::try_from(args::number(&song_raw)?)
        .map_err(|_| format!("song index out of u16 range: {song_raw}"))?;
    let ticks = u32::try_from(args::number(&ticks_raw)?)
        .map_err(|_| format!("ticks out of u32 range: {ticks_raw}"))?;
    if ticks == 0 {
        return Err("ticks must be at least 1".into());
    }
    if ticks > MAX_TICKS {
        return Err(format!("ticks {ticks} exceeds limit {MAX_TICKS}").into());
    }

    let (reload_mode, reload_label) = match reload_raw.as_str() {
        "next-period" => (CounterReload::NextPeriod, "next-period"),
        "immediate" => (CounterReload::Remainder, "immediate"),
        other => {
            return Err(format!(
                "invalid --reload-mode '{other}'; expected next-period or immediate"
            )
            .into())
        }
    };

    let tracks_override = match tracks_override {
        Some(raw) => {
            let n = u16::try_from(args::number(&raw)?)
                .map_err(|_| format!("tracks out of range: {raw}"))?;
            if !(1..=8).contains(&n) {
                return Err(format!("tracks must be in 1..=8, got {n}").into());
            }
            Some(n)
        }
        None => None,
    };

    let rom = io::read(&rom_path, MAX_FILE)?;
    let config: SdkLinkConfig = if let Some(path) = &link_path {
        let link_bytes = io::read(path, extract::MAX_DEFINITION_BYTES)?;
        serde_json::from_slice(&link_bytes)
            .map_err(|e| format!("invalid SDK link JSON ({path}): {e}"))?
    } else {
        let disc = discover_sdk_link(&rom)?;
        SdkLinkConfig {
            segment_file_offset: disc.segment_file_offset,
            link: disc.link,
            headphones: true,
            tracks_per_song: DEFAULT_TRACKS_PER_SONG,
            cycles_per_tick: DEFAULT_CYCLES_PER_TICK,
            song_count: None,
            songs: None,
        }
    };

    if config.link.slot_stride == 0 {
        return Err("SDK link slot_stride must be non-zero".into());
    }
    if !(1..=8).contains(&config.tracks_per_song) {
        return Err(format!(
            "SDK link tracks_per_song must be in 1..=8, got {}",
            config.tracks_per_song
        )
        .into());
    }
    if config.cycles_per_tick == 0 {
        return Err("SDK link cycles_per_tick must be non-zero".into());
    }
    if let Some(limit) = config.song_count {
        if song >= limit {
            return Err(format!("song index {song} out of range (song_count {limit})").into());
        }
    }

    let (first_track, tracks) = if let Some(songs) = &config.songs {
        let entry = songs.get(usize::from(song)).ok_or_else(|| {
            format!(
                "song index {song} out of range (songs table has {} entries)",
                songs.len()
            )
        })?;
        let count = tracks_override.unwrap_or(entry.count);
        if !(1..=8).contains(&count) {
            return Err(format!("song {song} track count must be in 1..=8, got {count}").into());
        }
        (entry.first_track, count)
    } else {
        let count = tracks_override.unwrap_or(config.tracks_per_song);
        let first = song
            .checked_mul(config.tracks_per_song)
            .ok_or_else(|| format!("song index {song} overflows track index"))?;
        (first, count)
    };

    if config.segment_file_offset >= rom.len() {
        return Err(format!(
            "segment_file_offset {} exceeds ROM length {}",
            config.segment_file_offset,
            rom.len()
        )
        .into());
    }
    let seg_end = config
        .segment_file_offset
        .saturating_add(0x10000)
        .min(rom.len());
    let segment = &rom[config.segment_file_offset..seg_end];

    let track_meta = validate_tracks(segment, &config.link, first_track, tracks)?;

    // Check WAV frame bounds before running the simulation.
    let wav_frames = if out_wav.is_some() {
        let total_cycles = u64::from(ticks)
            .checked_mul(config.cycles_per_tick)
            .ok_or("total render cycles overflow")?;
        let frames = usize::try_from(total_cycles / 128).map_err(|_| "WAV frame count overflow")?;
        if frames == 0 {
            return Err("WAV frame count is zero; increase --ticks or cycles_per_tick".into());
        }
        if frames > swankit::sound::render::MAX_FRAMES {
            return Err(format!(
                "WAV frame count {frames} exceeds limit {}",
                swankit::sound::render::MAX_FRAMES
            )
            .into());
        }
        Some(frames)
    } else {
        None
    };

    // Run the proven SDK driver model to completion.
    let mut model = SdkDriver::new(config.link, segment, config.headphones)?;
    model.ram[usize::from(config.link.globals.phase_counter)] = initial_phase;
    model.reset(0)?;
    model.start(first_track, tracks)?;
    let tick_cycles = config.cycles_per_tick.min(u64::from(u32::MAX)) as u32;
    for t in 0..ticks {
        if t > 0 {
            model.advance_hardware(tick_cycles);
        }
        model.tick()?;
    }
    if model.ticks != ticks {
        return Err(format!(
            "incomplete SDK driver run: completed {} of {ticks} ticks",
            model.ticks
        )
        .into());
    }

    // Prepare all outputs in memory before writing anything to disk.
    let mut port_writes = 0usize;
    let mut wave_writes = 0usize;
    for w in &model.writes {
        match w.target {
            Target::Port(_) => port_writes += 1,
            Target::WaveWord(_) => wave_writes += 1,
        }
    }

    let wav_bytes = if let Some(frames) = wav_frames {
        let mut image = [0u8; 64];
        let mut events = Vec::with_capacity(model.writes.len());
        for w in &model.writes {
            let cycle = u64::from(w.tick)
                .checked_mul(config.cycles_per_tick)
                .ok_or("event cycle overflow")?;
            let action = match w.target {
                Target::Port(port) => Action::Write {
                    port,
                    value: w.value as u8,
                },
                Target::WaveWord(addr) => {
                    let offset = addr
                        .checked_sub(config.link.wave_ram)
                        .map(usize::from)
                        .filter(|&at| at + 1 < image.len())
                        .ok_or_else(|| {
                            format!(
                                "wave RAM write {addr:04x} outside 64-byte bank at {:04x}",
                                config.link.wave_ram
                            )
                        })?;
                    image[offset] = w.value as u8;
                    image[offset + 1] = (w.value >> 8) as u8;
                    Action::Waves(WaveBank::decode(&image)?)
                }
            };
            events.push(CycleEvent { cycle, action });
        }
        if !model.voice_triggers.is_empty() {
            let total_cycles = u64::from(ticks).saturating_mul(config.cycles_per_tick);
            append_voice_pcm_events(
                &rom,
                segment,
                config.link.voice_table,
                &model.voice_triggers,
                config.cycles_per_tick,
                total_cycles,
                &mut events,
            )?;
            events.sort_by_key(|e| e.cycle);
        }
        let pcm = render_cycles(&events, frames, reload_mode)?;
        let bytes = pcm.to_wav_bytes()?;
        Some((pcm.sample_rate(), pcm.channels(), pcm.frame_count(), bytes))
    } else {
        None
    };

    let events_bytes = if let Some(path) = &out_events {
        Some(encode_events(path, &model.writes, config.cycles_per_tick)?)
    } else {
        None
    };

    let active_slots_at_end = (0..8u16)
        .filter(|&slot| {
            let base = usize::from(
                config
                    .link
                    .slots
                    .wrapping_add(slot * config.link.slot_stride),
            );
            let pos = u16::from(model.ram[base + usize::from(field::POSITION)])
                | (u16::from(model.ram[base + usize::from(field::POSITION) + 1]) << 8);
            pos != 0xffff
        })
        .count();

    let mut metadata = json!({
        "schema": "swankit-sdk-sound/1",
        "song": song,
        "first_track": first_track,
        "tracks": tracks,
        "ticks": ticks,
        "completed_ticks": model.ticks,
        "complete": true,
        "cycles_per_tick": config.cycles_per_tick,
        "headphones": config.headphones,
        "reload_mode": reload_label,
        "port_writes": port_writes,
        "wave_writes": wave_writes,
        "voice_triggers": model.voice_triggers.len(),
        "total_writes": model.writes.len(),
        "active_slots_at_end": active_slots_at_end,
        "track_entries": track_meta.iter().map(|t| json!({
            "track": t.track,
            "slot": t.slot,
            "channel": t.channel,
            "flags": t.flags,
            "sequence_offset": t.sequence_offset,
        })).collect::<Vec<_>>(),
    });
    if let (Some(path), Some((sample_rate, channels, frames, ref bytes))) = (&out_wav, &wav_bytes) {
        metadata["wav"] = json!({
            "path": path,
            "sample_rate": sample_rate,
            "channels": channels,
            "frames": frames,
            "bytes": bytes.len(),
        });
    }
    if let (Some(path), Some(ref bytes)) = (&out_events, &events_bytes) {
        metadata["events"] = json!({
            "path": path,
            "bytes": bytes.len(),
            "count": model.writes.len(),
        });
    }
    let json_bytes = serde_json::to_vec_pretty(&metadata)?;

    // Build the write plan and verify zero collisions / pre-existing files before touching disk.
    let mut plan: Vec<(&str, &[u8])> = vec![(&out_json, &json_bytes)];
    if let (Some(path), Some((_, _, _, ref bytes))) = (&out_wav, &wav_bytes) {
        plan.push((path, bytes));
    }
    if let (Some(path), Some(ref bytes)) = (&out_events, &events_bytes) {
        plan.push((path, bytes));
    }
    for i in 0..plan.len() {
        for j in (i + 1)..plan.len() {
            if plan[i].0 == plan[j].0 {
                return Err(format!("duplicate output path: {}", plan[i].0).into());
            }
        }
        if !force && Path::new(plan[i].0).exists() {
            return Err(format!(
                "write {}: output already exists; use --force to replace an existing output",
                plan[i].0
            )
            .into());
        }
    }

    let mut written: Vec<&str> = Vec::with_capacity(plan.len());
    for (path, bytes) in plan {
        if let Err(err) = io::write(path, bytes, force) {
            if !force {
                for created in written {
                    let _ = std::fs::remove_file(created);
                }
            }
            return Err(err);
        }
        written.push(path);
    }

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&metadata)?);
    } else {
        println!(
            "Rendered song {song} ({tracks} tracks, {ticks} ticks, {} writes) -> {out_json}",
            model.writes.len()
        );
    }
    Ok(())
}

fn validate_tracks(
    segment: &[u8],
    link: &SdkLink,
    first_track: u16,
    count: u16,
) -> Result<Vec<TrackMetadata>> {
    let mut meta = Vec::with_capacity(usize::from(count));
    for offset in 0..count {
        let track = first_track
            .checked_add(offset)
            .ok_or("track index overflow")?;
        let entry_pos = u32::from(link.track_table)
            .checked_add(u32::from(track) * 6)
            .ok_or("track table offset overflow")?;
        let entry_end = entry_pos
            .checked_add(6)
            .ok_or("track table entry overflow")?;
        if entry_end as usize > segment.len() {
            return Err(format!(
                "song track {track} at segment offset 0x{entry_pos:04x} exceeds segment length {}",
                segment.len()
            )
            .into());
        }
        let pos = entry_pos as usize;
        let slot_offset = u16::from_le_bytes([segment[pos], segment[pos + 1]]);
        let flags_word = u16::from_le_bytes([segment[pos + 2], segment[pos + 3]]);
        let sequence_offset = u16::from_le_bytes([segment[pos + 4], segment[pos + 5]]);
        if !slot_offset.is_multiple_of(link.slot_stride) || slot_offset / link.slot_stride >= 8 {
            return Err(format!(
                "song track {track} has invalid slot offset {slot_offset} (stride {})",
                link.slot_stride
            )
            .into());
        }
        if usize::from(sequence_offset) + 4 > segment.len() {
            return Err(format!(
                "song track {track} sequence offset 0x{sequence_offset:04x} exceeds segment length {}",
                segment.len()
            )
            .into());
        }
        meta.push(TrackMetadata {
            track,
            slot: slot_offset / link.slot_stride,
            channel: (flags_word as u8) & 3,
            flags: flags_word as u8,
            sequence_offset,
        });
    }
    Ok(meta)
}

fn encode_events(
    path: &str,
    writes: &[swankit::sound::drivers::sdk_driver::Write],
    cycles_per_tick: u64,
) -> Result<Vec<u8>> {
    let is_json = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
    if is_json {
        let rows: Vec<Value> = writes
            .iter()
            .map(|w| {
                let cycle = u64::from(w.tick).saturating_mul(cycles_per_tick);
                match w.target {
                    Target::Port(port) => json!({
                        "tick": w.tick,
                        "cycle": cycle,
                        "kind": "port",
                        "port": port,
                        "value": w.value,
                    }),
                    Target::WaveWord(addr) => json!({
                        "tick": w.tick,
                        "cycle": cycle,
                        "kind": "wave",
                        "address": addr,
                        "value": w.value,
                    }),
                }
            })
            .collect();
        Ok(serde_json::to_vec_pretty(&rows)?)
    } else {
        let mut out = String::with_capacity(writes.len() * 24);
        for w in writes {
            let cycle = u64::from(w.tick).saturating_mul(cycles_per_tick);
            match w.target {
                Target::Port(port) => {
                    let _ = writeln!(
                        out,
                        "{}\t{cycle}\tport\t{port:02x}\t{:02x}",
                        w.tick, w.value
                    );
                }
                Target::WaveWord(addr) => {
                    let _ = writeln!(
                        out,
                        "{}\t{cycle}\twave\t{addr:04x}\t{:04x}",
                        w.tick, w.value
                    );
                }
            }
        }
        Ok(out.into_bytes())
    }
}

/// Synthesize HBlank-pumped Channel 2 unsigned PCM writes (port 0x89) from
/// banked voice descriptors into `events` for cycle-timed WAV rendering.
fn append_voice_pcm_events(
    rom: &[u8],
    segment: &[u8],
    voice_table: Option<u16>,
    triggers: &[VoiceTrigger],
    cycles_per_tick: u64,
    total_cycles: u64,
    events: &mut Vec<CycleEvent>,
) -> Result<()> {
    let geometry = match RomGeometry::new(rom.len()) {
        Ok(g) => g,
        Err(_) => return Ok(()),
    };
    for (idx, trig) in triggers.iter().enumerate() {
        let next_start = triggers
            .get(idx + 1)
            .map(|t| u64::from(t.tick).saturating_mul(cycles_per_tick))
            .unwrap_or(total_cycles)
            .min(total_cycles);
        let step = u64::from(trig.hblank_reload).max(1) * CYCLES_PER_SCANLINE;
        let mut cycle = u64::from(trig.tick).saturating_mul(cycles_per_tick);
        let mut ptr = trig.pointer;
        let mut bank = trig.bank;
        let mut len = trig.length;
        let mut next = trig.next;
        let mut chains = 0usize;
        while cycle < next_start && events.len() < swankit::sound::render::MAX_EVENTS {
            if len == 0 {
                if next == 0xffff || chains >= MAX_VOICE_CHAIN {
                    events.push(CycleEvent {
                        cycle,
                        action: Action::Write {
                            port: 0x94,
                            value: 0,
                        },
                    });
                    events.push(CycleEvent {
                        cycle,
                        action: Action::Write {
                            port: 0x89,
                            value: 0,
                        },
                    });
                    break;
                }
                let Some(table) = voice_table else { break };
                let entry = usize::from(table.wrapping_add(next.wrapping_mul(8)));
                if entry + 8 > segment.len() {
                    break;
                }
                ptr = u16::from_le_bytes([segment[entry], segment[entry + 1]]);
                bank = segment[entry + 2];
                len = u16::from_le_bytes([segment[entry + 4], segment[entry + 5]]);
                next = u16::from_le_bytes([segment[entry + 6], segment[entry + 7]]);
                chains += 1;
                continue;
            }
            let banks = BankState {
                c2: u16::from(bank),
                ..BankState::RESET
            };
            let phys = 0x20000u32 + u32::from(ptr);
            if let Some(off) = geometry.address_to_file_offset(phys, banks) {
                if let Some(&sample) = rom.get(off) {
                    events.push(CycleEvent {
                        cycle,
                        action: Action::Write {
                            port: 0x89,
                            value: sample,
                        },
                    });
                }
            }
            ptr = ptr.wrapping_add(1);
            len -= 1;
            cycle = cycle.saturating_add(step);
        }
    }
    Ok(())
}
