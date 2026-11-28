//! Double-byte trigger RLE stream with `u16_le` output-length header.
//!
//! Single-plane (`planes = 1`):
//! - Header: `u16_le out_len` (exact decoded byte count).
//! - Body: maintain `prev: Option<u8> = None`. While `out.len() < out_len`:
//!   - Read `b: u8` and append `b` to `out`.
//!   - If `prev == Some(b)`: read `cnt: u8`, append `cnt` additional copies of `b`.
//!     If `cnt == 0xff`, keep `prev = Some(b)` (so a following `b` continues the run);
//!     otherwise reset `prev = None`.
//!   - Else: set `prev = Some(b)`.
//!
//! Two-plane split-word (`planes = 2`):
//! - Two consecutive single-plane streams decoding equal lengths `N` (low bytes then
//!   high bytes), interleaved into `N` little-endian `u16` words (`lo[i], hi[i]`).
use crate::{grow, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

fn decode_single(src: &[u8], limit: usize) -> Result<DecodeOutcome> {
    let mut r = Reader::new(src);
    let out_len = usize::from(r.u16_le()?);
    grow(&[], out_len, limit)?;
    let mut out = Vec::with_capacity(out_len);
    let mut prev: Option<u8> = None;

    while out.len() < out_len {
        let b = r.u8()?;
        out.push(b);
        if prev == Some(b) {
            let cnt = r.u8()?;
            let extra = usize::from(cnt);
            if out.len().saturating_add(extra) > out_len {
                return Err(Error::InvalidField {
                    what: "rle2-double-byte run exceeds declared output length",
                    value: out.len().saturating_add(extra) as u64,
                });
            }
            out.resize(out.len() + extra, b);
            prev = if cnt == 0xff { Some(b) } else { None };
        } else {
            prev = Some(b);
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Decode a single-plane (`planes = 1`) or two-plane split-word (`planes = 2`)
/// double-byte RLE stream.
pub fn decode(src: &[u8], planes: u8, max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    match planes {
        1 => decode_single(src, limit),
        2 => {
            let lo = decode_single(src, limit)?;
            let total = lo.bytes.len().checked_mul(2).ok_or(Error::ResourceLimit {
                resource: "decoded bytes",
                requested: u64::MAX,
                limit: limit as u64,
            })?;
            grow(&[], total, limit)?;
            let hi = decode_single(&src[lo.consumed..], limit)?;
            if hi.bytes.len() != lo.bytes.len() {
                return Err(Error::InvalidField {
                    what: "rle2-double-byte split-plane length mismatch",
                    value: hi.bytes.len() as u64,
                });
            }
            let mut out = Vec::with_capacity(total);
            for (&l, &h) in lo.bytes.iter().zip(hi.bytes.iter()) {
                out.push(l);
                out.push(h);
            }
            Ok(DecodeOutcome {
                bytes: out,
                consumed: lo.consumed + hi.consumed,
            })
        }
        other => Err(Error::InvalidField {
            what: "rle2-double-byte planes must be 1 or 2",
            value: u64::from(other),
        }),
    }
}

fn encode_single(src: &[u8]) -> Result<Vec<u8>> {
    let out_len = u16::try_from(src.len()).map_err(|_| Error::ResourceLimit {
        resource: "rle2-double-byte plane length",
        requested: src.len() as u64,
        limit: u64::from(u16::MAX),
    })?;
    let mut out = Vec::with_capacity(2 + src.len());
    out.extend_from_slice(&out_len.to_le_bytes());

    let mut i = 0usize;
    while i < src.len() {
        let b = src[i];
        let mut j = i + 1;
        while j < src.len() && src[j] == b {
            j += 1;
        }
        let run = j - i;
        if run == 1 {
            out.push(b);
        } else {
            out.push(b);
            out.push(b);
            let mut rem = run - 2;
            loop {
                if rem >= 255 {
                    out.push(0xff);
                    rem -= 255;
                    if rem > 0 {
                        out.push(b);
                        rem -= 1;
                    } else {
                        break;
                    }
                } else {
                    out.push(rem as u8);
                    break;
                }
            }
        }
        i = j;
    }

    Ok(out)
}

/// Deterministic encoder for single-plane (`planes = 1`) or two-plane split-word
/// (`planes = 2`) double-byte RLE streams.
pub fn encode(src: &[u8], planes: u8) -> Result<Vec<u8>> {
    grow(&[], src.len(), MAX_DECODED)?;
    match planes {
        1 => encode_single(src),
        2 => {
            if src.len() % 2 != 0 {
                return Err(Error::InvalidField {
                    what: "rle2-double-byte 2-plane input must have even length",
                    value: src.len() as u64,
                });
            }
            let half = src.len() / 2;
            let mut lo = Vec::with_capacity(half);
            let mut hi = Vec::with_capacity(half);
            for chunk in src.chunks_exact(2) {
                lo.push(chunk[0]);
                hi.push(chunk[1]);
            }
            let mut out = encode_single(&lo)?;
            out.extend_from_slice(&encode_single(&hi)?);
            Ok(out)
        }
        other => Err(Error::InvalidField {
            what: "rle2-double-byte planes must be 1 or 2",
            value: u64::from(other),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_single_plane_including_long_continuation_runs() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&[0x01, 0x02, 0x02, 0x03]);
        // Exact 257-byte run (2 + 255 -> cnt=0xff, rem=0, followed by different byte)
        raw.extend(std::iter::repeat_n(0xaa, 257));
        raw.push(0x55);
        // 600-byte run (requires multiple 0xff continuations)
        raw.extend(std::iter::repeat_n(0x00, 600));
        raw.extend_from_slice(&[0x7f, 0x7f, 0x7f, 0x80]);

        let enc = encode(&raw, 1).unwrap();
        let mut padded = enc.clone();
        padded.extend_from_slice(&[0xde, 0xad]);
        let dec = decode(&padded, 1, MAX_DECODED).unwrap();
        assert_eq!(dec.bytes, raw);
        assert_eq!(dec.consumed, enc.len());
    }

    #[test]
    fn round_trips_two_plane_split_word() {
        let mut words = Vec::new();
        for i in 0u16..320 {
            let w = if i < 100 {
                0x0200 | (i & 0x0f)
            } else if i < 200 {
                0x1400
            } else {
                (i.wrapping_mul(3) & 0x01ff) | 0x0800
            };
            words.extend_from_slice(&w.to_le_bytes());
        }

        let enc = encode(&words, 2).unwrap();
        let dec = decode(&enc, 2, MAX_DECODED).unwrap();
        assert_eq!(dec.bytes, words);
        assert_eq!(dec.consumed, enc.len());
    }

    #[test]
    fn rejects_invalid_planes_overshoot_and_mismatch() {
        assert!(decode(&[0x01, 0x00, 0x42], 0, 16).is_err());
        assert!(decode(&[0x01, 0x00, 0x42], 3, 16).is_err());
        // Declared length 3, but pair + count 5 overflows
        assert!(matches!(
            decode(&[0x03, 0x00, 0x11, 0x11, 0x05], 1, 16),
            Err(Error::InvalidField { .. })
        ));
        // Two-plane with mismatched plane lengths (2 vs 1)
        assert!(matches!(
            decode(&[0x02, 0x00, 0x01, 0x02, 0x01, 0x00, 0x03], 2, 16),
            Err(Error::InvalidField { .. })
        ));
        // Odd length for 2-plane encode
        assert!(encode(&[1, 2, 3], 2).is_err());
    }
}
