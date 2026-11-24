//! Synthetic tests for deterministic static SDK link-table discovery.
use ws_sound::drivers::{
    sdk_discover::{discover_sdk_link, FetchVariant},
    sdk_driver::{MuteTail, SdkGlobals, SdkLink},
};

/// Assemble a minimal synthetic 64 KiB ROM segment containing the standard SDK
/// x86 instruction patterns with caller-chosen linked addresses.
fn build_synthetic_sdk_rom(
    link: &SdkLink,
    data_segment: u16,
    short_jumps: bool,
    xor_taken: bool,
) -> Vec<u8> {
    let mut rom = vec![0x90u8; 0x10000];
    let mut p = 0x0100usize;
    let put = |buf: &mut [u8], pos: &mut usize, bytes: &[u8]| {
        buf[*pos..*pos + bytes.len()].copy_from_slice(bytes);
        *pos += bytes.len();
    };
    let put16 = |buf: &mut [u8], pos: &mut usize, val: u16| {
        buf[*pos..*pos + 2].copy_from_slice(&val.to_le_bytes());
        *pos += 2;
    };

    // 1. Play loop at 0x0100
    put(&mut rom, &mut p, &[0x51, 0xa3]);
    put16(&mut rom, &mut p, link.globals.play_index);
    put(&mut rom, &mut p, &[0xbf]);
    put16(&mut rom, &mut p, link.track_table);
    put(
        &mut rom,
        &mut p,
        &[0xd1, 0xe0, 0x8b, 0xc8, 0xd1, 0xe0, 0x03, 0xc1, 0x03, 0xf8],
    );

    // 2. Tick output-route head at 0x0200
    p = 0x0200;
    put(&mut rom, &mut p, &[0xe4, 0x91, 0x3a, 0x06]);
    put16(&mut rom, &mut p, link.globals.route_last);
    put(&mut rom, &mut p, &[0x74, 0x10, 0xa2]);
    put16(&mut rom, &mut p, link.globals.route_last);
    put(&mut rom, &mut p, &[0xa8, 0x80, 0x90, 0x90]);

    // Tick scan setup
    put(&mut rom, &mut p, &[0xa0]);
    put16(&mut rom, &mut p, link.globals.first_track);
    put(&mut rom, &mut p, &[0xa2]);
    put16(&mut rom, &mut p, link.globals.track_index);
    if xor_taken {
        put(&mut rom, &mut p, &[0x33, 0xc0, 0xa2]);
        put16(&mut rom, &mut p, link.globals.taken_mask);
    } else {
        put(&mut rom, &mut p, &[0xc6, 0x06]);
        put16(&mut rom, &mut p, link.globals.taken_mask);
        put(&mut rom, &mut p, &[0x00]);
    }
    put(&mut rom, &mut p, &[0xfe, 0x06]);
    put16(&mut rom, &mut p, link.globals.phase_counter);
    put(&mut rom, &mut p, &[0xbe]);
    put16(&mut rom, &mut p, link.slots);

    // Channel index + channel bits
    put(
        &mut rom,
        &mut p,
        &[0x8a, 0x4c, 0x02, 0x80, 0xe1, 0x03, 0x88, 0x0e],
    );
    put16(&mut rom, &mut p, link.globals.channel_index);
    put(
        &mut rom,
        &mut p,
        &[
            0xfe, 0xc1, 0xb0, 0x88, 0xd0, 0xc0, 0xfe, 0xc9, 0x75, 0xfa, 0xa2,
        ],
    );
    put16(&mut rom, &mut p, link.globals.channel_bits);

    // Vibrato table
    put(&mut rom, &mut p, &[0xbb]);
    put16(&mut rom, &mut p, link.vibrato_table);
    put(
        &mut rom,
        &mut p,
        &[
            0x03, 0xd8, 0x8b, 0x54, 0x10, 0x2e, 0x8a, 0x07, 0x98, 0x03, 0xc2, 0xba, 0x80, 0x00,
        ],
    );

    // Envelope groups + envelope volumes
    put(&mut rom, &mut p, &[0xbf]);
    put16(&mut rom, &mut p, link.envelope_groups);
    put(
        &mut rom,
        &mut p,
        &[
            0x83, 0xef, 0x10, 0x03, 0xf8, 0x8a, 0x64, 0x08, 0x2e, 0x8a, 0x05, 0x24, 0xf0, 0x0a,
            0xc4, 0xbf,
        ],
    );
    put16(&mut rom, &mut p, link.envelope_volumes);
    put(
        &mut rom,
        &mut p,
        &[0x32, 0xe4, 0x03, 0xf8, 0x2e, 0x8a, 0x05, 0x88, 0x44, 0x26],
    );

    // Mute check (rel8 = +2 -> right after jb)
    put(
        &mut rom,
        &mut p,
        &[0xe5, 0x84, 0x3d, 0x96, 0x00, 0x72, 0x02, 0x90, 0x90],
    );
    match link.mute_tail {
        MuteTail::SavedPointer => put(&mut rom, &mut p, &[0x5e]),
        MuteTail::LivePointer => put(&mut rom, &mut p, &[0xb8, 0x00, 0x00]),
    }

    // Slot stride
    put(
        &mut rom,
        &mut p,
        &[0x83, 0xc6, link.slot_stride as u8, 0xfe, 0x06],
    );
    put16(&mut rom, &mut p, link.globals.track_index);
    put(&mut rom, &mut p, &[0x80, 0x3e]);
    put16(&mut rom, &mut p, link.globals.track_index);
    put(&mut rom, &mut p, &[0x08]);

    // Control merge + deferred parameter apply
    put(&mut rom, &mut p, &[0xe4, 0x90, 0x8a, 0x26]);
    put16(&mut rom, &mut p, link.globals.control_shadow);
    put(
        &mut rom,
        &mut p,
        &[0x80, 0xe4, 0x0f, 0x0a, 0xc4, 0x80, 0x3e],
    );
    put16(&mut rom, &mut p, link.globals.voice_active);
    put(
        &mut rom,
        &mut p,
        &[0x00, 0x74, 0x02, 0x0c, 0x22, 0xe6, 0x90, 0xa0],
    );
    put16(&mut rom, &mut p, link.globals.expired_mask);
    put(&mut rom, &mut p, &[0x22, 0x06]);
    put16(&mut rom, &mut p, link.globals.deferred_mask);
    put(&mut rom, &mut p, &[0x74, 0x10, 0xbb]);
    put16(&mut rom, &mut p, link.deferred_target);
    put(&mut rom, &mut p, &[0x8a, 0x26]);
    put16(&mut rom, &mut p, link.globals.deferred_value);

    // 3. Fetch prefix at 0x0480..0x0580
    p = 0x0480;
    put(&mut rom, &mut p, &[0x32, 0xe4, 0xbb]);
    put16(&mut rom, &mut p, link.pitch_table);
    put(&mut rom, &mut p, &[0x03, 0xd8, 0x2e, 0x8b, 0x1f]);

    put(&mut rom, &mut p, &[0xbf]);
    put16(&mut rom, &mut p, link.wave_shadow);
    put(&mut rom, &mut p, &[0x8a, 0x0e]);
    put16(&mut rom, &mut p, link.globals.channel_index);
    put(
        &mut rom,
        &mut p,
        &[
            0x32, 0xed, 0x03, 0xf9, 0x8a, 0x44, 0x25, 0x3a, 0x05, 0x74, 0x10, 0x88, 0x05, 0x32,
            0xe4, 0x56, 0xbe,
        ],
    );
    put16(&mut rom, &mut p, link.wave_bank);
    put(&mut rom, &mut p, &[0x32, 0xe4, 0x03, 0xf0, 0xbf]);
    put16(&mut rom, &mut p, link.wave_ram);

    put(
        &mut rom,
        &mut p,
        &[0xe6, 0x8d, 0x8a, 0xc4, 0x25, 0x0f, 0x00, 0x48, 0xbe],
    );
    put16(&mut rom, &mut p, link.sweep_table);
    put(
        &mut rom,
        &mut p,
        &[0x03, 0xf0, 0x2e, 0x8a, 0x04, 0xe6, 0x8c],
    );

    // Voice clear / start far calls pointing to 0x0900 / 0x0920
    put(&mut rom, &mut p, &[0x9a]);
    put16(&mut rom, &mut p, 0x0900);
    put16(&mut rom, &mut p, data_segment);
    put(
        &mut rom,
        &mut p,
        &[0x8a, 0xc8, 0x32, 0xed, 0x8a, 0x44, 0x0c, 0x32, 0xe4, 0x9a],
    );
    put16(&mut rom, &mut p, 0x0920);
    put16(&mut rom, &mut p, data_segment);
    put(&mut rom, &mut p, &[0xc3]);

    // 4. Fetch entry at 0x0580
    p = 0x0580;
    put(
        &mut rom,
        &mut p,
        &[0x8b, 0x1c, 0xd1, 0xe3, 0x03, 0x5c, 0x03, 0xb8],
    );
    put16(&mut rom, &mut p, data_segment);
    put(
        &mut rom,
        &mut p,
        &[0x8e, 0xc0, 0xff, 0x04, 0x26, 0x8a, 0x07, 0x43],
    );
    // 8 bytes of dispatch + jump opcode at fetch+26
    put(
        &mut rom,
        &mut p,
        &[0x3c, 0xd0, 0x73, 0x05, 0x3c, 0xb0, 0x72, 0x03],
    );
    put(&mut rom, &mut p, &[if short_jumps { 0xeb } else { 0xe9 }]);

    // 5. clear_voice at 0x0900
    p = 0x0900;
    put(&mut rom, &mut p, &[0xc6, 0x06]);
    put16(&mut rom, &mut p, link.globals.voice_active);
    put(&mut rom, &mut p, &[0x00, 0xc7, 0x06]);
    put16(&mut rom, &mut p, link.voice_state);
    put(&mut rom, &mut p, &[0x00, 0x00, 0xcb]);

    // 6. start_voice at 0x0920
    if let Some(vt) = link.voice_table {
        p = 0x0920;
        put(&mut rom, &mut p, &[0xbb]);
        put16(&mut rom, &mut p, vt);
        put(&mut rom, &mut p, &[0xa1]);
        put16(&mut rom, &mut p, link.voice_state);
        put(
            &mut rom,
            &mut p,
            &[0xc1, 0xe0, 0x03, 0x03, 0xd8, 0x2e, 0x8b, 0x07, 0xcb],
        );
    }

    rom
}

