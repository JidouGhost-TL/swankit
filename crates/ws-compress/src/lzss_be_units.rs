//! Explicit BE unit-count carrier and LSB literal flags with direct BE distance.
//! Conversion is bounded and feeds the existing externally sized decoder.
use crate::{grow, lzss_12, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

pub struct Normalized {
    pub bytes: Vec<u8>,
    pub stored_consumed: usize,
    pub decoded_size: usize,
    pub header_count: u16,
}
fn invalid(code: &str) -> Error {
    Error::Malformed(format!("token.{code}"))
}
pub fn normalize(
    src: &[u8],
    unit_size: u8,
    source_width: u8,
    max_output: usize,
) -> Result<Normalized> {
    if !matches!(unit_size, 16 | 32) {
        return Err(invalid("invalid-unit"));
    }
    if !matches!(source_width, 16 | 32) {
        return Err(invalid("invalid-width"));
    }
    let mut r = Reader::new(src);
    let header_count = r.u16_be()?;
    if header_count == 0 {
        return Err(invalid("zero-header"));
    }
    let size = usize::from(header_count) * usize::from(unit_size);
    if source_width == 16 && size > 65535 {
        return Err(invalid("source-width-overflow"));
    }
    grow(&[], size, max_output.min(MAX_DECODED))?;
    let mut bytes = Vec::new();
    let mut produced = 0;
    while produced < size {
        let flags = r.u8()?;
        bytes.push(flags);
        for bit in 0..8 {
            if produced == size {
                break;
            }
            if flags & (1 << bit) != 0 {
                bytes.push(r.u8()?);
                produced += 1;
            } else {
                let first = usize::from(r.u8()?);
                let second = usize::from(r.u8()?);
                let distance = (first << 4) | (second >> 4);
                let count = (second & 15) + 3;
                if distance == 0 {
                    return Err(invalid("zero-distance"));
                }
                if distance > produced {
                    return Err(invalid("unknown-history"));
                }
                if count > size - produced {
                    return Err(invalid("token-overshoot"));
                }
                let token = (((count - 3) << 12) | (distance - 1)) as u16;
                bytes.extend_from_slice(&token.to_le_bytes());
                produced += count;
            }
        }
    }
    Ok(Normalized {
        bytes,
        stored_consumed: r.pos(),
        decoded_size: size,
        header_count,
    })
}
pub fn decode(
    src: &[u8],
    unit_size: u8,
    source_width: u8,
    max_output: usize,
) -> Result<DecodeOutcome> {
    let n = normalize(src, unit_size, source_width, max_output)?;
    let mut decoded = lzss_12::decode(&n.bytes, n.decoded_size, max_output)?;
    if decoded.consumed != n.bytes.len() {
        return Err(invalid("normalized-extent"));
    }
    decoded.consumed = n.stored_consumed;
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn literals(size: usize, unit: u8) -> Vec<u8> {
        let mut b = ((size / usize::from(unit)) as u16).to_be_bytes().to_vec();
        for _ in 0..size / 8 {
            b.push(255);
            b.extend_from_slice(b"abcdefgh");
        }
        b
    }
    #[test]
    fn flags_overlap_suffix_and_partial_flag() {
        let mut b = vec![0, 1, 1, b'A', 0, 29]; // literal then distance1/count16 -> overshoot
        assert!(decode(&b, 16, 16, 16).is_err());
        b[5] = 28; // count15, exactly 16 output; six unused flag bits
        b.extend_from_slice(&[0xde, 0xad]);
        let n = normalize(&b, 16, 16, 16).unwrap();
        let got = decode(&b, 16, 16, 16).unwrap();
        assert_eq!(got.bytes, vec![b'A'; 16]);
        assert_eq!(got.consumed, 6);
        assert_eq!(n.bytes, vec![1, b'A', 0, 192]);
        assert_eq!(n.stored_consumed, 6);
        for cut in 0..6 {
            assert!(decode(&b[..cut], 16, 16, 16).is_err());
        }
    }
    #[test]
    fn limits_and_refusals() {
        for (src, u, w, cap) in [
            (&[0, 0][..], 16, 16, 16),
            (&[0, 1, 0, 0, 0], 16, 16, 16),
            (&[0, 1, 0, 0, 16], 16, 16, 16),
            (&[0, 1], 8, 16, 16),
            (&[0, 1], 16, 8, 16),
            (&[0, 1], 16, 16, 15),
        ] {
            assert!(decode(src, u, w, cap).is_err());
        }
        let max = literals(65536, 16);
        assert!(decode(&max, 16, 16, 65536).is_err());
        assert_eq!(decode(&max, 16, 32, 65536).unwrap().bytes.len(), 65536);
        assert!(decode(&max, 16, 32, 65535).is_err());
        assert_eq!(
            decode(&literals(65520, 16), 16, 16, 65536)
                .unwrap()
                .bytes
                .len(),
            65520
        );
        assert!(decode(&[0x10, 1], 16, 32, usize::MAX).is_err());
    }
}
