use formatkit_core::Error;
use ws_compress::{lz_bits_16 as flags, lz_rle_7f as commands, okumura, BitOrder};

type Encoder = fn(&[u8]) -> formatkit_core::Result<Vec<u8>>;
type Decoder = fn(&[u8], usize) -> formatkit_core::Result<ws_compress::DecodeOutcome>;
const TERMINATED: [(Encoder, Decoder); 2] = [
    (commands::encode, commands::decode),
    (flags::encode, flags::decode),
];

#[test]
fn byte_commands_and_consumed_extent() {
    // Literal, zero-prefixed words, run, small/large zeros, overlapping match.
    let src = [
        0x81, 4, 5, 0xa1, 7, 8, 0xc0, 9, 0xe0, 0xff, 0, 0x83, 1, 2, 3, 4, 0x13, 0xfc, 0x7f, 0xff,
        0xaa,
    ];
    let mut want = vec![4, 5, 0, 7, 0, 8, 9, 9];
    want.extend_from_slice(&[0; 33]);
    want.extend_from_slice(&[1, 2, 3, 4, 1, 2, 3, 4, 1, 2]);
    let got = commands::decode(&src, want.len()).unwrap();
    assert_eq!(got.bytes, want);
    assert_eq!(got.consumed, src.len() - 1);
    assert!(matches!(
        commands::decode(&src, want.len() - 1),
        Err(Error::ResourceLimit { .. })
    ));
}

#[test]
fn reserved_end_marker_and_maximum_match() {
    let mut src = vec![0x81, 3, 4, 0x7f, 0xfe, 0x7f, 0xff];
    let got = commands::decode(&src, 35).unwrap();
    assert_eq!(
        got.bytes,
        (0..35)
            .map(|i| if i % 2 == 0 { 3 } else { 4 })
            .collect::<Vec<_>>()
    );
    src[4] = 0xff;
    assert_eq!(commands::decode(&src, 35).unwrap().bytes, [3, 4]);
    let run = vec![9; 400];
    let packed = commands::encode(&run).unwrap();
    assert_eq!(commands::decode(&packed, run.len()).unwrap().bytes, run);
}

#[test]
fn flag_word_refills_before_sixteenth_literal_operand() {
    // Sixteen literals followed by a long-form end marker: second flag word
    // occurs before literal #16, even though all its flags belong to word #1.
    let mut src = vec![0xff, 0xff];
    src.extend(0..15);
    src.extend_from_slice(&[2, 0, 15, 0, 0, 0, 0xaa]);
    let got = flags::decode(&src, 16).unwrap();
    assert_eq!(got.bytes, (0..16).collect::<Vec<_>>());
    assert_eq!(got.consumed, src.len() - 1);
    for cut in 0..src.len() - 1 {
        assert!(flags::decode(&src[..cut], 16).is_err());
    }
}

#[test]
fn flag_short_long_extended_and_overlap() {
    // Bits: 1 literal; 0,0,1,1 short length 5; 0,1 long length 3;
    // 0,1 extended length 10; 0,1 end. All matches distance 1.
    let src = [
        0x59, 0x05, b'x', 0xff, 0xff, 0xf9, 0xff, 0xf8, 9, 0, 0, 0, 0xaa,
    ];
    let got = flags::decode(&src, 19).unwrap();
    assert_eq!(got.bytes, vec![b'x'; 19]);
    assert_eq!(got.consumed, 12);
}

#[test]
fn prehistory_is_explicit_and_limits_are_checked() {
    let rle = [3, 0xff, 0x7f, 0xff];
    assert!(commands::decode(&rle, 2).is_err());
    assert_eq!(
        commands::decode_with_history(&rle, &[7], 2).unwrap().bytes,
        [7, 7]
    );
    // Short reference of length 2 distance 1, then end.
    let lz = [0x20, 0, 0xff, 0, 0, 0];
    assert!(flags::decode(&lz, 2).is_err());
    assert_eq!(
        flags::decode_with_history(&lz, &[8], 2).unwrap().bytes,
        [8, 8]
    );
    for encode in [commands::encode, flags::encode] {
        assert!(matches!(
            encode(&vec![0; 65537]),
            Err(Error::ResourceLimit { .. })
        ));
    }
    for decode in [commands::decode, flags::decode] {
        assert!(decode(&[], 0).is_err());
        assert!(decode(&[0; 8], usize::MAX).is_err());
    }
}

#[test]
fn deterministic_roundtrips_cover_windows_words_and_runs() {
    let mut state = 0x0123_4567u32;
    let mut noise = Vec::new();
    for _ in 0..11000 {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        noise.push((state >> 24) as u8);
    }
    let mut with_matches = noise.clone();
    with_matches.extend_from_slice(&noise[3000..5000]);
    let cases = [
        Vec::new(),
        vec![0; 65536],
        vec![5; 4097],
        (0..=255).cycle().take(3500).collect(),
        (0..=255).flat_map(|i| [0, i]).cycle().take(4096).collect(),
        noise,
        with_matches,
    ];
    for raw in cases {
        for (encode, decode) in TERMINATED {
            let packed = encode(&raw).unwrap();
            let got = decode(&packed, raw.len()).unwrap();
            assert_eq!(got.bytes, raw);
            assert_eq!(got.consumed, packed.len());
        }
    }
}

#[test]
fn malformed_prefixes_and_random_inputs_never_panic() {
    let raw = [0, 4, 0, 5, 0, 6, 8, 8, 8, 8, 9, 10, 11];
    for (encode, decode) in TERMINATED {
        let packed = encode(&raw).unwrap();
        for cut in 0..packed.len() {
            assert!(decode(&packed[..cut], 100).is_err());
        }
        let mut state = 123u32;
        for len in 0..128 {
            let src: Vec<_> = (0..len)
                .map(|_| {
                    state = state.wrapping_mul(1103515245).wrapping_add(12345);
                    (state >> 16) as u8
                })
                .collect();
            if let Ok(got) = decode(&src, 64) {
                assert!(got.bytes.len() <= 64);
                assert!(got.consumed <= src.len());
            }
        }
    }
}

#[test]
fn okumura_options_framing_and_strict_bounds() {
    for order in [BitOrder::LsbFirst, BitOrder::MsbFirst] {
        for fill in [0, 0x20, 0xa5] {
            let raw = [fill; 512];
            let mut packed = okumura::encode(&raw, order, fill);
            let extent = packed.len();
            packed.push(0xaa);
            let got = okumura::decode(&packed, raw.len(), order, fill, raw.len()).unwrap();
            assert_eq!(got.bytes, raw);
            assert_eq!(got.consumed, extent);
            assert!(okumura::decode(&packed, 512, order, fill, 511).is_err());
        }
    }
    assert!(okumura::decode(&[0, 0, 0], 2, BitOrder::LsbFirst, 0, 2).is_err());
    assert_eq!(
        okumura::decode(&[], 0, BitOrder::LsbFirst, 0, 0)
            .unwrap()
            .consumed,
        0
    );
}
