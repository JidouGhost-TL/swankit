use serde_json::{json, Value};
use std::{
    fmt::Write as _,
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
use swankit::sound::{
    drivers::sdk_driver::{MuteTail, SdkDriver, SdkGlobals, SdkLink, Target},
    render::{render_cycles, Action, CounterReload, CycleEvent},
    WaveBank,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

const PITCH: u16 = 0x0100;
const WAVES: u16 = 0x0200;
const GROUPS: u16 = 0x0300;
const VOLUMES: u16 = 0x0400;
const VIBRATO: u16 = 0x0500;
const SWEEP: u16 = 0x0600;
const TRACKS: u16 = 0x0700;
const SEQ: u16 = 0x1000;
const CYCLES_PER_TICK: u64 = 40_704;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "swankit-sound-sdk-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self, n: &str) -> String {
        self.0.join(n).to_str().unwrap().to_owned()
    }
    fn put(&self, n: &str, b: &[u8]) -> String {
        let p = self.path(n);
        fs::write(&p, b).unwrap();
        p
    }
    fn json(&self, n: &str, v: Value) -> String {
        self.put(n, &serde_json::to_vec_pretty(&v).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn command(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args(args)
        .env_remove("FORMATKIT_CORPUS_DIR")
        .env_remove("FORMATKIT_CORPUS_REQUIRED")
        .env_remove("FORMATKIT_CORPUS_WRITE")
        .env_remove("FORMATKIT_CORPUS_RUNNER_LOCKED")
        .output()
        .unwrap()
}

fn sdk_link() -> SdkLink {
    SdkLink {
        slots: 0x400,
        slot_stride: 0x2a,
        globals: SdkGlobals {
            taken_mask: 0x550,
            control_shadow: 0x551,
            channel_index: 0x552,
            channel_bits: 0x553,
            phase_counter: 0x554,
            track_index: 0x555,
            deferred_mask: 0x558,
            deferred_value: 0x559,
            expired_mask: 0x55a,
            first_track: 0x55b,
            route_last: 0x55d,
            voice_active: 0x55e,
            play_index: 0x556,
        },
        wave_shadow: 0x569,
        wave_ram: 0xc0,
        deferred_target: 0x406,
        pitch_table: PITCH,
        wave_bank: WAVES,
        envelope_groups: GROUPS,
        envelope_volumes: VOLUMES,
        vibrato_table: VIBRATO,
        sweep_table: SWEEP,
        track_table: TRACKS,
        voice_state: 0x55f,
        voice_table: None,
        mute_tail: MuteTail::SavedPointer,
    }
}

fn synthetic_segment(channel: u8, sequence: &[u8], pitch0: u16) -> Vec<u8> {
    let mut data = vec![0u8; 0x2000];
    for class in 0..24u16 {
        let value = if class == 0 {
            pitch0
        } else {
            0x0400 - class * 8
        };
        data[usize::from(PITCH + class * 2)..][..2].copy_from_slice(&value.to_le_bytes());
    }
    for wave in 0..4usize {
        for i in 0..16 {
            data[usize::from(WAVES) + wave * 16 + i] = (wave as u8) * 0x11;
        }
    }
    for i in 0..256usize {
        data[usize::from(VOLUMES) + i] = i as u8;
    }
    let entry = usize::from(TRACKS);
    data[entry..entry + 6].copy_from_slice(&[
        0xa8,
        0,
        channel,
        0,
        (SEQ & 0xff) as u8,
        (SEQ >> 8) as u8,
    ]);
    data[usize::from(SEQ)..][..sequence.len()].copy_from_slice(sequence);
    data
}

#[test]
fn sound_sdk_help_is_available() {
    for args in [&["sound", "--help"][..], &["sound", "sdk-render", "--help"]] {
        let out = command(args);
        assert!(out.status.success(), "{args:?}: {:?}", out.stderr);
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.contains("sound sdk-render"));
        assert!(text.contains("--reload-mode"));
    }
}

#[test]
fn sound_sdk_render_matches_library_events_and_wav() {
    let fixture = Fixture::new();
    let seq = [0x00, 0x01, 0x0f, 0x00, 0x10, 0x03, 0x20, 0x02, 0xff, 0xff];
    let seg = synthetic_segment(0, &seq, 0x05c6);
    let mut rom = vec![0u8; 0x1000];
    rom.extend_from_slice(&seg);
    let rom_path = fixture.put("synth.rom", &rom);

    let link = sdk_link();
    let link_path = fixture.json(
        "link.json",
        json!({
            "segment_file_offset": 0x1000,
            "headphones": true,
            "tracks_per_song": 1,
            "link": link,
        }),
    );

    let ticks = 8u32;
    let out_json = fixture.path("meta.json");
    let out_wav_next = fixture.path("out-next.wav");
    let out_events_tsv = fixture.path("events.tsv");

    let run = command(&[
        "sound",
        "sdk-render",
        "--rom",
        &rom_path,
        "--link",
        &link_path,
        "--song",
        "0",
        "--ticks",
        "8",
        "--out-json",
        &out_json,
        "--out-wav",
        &out_wav_next,
        "--out-events",
        &out_events_tsv,
        "--reload-mode",
        "next-period",
        "--json",
    ]);
    assert!(
        run.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    // Reference library run.
    let mut model = SdkDriver::new(link, &seg, true).unwrap();
    model.reset(0).unwrap();
    model.start(0, 1).unwrap();
    for _ in 0..ticks {
        model.tick().unwrap();
    }

    let mut expected_tsv = String::new();
    let mut image = [0u8; 64];
    let mut cycle_events = Vec::new();
    for w in &model.writes {
        let cycle = u64::from(w.tick) * CYCLES_PER_TICK;
        match w.target {
            Target::Port(p) => {
                let _ = writeln!(
                    expected_tsv,
                    "{}\t{cycle}\tport\t{p:02x}\t{:02x}",
                    w.tick, w.value
                );
                cycle_events.push(CycleEvent {
                    cycle,
                    action: Action::Write {
                        port: p,
                        value: w.value as u8,
                    },
                });
            }
            Target::WaveWord(a) => {
                let _ = writeln!(
                    expected_tsv,
                    "{}\t{cycle}\twave\t{a:04x}\t{:04x}",
                    w.tick, w.value
                );
                let at = usize::from(a - link.wave_ram);
                image[at] = w.value as u8;
                image[at + 1] = (w.value >> 8) as u8;
                cycle_events.push(CycleEvent {
                    cycle,
                    action: Action::Waves(WaveBank::decode(&image).unwrap()),
                });
            }
        }
    }
    let frames = (u64::from(ticks) * CYCLES_PER_TICK / 128) as usize;
    let expected_wav_next = render_cycles(&cycle_events, frames, CounterReload::NextPeriod)
        .unwrap()
        .to_wav_bytes()
        .unwrap();

    // Verify byte-for-byte parity for events TSV and WAV.
    assert_eq!(fs::read_to_string(&out_events_tsv).unwrap(), expected_tsv);
    assert_eq!(fs::read(&out_wav_next).unwrap(), expected_wav_next);

    let meta: Value = serde_json::from_slice(&fs::read(&out_json).unwrap()).unwrap();
    assert_eq!(meta["schema"], "swankit-sdk-sound/1");
    assert_eq!(meta["complete"], true);
    assert_eq!(meta["completed_ticks"], ticks);
    assert_eq!(meta["reload_mode"], "next-period");
    assert_eq!(meta["total_writes"], model.writes.len());
    assert_eq!(meta["wav"]["frames"], frames);
    assert_eq!(meta["wav"]["sample_rate"], 24_000);
    assert_eq!(meta["wav"]["channels"], 2);

    // Also verify immediate reload mode and JSON events output.
    let out_json2 = fixture.path("meta2.json");
    let out_wav_imm = fixture.path("out-imm.wav");
    let out_events_json = fixture.path("events.json");
    let run2 = command(&[
        "sound",
        "sdk-render",
        "--rom",
        &rom_path,
        "--link",
        &link_path,
        "--song",
        "0",
        "--ticks",
        "8",
        "--out-json",
        &out_json2,
        "--out-wav",
        &out_wav_imm,
        "--out-events",
        &out_events_json,
        "--reload-mode",
        "immediate",
    ]);
    assert!(
        run2.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&run2.stderr)
    );
    let expected_wav_imm = render_cycles(&cycle_events, frames, CounterReload::Remainder)
        .unwrap()
        .to_wav_bytes()
        .unwrap();
    assert_eq!(fs::read(&out_wav_imm).unwrap(), expected_wav_imm);
    let events_val: Value = serde_json::from_slice(&fs::read(&out_events_json).unwrap()).unwrap();
    assert_eq!(events_val.as_array().unwrap().len(), model.writes.len());
}

#[test]
fn sound_sdk_refuses_bad_inputs_with_zero_partial_outputs() {
    let fixture = Fixture::new();
    let seg = synthetic_segment(0, &[0x00, 0x01, 0x0f, 0x00, 0x10, 0x03, 0xff, 0xff], 0x05c6);
    let rom_path = fixture.put("synth.rom", &seg);
    let link = sdk_link();
    let link_path = fixture.json(
        "link.json",
        json!({
            "segment_file_offset": 0,
            "tracks_per_song": 1,
            "song_count": 1,
            "link": link,
        }),
    );

    // 1. Missing ROM -> non-zero exit, zero outputs.
    let missing_rom = fixture.path("does-not-exist.rom");
    let j1 = fixture.path("fail1.json");
    let w1 = fixture.path("fail1.wav");
    let e1 = fixture.path("fail1.tsv");
    let r = command(&[
        "sound",
        "sdk-render",
        "--rom",
        &missing_rom,
        "--link",
        &link_path,
        "--song",
        "0",
        "--ticks",
        "4",
        "--out-json",
        &j1,
        "--out-wav",
        &w1,
        "--out-events",
        &e1,
    ]);
    assert!(!r.status.success());
    assert!(!PathBuf::from(&j1).exists());
    assert!(!PathBuf::from(&w1).exists());
    assert!(!PathBuf::from(&e1).exists());

    // 2. Missing link fields -> non-zero exit, zero outputs.
    let bad_link = fixture.json(
        "bad-link.json",
        json!({"segment_file_offset": 0, "link": {"slots": 1024}}),
    );
    let j2 = fixture.path("fail2.json");
    let r = command(&[
        "sound",
        "sdk-render",
        "--rom",
        &rom_path,
        "--link",
        &bad_link,
        "--song",
        "0",
        "--ticks",
        "4",
        "--out-json",
        &j2,
    ]);
    assert!(!r.status.success());
    assert!(!PathBuf::from(&j2).exists());

    // 3. Out-of-range song index -> non-zero exit, zero outputs.
    let j3 = fixture.path("fail3.json");
    let r = command(&[
        "sound",
        "sdk-render",
        "--rom",
        &rom_path,
        "--link",
        &link_path,
        "--song",
        "1",
        "--ticks",
        "4",
        "--out-json",
        &j3,
    ]);
    assert!(!r.status.success());
    assert!(!PathBuf::from(&j3).exists());

    // 4. Truncated/incomplete run (voice note on channel 1 returns Unsupported) -> zero outputs.
    let voice_seg = synthetic_segment(1, &[0x00, 0x02, 0x0f, 0x01, 0x10, 0x04, 0xff, 0x00], 0x0400);
    let voice_rom = fixture.put("voice.rom", &voice_seg);
    let j4 = fixture.path("fail4.json");
    let w4 = fixture.path("fail4.wav");
    let e4 = fixture.path("fail4.tsv");
    let r = command(&[
        "sound",
        "sdk-render",
        "--rom",
        &voice_rom,
        "--link",
        &link_path,
        "--song",
        "0",
        "--ticks",
        "4",
        "--out-json",
        &j4,
        "--out-wav",
        &w4,
        "--out-events",
        &e4,
    ]);
    assert!(!r.status.success());
    assert!(!PathBuf::from(&j4).exists());
    assert!(!PathBuf::from(&w4).exists());
    assert!(!PathBuf::from(&e4).exists());

    // 5. Existing secondary output without --force -> fails without creating --out-json.
    let j5 = fixture.path("fail5.json");
    let w5 = fixture.put("existing.wav", b"keep");
    let r = command(&[
        "sound",
        "sdk-render",
        "--rom",
        &rom_path,
        "--link",
        &link_path,
        "--song",
        "0",
        "--ticks",
        "4",
        "--out-json",
        &j5,
        "--out-wav",
        &w5,
    ]);
    assert!(!r.status.success());
    assert!(!PathBuf::from(&j5).exists());
    assert_eq!(fs::read(&w5).unwrap(), b"keep");
}

#[test]
fn sound_sdk_voice_note_renders_wav_with_voice_table() {
    const VOICE_TABLE: u16 = 0x0800;
    let fixture = Fixture::new();
    // Build a 64 KiB ROM so RomGeometry accepts it and bank 0xff maps to 0x0000..0xffff.
    let mut rom = vec![0u8; 0x10000];
    let seg = synthetic_segment(1, &[0x00, 0x00, 0x0f, 0x01, 0x01, 0x00, 0xff, 0xff], 0x0400);
    rom[..seg.len()].copy_from_slice(&seg);
    // Voice descriptor #0 at VOICE_TABLE: ptr=0x2000, bank=0x00ff, len=32, next=0xffff.
    rom[usize::from(VOICE_TABLE)..usize::from(VOICE_TABLE) + 8]
        .copy_from_slice(&[0x00, 0x20, 0xff, 0x00, 0x20, 0x00, 0xff, 0xff]);
    // Non-constant unsigned PCM bytes at file offset 0x2000.
    for i in 0..32usize {
        rom[0x2000 + i] = if i % 2 == 0 { 0xf0 } else { 0x10 };
    }
    let rom_path = fixture.put("voice64k.rom", &rom);
    let mut link = sdk_link();
    link.voice_table = Some(VOICE_TABLE);
    let link_path = fixture.json(
        "voice-link.json",
        json!({
            "segment_file_offset": 0,
            "tracks_per_song": 1,
            "song_count": 1,
            "link": link,
        }),
    );
    let out_json = fixture.path("voice-meta.json");
    let out_wav = fixture.path("voice.wav");
    let r = command(&[
        "sound",
        "sdk-render",
        "--rom",
        &rom_path,
        "--link",
        &link_path,
        "--song",
        "0",
        "--ticks",
        "2",
        "--out-json",
        &out_json,
        "--out-wav",
        &out_wav,
    ]);
    assert!(
        r.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    let meta: Value = serde_json::from_slice(&fs::read(&out_json).unwrap()).unwrap();
    assert_eq!(meta["voice_triggers"], 1);
    assert!(fs::read(&out_wav).unwrap().len() > 44);

    // Reject missing both --link and --auto-link, or specifying both.
    let bad = command(&[
        "sound",
        "sdk-render",
        "--rom",
        &rom_path,
        "--song",
        "0",
        "--ticks",
        "2",
        "--out-json",
        &fixture.path("bad.json"),
    ]);
    assert!(!bad.status.success());

    // sdk-discover and --auto-link reject non-SDK ROM cleanly (and --auto-link takes no value).
    let bad_disc = command(&["sound", "sdk-discover", "--rom", &rom_path]);
    assert!(!bad_disc.status.success());
    let bad_auto = command(&[
        "sound",
        "sdk-render",
        "--rom",
        &rom_path,
        "--auto-link",
        "--song",
        "0",
        "--ticks",
        "2",
        "--out-json",
        &fixture.path("bad-auto.json"),
    ]);
    assert!(!bad_auto.status.success());
    assert!(
        !String::from_utf8_lossy(&bad_auto.stderr).contains("missing value for --auto-link"),
        "stderr: {}",
        String::from_utf8_lossy(&bad_auto.stderr)
    );
}
