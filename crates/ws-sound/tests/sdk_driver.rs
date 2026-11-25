//! Synthetic records for the tick-level SDK driver model. Expected writes are
//! derived by hand from the cell grammar and the WSdev Sound register rules.
use ws_sound::drivers::sdk_driver::{
    MuteTail, OracleVerdict, SdkDriver, SdkGlobals, SdkLink, Target, Write,
};

const PITCH: u16 = 0x0100;
const WAVES: u16 = 0x0200;
const GROUPS: u16 = 0x0300;
const VOLUMES: u16 = 0x0400;
const VIBRATO: u16 = 0x0500;
const SWEEP: u16 = 0x0600;
const TRACKS: u16 = 0x0700;
const SEQ: u16 = 0x1000;

fn link() -> SdkLink {
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
        envelope_group_table: None,
        extended_opcodes: false,
        branch_selector: None,
        inline_sweep: false,
        merge_voice_active: true,
        mute_tail: MuteTail::SavedPointer,
    }
}

/// A segment with one track-table entry per call: slot 4 + channel.
fn segment(channel: u8, sequence: &[u8], pitch0: u16) -> Vec<u8> {
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

fn ports(writes: &[Write], tick: u32) -> Vec<(u8, u16)> {
    writes
        .iter()
        .filter(|w| w.tick == tick)
        .filter_map(|w| match w.target {
            Target::Port(p) => Some((p, w.value)),
            Target::WaveWord(_) => None,
        })
        .collect()
}

fn started(data: &[u8]) -> SdkDriver<'_> {
    let mut model = SdkDriver::new(link(), data, true).unwrap();
    model.reset(0).unwrap();
    model.writes.clear();
    model.start(0, 1).unwrap();
    model
}

#[test]
fn note_on_hold_and_end() {
    // Header: tempo 0, wave 1, volume 0F, mode 0; note class 0 octave 1 for
    // three ticks; end.
    let data = segment(0, &[0x00, 0x01, 0x0f, 0x00, 0x10, 0x03, 0xff, 0xff], 0x05c6);
    let mut model = started(&data);
    for _ in 0..4 {
        model.tick().unwrap();
    }
    let divisor = 0x800 - (0x05c6 >> 1);
    // Tick 0: headphone route (bit 7 set: OR 8, clear bit 0), wave copy,
    // divisor, then channel-0 enable merged into 90h (reset left 40h).
    assert_eq!(
        ports(&model.writes, 0),
        vec![
            (0x91, 0x8e),
            (0x80, divisor & 0xff),
            (0x81, divisor >> 8),
            (0x90, 0x41)
        ]
    );
    let waves: Vec<_> = model
        .writes
        .iter()
        .filter(|w| matches!(w.target, Target::WaveWord(_)))
        .collect();
    assert_eq!(waves.len(), 8);
    assert_eq!(waves[0].target, Target::WaveWord(0xc0));
    assert_eq!(waves[0].value, 0x1111);
    // Ticks 1 and 2: volume from the swapped volume byte (F0 -> L=F, R=F);
    // tick 1 also repeats the route once (its readback changed).
    assert_eq!(
        ports(&model.writes, 1),
        vec![(0x91, 0x8e), (0x88, 0xff), (0x90, 0x41)]
    );
    assert_eq!(ports(&model.writes, 2), vec![(0x88, 0xff), (0x90, 0x41)]);
    // Tick 3: the volume is written before the duration expires; the end
    // cell then clears the channel's enable bit.
    assert_eq!(
        ports(&model.writes, 3),
        vec![(0x88, 0xff), (0x90, 0x40), (0x90, 0x40)]
    );
    assert_eq!(model.ram[0x4a8..0x4aa], [0xff, 0xff]);
}

#[test]
fn route_is_rewritten_until_readback_settles() {
    let data = segment(0, &[0x00, 0x00, 0x0f, 0x00, 0x0c, 0x10, 0xff, 0xff], 0x0400);
    let mut model = started(&data);
    model.tick().unwrap();
    model.tick().unwrap();
    model.tick().unwrap();
    let routes: Vec<_> = model
        .writes
        .iter()
        .filter(|w| w.target == Target::Port(0x91))
        .map(|w| (w.tick, w.value))
        .collect();
    // Reset wrote 06; reads 86 -> write 8E; reads 8E (new) -> write 8E; stable.
    assert_eq!(routes, vec![(0, 0x8e), (1, 0x8e)]);
}

