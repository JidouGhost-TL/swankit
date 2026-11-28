//! 7-bit literal/back-reference 4bpp tile LZ stream.
//!
//! Framing:
//! - Little-endian `u16` tile count (`n_tiles >= 1`; decoded size is `n_tiles * 32` bytes).
//! - 4-byte raw seed copied verbatim to output bytes `0..4`.
//! - Control-byte token stream until `n_tiles * 32` bytes are produced:
//!   - `b & 0x80 != 0`: literal run of `b & 0x7F` (`1..=127`) bytes.
//!   - `b & 0x80 == 0`: back-reference with `distance = b` (`1..=127`) and `count = next_byte` (`0..=255`).
use crate::{copy_match, grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

/// Decode one tile-count-prefixed 7-bit LZ stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let mut r = Reader::new(src);
    let n_tiles = usize::from(r.u16_le()?);
    if n_tiles == 0 {
        return Err(Error::InvalidField {
            what: "lz-tile-7b tile count",
            value: 0,
        });
    }
    let decoded_size = n_tiles.checked_mul(32).ok_or(Error::ResourceLimit {
        resource: "decoded bytes",
        requested: u64::MAX,
        limit: max_output.min(MAX_DECODED) as u64,
    })?;
    let limit = max_output.min(MAX_DECODED);
    grow(&[], decoded_size, limit)?;

    let seed = r.bytes(4)?;
    let mut out = Vec::with_capacity(decoded_size);
    out.extend_from_slice(seed);

    while out.len() < decoded_size {
        let ctrl = r.u8()?;
        if ctrl & 0x80 != 0 {
            let count = usize::from(ctrl & 0x7F);
            if count == 0 {
                return Err(Error::InvalidField {
                    what: "lz-tile-7b zero literal count",
                    value: 0,
                });
            }
            grow(&out, count, decoded_size)?;
            out.extend_from_slice(r.bytes(count)?);
        } else {
            let distance = usize::from(ctrl);
            if distance == 0 {
                return Err(Error::InvalidField {
                    what: "lz-tile-7b zero back-reference distance",
                    value: 0,
                });
            }
            let count = usize::from(r.u8()?);
            if count == 0 {
                if distance > out.len() {
                    return Err(Error::InvalidField {
                        what: "back-reference distance beyond available history",
                        value: distance as u64,
                    });
                }
            } else {
                copy_match(&mut out, &[], distance, count, decoded_size)?;
            }
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Deterministic encoder for 7-bit literal/back-reference 4bpp tile LZ streams.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    if src.is_empty() || src.len() % 32 != 0 {
        return Err(Error::InvalidField {
            what: "lz-tile-7b payload must be a positive multiple of 32 bytes",
            value: src.len() as u64,
        });
    }
    let n_tiles = u16::try_from(src.len() / 32).map_err(|_| Error::InvalidField {
        what: "lz-tile-7b tile count exceeds u16",
        value: (src.len() / 32) as u64,
    })?;

    let mut out = Vec::new();
    out.extend_from_slice(&n_tiles.to_le_bytes());
    out.extend_from_slice(&src[..4]);

    let mut i = 4;
    let mut lit_buf: Vec<u8> = Vec::new();
    let flush_lits = |out: &mut Vec<u8>, lit_buf: &mut Vec<u8>| {
        for chunk in lit_buf.chunks(127) {
            out.push(0x80 | (chunk.len() as u8));
            out.extend_from_slice(chunk);
        }
        lit_buf.clear();
    };

    while i < src.len() {
        let max_dist = i.min(127);
        let max_len = (src.len() - i).min(255);
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        for dist in 1..=max_dist {
            let mut matched = 0;
            while matched < max_len && src[i - dist + matched] == src[i + matched] {
                matched += 1;
            }
            if matched >= 3 && matched > best_len {
                best_len = matched;
                best_dist = dist;
                if best_len == max_len {
                    break;
                }
            }
        }
        if best_len >= 3 {
            flush_lits(&mut out, &mut lit_buf);
            out.push(best_dist as u8);
            out.push(best_len as u8);
            i += best_len;
        } else {
            lit_buf.push(src[i]);
            i += 1;
            if lit_buf.len() == 127 {
                flush_lits(&mut out, &mut lit_buf);
            }
        }
    }
    flush_lits(&mut out, &mut lit_buf);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_tiles_and_overlapping_matches() {
        let mut tile_data = vec![0xAA; 32];
        tile_data.extend((0..64).map(|i| (i * 3) as u8));
        tile_data.extend_from_slice(&[1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4]);
        tile_data.extend_from_slice(&[0; 16]);
        let enc = encode(&tile_data).unwrap();
        let dec = decode(&enc, tile_data.len()).unwrap();
        assert_eq!(dec.bytes, tile_data);
        assert_eq!(dec.consumed, enc.len());
    }

    #[test]
    fn rejects_invalid_headers_and_overshoot() {
        assert!(decode(&[], 64).is_err());
        assert!(decode(&[0, 0, 1, 2, 3, 4], 64).is_err());
        assert!(decode(&[1, 0, 1, 2, 3, 4, 0x80], 64).is_err());
        assert!(decode(&[1, 0, 1, 2, 3, 4, 0x00, 1], 64).is_err());
        assert!(decode(&[1, 0, 1, 2, 3, 4, 0x01, 29], 64).is_err()); // 4 + 29 = 33 > 32
        assert!(encode(&[0; 31]).is_err());
    }
}
