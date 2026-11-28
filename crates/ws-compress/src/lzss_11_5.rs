//! 2 KiB ring-buffer LZSS stream with 11-bit ring position and 5-bit length.
//!
//! Layout:
//! - `u32_le uncompressed_size` (4-byte header)
//! - 8-bit LSB-first flag byte: `1` = literal byte, `0` = 2-byte match `(lo, hi)`
//!   with `length = (hi & 0x1f) + 3` (`3..=34`) and `pos = ((hi >> 5) << 8) | lo`
//!   (`0..=2047`).
//! - 2048-byte zero-initialized ring buffer starting at cursor `0x7de` (`2014`).
use crate::{grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result, Writer};

const RING_SIZE: usize = 2048;
const RING_MASK: usize = RING_SIZE - 1;
const RING_INIT: usize = RING_SIZE - 34; // 0x7de

/// Decode a 2 KiB ring-buffer 11/5 LZSS stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let mut r = Reader::new(src);
    let declared_u32 = r.u32_le()?;
    let declared = usize::try_from(declared_u32).map_err(|_| Error::ResourceLimit {
        resource: "decoded bytes",
        requested: u64::from(declared_u32),
        limit: limit as u64,
    })?;
    grow(&[], declared, limit)?;
    if declared == 0 {
        return Ok(DecodeOutcome {
            bytes: Vec::new(),
            consumed: r.pos(),
        });
    }

    let mut ring = [0u8; RING_SIZE];
    let mut cursor = RING_INIT;
    let mut out = Vec::with_capacity(declared);

    while out.len() < declared {
        let mut flags = r.u8()?;
        for _ in 0..8 {
            if flags & 1 != 0 {
                let b = r.u8()?;
                ring[cursor] = b;
                cursor = (cursor + 1) & RING_MASK;
                out.push(b);
            } else {
                let lo = r.u8()?;
                let hi = r.u8()?;
                let length = usize::from(hi & 0x1f) + 3;
                let mut pos = (usize::from(hi >> 5) << 8) | usize::from(lo);
                for _ in 0..length {
                    let b = ring[pos];
                    pos = (pos + 1) & RING_MASK;
                    ring[cursor] = b;
                    cursor = (cursor + 1) & RING_MASK;
                    out.push(b);
                }
            }
            flags >>= 1;
            if out.len() >= declared {
                break;
            }
        }
    }
    out.truncate(declared);

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Deterministic encoder for 2 KiB ring-buffer 11/5 LZSS streams.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    let mut w = Writer::new();
    w.u32_le(src.len() as u32);
    if src.is_empty() {
        return Ok(w.into_vec());
    }

    let mut ring = [0u8; RING_SIZE];
    let mut cursor = RING_INIT;
    let mut pos = 0usize;

    while pos < src.len() {
        let mut flags = 0u8;
        let mut group = Vec::with_capacity(16);
        for bit in 0..8 {
            if pos >= src.len() {
                break;
            }
            let max_len = (src.len() - pos).min(34);
            let mut best_len = 0usize;
            let mut best_pos = 0usize;
            if max_len >= 3 {
                // Search ring positions (distance 1..=RING_SIZE-34 to avoid self-clobbering past cursor).
                for dist in 1..=(RING_SIZE - 34) {
                    let start = cursor.wrapping_sub(dist) & RING_MASK;
                    let mut l = 0usize;
                    while l < max_len {
                        let ring_byte = if l < dist {
                            ring[(start + l) & RING_MASK]
                        } else {
                            src[pos + l - dist]
                        };
                        if ring_byte != src[pos + l] {
                            break;
                        }
                        l += 1;
                    }
                    if l >= 3 && l > best_len {
                        best_len = l;
                        best_pos = start;
                        if best_len == max_len {
                            break;
                        }
                    }
                }
            }
            if best_len >= 3 {
                let lo = (best_pos & 0xff) as u8;
                let hi = (((best_pos >> 8) as u8) << 5) | ((best_len - 3) as u8);
                group.push(lo);
                group.push(hi);
                for _ in 0..best_len {
                    ring[cursor] = src[pos];
                    cursor = (cursor + 1) & RING_MASK;
                    pos += 1;
                }
            } else {
                flags |= 1 << bit;
                let b = src[pos];
                group.push(b);
                ring[cursor] = b;
                cursor = (cursor + 1) & RING_MASK;
                pos += 1;
            }
        }
        w.u8(flags);
        w.bytes(&group);
    }

    Ok(w.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_zero_prehistory_and_matches() {
        // 8 zero bytes from pre-initialized ring at pos 0 (len = 8 => hi = 5, lo = 0),
        // followed by literal 0x42.
        // flags = 0b0000_0010 (bit 0 = match, bit 1 = literal)
        let stream = [9, 0, 0, 0, 0x02, 0x00, 0x05, 0x42, 0xee];
        let got = decode(&stream, 64).unwrap();
        assert_eq!(got.bytes, vec![0, 0, 0, 0, 0, 0, 0, 0, 0x42]);
        assert_eq!(got.consumed, 8);
        assert_eq!(decode(&[0, 0, 0, 0, 0x99], 0).unwrap().consumed, 4);
    }

    #[test]
    fn rejects_truncation_and_overflow() {
        assert!(decode(&[], 64).is_err());
        assert!(decode(&[1, 0, 0], 64).is_err());
        assert!(decode(&[1, 0, 0, 0], 64).is_err());
        assert!(decode(&[1, 0, 0, 0, 0x01], 64).is_err());
        assert!(decode(&[3, 0, 0, 0, 0x00, 0x00], 64).is_err());
        assert!(decode(&[10, 0, 0, 0, 0xff], 8).is_err());
        assert!(encode(&vec![0; MAX_DECODED + 1]).is_err());
    }

    #[test]
    fn round_trips_patterns_and_ring_wrap() {
        for n in [0, 1, 3, 16, 256, 2500] {
            let raw: Vec<u8> = (0..n).map(|i| ((i / 7 + i % 13) & 0xff) as u8).collect();
            let enc = encode(&raw).unwrap();
            let dec = decode(&enc, n).unwrap();
            assert_eq!(dec.bytes, raw);
            assert_eq!(dec.consumed, enc.len());
            assert_eq!(encode(&raw).unwrap(), enc);
        }
    }
}
