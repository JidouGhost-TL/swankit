//! Length-prefixed LSB-flag 11-bit distance LZSS stream (single-plane and 2-plane).
//!
//! Single-plane framing starts with a little-endian `u16` header:
//! - `0`: invalid.
//! - `bit 15 == 1`: raw uncompressed copy of `header & 0x7fff` bytes.
//! - `bit 15 == 0`: compressed payload decoding to `header` bytes. Each 8-bit
//!   LSB-first flag byte selects literal (`1`) or a 2-byte match `(b0, b1)` (`0`)
//!   with `length = (b0 >> 3) + 2` (`2..=33`) and `distance = ((b0 & 7) << 8) | b1`
//!   (`1..=2047`).
//!
//! Two-plane framing optionally skips a carrier prefix (`header_skip` bytes),
//! reads `u16_le plane_len`, and either copies `2 * (plane_len & 0x7fff)` raw
//! bytes (`bit 15 == 1`) or runs two sequential single-plane LZSS passes
//! interleaved into even (`2 * i`) and odd (`2 * i + 1`) output bytes.
use crate::{grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

fn decode_plane_into(
    reader: &mut Reader<'_>,
    out: &mut [u8],
    plane_len: usize,
    stride: usize,
    offset: usize,
) -> Result<()> {
    let mut rem = plane_len;
    let mut pos = 0usize;
    while rem > 0 {
        let flags = reader.u8()?;
        for bit in 0..8 {
            if rem == 0 {
                break;
            }
            if flags & (1 << bit) != 0 {
                out[pos * stride + offset] = reader.u8()?;
                pos += 1;
                rem -= 1;
            } else {
                let b0 = reader.u8()?;
                let b1 = reader.u8()?;
                let length = usize::from(b0 >> 3) + 2;
                let dist = (usize::from(b0 & 0x07) << 8) | usize::from(b1);
                if dist == 0 || dist > pos {
                    return Err(Error::InvalidField {
                        what: "back-reference distance beyond available history",
                        value: dist as u64,
                    });
                }
                if length > rem {
                    return Err(Error::InvalidField {
                        what: "back-reference length exceeds remaining output",
                        value: length as u64,
                    });
                }
                for _ in 0..length {
                    let b = out[(pos - dist) * stride + offset];
                    out[pos * stride + offset] = b;
                    pos += 1;
                    rem -= 1;
                }
            }
        }
    }
    Ok(())
}

/// Decode a single-plane `ws-lzss-bandai-g` stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let mut reader = Reader::new(src);
    let header = reader.u16_le()?;
    if header == 0 {
        return Err(Error::InvalidField {
            what: "lzss-bandai-g output length must be nonzero",
            value: 0,
        });
    }
    if header & 0x8000 != 0 {
        let n = usize::from(header & 0x7fff);
        if n == 0 {
            return Err(Error::InvalidField {
                what: "lzss-bandai-g raw length must be nonzero",
                value: u64::from(header),
            });
        }
        grow(&[], n, limit)?;
        let bytes = reader.bytes(n)?.to_vec();
        return Ok(DecodeOutcome {
            bytes,
            consumed: reader.pos(),
        });
    }

    let out_len = usize::from(header);
    grow(&[], out_len, limit)?;
    let mut out = vec![0u8; out_len];
    decode_plane_into(&mut reader, &mut out, out_len, 1, 0)?;
    Ok(DecodeOutcome {
        bytes: out,
        consumed: reader.pos(),
    })
}

/// Decode a 2-plane interleaved `ws-lzss-bandai-g` stream, skipping `header_skip` prefix bytes.
pub fn decode_2plane(src: &[u8], header_skip: usize, max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let mut reader = Reader::new(src);
    if header_skip > 0 {
        reader.bytes(header_skip)?;
    }
    let header = reader.u16_le()?;
    if header == 0 {
        return Err(Error::InvalidField {
            what: "lzss-bandai-g 2-plane length must be nonzero",
            value: 0,
        });
    }
    if header & 0x8000 != 0 {
        let n = usize::from(header & 0x7fff);
        if n == 0 {
            return Err(Error::InvalidField {
                what: "lzss-bandai-g 2-plane raw length must be nonzero",
                value: u64::from(header),
            });
        }
        let total = n * 2;
        grow(&[], total, limit)?;
        let bytes = reader.bytes(total)?.to_vec();
        return Ok(DecodeOutcome {
            bytes,
            consumed: reader.pos(),
        });
    }

    let plane_len = usize::from(header);
    let total = plane_len * 2;
    grow(&[], total, limit)?;
    let mut out = vec![0u8; total];
    decode_plane_into(&mut reader, &mut out, plane_len, 2, 0)?;
    decode_plane_into(&mut reader, &mut out, plane_len, 2, 1)?;
    Ok(DecodeOutcome {
        bytes: out,
        consumed: reader.pos(),
    })
}