fn sample_link(mute_tail: MuteTail, voice_table: Option<u16>) -> SdkLink {
    SdkLink {
        slots: 0x0400,
        slot_stride: 42,
        globals: SdkGlobals {
            taken_mask: 0x0550,
            control_shadow: 0x0551,
            channel_index: 0x0552,
            channel_bits: 0x0553,
            phase_counter: 0x0554,
            track_index: 0x0555,
            deferred_mask: 0x0558,
            deferred_value: 0x0559,
            expired_mask: 0x055a,
            first_track: 0x055b,
            route_last: 0x055d,
            voice_active: 0x055e,
            play_index: 0x0556,
        },
        wave_shadow: 0x0569,
        wave_ram: 0x0100,
        deferred_target: 0x0406,
        pitch_table: 0x0800,
        wave_bank: 0x0b00,
        envelope_groups: 0x0c00,
        envelope_volumes: 0x0830,
        vibrato_table: 0x0930,
        sweep_table: 0x0af1,
        track_table: 0x0d00,
        voice_state: 0x055f,
        voice_table,
        mute_tail,
    }
}

#[test]
fn discovers_near_jump_saved_pointer_sdk_link() {
    let expected = sample_link(MuteTail::SavedPointer, Some(0x5442));
    let rom = build_synthetic_sdk_rom(&expected, 0xf000, false, false);
    let disc = discover_sdk_link(&rom).unwrap();
    assert_eq!(disc.segment_file_offset, 0);
    assert_eq!(disc.data_segment, 0xf000);
    assert_eq!(disc.fetch_variant, FetchVariant::NearJumps);
    assert_eq!(disc.link, expected);
    assert_eq!(disc.clear_voice_offset, Some(0x0900));
    assert_eq!(disc.start_voice_offset, Some(0x0920));
}

#[test]
fn discovers_short_jump_live_pointer_xor_taken_sdk_link() {
    let expected = sample_link(MuteTail::LivePointer, None);
    let rom = build_synthetic_sdk_rom(&expected, 0xe000, true, true);
    let disc = discover_sdk_link(&rom).unwrap();
    assert_eq!(disc.fetch_variant, FetchVariant::ShortJumps);
    assert_eq!(disc.link, expected);
}

#[test]
fn rejects_non_sdk_and_empty_roms_without_panicking() {
    assert!(discover_sdk_link(&[]).is_err());
    assert!(discover_sdk_link(&[0u8; 64]).is_err());
    assert!(discover_sdk_link(&[0x90u8; 0x10000]).is_err());
}
