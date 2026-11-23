//! LSB flag bytes and little-endian length4/distance12 backward tokens.
//! The carrier supplies decoded size; no implicit initialized ring is assumed.
use crate::{input_size, map_flag_lzss_err, DecodeOutcome, MAX_DECODED};
use formatkit_codec::find_longest_match;
use formatkit_codec::flag_lzss::{
    decode_flag_lzss, FlagBitOrder, FlagLzssConfig, FlagLzssMatch, FlagPolarity, FlagWidth,
    OvershootPolicy,
};
use formatkit_core::{u16_at, Result};

const CONFIG: FlagLzssConfig = FlagLzssConfig {
    flag_width: FlagWidth::U8,
    bit_order: FlagBitOrder::LsbFirst,
    polarity: FlagPolarity::LiteralIsOne,
    overshoot: OvershootPolicy::Reject,
};

pub fn decode(src: &[u8], decoded_size: usize, max_output: usize) -> Result<DecodeOutcome> {
    decode_with_history(src, &[], decoded_size, max_output)
}
pub fn decode_with_history(
    src: &[u8],
    history: &[u8],
    decoded_size: usize,
    max_output: usize,
) -> Result<DecodeOutcome> {
    let outcome = decode_flag_lzss(
        src,
        history,
        decoded_size,
        max_output.min(MAX_DECODED),
        CONFIG,
        |src, pos| {
            let token = usize::from(u16_at(src, pos)?);
            Ok((
                FlagLzssMatch {
                    distance: (token & 4095) + 1,
                    length: (token >> 12) + 3,
                },
                2,
            ))
        },
    )
    .map_err(map_flag_lzss_err)?;
    Ok(DecodeOutcome {
        bytes: outcome.output,
        consumed: outcome.bytes_consumed,
    })
}

pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    let mut out = Vec::new();
    let mut at = 0;
    while at < src.len() {
        let flag_at = out.len();
        out.push(0);
        let mut flags = 0;
        for bit in 0..8 {
            if at == src.len() {
                break;
            }
            if let Some((distance, count)) = find_longest_match(src, at, 1, 4096, 3, 18) {
                let token = (((count - 3) << 12) | (distance - 1)) as u16;
                out.extend_from_slice(&token.to_le_bytes());
                at += count;
            } else {
                flags |= 1 << bit;
                out.push(src[at]);
                at += 1;
            }
        }
        out[flag_at] = flags;
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literals_overlap_and_history() {
        assert_eq!(
            decode(&[7, b'A', b'B', b'C', 2, 0x30], 9, 9).unwrap().bytes,
            b"ABCABCABC"
        );
        assert_eq!(
            decode_with_history(&[0, 2, 0], b"ABC", 3, 3).unwrap().bytes,
            b"ABC"
        );
        assert!(decode(&[0, 2, 0], 3, 3).is_err());
    }
    #[test]
    fn boundaries() {
        assert!(decode(&[0, 0, 0], 2, 2).is_err());
        assert!(decode(&[], 1, 1).is_err());
        assert!(decode(&[1, 42], 1, 0).is_err());
        assert_eq!(decode(&[1, 42, 99], 1, 1).unwrap().consumed, 2);
    }
    #[test]
    fn round_trips() {
        for n in [0, 1, 8, 17, 300, 5000] {
            let b: Vec<_> = (0..n).map(|i| ((i / 7 + i % 11) & 255) as u8).collect();
            let e = encode(&b).unwrap();
            let d = decode(&e, n, n).unwrap();
            assert_eq!(d.bytes, b);
            assert_eq!(d.consumed, e.len());
        }
    }
}
