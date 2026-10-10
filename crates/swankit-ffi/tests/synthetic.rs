use swankit_ffi::{decode, swankit_decode_v1, HEADER_BYTES, MAX_INPUT, MAX_OUTPUT, MAX_TILES};

#[test]
fn native_errors_do_not_write_output() {
    let input = [0; 16];
    let mut output = [0x55; 256];
    // SAFETY: these arrays remain live and valid; nulls are explicitly rejected before dereference.
    unsafe {
        assert_eq!(
            swankit_decode_v1(1, std::ptr::null(), 0, output.as_mut_ptr(), 256),
            -1
        );
        assert_eq!(
            swankit_decode_v1(
                1,
                input.as_ptr(),
                MAX_INPUT as u32 + 1,
                output.as_mut_ptr(),
                256
            ),
            -1
        );
        assert_eq!(
            swankit_decode_v1(
                1,
                input.as_ptr(),
                16,
                output.as_mut_ptr(),
                MAX_OUTPUT as u32 + 1
            ),
            -1
        );
        assert_eq!(
            swankit_decode_v1(1, input.as_ptr(), 16, output.as_mut_ptr(), 256),
            -2
        );
        let mut rom = vec![0; ws_rom::MIN_ROM_SIZE];
        let footer = rom.len() - 16;
        rom[footer] = 0xea;
        assert_eq!(
            swankit_decode_v1(1, rom.as_ptr(), rom.len() as u32, output.as_mut_ptr(), 1),
            -3
        );
        assert!(output.iter().all(|&b| b == 0x55));
        assert_eq!(
            swankit_decode_v1(1, rom.as_ptr(), rom.len() as u32, output.as_mut_ptr(), 256),
            HEADER_BYTES as i32
        );
        assert_eq!(&output[..HEADER_BYTES], decode(1, &rom).unwrap());
    }
}

#[test]
fn tile_extents_and_palette_are_checked() {
    for op in [2, 3] {
        for n in [0, 16, 511, 512, 513, 543, 545, 512 + (MAX_TILES + 1) * 32] {
            assert!(decode(op, &vec![0; n]).is_err());
        }
        let mut bytes = vec![0; 544];
        bytes[..2].copy_from_slice(&0x0f84u16.to_le_bytes());
        let output = decode(op, &bytes).unwrap();
        assert_eq!(output.len(), 256);
        assert!(output.chunks_exact(4).all(|p| p == [255, 136, 68, 255]));
    }
    assert!(decode(0, &[]).is_err());
}

#[test]
fn server_recovers_and_bounds_frames() {
    let mut output = Vec::new();
    swankit_ffi::protocol::serve(std::io::Cursor::new(b"bad\n{}\n"), &mut output, false).unwrap();
    assert_eq!(
        output
            .split(|&b| b == b'\n')
            .filter(|line| !line.is_empty())
            .count(),
        2
    );
    let large = vec![b'x'; swankit_ffi::protocol::MAX_LINE + 10];
    output.clear();
    swankit_ffi::protocol::serve(std::io::Cursor::new(large), &mut output, false).unwrap();
    assert!(String::from_utf8(output).unwrap().contains("exceeds bound"));
}
