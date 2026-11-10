//! Decode-only groups with a fill byte and a sixteen-bit literal mask.
//!
//! A nonzero byte count selects groups. Each group stores a fill byte followed
//! by a little-endian mask. Bits are consumed from the low byte, most
//! significant first, then the high byte, most significant first. Set bits
//! emit the fill byte; clear bits consume and emit one literal.
use crate::{grow, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

/// Read the group-count header and return its exact decoded size.
///
/// A zero count is invalid for this bounded decoder. In the source routine it
/// underflows the 16-bit loop counter and requests 65,536 groups.
pub fn decoded_size(src: &[u8]) -> Result<usize> {
    let mut reader = Reader::new(src);
    let groups = reader.u8()?;
    if groups == 0 {
        return Err(Error::InvalidField {
            what: "mask16-fill group count must be nonzero",
            value: 0,
        });
    }
    Ok(usize::from(groups) * 16)
}

/// Decode a count-prefixed stream within an explicit output budget.
///
/// The count byte requests 16 output bytes per group. Zero counts are refused
/// before allocation or payload reads. Trailing bytes are left unconsumed.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let decoded = decoded_size(src)?;
    let limit = max_output.min(MAX_DECODED);
    grow(&[], decoded, limit)?;

    let mut reader = Reader::new(src);
    let groups = reader.u8()?;
    let mut out = Vec::with_capacity(decoded);
    for _ in 0..groups {
        let fill = reader.u8()?;
        let mask = reader.u16_le()?;
        let low = mask as u8;
        let high = (mask >> 8) as u8;
        for byte in [low, high] {
            for bit in (0..8).rev() {
                if byte & (1 << bit) != 0 {
                    out.push(fill);
                } else {
                    out.push(reader.u8()?);
                }
            }
        }
    }

    debug_assert_eq!(out.len(), decoded);
    Ok(DecodeOutcome {
        bytes: out,
        consumed: reader.pos(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use formatkit_core::Error;

    #[test]
    fn low_then_high_masks_use_msb_first_and_one_means_fill() {
        // DL=01 fills output 7; DH=80 fills output 8. Clear bits are literals.
        let mut stream = vec![1, 0xee, 0x01, 0x80];
        stream.extend(0..7);
        stream.extend(7..14);
        stream.push(0xaa); // trailing bytes remain outside this stream
        let decoded = decode(&stream, 16).unwrap();
        let mut expected: Vec<u8> = (0..7).collect();
        expected.extend_from_slice(&[0xee, 0xee]);
        expected.extend(7..14);
        assert_eq!(decoded.bytes, expected);
        assert_eq!(decoded.consumed, 18);
    }

    #[test]
    fn fill_is_read_per_group_and_literals_can_equal_controls() {
        let literals: Vec<u8> = (0..16).map(|i| if i == 0 { 0 } else { 0xff }).collect();
        let mut stream = vec![2, 0xa5, 0xff, 0xff, 0x5a, 0, 0];
        stream.extend_from_slice(&literals);
        let decoded = decode(&stream, 32).unwrap();
        let mut expected = vec![0xa5; 16];
        expected.extend_from_slice(&literals);
        assert_eq!(decoded.bytes, expected);
        assert_eq!(decoded.consumed, 23);
    }

    #[test]
    fn maximum_count_and_exact_input_consumption() {
        let mut stream = vec![u8::MAX];
        for _ in 0..u8::MAX {
            stream.extend_from_slice(&[0x37, 0xff, 0xff]);
        }
        stream.push(0x99);
        let decoded = decode(&stream, 4080).unwrap();
        assert_eq!(decoded.bytes, vec![0x37; 4080]);
        assert_eq!(decoded.consumed, 1 + 3 * usize::from(u8::MAX));
    }

    #[test]
    fn zero_count_and_output_limit_refuse_before_payload() {
        assert!(matches!(
            decode(&[0], MAX_DECODED),
            Err(Error::InvalidField { .. })
        ));
        assert!(matches!(
            decode(&[1], 15),
            Err(Error::ResourceLimit {
                requested: 16,
                limit: 15,
                ..
            })
        ));
        assert_eq!(decoded_size(&[1]).unwrap(), 16);
    }

    #[test]
    fn truncated_count_group_header_and_literals_are_checked() {
        assert!(matches!(decode(&[], 16), Err(Error::Truncated { .. })));
        assert!(matches!(decode(&[1], 16), Err(Error::Truncated { .. })));
        assert!(matches!(
            decode(&[1, 0x44], 16),
            Err(Error::Truncated { .. })
        ));
        assert!(matches!(
            decode(&[1, 0x44, 0, 0], 16),
            Err(Error::Truncated { .. })
        ));
        let mut short_literals = vec![1, 0x44, 0, 0];
        short_literals.extend(0..15);
        assert!(matches!(
            decode(&short_literals, 16),
            Err(Error::Truncated { .. })
        ));
    }
}
