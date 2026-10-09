use formatkit_core::{Error, Reader};
use ws_rom::BankState;
use ws_sound::{
    drivers::{
        byte::{self, Dialect, Header, Instruction, Mode},
        cell, sdk, RomView,
    },
    hardware::{HyperVoice, Noise, Registers, NOISE_TAPS},
    instrument::{PitchEnvelope, VolumeEnvelope},
    render::{self, Action, Event, Synth},
    Wave, WaveBank,
};

#[test]
fn register_masks_readonly_state_and_voice_precedence() {
    let mut registers = Registers::default();
    registers.set_divisor(0, 0xffff).unwrap();
    assert_eq!(registers.divisor(0).unwrap(), 2047);
    assert_eq!(registers.period(0).unwrap(), 1);
    assert!(registers.divisor(4).is_err());
    assert!(registers.write(0x7f, 0).is_err());
    assert!(registers.write(0x92, 0).is_err());
    registers.set_headphones_connected(true);
    registers.write(0x91, 8).unwrap();
    assert_eq!(registers.read(0x91).unwrap(), 0x88);
    registers.write(0x89, 200).unwrap();
    for (control, expected) in [
        (1, (0, 200)),
        (2, (0, 100)),
        (4, (200, 0)),
        (8, (100, 0)),
        (15, (200, 200)),
    ] {
        registers.write(0x94, control).unwrap();
        assert_eq!(registers.voice_output(), expected);
    }
    let mut snapshot = [0; 21];
    snapshot[17] = 0x88;
    snapshot[18] = 0xff;
    snapshot[19] = 0xff;
    let parsed = Registers::decode(&snapshot).unwrap();
    assert_eq!(parsed.read(0x91).unwrap(), 0x88);
    assert_eq!(parsed.noise.state(), 0x7fff);
}

#[test]
fn all_noise_modes_match_explicit_bit_recurrence() {
    for (mode, tap) in NOISE_TAPS.into_iter().enumerate() {
        let mut noise = Noise::default();
        noise.write(0x18 | mode as u8);
        let mut expected = 0u16;
        for _ in 0..1000 {
            expected = ((expected << 1) | (1 ^ ((expected >> 7 ^ expected >> tap) & 1))) & 0x7fff;
            noise.clock(true);
            assert_eq!(noise.state(), expected);
        }
        noise.clock(false);
        assert_eq!(noise.state(), expected);
        noise.write(8 | mode as u8);
        assert_eq!(noise.state(), 0);
        noise.clock(true);
        assert_eq!(noise.state(), 0);
        assert_eq!(noise.control() & 8, 0);
    }
}

#[test]
fn wave_mode_also_clocks_noise() {
    let mut synth = Synth::default();
    synth.registers.set_divisor(3, 1920).unwrap();
    synth.registers.write(0x90, 8).unwrap();
    synth.registers.write(0x8e, 0x18).unwrap();
    synth.frame().unwrap();
    assert_eq!(synth.registers.noise.state(), 1);
}

#[test]
fn sweep_rate_signed_wrap_reset_and_enable() {
    let mut synth = Synth::default();
    synth.registers.set_divisor(2, 2047).unwrap();
    synth
        .apply(Action::Write {
            port: 0x8c,
            value: 1,
        })
        .unwrap();
    synth
        .apply(Action::Write {
            port: 0x90,
            value: 0x44,
        })
        .unwrap();
    for _ in 0..63 {
        synth.frame().unwrap();
    }
    assert_eq!(synth.registers.divisor(2).unwrap(), 2047);
    synth.frame().unwrap();
    assert_eq!(synth.registers.divisor(2).unwrap(), 0);
    synth
        .apply(Action::Write {
            port: 0x8c,
            value: 255,
        })
        .unwrap();
    synth
        .apply(Action::Write {
            port: 0x8d,
            value: 1,
        })
        .unwrap();
    for _ in 0..64 {
        synth.frame().unwrap();
    }
    assert_eq!(synth.registers.divisor(2).unwrap(), 0);
    synth
        .apply(Action::Write {
            port: 0x8d,
            value: 1,
        })
        .unwrap();
    for _ in 0..64 {
        synth.frame().unwrap();
    }
    assert_eq!(synth.registers.divisor(2).unwrap(), 0);
    for _ in 0..64 {
        synth.frame().unwrap();
    }
    assert_eq!(synth.registers.divisor(2).unwrap(), 2047);
    synth
        .apply(Action::Write {
            port: 0x90,
            value: 0x40,
        })
        .unwrap();
    for _ in 0..128 {
        synth.frame().unwrap();
    }
    assert_eq!(synth.registers.divisor(2).unwrap(), 2047);
}

