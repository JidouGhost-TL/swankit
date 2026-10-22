//! Mode byte selects counted literals or lazy MSB-first little-endian flag words.
//! The stream starts at the mode byte; carrier prefixes are not codec input.
use crate::{copy_match, grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Reader, Result, Writer};

/// Decode one stream without assuming any initial destination history.
/// Counted mode ends after its payload; flagged mode ends at the zero token.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let mut r = Reader::new(src);
    let mut out = Vec::new();
    if r.u8()? == 0 {
        let count = usize::from(r.u16_le()?);
        grow(&out, count, limit)?;
        out.extend_from_slice(r.bytes(count)?);
    } else {
        let mut flags = 0u16;
        let mut remaining = 0;
        loop {
            if remaining == 0 {
                flags = r.u16_le()?;
                remaining = 16;
            }
            let literal = flags & 0x8000 != 0;
            flags <<= 1;
            remaining -= 1;
            if literal {
                grow(&out, 1, limit)?;
                out.push(r.u8()?);
            } else {
                let token = r.u16_le()?;
                if token == 0 {
                    break;
                }
                let distance = 4096 - usize::from(token >> 4);
                let mut count = usize::from(token & 15) + 3;
                if token & 15 == 15 {
                    count += usize::from(r.u8()?);
                }
                copy_match(&mut out, &[], distance, count, limit)?;
            }
        }
    }
    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Deterministic literal-only encoding, without a compression-size guarantee.
/// Counted mode handles up to 65535 bytes; flagged literals handle 65536.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    let mut w = Writer::new();
    if src.len() <= usize::from(u16::MAX) {
        w.u8(0);
        w.u16_le(src.len() as u16);
        w.bytes(src);
    } else {
        w.u8(1);
        for chunk in src.chunks(16) {
            w.u16_le(u16::MAX << (16 - chunk.len()));
            w.bytes(chunk);
        }
        if src.len().is_multiple_of(16) {
            w.u16_le(0);
        }
        w.u16_le(0);
    }
    Ok(w.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counted_mode_and_lazy_sixteenth_operand() {
        let got = decode(&[0, 3, 0, 9, 8, 7, 0xaa], 3).unwrap();
        assert_eq!(got.bytes, [9, 8, 7]);
        assert_eq!(got.consumed, 6);
        assert_eq!(decode(&[0, 0, 0], 0).unwrap().bytes, []);
        let mut stream = vec![7, 0xff, 0xff];
        stream.extend(0..16);
        stream.extend([0, 0, 0, 0, 0xaa]);
        let got = decode(&stream, 16).unwrap();
        assert_eq!(got.bytes, (0..16).collect::<Vec<_>>());
        assert_eq!(got.consumed, 23);
        // The 16th command may terminate without fetching an unused flag word.
        let mut stream = vec![1, 0xfe, 0xff];
        stream.extend(0..15);
        stream.extend([0, 0]);
        assert_eq!(decode(&stream, 15).unwrap().consumed, 20);
    }

    #[test]
    fn msb_flags_little_endian_tokens_extended_overlap_and_mutation() {
        // Literal A, literal B, distance2/length6, distance1/length273, end.
        let src = [
            1, 0, 0xc0, b'A', b'B', 0xe3, 0xff, 0xff, 0xff, 255, 0, 0, 42,
        ];
        let got = decode(&src, 281).unwrap();
        let mut want = b"ABABABAB".to_vec();
        want.extend([b'B'; 273]);
        assert_eq!(got.bytes, want);
        assert_eq!(got.consumed, 12);
        let mut changed = src;
        changed[3] = b'X';
        assert_eq!(&decode(&changed, 281).unwrap().bytes[..8], b"XBXBXBXB");
        assert!(decode(&src, 280).is_err());
        assert!(decode(&[1, 0, 0, 1, 0], 4).is_err());
    }

    #[test]
    fn truncation_bounds_and_full_literal_round_trips() {
        let src = [1, 0, 0x80, 42, 0xf0, 0xff, 0, 0];
        assert_eq!(decode(&src, 4).unwrap().bytes, [42; 4]);
        for cut in 0..src.len() {
            assert!(decode(&src[..cut], 4).is_err());
        }
        assert!(decode(&[0, 2, 0, 1], 2).is_err());
        assert!(decode(&[0, 2, 0, 1, 2], 1).is_err());
        assert!(encode(&vec![0; 65537]).is_err());
        for n in [0, 1, 15, 16, 17, 65535, 65536] {
            let raw: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
            let packed = encode(&raw).unwrap();
            let got = decode(&packed, n).unwrap();
            assert_eq!(got.bytes, raw);
            assert_eq!(got.consumed, packed.len());
            assert_eq!(encode(&raw).unwrap(), packed);
        }
    }

    #[test]
    fn farthest_match_uses_produced_history_and_global_limit() {
        let mut source = vec![1];
        for group in 0..256 {
            source.extend([0xff, 0xff]);
            source.extend((0..16).map(|i| (group * 16 + i) as u8));
        }
        // Distance4096, length4 is token1; token0 remains the terminator.
        source.extend([0, 0, 1, 0, 0, 0]);
        let got = decode(&source, usize::MAX).unwrap();
        assert_eq!(got.bytes.len(), 4100);
        assert_eq!(&got.bytes[4096..], &[0, 1, 2, 3]);
        assert_eq!(got.consumed, source.len());
        assert!(decode(&source, 4099).is_err());
    }
}
