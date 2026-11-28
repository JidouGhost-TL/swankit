//! 4-mode byte-command RLE/LZ stream.
//!
//! Each command byte `c` selects one of four modes via its top two bits (`c >> 6`)
//! with `count = c & 0x3f` (`1..=63`; zero count is invalid):
//! - `0x01..=0x3f`: copy `count` literal bytes.
//! - `0x41..=0x7f`: repeat the next byte `count` times.
//! - `0x81..=0xbf`: LZ back-reference of `count` bytes with 1-byte distance `u8 + 1` (`1..=256`).
//! - `0xc1..=0xff`: LZ back-reference of `count` bytes with 2-byte LE distance `u16_le + 1` (`1..=65536`).
//!
//! Stream extent is supplied by the enclosing carrier.
use crate::{copy_match, grow, DecodeOutcome, MAX_STREAM_DECODED};
use formatkit_core::{Error, Reader, Result, Writer};

const MAX_CMD_COUNT: usize = 63;

/// Decode a carrier-bounded 4-mode byte-command RLE/LZ stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_STREAM_DECODED);
    let mut r = Reader::new(src);
    let mut out = Vec::new();

    while r.remaining() > 0 {
        let cmd = r.u8()?;
        let mode = cmd >> 6;
        let count = usize::from(cmd & 0x3f);
        if count == 0 {
            return Err(Error::InvalidField {
                what: "rle-lz4-cmd zero-count command",
                value: u64::from(cmd),
            });
        }
        match mode {
            0 => {
                grow(&out, count, limit)?;
                out.extend_from_slice(r.bytes(count)?);
            }
            1 => {
                let fill = r.u8()?;
                grow(&out, count, limit)?;
                out.resize(out.len() + count, fill);
            }
            2 => {
                let dist = usize::from(r.u8()?) + 1;
                copy_match(&mut out, &[], dist, count, limit)?;
            }
            3 => {
                let dist = usize::from(r.u16_le()?) + 1;
                copy_match(&mut out, &[], dist, count, limit)?;
            }
            _ => unreachable!(),
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Deterministic encoder for 4-mode byte-command RLE/LZ streams.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    grow(&[], src.len(), MAX_STREAM_DECODED)?;
    let mut w = Writer::new();
    let mut pos = 0usize;
    let mut lit_start = 0usize;

    let flush_literals = |w: &mut Writer, src: &[u8], start: usize, end: usize| {
        let mut p = start;
        while p < end {
            let chunk = (end - p).min(MAX_CMD_COUNT);
            w.u8(chunk as u8);
            w.bytes(&src[p..p + chunk]);
            p += chunk;
        }
    };

    while pos < src.len() {
        // 1. Check byte RLE run
        let mut rle = 1usize;
        while pos + rle < src.len() && rle < MAX_CMD_COUNT && src[pos + rle] == src[pos] {
            rle += 1;
        }

        // 2. Check LZ back-reference (prefer 1-byte dist if tied)
        let max_len = (src.len() - pos).min(MAX_CMD_COUNT);
        let max_dist = pos.min(65536);
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        if max_len >= 3 && max_dist >= 1 {
            // Search near window (1..=256) first
            let near_max = max_dist.min(256);
            for dist in 1..=near_max {
                let mut l = 0usize;
                while l < max_len && src[pos - dist + l] == src[pos + l] {
                    l += 1;
                }
                if l >= 3 && l > best_len {
                    best_len = l;
                    best_dist = dist;
                    if best_len == max_len {
                        break;
                    }
                }
            }
            // Search far window (257..=65536) only if strictly better by >= 2 bytes
            if best_len < max_len && max_dist > 256 {
                let far_limit = max_dist.min(4096);
                for dist in 257..=far_limit {
                    let mut l = 0usize;
                    while l < max_len && src[pos - dist + l] == src[pos + l] {
                        l += 1;
                    }
                    if l >= 4 && l > best_len + 1 {
                        best_len = l;
                        best_dist = dist;
                        if best_len == max_len {
                            break;
                        }
                    }
                }
            }
        }

        if rle >= 3 && rle >= best_len {
            flush_literals(&mut w, src, lit_start, pos);
            w.u8(0x40 | (rle as u8));
            w.u8(src[pos]);
            pos += rle;
            lit_start = pos;
        } else if best_len >= 3 && (best_dist <= 256 || best_len >= 4) {
            flush_literals(&mut w, src, lit_start, pos);
            if best_dist <= 256 {
                w.u8(0x80 | (best_len as u8));
                w.u8((best_dist - 1) as u8);
            } else {
                w.u8(0xc0 | (best_len as u8));
                w.u16_le((best_dist - 1) as u16);
            }
            pos += best_len;
            lit_start = pos;
        } else {
            pos += 1;
        }
    }
    flush_literals(&mut w, src, lit_start, pos);

    Ok(w.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_all_four_modes_and_round_trips() {
        let mut payload = Vec::new();
        payload.extend_from_slice(b"HEADER_0123456789");
        payload.extend_from_slice(&[0x77; 20]);
        payload.extend_from_slice(b"HEADER_0123456789");
        payload.extend(0u8..=255u8);
        payload.extend_from_slice(&[0xaa; 30]);
        payload.extend_from_slice(b"HEADER_0123456789");

        let enc = encode(&payload).unwrap();
        assert!(enc.len() < payload.len());
        let dec = decode(&enc, payload.len()).unwrap();
        assert_eq!(dec.bytes, payload);
        assert_eq!(dec.consumed, enc.len());
    }

    #[test]
    fn rejects_zero_count_and_oob_distance() {
        for bad in [0x00u8, 0x40, 0x80, 0xc0] {
            assert!(matches!(
                decode(&[bad], 64),
                Err(Error::InvalidField { .. })
            ));
        }
        // LZ1 distance > output
        assert!(matches!(
            decode(&[0x83, 0x00], 64),
            Err(Error::InvalidField { .. })
        ));
        // LZ2 distance > output
        assert!(matches!(
            decode(&[0x02, 0x11, 0x22, 0xc3, 0x05, 0x00], 64),
            Err(Error::InvalidField { .. })
        ));
        // Truncated literal
        assert!(matches!(
            decode(&[0x05, 0x01, 0x02], 64),
            Err(Error::Truncated { .. })
        ));
    }
}