#[test]
fn hyper_voice_conversion_routes_and_rates() {
    let mut hyper = HyperVoice::default();
    hyper.write_control(0x80 | 8); // signed, no shift
    assert_eq!(hyper.extend(128).unwrap(), -32768);
    assert_eq!(hyper.extend(127).unwrap(), 32512);
    hyper.input(1, true).unwrap();
    hyper.input(2, true).unwrap();
    assert_eq!((hyper.left, hyper.right), (256, 512));
    hyper.write_control(0x7080 | 8); // reset, mono both
    hyper.input(3, true).unwrap();
    assert_eq!((hyper.left, hyper.right), (768, 768));
    hyper.input(4, false).unwrap(); // manual always alternates
    assert_eq!((hyper.left, hyper.right), (1024, 768));
    hyper.write_control(0x80 | 5); // unsigned negated, shift 1
    assert_eq!(hyper.extend(0).unwrap(), -32768);
    assert_eq!(hyper.extend(255).unwrap(), -128);
    for (code, rate) in [24000, 12000, 8000, 6000, 4800, 4000, 3000, 2000]
        .into_iter()
        .enumerate()
    {
        hyper.write_control(0x80 | (code as u16) << 4);
        assert_eq!(hyper.rate(), rate);
    }
    hyper.write_control(0x8c);
    assert!(matches!(hyper.extend(0), Err(Error::Unsupported(_))));
}

fn write(frame: usize, port: u8, value: u8) -> Event {
    Event {
        frame,
        action: Action::Write { port, value },
    }
}

#[test]
fn bounded_event_sequence_stereo_and_voice_without_wave_enable() {
    let events = [
        write(0, 0x91, 8),
        write(0, 0x90, 0x20),
        write(0, 0x94, 5),
        write(0, 0x89, 128),
        write(2, 0x89, 255),
    ];
    let pcm = render::render(&events, 3).unwrap();
    assert_eq!(pcm.channels(), 2);
    assert_eq!(pcm.samples(), &[4096, 4096, 4096, 4096, 8160, 8160]);
    assert!(render::render(&events, 2).is_err());
    assert!(render::render(&[write(1, 0x90, 0), write(0, 0x90, 0)], 2).is_err());
    assert!(render::render(&[], render::MAX_FRAMES + 1).is_err());
    assert!(render::render(&[write(0, 0x7f, 0)], 1).is_err());
    assert_eq!(render::render(&[], 0).unwrap().frame_count(), 0);
    assert_eq!(
        render::unsigned_pcm(&[0, 128, 255], 8000, 3)
            .unwrap()
            .samples(),
        &[-32768, 0, 32512]
    );
    assert!(render::unsigned_pcm(&[0], 0, 1).is_err());
    assert!(render::unsigned_pcm(&[0], 8000, 0).is_err());
}

#[test]
fn three_header_dialects_preserve_bit_order_and_bank() {
    let a = Header::parse(&[0x81, 7, 0, 0xe1, 0x34, 0x12, 0x78, 0x56], Dialect::A, 0).unwrap();
    assert_eq!(
        (a.tracks[0].slot, a.tracks[1].slot, a.segment),
        (0, 7, 0xe100)
    );
    let b = Header::parse(&[0x81, 7, 0x34, 0x12, 0x78, 0x56], Dialect::B, 0xe100).unwrap();
    assert_eq!((b.tracks[0].slot, b.tracks[1].slot), (0, 7));
    assert_eq!(
        (
            b.tracks[0].channel(Dialect::B),
            b.tracks[1].channel(Dialect::B)
        ),
        (3, 0)
    );
    let c = Header::parse(&[1, 7, 0x20, 0, 0x30, 0x34, 0x12], Dialect::C, 0).unwrap();
    assert_eq!(
        (c.bank, c.segment, c.tracks[0].offset),
        (Some(0x20), 0x3000, 0x1234)
    );
    assert!(Header::parse(&[1, 7, 0, 0xe1, 0x34], Dialect::A, 0).is_err());
}

