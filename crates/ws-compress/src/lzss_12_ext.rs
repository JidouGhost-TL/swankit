//! Externally sized LZSS with LSB flags, extended lengths and clamped matches.
//! Zero flag bits are literals; one bits carry a little-endian token with a
//! 12-bit distance minus one and a four-bit length code. Nonzero length codes
//! add two; zero codes read another byte and add eighteen.
use crate::{map_flag_lzss_err, DecodeOutcome, MAX_DECODED};
use formatkit_codec::flag_lzss::{
    decode_flag_lzss, FlagBitOrder, FlagLzssConfig, FlagLzssMatch, FlagPolarity, FlagWidth,
    OvershootPolicy,
};
use formatkit_core::{u16_at, u8_at, Result};

const CONFIG: FlagLzssConfig = FlagLzssConfig {
    flag_width: FlagWidth::U8,
    bit_order: FlagBitOrder::LsbFirst,
    polarity: FlagPolarity::LiteralIsZero,
    overshoot: OvershootPolicy::ClampToDecodedSize,
};

/// Decode an explicitly sized stream without assuming initial history.
///
/// The carrier supplies `decoded_size` separately. A zero API size requests
/// empty output and consumes no input; this does not interpret a carrier's
/// zero header or establish its runtime behavior. Trailing input is unconsumed.
pub fn decode(src: &[u8], decoded_size: usize, max_output: usize) -> Result<DecodeOutcome> {
    decode_with_history(src, &[], decoded_size, max_output)
}

/// Decode with the known bytes immediately preceding the output destination.
///
/// Only the last 4096 history bytes can be referenced. Matches copy forward,
/// allowing self-overlap, and clamp to the remaining declared output. A match
/// still consumes its complete token and any extension before being clamped.
/// Missing history is an error, never an implicit zero fill. The decoded size
/// must fit both `max_output` and the crate's maximum decoded-size bound.
pub fn decode_with_history(
    src: &[u8],
    history: &[u8],
    decoded_size: usize,
    max_output: usize,
) -> Result<DecodeOutcome> {
    let history = &history[history.len().saturating_sub(4096)..];
    let outcome = decode_flag_lzss(
        src,
        history,
        decoded_size,
        max_output.min(MAX_DECODED),
        CONFIG,
        |src, pos| {
            let token = usize::from(u16_at(src, pos)?);
            let code = token >> 12;
            let (length, consumed) = if code == 0 {
                (usize::from(u8_at(src, pos + 2)?) + 18, 3)
            } else {
                (code + 2, 2)
            };
            Ok((
                FlagLzssMatch {
                    distance: (token & 4095) + 1,
                    length,
                },
                consumed,
            ))
        },
    )
    .map_err(map_flag_lzss_err)?;
    Ok(DecodeOutcome {
        bytes: outcome.output,
        consumed: outcome.bytes_consumed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use formatkit_core::Error;

    #[test]
    fn flag_polarity_distance_one_and_forward_overlap() {
        assert_eq!(decode(&[0, b'A', b'B'], 2, 2).unwrap().bytes, b"AB");
        let stream = [2, b'A', 0, 0x10];
        let decoded = decode(&stream, 4, 4).unwrap();
        assert_eq!(decoded.bytes, b"AAAA");
        assert_eq!(decoded.consumed, stream.len());
        assert!(crate::lzss_12::decode(&stream, 4, 4).is_err());
        assert_eq!(
            decode(&[4, b'A', b'B', 1, 0x10], 5, 5).unwrap().bytes,
            b"ABABA"
        );
    }

    #[test]
    fn normal_and_extended_lengths_clamp_without_skipping_operands() {
        assert_eq!(decode(&[2, b'A', 0, 0x20], 5, 5).unwrap().bytes, b"AAAAA");
        let short = decode(&[2, b'A', 0, 0xf0, 99], 2, 2).unwrap();
        assert_eq!(short.bytes, b"AA");
        assert_eq!(short.consumed, 4);
        let extended = decode(&[2, b'B', 0, 0, 255, 99], 3, 3).unwrap();
        assert_eq!(extended.bytes, b"BBB");
        assert_eq!(extended.consumed, 5);
        let minimum = decode_with_history(&[1, 0, 0, 0], b"Z", 18, 18).unwrap();
        assert_eq!(minimum.bytes, vec![b'Z'; 18]);
        assert_eq!(minimum.consumed, 4);
    }

    #[test]
    fn history_is_explicit_and_uses_only_the_reachable_suffix() {
        assert_eq!(
            decode_with_history(&[1, 1, 0x10], b"AB", 3, 3)
                .unwrap()
                .bytes,
            b"ABA"
        );
        let history: Vec<u8> = (0..=4096).map(|n| n as u8).collect();
        assert_eq!(
            decode_with_history(&[1, 0xff, 0x1f], &history, 3, 3)
                .unwrap()
                .bytes,
            [1, 2, 3]
        );
        assert!(decode_with_history(&[1, 0xff, 0x1f], &history[2..], 3, 3).is_err());
        assert!(matches!(
            decode(&[1, 0, 0x10], 1, 1),
            Err(Error::InvalidField { .. })
        ));
        assert!(decode(&[1, 0, 0, 0], 1, 1).is_err());
    }

    #[test]
    fn lazy_flag_refill_and_declared_extent_consumption() {
        let stream = [0, b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', 0, b'I'];
        let first = decode(&stream, 8, 8).unwrap();
        assert_eq!(first.bytes, b"ABCDEFGH");
        assert_eq!(first.consumed, 9);
        let all = decode(&stream, 9, 9).unwrap();
        assert_eq!(all.bytes, b"ABCDEFGHI");
        assert_eq!(all.consumed, stream.len());
    }

    #[test]
    fn truncation_and_missing_extension_use_checked_reader_errors() {
        for stream in [&[][..], &[0][..], &[1][..], &[1, 0][..], &[1, 0, 0][..]] {
            assert!(matches!(decode(stream, 1, 1), Err(Error::Truncated { .. })));
        }
        assert!(matches!(
            decode_with_history(&[1, 0, 0], b"A", 1, 1),
            Err(Error::Truncated {
                offset: 3,
                needed: 1,
                available: 0
            })
        ));
    }

    #[test]
    fn explicit_output_budget_and_zero_request() {
        let empty = decode(&[255], 0, 0).unwrap();
        assert!(empty.bytes.is_empty());
        assert_eq!(empty.consumed, 0);
        assert!(matches!(
            decode(&[], 1, 0),
            Err(Error::ResourceLimit { .. })
        ));
        assert!(matches!(
            decode(&[], MAX_DECODED + 1, usize::MAX),
            Err(Error::ResourceLimit { .. })
        ));
        assert!(matches!(
            decode(&[], usize::MAX, usize::MAX),
            Err(Error::ResourceLimit { .. })
        ));
        let mut stream = Vec::new();
        let expected: Vec<u8> = (0..MAX_DECODED).map(|n| n as u8).collect();
        for chunk in expected.chunks(8) {
            stream.push(0);
            stream.extend_from_slice(chunk);
        }
        let decoded = decode(&stream, MAX_DECODED, MAX_DECODED).unwrap();
        assert_eq!(decoded.bytes, expected);
        assert_eq!(decoded.consumed, stream.len());
    }
}