#[test]
fn marker_jump_and_counted_loop() {
    // FD 00 marks position 3; A plays; B2 00 loops back twice (count 2+1,
    // decremented before testing); then B plays; end.
    let data = segment(
        0,
        &[
            0x00, 0x00, 0x0f, 0x00, 0xfd, 0x00, 0x10, 0x01, 0xb2, 0x00, 0x20, 0x01, 0xff, 0x00,
        ],
        0x0400,
    );
    let mut model = started(&data);
    for _ in 0..6 {
        model.tick().unwrap();
    }
    let a = 0x800 - (0x0400 >> 1);
    let b = 0x800 - (0x0400 >> 2);
    let notes: Vec<u16> = model
        .writes
        .iter()
        .filter(|w| w.target == Target::Port(0x80))
        .map(|w| w.value)
        .collect();
    assert_eq!(notes, vec![a & 0xff, a & 0xff, a & 0xff, b & 0xff]);
}

#[test]
fn tempo_accumulator_skips_ticks() {
    // Header tempo 8: every second tick skips duration processing.
    let data = segment(
        0,
        &[0x08, 0x00, 0x0f, 0x00, 0x10, 0x02, 0x20, 0x01, 0xff, 0x00],
        0x0400,
    );
    let mut model = started(&data);
    for _ in 0..5 {
        model.tick().unwrap();
    }
    let ticks: Vec<u32> = model
        .writes
        .iter()
        .filter(|w| w.target == Target::Port(0x80))
        .map(|w| w.tick)
        .collect();
    // Note at tick 0; accumulator 8 (process), 16 (skip), 8, 16: two processed
    // ticks expire the duration at tick 3.
    assert_eq!(ticks, vec![0, 3]);
}

#[test]
fn volume_slide_steps_high_nibble() {
    // D2 01: budget +2, period 1; volume 00 rises to 10 and 20.
    let data = segment(
        0,
        &[0x00, 0x00, 0x00, 0x00, 0xd2, 0x01, 0x10, 0x08, 0xff, 0x00],
        0x0400,
    );
    let mut model = started(&data);
    for _ in 0..5 {
        model.tick().unwrap();
    }
    let volumes: Vec<u16> = model
        .writes
        .iter()
        .filter(|w| w.target == Target::Port(0x88))
        .map(|w| w.value)
        .collect();
    assert_eq!(volumes, vec![0x00, 0x11, 0x22, 0x22]);
}

#[test]
fn channel_three_low_divisor_is_muted() {
    // Channel index 2, pitch word 07F0 gives divisor 0010 (< 0096h).
    let data = segment(2, &[0x00, 0x00, 0x0f, 0x00, 0x00, 0x04, 0xff, 0x00], 0x07f0);
    let mut model = started(&data);
    model.tick().unwrap();
    let tick0 = ports(&model.writes, 0);
    let at = tick0.iter().position(|&w| w == (0x84, 0x10)).unwrap();
    assert_eq!(tick0[at + 1], (0x85, 0x00));
    // Sweep operand 0 writes 8Ch = 0, then the mute tail clears 84h/85h.
    assert_eq!(
        &tick0[at + 2..at + 5],
        &[(0x8c, 0x00), (0x84, 0x00), (0x85, 0x00)]
    );
    assert_eq!(model.ram[0x4a8 + 0x10..0x4a8 + 0x12], [0, 0]);
}

#[test]
fn first_track_four_scans_low_slots_only() {
    let data = segment(0, &[0x00, 0x00, 0x0f, 0x00, 0x10, 0x10, 0xff, 0x00], 0x0400);
    let mut model = started(&data);
    model.set_first_track(4, Some(0));
    model.tick().unwrap();
    // The track counter starts at 4 but the slot pointer at slot 0; slot 4
    // (the started music track) is never reached.
    assert!(ports(&model.writes, 0)
        .iter()
        .all(|&(p, _)| p == 0x90 || p == 0x91));
    assert_eq!(model.ram[0x4a8..0x4aa], [0, 0]);
}

#[test]
fn voice_notes_are_reported_unsupported() {
    let data = segment(1, &[0x00, 0x02, 0x0f, 0x01, 0x10, 0x04, 0xff, 0x00], 0x0400);
    let mut model = started(&data);
    assert!(model.tick().is_err());
}

#[test]
fn start_rejects_out_of_range_counts() {
    let data = segment(0, &[0xff, 0xff], 0x0400);
    let mut model = SdkDriver::new(link(), &data, true).unwrap();
    assert!(model.start(0, 0).is_err());
    assert!(model.start(0, 9).is_err());
}