#[test]
fn varlen_and_pcm_skip_quirk_consumed_extent() {
    let mut reader = Reader::new(&[0xff, 0xff, 0x33]);
    assert_eq!(byte::varlen(&mut reader).unwrap(), 16383);
    assert_eq!(reader.pos(), 2);
    let bytes = [0x8f, 0, 0xe1, 0x20, 0, 4, 0, 1, 0, 2, 0xff];
    let mut reader = Reader::new(&bytes);
    assert!(matches!(
        byte::instruction(
            &mut reader,
            Dialect::A,
            Mode {
                channel: 1,
                ..Mode::default()
            }
        )
        .unwrap(),
        Instruction::Pcm(_)
    ));
    assert_eq!(reader.pos(), 10);
    let mut reader = Reader::new(&bytes);
    byte::instruction(&mut reader, Dialect::A, Mode::default()).unwrap();
    assert_eq!(reader.pos(), 9);
    assert_eq!(byte::pcm_se(&bytes).unwrap().rate, 12000);
    let mut reader = Reader::new(&[5, 2, 7]);
    assert_eq!(
        byte::instruction(
            &mut reader,
            Dialect::A,
            Mode {
                direct: true,
                ..Mode::default()
            }
        )
        .unwrap(),
        Instruction::Direct {
            wait: 5,
            opcode: 2,
            operands: vec![7]
        }
    );
    let mut reader = Reader::new(&[0x99, 0x34, 0x12, 0x81, 0x80, 5]);
    assert_eq!(
        byte::instruction(&mut reader, Dialect::A, Mode::default()).unwrap(),
        Instruction::Trigger {
            header: 0x1234,
            on: 128,
            off: 5
        }
    );
}

#[test]
fn byte_c_termination_noop_and_direct_divisors() {
    for opcode in [0x8a, 0x90, 0x91] {
        assert_eq!(
            byte::instruction(&mut Reader::new(&[opcode]), Dialect::C, Mode::default()).unwrap(),
            Instruction::End
        );
    }
    let mut reader = Reader::new(&[0x8f, 0xff]);
    assert_eq!(
        byte::instruction(&mut reader, Dialect::C, Mode::default()).unwrap(),
        Instruction::Command {
            opcode: 0x8f,
            operands: vec![]
        }
    );
    let bytes = [7, 0xff, 1, 2];
    assert_eq!(
        byte::instruction(
            &mut Reader::new(&bytes),
            Dialect::C,
            Mode {
                direct_divisor: true,
                ..Mode::default()
            }
        )
        .unwrap(),
        Instruction::Note {
            pitch: 2047,
            on: 1,
            off: 2
        }
    );
}

#[test]
fn song_cells_commands_aliases_and_bounds() {
    assert_eq!(
        cell::Cell::parse(&[1, 0x80, 4, 0]).unwrap(),
        cell::Cell::Note {
            duration: 1,
            key: 128,
            divisor: 4
        }
    );
    assert_eq!(
        cell::Cell::parse(&[10, 0x80, 4, 0]).unwrap(),
        cell::Cell::Command {
            opcode: 10,
            operand: 4
        }
    );
    assert_eq!(
        cell::Cell::parse(&[255, 255, 255, 255]).unwrap(),
        cell::Cell::End
    );
    assert!(cell::Cell::parse(&[12, 0x80, 0, 0]).is_err());
    assert!(cell::Song::parse(&[0; 12]).is_err());
    assert!(cell::groups(&[0, 0xa0, 5, 0, 0, 0xb0, 4, 0], 2).is_err());
    let mut rom = vec![0; 0x10000];
    rom[0x100..0x108].copy_from_slice(&[2, 1, 0, 7, 255, 255, 255, 255]);
    let view = RomView::new(&rom, BankState::RESET).unwrap();
    assert!(cell::track(view, 0xf000, 0x100, 10).unwrap().empty);
    rom[0x104..0x108].copy_from_slice(&[2, 1, 0, 7]);
    rom[0x108..0x10c].fill(255);
    let view = RomView::new(&rom, BankState::RESET).unwrap();
    assert_eq!(cell::track(view, 0xf000, 0x100, 10).unwrap().cells.len(), 3);
    assert!(cell::track(view, 0xf000, 0xfffd, 1).is_err());
    assert!(cell::track(view, 0xf000, 0x100, 1).is_err());
    assert!(view.span(0x1000, 0, 1).is_err());
    assert!(view.span(0xffff, 0xffff, usize::MAX).is_err());
}

