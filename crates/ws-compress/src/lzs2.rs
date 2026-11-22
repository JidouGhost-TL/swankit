//! LZS2 framing around the MSB-first distance-12/length-4 stream.
use crate::{lzss_msb_distance12, DecodeOutcome};
use formatkit_core::{Error, Reader, Result};

const HEADER_SIZE: usize = 16;

/// Decode one container. The returned extent includes the header and token bytes.
pub fn decode(source: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let mut reader = Reader::new(source);
    if reader.bytes(4)? != b"LZS2" {
        return Err(Error::Malformed("invalid LZS2 signature".into()));
    }
    let decoded_size = usize::from(reader.u16_le()?);
    reader.skip(2)?;
    let stored_size = usize::from(reader.u16_le()?);
    reader.skip(HEADER_SIZE - 10)?;
    let tokens = reader.bytes(stored_size)?;
    let mut result = lzss_msb_distance12::decode(tokens, decoded_size, max_output)?;
    if result.consumed != stored_size {
        return Err(Error::Malformed(format!(
            "LZS2 consumed {} of {stored_size} token bytes",
            result.consumed
        )));
    }
    result.consumed = reader.pos();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_header_consumed_extent_and_output_budget() {
        let mut data = vec![0; HEADER_SIZE];
        data[..4].copy_from_slice(b"LZS2");
        data[4] = 5;
        data[8] = 4;
        data.extend_from_slice(&[0x40, b'A', 3, 0, 0xff]);
        let decoded = decode(&data, 5).unwrap();
        assert_eq!(decoded.bytes, b"AAAAA");
        assert_eq!(decoded.consumed, HEADER_SIZE + 4);
        assert!(decode(&data[..HEADER_SIZE + 3], 5).is_err());
        assert!(decode(&data, 4).is_err());
        data[8] = 5;
        assert!(decode(&data, 5).is_err());
        data[0] = b'X';
        assert!(decode(&data, 5).is_err());
    }
}
