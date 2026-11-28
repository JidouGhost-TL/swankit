//! Escape-byte word and byte RLE stream terminated by 0xEE.
//! Opcodes: 0x77 literal escape, 0x88 word repeat, 0x99 word pair,
//! 0xAA incrementing word run, 0xBB zero run, 0xCC 0xFF run,
//! 0xDD byte repeat, 0xEE end of stream.
use crate::{grow, DecodeOutcome, MAX_STREAM_DECODED};
use formatkit_core::{Error, Reader, Result, Writer};

fn is_escape(byte: u8) -> bool {
    matches!(byte, 0x77 | 0x88 | 0x99 | 0xaa | 0xbb | 0xcc | 0xdd | 0xee)
}

/// Decode through the 0xEE terminator, leaving trailing carrier bytes unconsumed.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_STREAM_DECODED);
    let mut r = Reader::new(src);
    let mut out = Vec::new();
    loop {
        match r.u8()? {
            0x88 => {
                let count = usize::from(r.u8()?);
                let lo = r.u8()?;
                let hi = r.u8()?;
                if count == 0 {
                    return Err(Error::InvalidField {
                        what: "rle-77ee word repeat count",
                        value: 0,
                    });
                }
                grow(&out, count * 2, limit)?;
                for _ in 0..count {
                    out.push(lo);
                    out.push(hi);
                }
            }
            0x99 => {
                let lo = r.u8()?;
                let hi = r.u8()?;
                grow(&out, 4, limit)?;
                out.extend_from_slice(&[lo, hi, lo, hi]);
            }
            0xaa => {
                let count = usize::from(r.u8()?);
                let lo = r.u8()?;
                let hi = r.u8()?;
                if count == 0 {
                    return Err(Error::InvalidField {
                        what: "rle-77ee incrementing word count",
                        value: 0,
                    });
                }
                grow(&out, count * 2, limit)?;
                let start = u16::from_le_bytes([lo, hi]);
                for k in 0..count {
                    out.extend_from_slice(&start.wrapping_add(k as u16).to_le_bytes());
                }
            }
            0xbb => {
                let count = usize::from(r.u8()?);
                if count == 0 {
                    return Err(Error::InvalidField {
                        what: "rle-77ee zero run count",
                        value: 0,
                    });
                }
                grow(&out, count, limit)?;
                out.resize(out.len() + count, 0x00);
            }
            0xcc => {
                let count = usize::from(r.u8()?);
                if count == 0 {
                    return Err(Error::InvalidField {
                        what: "rle-77ee 0xff run count",
                        value: 0,
                    });
                }
                grow(&out, count, limit)?;
                out.resize(out.len() + count, 0xff);
            }
            0xdd => {
                let count = usize::from(r.u8()?);
                let value = r.u8()?;
                if count == 0 {
                    return Err(Error::InvalidField {
                        what: "rle-77ee byte repeat count",
                        value: 0,
                    });
                }
                grow(&out, count, limit)?;
                out.resize(out.len() + count, value);
            }
            0xee => {
                return Ok(DecodeOutcome {
                    bytes: out,
                    consumed: r.pos(),
                });
            }
            0x77 => {
                let literal = r.u8()?;
                grow(&out, 1, limit)?;
                out.push(literal);
            }
            literal => {
                grow(&out, 1, limit)?;
                out.push(literal);
            }
        }
    }
}

