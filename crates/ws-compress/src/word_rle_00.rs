//! 16-bit little-endian word RLE with bit 15 selecting single 15-bit literal words.
//!
//! Each token starts with a little-endian `u16` word `w`:
//! - `w & 0x8000 != 0`: emit one little-endian 16-bit word `w & 0x7fff`.
//! - `w & 0x8000 == 0`: read the next little-endian `u16` word `val` and emit `val`
//!   repeated `w` times (`w` must be non-zero).
//!
//! Streams may be bounded either by an explicit `decoded_size` (in bytes, even) or by the
//! enclosing carrier slice.
use crate::{grow, DecodeOutcome, MAX_STREAM_DECODED};
use formatkit_core::{Error, Reader, Result, Writer};

/// Decode a 16-bit word-RLE stream (bit 15 literal, bit 15 clear repeat run).
pub fn decode(
    src: &[u8],
    decoded_size: Option<usize>,
    max_output: usize,
) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_STREAM_DECODED);
    if let Some(target) = decoded_size {
        if target % 2 != 0 {
            return Err(Error::InvalidField {
                what: "word-rle-00 decoded_size must be even",
                value: target as u64,
            });
        }
        grow(&[], target, limit)?;
        if target == 0 {
            return Ok(DecodeOutcome {
                bytes: Vec::new(),
                consumed: 0,
            });
        }
    } else if src.len() % 2 != 0 {
        return Err(Error::Truncated {
            offset: src.len() - 1,
            needed: 2,
            available: 1,
        });
    }

    let mut r = Reader::new(src);
    let mut out = decoded_size.map_or_else(Vec::new, Vec::with_capacity);

    loop {
        if let Some(target) = decoded_size {
            if out.len() >= target {
                break;
            }
        } else if r.remaining() == 0 {
            break;
        }

        let w = r.u16_le()?;
        if w & 0x8000 != 0 {
            let val = w & 0x7fff;
            grow(&out, 2, limit)?;
            out.extend_from_slice(&val.to_le_bytes());
        } else {
            let count = usize::from(w);
            if count == 0 {
                return Err(Error::InvalidField {
                    what: "word-rle-00 zero repeat count",
                    value: 0,
                });
            }
            let val = r.u16_le()?;
            let mut byte_len = count * 2;
            if let Some(target) = decoded_size {
                let rem = target - out.len();
                if byte_len > rem {
                    byte_len = rem;
                }
            }
            grow(&out, byte_len, limit)?;
            out.reserve(byte_len);
            let val_bytes = val.to_le_bytes();
            for _ in 0..(byte_len / 2) {
                out.extend_from_slice(&val_bytes);
            }
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Deterministic encoder for even-length byte streams into `word-rle-00`.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    grow(&[], src.len(), MAX_STREAM_DECODED)?;
    if src.len() % 2 != 0 {
        return Err(Error::InvalidField {
            what: "word-rle-00 input must have even byte length",
            value: src.len() as u64,
        });
    }
    let words: Vec<u16> = src
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let mut w = Writer::new();
    let mut i = 0usize;

    while i < words.len() {
        let val = words[i];
        let mut run = 1usize;
        while i + run < words.len() && run < 0x7fff && words[i + run] == val {
            run += 1;
        }
        if run >= 2 || (val & 0x8000) != 0 {
            w.u16_le(run as u16);
            w.u16_le(val);
            i += run;
        } else {
            w.u16_le(val | 0x8000);
            i += 1;
        }
    }
    Ok(w.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_literals_repeats_and_high_bit_words() {
        let words: [u16; 7] = [0x0123, 0x0000, 0x0000, 0x0000, 0x8042, 0x1234, 0x1234];
        let mut payload = Vec::new();
        for w in words {
            payload.extend_from_slice(&w.to_le_bytes());
        }

        let enc = encode(&payload).unwrap();
        let dec_carrier = decode(&enc, None, 64).unwrap();
        assert_eq!(dec_carrier.bytes, payload);
        assert_eq!(dec_carrier.consumed, enc.len());

        let dec_sized = decode(&enc, Some(payload.len()), 64).unwrap();
        assert_eq!(dec_sized.bytes, payload);
        assert_eq!(dec_sized.consumed, enc.len());
    }

    #[test]
    fn rejects_zero_count_and_truncation() {
        assert!(matches!(
            decode(&[0x00, 0x00, 0x11, 0x22], None, 64),
            Err(Error::InvalidField { .. })
        ));
        assert!(matches!(
            decode(&[0x03, 0x00], None, 64),
            Err(Error::Truncated { .. })
        ));
        assert!(matches!(
            decode(&[0x01], None, 64),
            Err(Error::Truncated { .. })
        ));
        assert!(matches!(
            decode(&[], Some(3), 64),
            Err(Error::InvalidField { .. })
        ));
    }
}
