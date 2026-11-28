//! 16-bit little-endian zero-run RLE stream.
//!
//! Grammar:
//! - Stream of little-endian `u16` words (caller supplies the stored byte slice).
//! - Non-zero `u16` word: emitted verbatim (2 bytes LE).
//! - `0x0000` followed by `u16` LE `count`: emits `count` zero `u16` words (`count * 2` zero bytes).
use crate::{grow, DecodeOutcome, MAX_STREAM_DECODED};
use formatkit_core::{Error, Reader, Result};

/// Decode an entire zero-RLE16 word stream up to `max_output` bytes.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    if src.len() % 2 != 0 {
        return Err(Error::Malformed(
            "zero-rle16 stream length must be even".into(),
        ));
    }
    let limit = max_output.min(MAX_STREAM_DECODED);
    let mut r = Reader::new(src);
    let mut out = Vec::new();
    while r.remaining() > 0 {
        let word = r.u16_le()?;
        if word == 0 {
            let count = usize::from(r.u16_le()?);
            let zero_bytes = count.checked_mul(2).ok_or(Error::ResourceLimit {
                resource: "decoded bytes",
                requested: u64::MAX,
                limit: limit as u64,
            })?;
            grow(&out, zero_bytes, limit)?;
            out.resize(out.len() + zero_bytes, 0);
        } else {
            grow(&out, 2, limit)?;
            out.extend_from_slice(&word.to_le_bytes());
        }
    }
    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Deterministic encoder for 16-bit little-endian zero-run RLE.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    if src.len() % 2 != 0 {
        return Err(Error::Malformed(
            "zero-rle16 payload length must be even".into(),
        ));
    }
    grow(&[], src.len(), MAX_STREAM_DECODED)?;
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let w = u16::from_le_bytes([src[i], src[i + 1]]);
        if w == 0 {
            let mut run = 0usize;
            while i < src.len() && src[i] == 0 && src[i + 1] == 0 && run < u16::MAX as usize {
                run += 1;
                i += 2;
            }
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&(run as u16).to_le_bytes());
        } else {
            out.extend_from_slice(&w.to_le_bytes());
            i += 2;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_zero_runs_and_literals() {
        let cases: &[Vec<u8>] = &[
            vec![],
            vec![0; 672],
            vec![0x12, 0x34, 0x00, 0x00, 0x00, 0x00, 0x56, 0x78],
            (0..336)
                .flat_map(|i| {
                    if i % 5 == 0 {
                        0u16.to_le_bytes()
                    } else {
                        ((i * 17) as u16).to_le_bytes()
                    }
                })
                .collect(),
        ];
        for src in cases {
            let enc = encode(src).unwrap();
            let dec = decode(&enc, src.len().max(2)).unwrap();
            assert_eq!(dec.bytes, *src);
            assert_eq!(dec.consumed, enc.len());
        }
    }

    #[test]
    fn rejects_odd_length_truncation_and_limits() {
        assert!(decode(&[0], 64).is_err());
        assert!(decode(&[0, 0], 64).is_err());
        assert!(decode(&[0, 0, 10, 0], 18).is_err());
        assert!(encode(&[1, 2, 3]).is_err());
    }
}