/// Deterministic encoder for escape-byte word/byte RLE streams.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    grow(&[], src.len(), MAX_STREAM_DECODED)?;
    let mut w = Writer::new();
    let n = src.len();
    let mut i = 0;
    while i < n {
        let b = src[i];
        let mut run = 1usize;
        while i + run < n && run < 255 && src[i + run] == b {
            run += 1;
        }
        if b == 0x00 && run >= 3 {
            w.u8(0xbb);
            w.u8(run as u8);
            i += run;
            continue;
        }
        if b == 0xff && run >= 3 {
            w.u8(0xcc);
            w.u8(run as u8);
            i += run;
            continue;
        }
        if run >= 4 || (run >= 3 && is_escape(b)) {
            w.u8(0xdd);
            w.u8(run as u8);
            w.u8(b);
            i += run;
            continue;
        }
        if i + 4 <= n {
            let word = [src[i], src[i + 1]];
            let mut wrun = 1usize;
            while i + (wrun + 1) * 2 <= n
                && wrun < 255
                && src[i + wrun * 2] == word[0]
                && src[i + wrun * 2 + 1] == word[1]
            {
                wrun += 1;
            }
            if wrun == 2 {
                w.u8(0x99);
                w.u8(word[0]);
                w.u8(word[1]);
                i += 4;
                continue;
            }
            if wrun >= 3 {
                w.u8(0x88);
                w.u8(wrun as u8);
                w.u8(word[0]);
                w.u8(word[1]);
                i += wrun * 2;
                continue;
            }
        }
        if i + 6 <= n {
            let start = u16::from_le_bytes([src[i], src[i + 1]]);
            let mut irun = 1usize;
            while i + (irun + 1) * 2 <= n && irun < 255 {
                let next = u16::from_le_bytes([src[i + irun * 2], src[i + irun * 2 + 1]]);
                if next != start.wrapping_add(irun as u16) {
                    break;
                }
                irun += 1;
            }
            if irun >= 3 {
                w.u8(0xaa);
                w.u8(irun as u8);
                w.u8(src[i]);
                w.u8(src[i + 1]);
                i += irun * 2;
                continue;
            }
        }
        if is_escape(b) {
            w.u8(0x77);
        }
        w.u8(b);
        i += 1;
    }
    w.u8(0xee);
    Ok(w.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_all_opcodes_and_stops_at_terminator() {
        let stream = [
            0x01, 0x77, 0x77, 0x77, 0xee, // literals 0x01, 0x77, 0xee
            0x88, 3, 0x12, 0x34, // 3x word 0x3412
            0x99, 0xab, 0xcd, // 2x word 0xcdab
            0xaa, 3, 0xff, 0xff, // 0xffff, 0x0000, 0x0001
            0xbb, 4, // 4x 0x00
            0xcc, 3, // 3x 0xff
            0xdd, 5, 0x5a, // 5x 0x5a
            0xee, 0xde, 0xad, // terminator + trailing bytes
        ];
        let got = decode(&stream, 64).unwrap();
        let expected = vec![
            0x01, 0x77, 0xee, //
            0x12, 0x34, 0x12, 0x34, 0x12, 0x34, //
            0xab, 0xcd, 0xab, 0xcd, //
            0xff, 0xff, 0x00, 0x00, 0x01, 0x00, //
            0x00, 0x00, 0x00, 0x00, //
            0xff, 0xff, 0xff, //
            0x5a, 0x5a, 0x5a, 0x5a, 0x5a,
        ];
        assert_eq!(got.bytes, expected);
        assert_eq!(got.consumed, stream.len() - 2);
        assert_eq!(decode(&[0xee, 0x42], 0).unwrap().consumed, 1);
        assert_eq!(encode(&[]).unwrap(), [0xee]);
    }

    #[test]
    fn rejects_zero_counts_truncation_and_overflow() {
        for bad in [
            &[0x88, 0, 1, 2, 0xee][..],
            &[0xaa, 0, 1, 2, 0xee][..],
            &[0xbb, 0, 0xee][..],
            &[0xcc, 0, 0xee][..],
            &[0xdd, 0, 5, 0xee][..],
            &[][..],
            &[0x01][..],
            &[0x77][..],
            &[0x88, 1, 2][..],
            &[0x99, 1][..],
            &[0xaa, 1, 2][..],
            &[0xbb][..],
            &[0xcc][..],
            &[0xdd, 1][..],
        ] {
            assert!(decode(bad, 64).is_err(), "expected error for {bad:02x?}");
        }
        assert!(decode(&[0xbb, 5, 0xee], 4).is_err());
        assert!(decode(&[0x99, 1, 2, 0xee], 3).is_err());
        assert!(encode(&vec![0; MAX_STREAM_DECODED + 1]).is_err());
    }

    #[test]
    fn round_trips_all_patterns_and_limits() {
        let escapes: Vec<u8> = vec![
            0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x77, 0x77, 0x77, 0x00, 0x00, 0x00,
            0x00, 0xff, 0xff, 0xff, 0x10, 0x20, 0x10, 0x20, 0x00, 0x01, 0x01, 0x01, 0x02, 0x01,
            0x03, 0x01,
        ];
        let enc = encode(&escapes).unwrap();
        let dec = decode(&enc, escapes.len()).unwrap();
        assert_eq!(dec.bytes, escapes);
        assert_eq!(dec.consumed, enc.len());

        for n in [0, 1, 2, 3, 4, 255, 256, 1024, 0x10000, 0x18000] {
            let raw: Vec<u8> = (0..n).map(|i| ((i / 5 + i % 13) & 0xff) as u8).collect();
            let encoded = encode(&raw).unwrap();
            let got = decode(&encoded, n).unwrap();
            assert_eq!(got.bytes, raw);
            assert_eq!(got.consumed, encoded.len());
            assert_eq!(encode(&raw).unwrap(), encoded);
        }
    }
}
