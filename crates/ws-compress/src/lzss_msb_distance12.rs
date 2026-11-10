//! MSB flag bytes and little-endian distance12/length4 backward tokens.
//! The enclosing definition supplies the output size and any known history.
use crate::{copy_match, grow, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Reader, Result};

pub fn decode(src: &[u8], decoded_size: usize, max_output: usize) -> Result<DecodeOutcome> {
    decode_with_history(src, &[], decoded_size, max_output)
}

pub fn decode_with_history(
    src: &[u8],
    history: &[u8],
    decoded_size: usize,
    max_output: usize,
) -> Result<DecodeOutcome> {
    grow(&[], decoded_size, max_output.min(MAX_DECODED))?;
    let mut reader = Reader::new(src);
    let mut out = Vec::with_capacity(decoded_size);
    while out.len() < decoded_size {
        let flags = reader.u8()?;
        for bit in (0..8).rev() {
            let remaining = decoded_size - out.len();
            if remaining == 0 {
                break;
            }
            if flags & (1 << bit) == 0 {
                out.push(reader.u8()?);
            } else {
                let token = usize::from(reader.u16_le()?);
                let distance = (token >> 4) + 1;
                let count = ((token & 0xf) + 3).min(remaining);
                copy_match(&mut out, history, distance, count, decoded_size)?;
            }
        }
    }
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
    fn flags_tokens_overlap_and_stop() {
        let mixed = decode(&[0x20, b'A', b'B', 0x10, 0], 5, 5).unwrap();
        assert_eq!(mixed.bytes, b"ABABA");
        assert_eq!(mixed.consumed, 5);

        let clipped = decode(&[0x40, b'A', 3, 0, 0xde], 5, 5).unwrap();
        assert_eq!(clipped.bytes, b"AAAAA");
        assert_eq!(clipped.consumed, 4);

        let refill = decode(b"\x00abcdefgh\x00i", 9, 9).unwrap();
        assert_eq!(refill.bytes, b"abcdefghi");
        assert_eq!(refill.consumed, 11);
        assert!(decode(b"\x00abcdefghi", 9, 9).is_err());
    }

    #[test]
    fn explicit_history_and_maximum_distance() {
        assert!(matches!(
            decode(&[0x80, 0, 0], 3, 3),
            Err(Error::InvalidField { .. })
        ));
        let overlapping = decode_with_history(&[0x80, 0, 0], b"Q", 3, 3).unwrap();
        assert_eq!(overlapping.bytes, b"QQQ");
        assert_eq!(overlapping.consumed, 3);

        let mut history = vec![0; 4096];
        history[..3].copy_from_slice(b"XYZ");
        let maximum = decode_with_history(&[0x80, 0xf0, 0xff], &history, 3, 3).unwrap();
        assert_eq!(maximum.bytes, b"XYZ");
        assert!(decode_with_history(&[0x80, 0xf0, 0xff], &history[1..], 3, 3).is_err());
    }

    #[test]
    fn rejects_truncation_and_size_limits() {
        for src in [&[][..], &[0][..], &[0x80][..], &[0x80, 0][..]] {
            assert!(decode(src, 1, 1).is_err());
        }
        assert!(matches!(
            decode(&[], 1, 0),
            Err(Error::ResourceLimit { .. })
        ));
        assert!(matches!(
            decode(&[], MAX_DECODED + 1, usize::MAX),
            Err(Error::ResourceLimit { .. })
        ));
        let empty = decode(&[0x80], 0, 0).unwrap();
        assert!(empty.bytes.is_empty());
        assert_eq!(empty.consumed, 0);
    }
}