/// Replays `ticks` ticks and compares the writes with `oracle`, the way the
/// oracle diff does: a model error stops the replay.
fn replay(data: &[u8], ticks: u32, oracle: &[Write]) -> OracleVerdict {
    let mut model = started(data);
    let mut error = None;
    for _ in 0..ticks {
        if let Err(e) = model.tick() {
            error = Some(e.to_string());
            break;
        }
    }
    let compared: Vec<Write> = oracle
        .iter()
        .copied()
        .filter(|w| w.tick < model.ticks)
        .collect();
    OracleVerdict {
        oracle_ticks: ticks,
        model_ticks: model.ticks,
        model_error: error,
        ports_equal: compared
            == model
                .writes
                .iter()
                .copied()
                .filter(|w| w.tick < model.ticks)
                .collect::<Vec<_>>(),
        waves_equal: true,
        ram_equal: true,
    }
}

#[test]
fn complete_replay_passes() {
    let data = segment(0, &[0x00, 0x00, 0x0f, 0x00, 0x10, 0x03, 0xff, 0xff], 0x0400);
    let mut reference = started(&data);
    for _ in 0..4 {
        reference.tick().unwrap();
    }
    let verdict = replay(&data, 4, &reference.writes);
    assert!(verdict.complete() && verdict.passed());
}

#[test]
fn truncated_replay_fails_even_when_its_prefix_matches() {
    // Channel index 1 in voice mode: the first note is an unmodelled voice
    // note, so the model stops at tick 0. The empty prefix "matches" an empty
    // oracle prefix, which must not count as a pass.
    let data = segment(1, &[0x00, 0x02, 0x0f, 0x01, 0x10, 0x04, 0xff, 0x00], 0x0400);
    let verdict = replay(&data, 100, &[]);
    assert!(verdict.ports_equal);
    assert!(verdict.model_error.is_some());
    assert_eq!(verdict.model_ticks, 0);
    assert!(!verdict.complete());
    assert!(!verdict.port_exact() && !verdict.wave_exact() && !verdict.ram_exact());
    assert!(!verdict.passed());
    // A run that stops early without an error is also incomplete.
    let short = OracleVerdict {
        oracle_ticks: 10,
        model_ticks: 9,
        model_error: None,
        ports_equal: true,
        waves_equal: true,
        ram_equal: true,
    };
    assert!(!short.passed());
}

#[test]
fn voice_note_loads_descriptor_and_enables_voice_mode() {
    const VOICE_TABLE: u16 = 0x0800;
    let mut data = segment(1, &[0x00, 0x02, 0x0f, 0x01, 0x01, 0x00, 0xff, 0xff], 0x0400);
    // Descriptor #2 at VOICE_TABLE + 16: ptr=0x1234, bank=0x00ef, len=0x0400, next=0xffff.
    let at = usize::from(VOICE_TABLE + 2 * 8);
    data[at..at + 8].copy_from_slice(&[0x34, 0x12, 0xef, 0x00, 0x00, 0x04, 0xff, 0xff]);
    let mut lnk = link();
    lnk.voice_table = Some(VOICE_TABLE);
    let mut model = SdkDriver::new(lnk, &data, true).unwrap();
    model.reset(0).unwrap();
    model.writes.clear();
    model.start(0, 1).unwrap();
    model.tick().unwrap();
    let tick0 = ports(&model.writes, 0);
    // start_voice writes 89h=00, 90h|=22 (40|22=62), 94h=0f; merge_control writes 90h=62.
    assert!(tick0.contains(&(0x89, 0x00)));
    assert!(tick0.contains(&(0x90, 0x62)));
    assert!(tick0.contains(&(0x94, 0x0f)));
    assert_eq!(model.ram[usize::from(lnk.globals.voice_active)], 1);
    assert_eq!(u16::from_le_bytes([model.ram[0x55f], model.ram[0x560]]), 2);
    assert_eq!(
        u16::from_le_bytes([model.ram[0x561], model.ram[0x562]]),
        0x1234
    );
    assert_eq!(
        u16::from_le_bytes([model.ram[0x563], model.ram[0x564]]),
        0x0400
    );
    assert_eq!(
        u16::from_le_bytes([model.ram[0x565], model.ram[0x566]]),
        0xffff
    );
    assert_eq!(model.ram[0x567], 0xef);
    assert_eq!(model.voice_triggers.len(), 1);
    assert_eq!(model.voice_triggers[0].sample_id, 2);
    assert_eq!(model.voice_triggers[0].hblank_reload, 1);
}

