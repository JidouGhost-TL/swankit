//! Counted little-endian word commands followed by a separate literal pool.
//! This is logical software expansion, not a hardware transfer interpretation.
use crate::{copy_match, grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result, Writer};

/// Decode relative to a source window beginning at offset zero.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    decode_at(src, max_output, 0)
}

/// Decode one explicit stream without initial history or 16-bit pointer wrap.
/// `source_offset` anchors the consumed bytes inside their 64 KiB source window.
pub fn decode_at(src: &[u8], max_output: usize, source_offset: usize) -> Result<DecodeOutcome> {
    let available = MAX_DECODED - (source_offset & (MAX_DECODED - 1));
    let mut header = Reader::new(&src[..src.len().min(available)]);
    let count = usize::from(header.u16_le()?);
    if count == 0 {
        return Err(Error::InvalidField {
            what: "zero command count requires unqualified loop wrap",
            value: 0,
        });
    }
    let commands = header.bytes(count * 2)?;
    let table_end = header.pos();
    let mut table = Reader::new(commands);
    let mut pool = Reader::new(&src[table_end..src.len().min(available)]);
    let limit = max_output.min(MAX_DECODED);
    let mut out = Vec::new();
    for _ in 0..count {
        let command = table.u16_le()?;
        if command & 0x8000 != 0 {
            let distance = usize::from(command & 0x3ff) * 2;
            let length = (usize::from((command >> 10) & 0x1f) + 2) * 2;
            copy_match(&mut out, &[], distance, length, limit)?;
        } else {
            let literals = if command & 0x4000 != 0 {
                usize::from((command >> 10) & 0xf)
            } else {
                usize::from(command)
            };
            grow(&out, literals * 2, limit)?;
            out.extend_from_slice(pool.bytes(literals * 2)?);
            if command & 0x4000 != 0 {
                let distance = usize::from(command & 0xff) * 2;
                let length = (usize::from((command >> 8) & 3) + 2) * 2;
                copy_match(&mut out, &[], distance, length, limit)?;
            }
        }
    }
    Ok(DecodeOutcome {
        bytes: out,
        consumed: table_end + pool.pos(),
    })
}

/// Literal-only encoding; it can grow beyond an existing compressed allocation.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    encode_at(src, 0)
}

/// Encode only when the literal stream fits the current 64 KiB source window.
pub fn encode_at(src: &[u8], source_offset: usize) -> Result<Vec<u8>> {
    input_size(src)?;
    if !src.len().is_multiple_of(2) {
        return Err(Error::InvalidField {
            what: "word stream requires an even decoded byte count",
            value: src.len() as u64,
        });
    }
    let count = src.len().div_ceil(0x7ffe).max(1);
    let available = MAX_DECODED - (source_offset & (MAX_DECODED - 1));
    grow(&[], 2 + count * 2 + src.len(), available)?;
    let mut w = Writer::new();
    w.u16_le(count as u16);
    if src.is_empty() {
        w.u16_le(0);
    } else {
        for chunk in src.chunks(0x7ffe) {
            w.u16_le((chunk.len() / 2) as u16);
        }
    }
    w.bytes(src);
    Ok(w.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separate_table_pool_three_command_classes_and_word_overlap() {
        // Literal1; literal2 + distance1/length2; distance2/length5.
        let src = [3, 0, 1, 0, 1, 0x48, 2, 0x8c, 2, 1, 4, 3, 6, 5, 99, 88];
        let got = decode(&src, 20).unwrap();
        let mut want = vec![2, 1, 4, 3];
        want.extend([6, 5].repeat(8));
        assert_eq!(got.bytes, want);
        assert_eq!(got.consumed, 14);
        let mut changed = src;
        changed[12] = 42;
        assert_eq!(&decode(&changed, 20).unwrap().bytes[4..], [42, 5].repeat(8));
        assert!(decode(&src, 19).is_err());
        for n in 0..14 {
            assert!(decode(&src[..n], 20).is_err());
        }
    }

    #[test]
    fn no_invented_history_zero_count_or_source_pointer_wrap() {
        assert!(decode(&[0, 0], 32).is_err());
        assert!(decode(&[1, 0, 0, 0x80], 32).is_err());
        assert!(decode(&[1, 0, 1, 0x80], 32).is_err());
        assert_eq!(decode(&[1, 0, 0, 0], 0).unwrap().bytes, []);
        let src = [1, 0, 1, 0, 7, 9];
        assert_eq!(decode_at(&src, 2, 0xfffa).unwrap().bytes, [7, 9]);
        assert!(decode_at(&src, 2, 0xfffb).is_err());
        assert!(decode_at(&src, 2, 0xffff).is_err());
        assert!(encode_at(&[7, 9], 0xfffb).is_err());
    }

    #[test]
    fn bounded_literal_encoder_with_explicit_growth_and_word_alignment() {
        assert!(encode(&[1]).is_err());
        assert!(encode(&vec![0; 65532]).is_err());
        assert!(encode(&vec![0; 65538]).is_err());
        for n in [0, 2, 32766, 32768, 65530] {
            let raw: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
            let packed = encode(&raw).unwrap();
            let got = decode(&packed, n).unwrap();
            assert_eq!(got.bytes, raw);
            assert_eq!(got.consumed, packed.len());
            assert!(packed.len() > raw.len());
        }
    }
}