#[test]
fn sdk_cells_and_restricted_sequence_render() {
    let wave = Wave::decode(&[0xff; 16]).unwrap();
    let pitch = [0x80, 0x07]; // period=1920, divisor=128
    assert_eq!(sdk::note_divisor(&pitch, 0, 0, false).unwrap(), 128);
    assert!(sdk::note_divisor(&pitch, 12, 0, false).is_err());
    let pcm = sdk::render_sequence(
        &[0xa0, 0x21, 0, 2, 0x0f, 1, 0xff, 0],
        &pitch,
        &[wave],
        0,
        3,
        9,
    )
    .unwrap();
    assert_eq!(
        pcm.samples(),
        &[960, 480, 960, 480, 960, 480, 960, 480, 960, 480, 960, 480, 0, 0, 0, 0, 0, 0]
    );
    assert!(sdk::render_sequence(&[0xa1, 1, 0xff, 0], &pitch, &[wave], 0, 3, 9).is_err());
    assert!(sdk::render_sequence(&[0, 255, 0xff, 0], &pitch, &[wave], 0, usize::MAX, 9).is_err());
    assert!(sdk::sequence(&[0, 1], 1).is_err());
    assert!(sdk::tracks(&[], usize::MAX).is_err());
}

#[test]
fn instrument_envelopes_are_bounded_and_keep_signed_deltas() {
    let env = PitchEnvelope::parse(&mut Reader::new(&[0x81, 0, 2, 255, 0xff]), 8).unwrap();
    assert_eq!(env.delay, 128);
    assert_eq!(env.steps, vec![(2, -1)]);
    let env = PitchEnvelope::parse(&mut Reader::new(&[0, 2, 255, 0xff]), 1).unwrap();
    assert_eq!(env.steps, vec![(2, -1)]);
    assert!(PitchEnvelope::parse(&mut Reader::new(&[0, 1, 1]), 1).is_err());
    let env = VolumeEnvelope::parse(&mut Reader::new(&[1, 7, 2, 4, 0]), 8).unwrap();
    assert_eq!(env.steps, vec![(7, 2), (4, 0)]);
    assert_eq!(
        VolumeEnvelope::parse(&mut Reader::new(&[1, 7, 0]), 1)
            .unwrap()
            .steps,
        vec![(7, 0)]
    );
    assert_eq!(
        VolumeEnvelope::parse(&mut Reader::new(&[0]), 0)
            .unwrap()
            .steps,
        vec![]
    );
    let table = VolumeEnvelope::parse(&mut Reader::new(&[6, 2, 0, 3, 0x10]), 2).unwrap();
    assert_eq!(table.table, vec![(6, 2), (0, 3)]);
    assert!(VolumeEnvelope::parse(&mut Reader::new(&[1, 7, 1]), 1).is_err());
    assert!(ws_sound::instrument::wave_bank(&[], usize::MAX).is_err());
    assert_eq!(
        ws_sound::instrument::wave_bank(&[0xff; 32], 2)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn direct_end_keeps_wait_and_headphone_sum_wraps_without_clamping() {
    let mut reader = Reader::new(&[7, 8]);
    assert_eq!(
        byte::instruction(
            &mut reader,
            Dialect::A,
            Mode {
                direct: true,
                ..Mode::default()
            }
        )
        .unwrap(),
        Instruction::DirectEnd { wait: 7 }
    );
    assert_eq!(reader.pos(), 2);
    let events = [
        Event {
            frame: 0,
            action: Action::Waves(WaveBank::decode(&[0xff; 64]).unwrap()),
        },
        write(0, 0x91, 8),
        write(0, 0x90, 15),
        write(0, 0x88, 255),
        write(0, 0x89, 255),
        write(0, 0x8a, 255),
        write(0, 0x8b, 255),
        Event {
            frame: 0,
            action: Action::HyperOutput {
                left: 10000,
                right: -30000,
            },
        },
    ];
    assert_eq!(
        render::render(&events, 1).unwrap().samples(),
        &[-26736, -1200]
    );
}

#[test]
fn arbitrary_short_inputs_return_errors_without_panicking() {
    for opcode in 0..=255 {
        for len in 0..14 {
            let bytes = vec![opcode; len];
            for dialect in [Dialect::A, Dialect::B, Dialect::C] {
                let _ = Header::parse(&bytes, dialect, 0xe100);
                let _ = byte::instruction(&mut Reader::new(&bytes), dialect, Mode::default());
                let _ = byte::instruction(
                    &mut Reader::new(&bytes),
                    dialect,
                    Mode {
                        direct: true,
                        ..Mode::default()
                    },
                );
            }
            let _ = sdk::Cell::parse(&bytes);
            let _ = cell::Cell::parse(&bytes);
            let _ = cell::Song::parse(&bytes);
            let _ = Registers::decode(&bytes);
            let _ = byte::ByteCPcm::parse(&bytes);
            let _ = WaveBank::decode(&bytes);
        }
    }
}