fn encode_plane(data: &[u8], out: &mut Vec<u8>) {
    let n = data.len();
    let mut pos = 0usize;
    while pos < n {
        let flag_pos = out.len();
        out.push(0);
        let mut flags = 0u8;
        for bit in 0..8 {
            if pos >= n {
                break;
            }
            let max_l = (n - pos).min(33);
            let mut best_len = 1usize;
            let mut best_dist = 0usize;
            if max_l >= 2 && pos > 0 {
                let start = pos.saturating_sub(2047);
                for cand in (start..pos).rev() {
                    if data[cand] != data[pos] || data[cand + 1] != data[pos + 1] {
                        continue;
                    }
                    let mut l = 2usize;
                    while l < max_l && data[cand + l] == data[pos + l] {
                        l += 1;
                    }
                    if l > best_len {
                        best_len = l;
                        best_dist = pos - cand;
                        if best_len == max_l {
                            break;
                        }
                    }
                }
            }
            if best_len >= 2 {
                let b0 = (((best_len - 2) as u8) << 3) | (((best_dist >> 8) & 0x07) as u8);
                let b1 = (best_dist & 0xff) as u8;
                out.push(b0);
                out.push(b1);
                pos += best_len;
            } else {
                flags |= 1 << bit;
                out.push(data[pos]);
                pos += 1;
            }
        }
        out[flag_pos] = flags;
    }
}

/// Deterministic single-plane encoder (supports `1..=32767` bytes).
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    if src.is_empty() || src.len() > 0x7fff {
        return Err(Error::InvalidField {
            what: "lzss-bandai-g input length must be 1..=32767",
            value: src.len() as u64,
        });
    }
    let mut out = Vec::with_capacity(src.len() + 2);
    out.extend_from_slice(&(src.len() as u16).to_le_bytes());
    encode_plane(src, &mut out);
    Ok(out)
}

/// Deterministic 2-plane interleaved encoder (even length `2..=65534`, 0-byte prefix).
pub fn encode_2plane(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    if src.is_empty() || src.len() % 2 != 0 || src.len() / 2 > 0x7fff {
        return Err(Error::InvalidField {
            what: "lzss-bandai-g 2-plane input length must be even and 2..=65534",
            value: src.len() as u64,
        });
    }
    let plane_len = src.len() / 2;
    let even: Vec<u8> = src.iter().step_by(2).copied().collect();
    let odd: Vec<u8> = src.iter().skip(1).step_by(2).copied().collect();
    let mut out = Vec::with_capacity(src.len() + 2);
    out.extend_from_slice(&(plane_len as u16).to_le_bytes());
    encode_plane(&even, &mut out);
    encode_plane(&odd, &mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_and_compressed_single_plane_round_trip() {
        let raw_stream = [0x04, 0x80, b'W', b'S', b'0', b'1', 0xff];
        let raw_dec = decode(&raw_stream, 16).unwrap();
        assert_eq!(raw_dec.bytes, b"WS01");
        assert_eq!(raw_dec.consumed, 6);

        let payload = b"ABABABABAB_01234567_ABABABABAB_01234567".to_vec();
        let encoded = encode(&payload).unwrap();
        let decoded = decode(&encoded, MAX_DECODED).unwrap();
        assert_eq!(decoded.bytes, payload);
        assert_eq!(decoded.consumed, encoded.len());
    }

    #[test]
    fn two_plane_interleaved_with_and_without_prefix() {
        let mut payload = Vec::new();
        for i in 0..64u8 {
            payload.push(i & 0x0f);
            payload.push(0xa0 | (i & 0x07));
        }
        let encoded = encode_2plane(&payload).unwrap();
        let decoded = decode_2plane(&encoded, 0, MAX_DECODED).unwrap();
        assert_eq!(decoded.bytes, payload);
        assert_eq!(decoded.consumed, encoded.len());

        let mut with_prefix = vec![0x11, 0x22];
        with_prefix.extend_from_slice(&encoded);
        let decoded_prefixed = decode_2plane(&with_prefix, 2, MAX_DECODED).unwrap();
        assert_eq!(decoded_prefixed.bytes, payload);
        assert_eq!(decoded_prefixed.consumed, with_prefix.len());
    }

    #[test]
    fn rejects_invalid_headers_and_distances() {
        assert!(decode(&[0x00, 0x00], 16).is_err());
        assert!(decode(&[0x00, 0x80], 16).is_err());
        // Compressed length 4, flag 0x00 (match), dist 0 => error
        assert!(decode(&[0x04, 0x00, 0x00, 0x00, 0x00], 16).is_err());
        // Match length exceeds remaining output
        assert!(decode(&[0x02, 0x00, 0x01, b'A', 0x08, 0x01], 16).is_err());
    }
}
