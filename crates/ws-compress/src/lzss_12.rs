//! LSB flag bytes and little-endian length4/distance12 backward tokens.
//! The carrier supplies decoded size; no implicit initialized ring is assumed.
use crate::{copy_match, grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Reader, Result};

pub fn decode(src: &[u8], decoded_size: usize, max_output: usize) -> Result<DecodeOutcome> {
    decode_with_history(src, &[], decoded_size, max_output)
}
pub fn decode_with_history(
    src: &[u8],
    history: &[u8],
    decoded_size: usize,
    max_output: usize,
) -> Result<DecodeOutcome> {
    grow(&[], decoded_size, max_output.min(MAX_DECODED))?;
    let mut r = Reader::new(src);
    let mut out = Vec::with_capacity(decoded_size);
    while out.len() < decoded_size {
        let flags = r.u8()?;
        for bit in 0..8 {
            if out.len() == decoded_size {
                break;
            }
            if flags & (1 << bit) != 0 {
                out.push(r.u8()?);
            } else {
                let token = usize::from(r.u16_le()?);
                copy_match(
                    &mut out,
                    history,
                    (token & 4095) + 1,
                    (token >> 12) + 3,
                    decoded_size,
                )?;
            }
        }
    }
    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
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
            let mut best = (0, 0);
            for distance in 1..=at.min(4096) {
                let mut count = 0;
                while count < 18
                    && at + count < src.len()
                    && src[at + count] == src[at + count - distance]
                {
                    count += 1;
                }
                if count > best.1 {
                    best = (distance, count);
                }
            }
            if best.1 >= 3 {
                let token = (((best.1 - 3) << 12) | (best.0 - 1)) as u16;
                out.extend_from_slice(&token.to_le_bytes());
                at += best.1;
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
