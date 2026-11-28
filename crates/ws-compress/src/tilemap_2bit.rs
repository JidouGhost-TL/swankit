//! 2-bit MSB-first delta/repeat/literal 16-bit tilemap stream.
//!
//! Header (2 bytes):
//! - `width: u8` (`1..=64`)
//! - `height: u8` (`1..=64`)
//!
//! Decoded payload is `width * height` little-endian `u16` tilemap words
//! (`width * height * 2` bytes). Accumulator `cur` starts at `0`.
//!
//! Stream body:
//! - Repeating control bytes, each holding four 2-bit opcodes MSB-first
//!   (`shift = 6, 4, 2, 0`):
//!   - `11` (`3`): read `u16_be` into `cur`, emit `cur` as `u16_le`.
//!   - `10` (`2`): `cur = cur.wrapping_add(1)`, emit `cur` as `u16_le`.
//!   - `01` (`1`): emit `cur` (repeat) as `u16_le`.
//!   - `00` (`0`): reserved / invalid.
use crate::{grow, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

/// Decode a self-terminated 2-bit-opcode tilemap stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let mut r = Reader::new(src);
    let width = r.u8()?;
    let height = r.u8()?;
    if !(1..=64).contains(&width) {
        return Err(Error::InvalidField {
            what: "tilemap-2bit width must be 1..=64",
            value: u64::from(width),
        });
    }
    if !(1..=64).contains(&height) {
        return Err(Error::InvalidField {
            what: "tilemap-2bit height must be 1..=64",
            value: u64::from(height),
        });
    }
    let need = usize::from(width) * usize::from(height);
    let out_bytes = need * 2;
    let limit = max_output.min(MAX_DECODED);
    grow(&[], out_bytes, limit)?;

    let mut out = Vec::with_capacity(out_bytes);
    let mut cur: u16 = 0;
    let mut emitted = 0usize;

    while emitted < need {
        let ctrl = r.u8()?;
        for shift in [6u8, 4, 2, 0] {
            if emitted >= need {
                break;
            }
            match (ctrl >> shift) & 3 {
                3 => {
                    cur = r.u16_be()?;
                    out.extend_from_slice(&cur.to_le_bytes());
                }
                2 => {
                    cur = cur.wrapping_add(1);
                    out.extend_from_slice(&cur.to_le_bytes());
                }
                1 => {
                    out.extend_from_slice(&cur.to_le_bytes());
                }
                _ => {
                    return Err(Error::Malformed(
                        "tilemap-2bit encountered reserved opcode 00".into(),
                    ));
                }
            }
            emitted += 1;
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Deterministic encoder for `width * height` little-endian `u16` tilemap words.
pub fn encode(width: u8, height: u8, tilemap_u16le: &[u8]) -> Result<Vec<u8>> {
    if !(1..=64).contains(&width) {
        return Err(Error::InvalidField {
            what: "tilemap-2bit width must be 1..=64",
            value: u64::from(width),
        });
    }
    if !(1..=64).contains(&height) {
        return Err(Error::InvalidField {
            what: "tilemap-2bit height must be 1..=64",
            value: u64::from(height),
        });
    }
    let need = usize::from(width) * usize::from(height);
    if tilemap_u16le.len() != need * 2 {
        return Err(Error::InvalidField {
            what: "tilemap-2bit payload byte length mismatch",
            value: tilemap_u16le.len() as u64,
        });
    }
    let mut out = Vec::with_capacity(2 + need);
    out.push(width);
    out.push(height);

    let mut cur: u16 = 0;
    let mut words = tilemap_u16le.chunks_exact(2);
    let mut emitted = 0usize;
    while emitted < need {
        let ctrl_pos = out.len();
        out.push(0);
        let mut ctrl = 0u8;
        for shift in [6u8, 4, 2, 0] {
            let Some(pair) = words.next() else {
                break;
            };
            let w = u16::from_le_bytes([pair[0], pair[1]]);
            let code = if w == cur {
                1u8
            } else if w == cur.wrapping_add(1) {
                cur = w;
                2u8
            } else {
                cur = w;
                out.extend_from_slice(&w.to_be_bytes());
                3u8
            };
            ctrl |= code << shift;
            emitted += 1;
        }
        out[ctrl_pos] = ctrl;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_and_round_trips_all_opcodes() {
        // 3x2 = 6 words: 0x0000 (repeat), 0x0001 (+1), 0x1234 (lit), 0x1235 (+1), 0x1235 (rep), 0xffff (lit)
        let words: [u16; 6] = [0x0000, 0x0001, 0x1234, 0x1235, 0x1235, 0xffff];
        let mut raw = Vec::new();
        for w in words {
            raw.extend_from_slice(&w.to_le_bytes());
        }
        let enc = encode(3, 2, &raw).unwrap();
        let mut with_tail = enc.clone();
        with_tail.extend_from_slice(&[0xaa, 0xbb]);
        let dec = decode(&with_tail, 64).unwrap();
        assert_eq!(dec.bytes, raw);
        assert_eq!(dec.consumed, enc.len());
    }

    #[test]
    fn rejects_invalid_geometry_opcode_zero_and_truncation() {
        assert!(decode(&[0, 1, 0x40], 16).is_err());
        assert!(decode(&[1, 65, 0x40], 256).is_err());
        // Opcode 00 in top 2 bits
        assert!(decode(&[1, 1, 0x00], 16).is_err());
        // Truncated literal u16_be
        assert!(decode(&[1, 1, 0xc0, 0x12], 16).is_err());
        // Output limit
        assert!(decode(&[2, 2, 0x55], 7).is_err());
    }
}
