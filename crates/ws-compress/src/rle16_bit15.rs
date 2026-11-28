//! 16-bit little-endian word RLE with bit 15 selecting repeat runs.
//!
//! Each command is a little-endian `u16`:
//! - `bit 15 == 1`: repeat the next little-endian `u16` word `cmd & 0x7fff` times.
//! - `bit 15 == 0`: copy `cmd` literal little-endian `u16` words verbatim.
//!
//! Stream extent is supplied by the enclosing carrier.
use crate::{grow, DecodeOutcome, MAX_STREAM_DECODED};
use formatkit_core::{Error, Reader, Result};

/// Decode a carrier-bounded 16-bit word RLE stream within an explicit output budget.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    if src.len() % 2 != 0 {
        return Err(Error::Truncated {
            offset: src.len() - 1,
            needed: 2,
            available: 1,
        });
    }
    let limit = max_output.min(MAX_STREAM_DECODED);
    let mut reader = Reader::new(src);
    let mut out = Vec::new();

    while reader.remaining() > 0 {
        let cmd = reader.u16_le()?;
        if cmd & 0x8000 != 0 {
            let count = usize::from(cmd & 0x7fff);
            let word = reader.bytes(2)?;
            let byte_len = count * 2;
            grow(&out, byte_len, limit)?;
            out.reserve(byte_len);
            for _ in 0..count {
                out.extend_from_slice(word);
            }
        } else {
            let count = usize::from(cmd);
            let byte_len = count * 2;
            grow(&out, byte_len, limit)?;
            out.extend_from_slice(reader.bytes(byte_len)?);
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: reader.pos(),
    })
}

/// Deterministic encoder for even-length byte streams into 16-bit bit15 RLE.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    grow(&[], src.len(), MAX_STREAM_DECODED)?;
    if src.len() % 2 != 0 {
        return Err(Error::InvalidField {
            what: "rle16-bit15 input must have even byte length",
            value: src.len() as u64,
        });
    }
    let words: Vec<[u8; 2]> = src.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < words.len() {
        let mut run = 1usize;
        while i + run < words.len() && run < 0x7fff && words[i + run] == words[i] {
            run += 1;
        }
        if run >= 2 {
            let cmd = 0x8000u16 | (run as u16);
            out.extend_from_slice(&cmd.to_le_bytes());
            out.extend_from_slice(&words[i]);
            i += run;
            continue;
        }
        let lit_start = i;
        i += 1;
        while i < words.len() && (i - lit_start) < 0x7fff {
            if i + 1 < words.len() && words[i] == words[i + 1] {
                break;
            }
            i += 1;
        }
        let lit_count = i - lit_start;
        out.extend_from_slice(&(lit_count as u16).to_le_bytes());
        for w in &words[lit_start..i] {
            out.extend_from_slice(w);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_repeats_literals_and_round_trips() {
        // Repeat 3x 0x1234, then 2 literals 0x5678, 0x9abc
        let stream = [
            0x03, 0x80, 0x34, 0x12, // repeat 3 of [0x34, 0x12]
            0x02, 0x00, 0x78, 0x56, 0xbc, 0x9a, // 2 literal words
        ];
        let dec = decode(&stream, 64).unwrap();
        assert_eq!(
            dec.bytes,
            vec![0x34, 0x12, 0x34, 0x12, 0x34, 0x12, 0x78, 0x56, 0xbc, 0x9a]
        );
        assert_eq!(dec.consumed, stream.len());

        let reenc = encode(&dec.bytes).unwrap();
        assert_eq!(reenc, stream);
    }

    #[test]
    fn supports_large_streams_and_rejects_truncation() {
        let mut payload = vec![0xaa, 0x55].repeat(40_000);
        payload.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let enc = encode(&payload).unwrap();
        let dec = decode(&enc, payload.len()).unwrap();
        assert_eq!(dec.bytes, payload);

        assert!(matches!(
            decode(&enc, payload.len() - 2),
            Err(Error::ResourceLimit { .. })
        ));
        assert!(matches!(decode(&[0x01], 16), Err(Error::Truncated { .. })));
        assert!(matches!(
            decode(&[0x03, 0x80], 16),
            Err(Error::Truncated { .. })
        ));
        assert!(matches!(
            decode(&[0x02, 0x00, 0x11, 0x22], 16),
            Err(Error::Truncated { .. })
        ));
    }
}
