//! Counted commands with a lazy, interleaved least-significant-bit flag stream.
use crate::{copy_match, grow, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Reader, Result};

/// Decode with no bytes available before the output window.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    decode_with_history(src, &[], max_output)
}

/// Decode with explicit bytes immediately before the output window.
/// Only the last 4096 bytes can be reached by this grammar.
pub fn decode_with_history(src: &[u8], history: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let mut reader = Reader::new(src);
    let command_count = usize::from(reader.u16_be()?);
    if command_count == 0 {
        return Ok(DecodeOutcome {
            bytes: Vec::new(),
            consumed: reader.pos(),
        });
    }

    let limit = max_output.min(MAX_DECODED);
    let history = &history[history.len().saturating_sub(4096)..];
    let mut flags = 0u8;
    let mut bits_left = 0u8;
    let mut out = Vec::new();

    for _ in 0..command_count {
        if !next_flag(&mut reader, &mut flags, &mut bits_left)? {
            grow(&out, 1, limit)?;
            out.push(reader.u8()?);
            continue;
        }

        if !next_flag(&mut reader, &mut flags, &mut bits_left)? {
            let length = usize::from(reader.u8()?) + 11;
            grow(&out, length, limit)?;
            out.extend_from_slice(reader.bytes(length)?);
            continue;
        }

        let token = usize::from(reader.u16_le()?);
        let distance = (token & 0x0fff) + 1;
        let length_code = token >> 12;
        let length = if length_code == 0 {
            usize::from(reader.u8()?) + 18
        } else {
            length_code + 2
        };
        copy_match(&mut out, history, distance, length, limit)?;
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: reader.pos(),
    })
}

