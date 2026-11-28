//! 1-byte-token 4-bit distance / 4-bit length LSB-flag LZ77 stream.
//!
//! Stream consists of repeating 9-byte groups (final group may be truncated):
//! - Byte `0`: `flags: u8`, scanned LSB-first (`bit = 0..8`).
//! - For each bit `b` in `0..8`:
//!   - `(flags >> b) & 1 == 1`: read 1 literal byte `v` and emit `v`.
//!   - `(flags >> b) & 1 == 0`: read 1 token byte `tok`:
//!     - `dist   = (tok >> 4) + 1` (`1..=16` bytes back)
//!     - `length = (tok & 0x0F) + 1` (`1..=16` bytes copied sequentially, supporting RLE overlap).
//!
//! Stream extent is supplied by the enclosing carrier (and optionally validated
//! against an explicit `decoded_length`).
use crate::{copy_match, grow, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

/// Decode a carrier-bounded 4-bit/4-bit LSB-flag LZ77 stream.
pub fn decode(
    src: &[u8],
    decoded_length: Option<usize>,
    max_output: usize,
) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let effective_limit = match decoded_length {
        Some(expected) => {
            grow(&[], expected, limit)?;
            expected
        }
        None => limit,
    };

    let mut r = Reader::new(src);
    let mut out = Vec::with_capacity(decoded_length.unwrap_or(0).min(limit));

    while r.remaining() > 0 && out.len() < effective_limit {
        let flags = r.u8()?;
        for bit in 0..8 {
            if r.remaining() == 0 || out.len() >= effective_limit {
                break;
            }
            let b = r.u8()?;
            if (flags >> bit) & 1 != 0 {
                grow(&out, 1, effective_limit)?;
                out.push(b);
            } else {
                let dist = usize::from(b >> 4) + 1;
                let length = usize::from(b & 0x0f) + 1;
                copy_match(&mut out, &[], dist, length, effective_limit)?;
            }
        }
    }

    if let Some(expected) = decoded_length {
        if out.len() != expected {
            return Err(Error::Malformed(format!(
                "lz44 decoded {} bytes, expected {expected}",
                out.len()
            )));
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Deterministic greedy encoder for the 4-bit/4-bit LSB-flag LZ77 format.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    grow(&[], src.len(), MAX_DECODED)?;
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0usize;

    while i < src.len() {
        let flag_pos = out.len();
        out.push(0);
        let mut flags = 0u8;

        for bit in 0..8 {
            if i >= src.len() {
                break;
            }
            let mut best_len = 0usize;
            let mut best_dist = 1usize;
            let max_d = i.min(16);
            let max_l = (src.len() - i).min(16);
            for d in 1..=max_d {
                let mut l = 0usize;
                while l < max_l && src[i - d + l] == src[i + l] {
                    l += 1;
                }
                if l > best_len {
                    best_len = l;
                    best_dist = d;
                    if best_len == max_l {
                        break;
                    }
                }
            }
            if best_len >= 2 {
                let tok = (((best_dist - 1) as u8) << 4) | ((best_len - 1) as u8);
                out.push(tok);
                i += best_len;
            } else {
                flags |= 1 << bit;
                out.push(src[i]);
                i += 1;
            }
        }
        out[flag_pos] = flags;
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_literals_and_overlapping_matches() {
        for sample in [
            b"".as_slice(),
            b"A",
            b"AAAAAAAAAAAAAAAAAAAA",
            b"0123012301230123ABCDABCD",
        ] {
            let enc = encode(sample).unwrap();
            let dec = decode(&enc, Some(sample.len()), 1024).unwrap();
            assert_eq!(dec.bytes, sample);
            assert_eq!(dec.consumed, enc.len());

            let dec_open = decode(&enc, None, 1024).unwrap();
            assert_eq!(dec_open.bytes, sample);
            assert_eq!(dec_open.consumed, enc.len());
        }
    }

    #[test]
    fn rejects_invalid_distance_and_length_mismatch() {
        // Backref at position 0 (no history)
        assert!(decode(&[0x00, 0x00], None, 64).is_err());
        let enc = encode(b"ABCDEFABCDEF").unwrap();
        assert!(decode(&enc, Some(8), 64).is_err());
        assert!(decode(&enc, Some(99), 64).is_err());
    }
}