#[test]
fn sweep_readback_triggers_channel_three_mute_below_96h() {
    // Channel 2 (index 2), pitch word 0x0760 -> divisor 0x00a0 (>= 0x0096).
    // Sweep operand 0x01: sweep_ticks = 0 (every 1*8192 cycles), sweep_table[0] = -16 (0xf0).
    let mut data = segment(2, &[0x00, 0x00, 0x0f, 0x01, 0x00, 0x04, 0xff, 0x00], 0x0760);
    data[usize::from(SWEEP)] = 0xf0; // -16 signed
    let mut model = started(&data);
    // Tick 0: sets divisor 0x00a0, 8Dh=0, 8Ch=0xf0, enables ch3 (90h=0x44). Not muted yet.
    model.tick().unwrap();
    assert_eq!(model.ram[0x4a8 + 0x10..0x4a8 + 0x12], [0xa0, 0x00]);
    // Advance 8192 cycles -> one sweep step: 0x00a0 - 16 = 0x0090 (< 0x0096).
    model.advance_hardware(8192);
    model.tick().unwrap();
    let tick1 = ports(&model.writes, 1);
    assert!(tick1.contains(&(0x84, 0x00)));
    assert!(tick1.contains(&(0x85, 0x00)));
    assert_eq!(model.ram[0x4a8 + 0x10..0x4a8 + 0x12], [0x00, 0x00]);
}

#[test]
fn inline_sweep_and_no_mute_tail_behave_as_reconstructed() {
    // Channel 2 with sweep byte 0x2b (period=2, sign=1, mag=3 -> -3 = 0xfd), pitch 0x0760 -> divisor 0x00a0.
    let data = segment(2, &[0x00, 0x00, 0x0f, 0x2b, 0x00, 0x04, 0xff, 0x00], 0x0760);
    let mut lnk = link();
    lnk.inline_sweep = true;
    lnk.sweep_table = 0;
    lnk.mute_tail = MuteTail::None;
    let mut model = SdkDriver::new(lnk, &data, true).unwrap();
    model.reset(0).unwrap();
    model.writes.clear();
    model.start(0, 1).unwrap();
    model.tick().unwrap();
    let tick0 = ports(&model.writes, 0);
    assert!(tick0.contains(&(0x8d, 0x02)));
    assert!(tick0.contains(&(0x8c, 0xfd)));
    // Even if divisor drops below 0x96, MuteTail::None does not zero it.
    model.ports.write(0x84, 0x50).unwrap();
    model.ports.write(0x85, 0x00).unwrap();
    model.tick().unwrap();
    assert_eq!(model.ram[0x4a8 + 0x10..0x4a8 + 0x12], [0xa0, 0x00]);
}

#[test]
fn extended_indirect_envelope_and_ch3_noise_on_note() {
    // Channel 3 with mode=1, envelope bank switched via A8 01, envelope group 1 (C1 04), note 00 04.
    let mut data = segment(
        3,
        &[
            0x00, 0x00, 0x0f, 0x01, // header: mode=1 (noise)
            0xa8, 0x01, // select envelope bank 1
            0xc1, 0x04, // group 1, period 4
            0x00, 0x04, // note
            0xff, 0x00,
        ],
        0x0400,
    );
    // Envelope group table at 0x0380: bank 0 -> 0x0300, bank 1 -> 0x0320.
    data[0x0380..0x0382].copy_from_slice(&0x0300u16.to_le_bytes());
    data[0x0382..0x0384].copy_from_slice(&0x0320u16.to_le_bytes());
    data[0x0320..0x0330].fill(0x50); // bank 1 group 1 step = 0x50 -> combined 0x5f

    let mut lnk = link();
    lnk.slot_stride = 0x2c;
    lnk.envelope_group_table = Some(0x0380);
    lnk.extended_opcodes = true;
    // Track table slot offset for slot 4 with stride 0x2c = 0xb0.
    data[usize::from(TRACKS)..usize::from(TRACKS) + 2].copy_from_slice(&0x00b0u16.to_le_bytes());

    let mut model = SdkDriver::new(lnk, &data, true).unwrap();
    model.reset(0).unwrap();
    model.writes.clear();
    model.start(0, 1).unwrap();
    model.tick().unwrap();
    let tick0 = ports(&model.writes, 0);
    // In extended mode, ch3 noise bit (0x80) and noise port (0x8e) are written on note, after divisor.
    let pos_div = tick0.iter().position(|&(p, _)| p == 0x86).unwrap();
    let pos_8e = tick0.iter().position(|&(p, _)| p == 0x8e).unwrap();
    let pos_90_noise = tick0
        .iter()
        .position(|&(p, v)| p == 0x90 && v & 0x80 != 0)
        .unwrap();
    assert!(pos_div < pos_8e && pos_8e < pos_90_noise);
    // Tick 1 reads indirect bank-1 envelope (0x50 | 0x0f = 0x5f -> mirror=0x5f -> vol port 8Bh = 0x5f).
    model.tick().unwrap();
    let tick1 = ports(&model.writes, 1);
    assert!(tick1.contains(&(0x8b, 0x5f)));
}