fn next_flag(reader: &mut Reader<'_>, flags: &mut u8, bits_left: &mut u8) -> Result<bool> {
    if *bits_left == 0 {
        *flags = reader.u8()?;
        *bits_left = 8;
    }
    let bit = *flags & 1 != 0;
    *flags >>= 1;
    *bits_left -= 1;
    Ok(bit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use formatkit_core::Error;

    #[test]
    fn catalog_selection_is_explicit_and_decode_only() {
        let format = crate::catalog::FORMATS
            .iter()
            .find(|format| format.id == crate::catalog::LZ_COMMAND16_LSB_ID)
            .expect("command codec catalog entry");
        assert_eq!(format.decoder, Some("ws-lz-command16-lsb"));
        assert!(format.probes.is_empty());
        assert!(format.capabilities.decode);
        assert!(!format.capabilities.write);

        let support = crate::catalog::EMBEDDED
            .iter()
            .find(|support| support.id == crate::catalog::LZ_COMMAND16_LSB_ID)
            .expect("explicit embedded support row");
        assert!(matches!(
            support.context,
            formatkit_catalog::catalog::ContextRequirement::ExplicitSelection
        ));
        assert!(support.detectable_carrier.is_none());
        assert_eq!(support.local_discriminator, "lz-command16-lsb");
    }

    #[test]
    fn zero_count_consumes_only_big_endian_count() {
        let decoded = decode(&[0, 0, 0xaa, 0xbb], 0).unwrap();
        assert!(decoded.bytes.is_empty());
        assert_eq!(decoded.consumed, 2);
    }

    #[test]
    fn flags_refill_after_eight_commands_and_tail_is_not_consumed() {
        let source = [
            0, 9, 0, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', 0, b'i', 0xee,
        ];
        let decoded = decode(&source, 9).unwrap();
        assert_eq!(decoded.bytes, b"abcdefghi");
        assert_eq!(decoded.consumed, 13);
    }

    #[test]
    fn independent_source_vectors_cover_lazy_selector_refill_and_overlap() {
        // The independently frozen source model's two-command witness:
        // flags 06, literal A, then LE16 token 1000 => A followed by AAA.
        let source = [0, 2, 0x06, b'A', 0x00, 0x10];
        let decoded = decode(&source, 4).unwrap();
        assert_eq!(decoded.bytes, b"AAAA");
        assert_eq!(decoded.consumed, 6);

        // Seven literals use bits 0..6. The second branch bit for command 8
        // is fetched from the next flag byte before reading its token.
        let source = [
            0, 8, 0x80, b'A', b'B', b'C', b'D', b'E', b'F', b'G', 0x01, 0x00, 0x10,
        ];
        let decoded = decode(&source, 10).unwrap();
        assert_eq!(decoded.bytes, b"ABCDEFGGGG");
        assert_eq!(decoded.consumed, 13);
    }

    #[test]
    fn run_selector_and_literal_bytes_are_interleaved() {
        // Command 1 is literal; command 2 is 10 + zero byte + eleven literals.
        let source = [
            0, 2, 0x02, b'A', 0, b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J', b'K', b'L',
            0xee,
        ];
        let decoded = decode(&source, 12).unwrap();
        assert_eq!(decoded.bytes, b"ABCDEFGHIJKL");
        assert_eq!(decoded.consumed, 16);
    }

    #[test]
    fn backreference_fields_and_forward_overlap() {
        // Literal X, then distance 1 / length 3 (token 0x1000).
        let source = [0, 2, 0x06, b'X', 0x00, 0x10, 0xee];
        let decoded = decode(&source, 4).unwrap();
        assert_eq!(decoded.bytes, b"XXXX");
        assert_eq!(decoded.consumed, 6);

        // Zero length nibble consumes one extra byte: 0 + 18, with distance 1.
        let extended = [0, 2, 0x06, b'Y', 0, 0, 0, 0xee];
        let decoded = decode(&extended, 19).unwrap();
        assert_eq!(decoded.bytes, [b'Y'; 19]);
        assert_eq!(decoded.consumed, 7);
    }

    #[test]
    fn maximum_match_extension_and_literal_run_lengths() {
        // Nonzero high nibble 15 means 17; zero nibble plus 255 means 273.
        let short_match = [0, 1, 0x03, 0, 0xf0];
        let decoded = decode_with_history(&short_match, b"Q", 17).unwrap();
        assert_eq!(decoded.bytes, [b'Q'; 17]);
        assert_eq!(decoded.consumed, 5);
        let long_match = [0, 1, 0x03, 0, 0, 255];
        let decoded = decode_with_history(&long_match, b"Q", 273).unwrap();
        assert_eq!(decoded.bytes, [b'Q'; 273]);
        assert_eq!(decoded.consumed, 6);

        // A 255 count byte extends the literal run by eleven to 266 bytes.
        let mut literal_run = vec![0, 1, 0x01, 255];
        literal_run.extend((0..266).map(|n| n as u8));
        let decoded = decode(&literal_run, 266).unwrap();
        assert_eq!(decoded.bytes, (0..266).map(|n| n as u8).collect::<Vec<_>>());
        assert_eq!(decoded.consumed, 270);
    }

    #[test]
    fn global_decoded_limit_is_preserved() {
        let commands = 241usize;
        let mut source = vec![0, commands as u8];
        for group in 0..commands.div_ceil(4) {
            source.push(0xff);
            let group_commands = (commands - group * 4).min(4);
            for _ in 0..group_commands {
                source.extend_from_slice(&[0, 0, 255]);
            }
        }
        let result = decode_with_history(&source, b"Q", usize::MAX);
        match result {
            Err(Error::ResourceLimit {
                requested, limit, ..
            }) => assert_eq!((requested, limit), (65_793, 65_536)),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[test]
    fn explicit_history_suffix_and_missing_history_refusal() {
        let source = [0, 1, 0x03, 0xff, 0x1f]; // distance 4096, length 3
        let history: Vec<u8> = (0..4097).map(|n| n as u8).collect();
        assert_eq!(
            decode(&source, 3).unwrap_err(),
            Error::InvalidField {
                what: "back-reference distance beyond available history",
                value: 4096,
            }
        );
        assert_eq!(
            decode_with_history(&source, &history, 3).unwrap().bytes,
            [1, 2, 3]
        );
        assert!(decode_with_history(&source, &history[2..], 3).is_err());
    }

    #[test]
    fn truncated_inputs_and_output_budget_are_errors() {
        for source in [
            &[][..],
            &[0][..],
            &[0, 1][..],
            &[0, 1, 0][..],
            &[0, 1, 1][..],
        ] {
            assert!(decode(source, 32).is_err(), "{source:?}");
        }
        assert!(matches!(
            decode(&[0, 1, 0x01, 0], 10),
            Err(Error::ResourceLimit { .. })
        ));
        assert!(decode(&[0, 1, 0x01], 32).is_err());
        assert!(decode(&[0, 1, 0x01, 0], 11).is_err());
        assert!(decode(&[0, 1, 0x03], 32).is_err());
        assert!(decode(&[0, 1, 0x03, 0], 32).is_err());
        assert!(decode(&[0, 1, 0x03, 0, 0], 32).is_err());
    }
}
