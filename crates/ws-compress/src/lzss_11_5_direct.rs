//! Direct-distance 11/5 LZSS stream with a 4-byte header.
//!
//! Layout:
//! - `u16_le uncompressed_size`
//! - `u16_le tag` (2-byte stream tag word)
//! - 8-bit LSB-first flag byte:
//!   - `1`: 1 literal byte
//!   - `0`: `u16_le` back-reference word `w` with
//!     `length = ((w >> 8) & 0x1f) + 3` (`3..=34`) and
//!     direct negative distance `dist = ((w >> 13) << 8) | (w & 0xff)` (`1..=2047`).
use crate::{copy_match, grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result, Writer};

const MAX_DIST: usize = 2047;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 34;

/// Decode a direct-distance 11/5 LZSS stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let mut r = Reader::new(src);
    let declared = usize::from(r.u16_le()?);
    let _tag = r.u16_le()?;
    grow(&[], declared, limit)?;
    if declared == 0 {
        return Ok(DecodeOutcome {
            bytes: Vec::new(),
            consumed: r.pos(),
        });
    }

    let mut out = Vec::with_capacity(declared);
    while out.len() < declared {
        let mut flags = r.u8()?;
        for _ in 0..8 {
            if flags & 1 != 0 {
                let b = r.u8()?;
                out.push(b);
                if out.len() == declared {
                    break;
                }
            } else {
                let w = r.u16_le()?;
                let length = usize::from((w >> 8) & 0x1f) + MIN_MATCH;
                let dist = (usize::from(w >> 13) << 8) | usize::from(w & 0xff);
                if dist == 0 || dist > out.len() {
                    return Err(Error::InvalidField {
                        what: "back-reference distance beyond available history",
                        value: dist as u64,
                    });
                }
                if out.len() + length > declared {
                    return Err(Error::Malformed(
                        "lzss-11-5-direct match overshoots declared output length".into(),
                    ));
                }
                copy_match(&mut out, &[], dist, length, declared)?;
                if out.len() == declared {
                    break;
                }
            }
            flags >>= 1;
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Deterministic encoder for direct-distance 11/5 LZSS streams with an explicit 16-bit header tag.
pub fn encode_with_tag(src: &[u8], tag: u16) -> Result<Vec<u8>> {
    input_size(src)?;
    let declared = u16::try_from(src.len()).map_err(|_| Error::ResourceLimit {
        resource: "decoded bytes",
        requested: src.len() as u64,
        limit: u64::from(u16::MAX),
    })?;
    let mut w = Writer::new();
    w.u16_le(declared);
    w.u16_le(tag);
    if src.is_empty() {
        return Ok(w.into_vec());
    }

    let mut pos = 0usize;
    while pos < src.len() {
        let mut flags = 0u8;
        let mut group = Vec::with_capacity(16);
        for bit in 0..8 {
            if pos >= src.len() {
                break;
            }
            let max_len = (src.len() - pos).min(MAX_MATCH);
            let max_dist = pos.min(MAX_DIST);
            let mut best_len = 0usize;
            let mut best_dist = 0usize;
            if max_len >= MIN_MATCH && max_dist >= 1 {
                for dist in 1..=max_dist {
                    let mut l = 0usize;
                    while l < max_len && src[pos - dist + l] == src[pos + l] {
                        l += 1;
                    }
                    if l >= MIN_MATCH && l > best_len {
                        best_len = l;
                        best_dist = dist;
                        if best_len == max_len {
                            break;
                        }
                    }
                }
            }

            if best_len >= MIN_MATCH {
                let word = (((best_dist >> 8) as u16) << 13)
                    | ((((best_len - MIN_MATCH) & 0x1f) as u16) << 8)
                    | ((best_dist & 0xff) as u16);
                group.extend_from_slice(&word.to_le_bytes());
                pos += best_len;
            } else {
                flags |= 1 << bit;
                group.push(src[pos]);
                pos += 1;
            }
        }
        w.u8(flags);
        w.bytes(&group);
    }

    Ok(w.into_vec())
}

/// Deterministic encoder for direct-distance 11/5 LZSS streams using a zero header tag.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    encode_with_tag(src, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_literals_and_overlapping_matches() {
        let mut payload = b"ABCDABCDABCDABCD".to_vec();
        payload.extend_from_slice(&[0x5a; 48]);
        payload.extend_from_slice(b"tail-bytes-0123456789");

        for tag in [0x0000u16, 0x7361u16] {
            let enc = encode_with_tag(&payload, tag).unwrap();
            assert!(enc.len() < payload.len());
            let dec = decode(&enc, MAX_DECODED).unwrap();
            assert_eq!(dec.bytes, payload);
            assert_eq!(dec.consumed, enc.len());
        }
    }

    #[test]
    fn empty_stream_and_error_checks() {
        let enc = encode(&[]).unwrap();
        assert_eq!(enc, vec![0, 0, 0, 0]);
        let dec = decode(&enc, 64).unwrap();
        assert!(dec.bytes.is_empty());
        assert_eq!(dec.consumed, 4);

        // Zero distance rejected
        let bad_dist = [0x04, 0x00, 0x00, 0x00, 0x01, 0xaa, 0x00, 0x00];
        assert!(matches!(
            decode(&bad_dist, 64),
            Err(Error::InvalidField { .. })
        ));

        // Match overshoot rejected
        let overshoot = [0x03, 0x00, 0x00, 0x00, 0x01, 0xaa, 0x01, 0x00];
        assert!(matches!(decode(&overshoot, 64), Err(Error::Malformed(_))));

        // Truncated header rejected
        assert!(matches!(decode(&[0x04, 0x00], 64), Err(Error::Truncated { .. })));
    }
}
