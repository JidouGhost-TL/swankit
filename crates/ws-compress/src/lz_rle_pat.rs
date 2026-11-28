//! Byte-tagged literal, pattern-repeat, 10-bit LZ77, and RLE fill stream.
//!
//! Output size is supplied by the enclosing carrier. Each command byte `b`
//! selects one of four operations based on its top two bits:
//! - `0x00..=0x3f`: literal run of `((b & 0x1f) | (ext << 5)) + 1` bytes, where
//!   an extra high byte `ext` is read when `b & 0x20 != 0`.
//! - `0x40..=0x7f`: pattern repeat of length `(b & 0x3f) + 2` repeated
//!   `next_byte + 1` times.
//! - `0x80..=0xbf`: LZ77 back-reference of length `((b & 0x3c) >> 2) + 2`
//!   at distance `(((b & 0x03) << 8) | next_byte) + 1`.
//! - `0xc0..=0xff`: RLE byte fill of `((b & 0x1f) | (ext << 5)) + 2` copies of
//!   `fill_byte`, reading `ext` first when `b & 0x20 != 0`.
use crate::{copy_match, grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Reader, Result};

/// Decode a carrier-sized stream within an explicit output budget.
pub fn decode(src: &[u8], decoded_size: usize, max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    grow(&[], decoded_size, limit)?;
    let mut reader = Reader::new(src);
    let mut out = Vec::with_capacity(decoded_size);
    let mut rem = decoded_size;

    while rem > 0 {
        let b = reader.u8()?;
        match b & 0xc0 {
            0x00 => {
                let mut cnt = usize::from(b & 0x1f);
                if b & 0x20 != 0 {
                    cnt |= usize::from(reader.u8()?) << 5;
                }
                cnt += 1;
                let take = cnt.min(rem);
                out.extend_from_slice(reader.bytes(take)?);
                rem -= take;
            }
            0x40 => {
                let pat_len = usize::from(b & 0x3f) + 2;
                let reps = usize::from(reader.u8()?) + 1;
                let pat = reader.bytes(pat_len)?;
                for _ in 0..reps {
                    if rem == 0 {
                        break;
                    }
                    let take = pat_len.min(rem);
                    out.extend_from_slice(&pat[..take]);
                    rem -= take;
                }
            }
            0x80 => {
                let cnt = usize::from((b & 0x3c) >> 2) + 2;
                let lo = reader.u8()?;
                let dist = ((usize::from(b & 0x03) << 8) | usize::from(lo)) + 1;
                let take = cnt.min(rem);
                copy_match(&mut out, &[], dist, take, limit)?;
                rem -= take;
            }
            _ => {
                let mut cnt = usize::from(b & 0x1f);
                if b & 0x20 != 0 {
                    cnt |= usize::from(reader.u8()?) << 5;
                }
                cnt += 2;
                let val = reader.u8()?;
                let take = cnt.min(rem);
                out.resize(out.len() + take, val);
                rem -= take;
            }
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: reader.pos(),
    })
}

/// Deterministic encoder producing RLE fills, 10-bit LZ77 matches, and literal runs.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut lit_start = 0usize;

    let flush_literals = |out: &mut Vec<u8>, src: &[u8], start: usize, end: usize| {
        let mut p = start;
        while p < end {
            let chunk = (end - p).min(8192);
            let n = chunk - 1;
            if n < 32 {
                out.push(n as u8);
            } else {
                out.push(0x20 | ((n & 0x1f) as u8));
                out.push((n >> 5) as u8);
            }
            out.extend_from_slice(&src[p..p + chunk]);
            p += chunk;
        }
    };

    while pos < src.len() {
        // Check RLE run of identical bytes (min length 3 to beat literal overhead)
        let mut rle_len = 1usize;
        while pos + rle_len < src.len() && rle_len < 8193 && src[pos + rle_len] == src[pos] {
            rle_len += 1;
        }

        // Check best 10-bit LZ match (len 2..=17, dist 1..=1024)
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        let max_match = (src.len() - pos).min(17);
        if max_match >= 3 && pos > 0 {
            let win_start = pos.saturating_sub(1024);
            for cand in (win_start..pos).rev() {
                if src[cand] != src[pos] || src[cand + 1] != src[pos + 1] {
                    continue;
                }
                let mut l = 2usize;
                while l < max_match && src[cand + l] == src[pos + l] {
                    l += 1;
                }
                if l > best_len {
                    best_len = l;
                    best_dist = pos - cand;
                    if best_len == max_match {
                        break;
                    }
                }
            }
        }

        if rle_len >= 3 && rle_len >= best_len {
            flush_literals(&mut out, src, lit_start, pos);
            let n = rle_len - 2;
            if n < 32 {
                out.push(0xc0 | (n as u8));
            } else {
                out.push(0xe0 | ((n & 0x1f) as u8));
                out.push((n >> 5) as u8);
            }
            out.push(src[pos]);
            pos += rle_len;
            lit_start = pos;
        } else if best_len >= 3 {
            flush_literals(&mut out, src, lit_start, pos);
            let d = best_dist - 1;
            let b0 = 0x80 | (((best_len - 2) as u8) << 2) | ((d >> 8) as u8);
            let b1 = (d & 0xff) as u8;
            out.push(b0);
            out.push(b1);
            pos += best_len;
            lit_start = pos;
        } else {
            pos += 1;
        }
    }
    flush_literals(&mut out, src, lit_start, src.len());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use formatkit_core::Error;

    #[test]
    fn decodes_all_four_command_families() {
        // 1) literal 3 bytes: 0x02 "ABC"
        // 2) pattern len 2, reps 3: 0x40 0x02 "XY" -> "XYXYXY"
        // 3) LZ backref len 4, dist 2: 0x88 0x01 -> "XYXY"
        // 4) RLE fill len 5 of 'Z': 0xc3 'Z' -> "ZZZZZ"
        let stream = [
            0x02, b'A', b'B', b'C', 0x40, 0x02, b'X', b'Y', 0x88, 0x01, 0xc3, b'Z', 0xee,
        ];
        let got = decode(&stream, 18, 64).unwrap();
        assert_eq!(got.bytes, b"ABCXYXYXYXYXYZZZZZ");
        assert_eq!(got.consumed, 12);
    }

    #[test]
    fn extended_counts_and_round_trip() {
        let mut payload = vec![0x5a; 400];
        payload.extend_from_slice(b"pattern_block_0123456789");
        payload.extend_from_slice(b"pattern_block_0123456789");
        payload.extend((0..120u8).map(|i| i.wrapping_mul(13)));
        let encoded = encode(&payload).unwrap();
        let decoded = decode(&encoded, payload.len(), MAX_DECODED).unwrap();
        assert_eq!(decoded.bytes, payload);
        assert_eq!(decoded.consumed, encoded.len());

        assert!(matches!(
            decode(&encoded, payload.len(), payload.len() - 1),
            Err(Error::ResourceLimit { .. })
        ));
        assert!(matches!(
            decode(&[0x80, 0x00], 2, 16),
            Err(Error::InvalidField { .. })
        ));
        assert!(matches!(decode(&[0x05, 1, 2], 6, 16), Err(Error::Truncated { .. })));
    }
}
