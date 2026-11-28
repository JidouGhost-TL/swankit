//! Byte-tagged 16-bit little-endian word RLE stream (carrier-bounded or `0x00`-terminated).
//!
//! Each token starts with a control byte `b`:
//! - End of input or `b == 0x00`: end-of-stream terminator (`0x00` consumed if present).
//! - `b & 0x80 != 0`: literal run of `n = b & 0x7f` (`1..=127`) little-endian `u16` words
//!   (`2 * n` bytes copied verbatim). `b == 0x80` (`n == 0`) is rejected.
//! - `1 <= b <= 0x7f`: repeat run — read one little-endian `u16` word and emit it
//!   `b` times (`2 * b` output bytes).
use crate::{grow, DecodeOutcome, MAX_STREAM_DECODED};
use formatkit_core::{Error, Reader, Result};

/// Decode a byte-tagged 16-bit word RLE stream (stops on `0x00` or end of slice).
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_STREAM_DECODED);
    let mut r = Reader::new(src);
    let mut out = Vec::new();

    while r.remaining() > 0 {
        let ctrl = r.u8()?;
        if ctrl == 0 {
            break;
        }
        if ctrl & 0x80 != 0 {
            let count = usize::from(ctrl & 0x7f);
            if count == 0 {
                return Err(Error::InvalidField {
                    what: "rle-word-7b zero-length literal run",
                    value: u64::from(ctrl),
                });
            }
            let byte_len = count * 2;
            grow(&out, byte_len, limit)?;
            out.extend_from_slice(r.bytes(byte_len)?);
        } else {
            let count = usize::from(ctrl);
            let word = r.bytes(2)?;
            let byte_len = count * 2;
            grow(&out, byte_len, limit)?;
            out.reserve(byte_len);
            for _ in 0..count {
                out.extend_from_slice(word);
            }
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Deterministic encoder for even-length byte streams into `rle-word-7b`.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    grow(&[], src.len(), MAX_STREAM_DECODED)?;
    if src.len() % 2 != 0 {
        return Err(Error::InvalidField {
            what: "rle-word-7b input must have even byte length",
            value: src.len() as u64,
        });
    }
    let words: Vec<[u8; 2]> = src.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
    let mut out = Vec::with_capacity(src.len() + 2);
    let mut i = 0usize;

    while i < words.len() {
        let mut run = 1usize;
        while i + run < words.len() && run < 127 && words[i + run] == words[i] {
            run += 1;
        }
        if run >= 2 {
            out.push(run as u8);
            out.extend_from_slice(&words[i]);
            i += run;
        } else {
            let start = i;
            i += 1;
            while i < words.len() && (i - start) < 127 {
                if i + 1 < words.len() && words[i] == words[i + 1] {
                    break;
                }
                i += 1;
            }
            let lit_len = i - start;
            out.push(0x80 | (lit_len as u8));
            for w in &words[start..i] {
                out.extend_from_slice(w);
            }
        }
    }

    out.push(0x00);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_literals_repeats_and_trailing_bytes() {
        let mut raw = Vec::new();
        for w in [
            0x0001u16, 0x0002, 0x0003, 0x0040, 0x0040, 0x0040, 0x0040, 0x8123, 0x0000, 0x0000,
        ] {
            raw.extend_from_slice(&w.to_le_bytes());
        }
        // Long repeat > 127 words
        for _ in 0..200 {
            raw.extend_from_slice(&0x0555u16.to_le_bytes());
        }
        // Long literal run > 127 words
        for i in 0u16..150 {
            raw.extend_from_slice(&(0x1000 + i).to_le_bytes());
        }

        let enc = encode(&raw).unwrap();
        let mut padded = enc.clone();
        padded.extend_from_slice(&[0xff, 0xee]);
        let dec = decode(&padded, MAX_STREAM_DECODED).unwrap();
        assert_eq!(dec.bytes, raw);
        assert_eq!(dec.consumed, enc.len());

        // Also accepts carrier-bounded streams without trailing 0x00 terminator
        let unterminated = &enc[..enc.len() - 1];
        let dec_carrier = decode(unterminated, MAX_STREAM_DECODED).unwrap();
        assert_eq!(dec_carrier.bytes, raw);
        assert_eq!(dec_carrier.consumed, unterminated.len());
    }

    #[test]
    fn rejects_invalid_control_truncation_and_limit() {
        assert!(matches!(
            decode(&[0x80, 0x00], 64),
            Err(Error::InvalidField { .. })
        ));
        assert!(matches!(
            decode(&[0x82, 0x01, 0x02], 64),
            Err(Error::Truncated { .. })
        ));
        assert!(matches!(
            decode(&[0x03, 0x01], 64),
            Err(Error::Truncated { .. })
        ));
        assert!(matches!(
            decode(&[0x05, 0x11, 0x22, 0x00], 8),
            Err(Error::ResourceLimit { .. })
        ));
        assert!(encode(&[1, 2, 3]).is_err());
    }
}
