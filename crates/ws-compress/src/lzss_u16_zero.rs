//! Length-prefixed LSB-flag LZSS with destination-relative history and zero prefix.
//! Grammar recovered from a cartridge routine; external evidence belongs in the
//! corpus. This is not an Okumura absolute-ring-address stream.
use crate::{grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

/// Prehistory dialects observed in separately inspected cartridge routines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Prehistory {
    Low5Prefix,
    Low8Prefix,
    ZeroUntilStart,
}

/// Decode through the u16 declared output length. A token crossing that length
/// is refused: the caller's storage boundary must not be overwritten.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    decode_with_destination(src, max_output, 0x2000)
}

/// Decode with the original destination offset. The zero-prefix counter depends
/// on its low five bits; unsigned pointer wrapping is not invented as history.
pub fn decode_with_destination(
    src: &[u8],
    max_output: usize,
    destination: u16,
) -> Result<DecodeOutcome> {
    decode_with_policy(src, max_output, destination, Prehistory::Low5Prefix)
}

pub fn decode_with_policy(
    src: &[u8],
    max_output: usize,
    destination: u16,
    policy: Prehistory,
) -> Result<DecodeOutcome> {
    let mut r = Reader::new(src);
    let size = usize::from(r.u16_le()?);
    grow(&[], size, max_output.min(MAX_DECODED))?;
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        let flags = r.u8()?;
        for bit in 0..8 {
            if out.len() == size {
                break;
            }
            if flags & (1 << bit) != 0 {
                out.push(r.u8()?);
            } else {
                let token = usize::from(r.u16_le()?);
                let count = (token & 15) + 3;
                let distance = token >> 4;
                if distance == 0 {
                    return Err(Error::InvalidField {
                        what: "zero LZSS distance",
                        value: 0,
                    });
                }
                grow(&out, count, size)?;
                let mut remaining = count;
                if distance > out.len() {
                    // The game's prehistory loop uses an 8-bit counter masked
                    // to 31. Zero enters the do-loop and wraps to 255.
                    if distance > usize::from(destination) + out.len() {
                        return Err(Error::InvalidField {
                            what: "LZSS history wraps destination segment",
                            value: distance as u64,
                        });
                    }
                    let mask = match policy {
                        Prehistory::Low5Prefix => 31,
                        _ => 255,
                    };
                    let prefix = distance.wrapping_sub(usize::from(destination) + out.len()) & mask;
                    let zeros = if prefix == 0 { 256 } else { prefix };
                    let deficit = distance - out.len();
                    let zeros = match policy {
                        // The five-bit routine tests only the counter and the
                        // token length, so it can zero beyond the history gap.
                        Prehistory::Low5Prefix => zeros,
                        // The eight-bit routine additionally tests whether SI
                        // has reached the original destination each iteration.
                        Prehistory::Low8Prefix => zeros.min(deficit),
                        Prehistory::ZeroUntilStart => deficit,
                    };
                    let n = remaining.min(zeros);
                    out.resize(out.len() + n, 0);
                    remaining -= n;
                    // The routine resets SI to the original destination after
                    // the zero loop, rather than retaining a negative offset.
                    for i in 0..remaining {
                        let b = out[i];
                        out.push(b);
                    }
                } else {
                    for _ in 0..remaining {
                        let b = out[out.len() - distance];
                        out.push(b);
                    }
                }
            }
        }
    }
    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Minimum-byte parse over literals and ordinary backward matches. Accounting
/// for each eight-token flag group avoids greedy matches exhausting a slot.
/// Produced-byte references avoid prehistory subtleties.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    if src.len() > u16::MAX as usize {
        return Err(Error::InvalidField {
            what: "u16 LZSS output length",
            value: src.len() as u64,
        });
    }
    let mut matches = vec![(0, 0); src.len()];
    for at in 0..src.len() {
        for d in 1..=at.min(4095) {
            let mut n = 0;
            while n < 18 && at + n < src.len() && src[at + n] == src[at + n - d] {
                n += 1;
            }
            if n > matches[at].1 {
                matches[at] = (d, n);
                if n == 18 {
                    break;
                }
            }
        }
    }
    // A longest match's distance also supports every shorter match at that
    // position. Keep token phase in the cost: a new group costs one flag byte.
    let mut costs = vec![[0usize; 8]; src.len() + 1];
    let mut lengths = vec![[1u8; 8]; src.len()];
    for at in (0..src.len()).rev() {
        for phase in 0..8 {
            let next = (phase + 1) % 8;
            let flag = usize::from(phase == 0);
            costs[at][phase] = flag + 1 + costs[at + 1][next];
            for n in 3..=matches[at].1 {
                let cost = flag + 2 + costs[at + n][next];
                if cost < costs[at][phase] {
                    costs[at][phase] = cost;
                    lengths[at][phase] = n as u8;
                }
            }
        }
    }
    let mut out = Vec::with_capacity(2 + costs[0][0]);
    out.extend_from_slice(&(src.len() as u16).to_le_bytes());
    let mut at = 0;
    while at < src.len() {
        let flag_at = out.len();
        out.push(0);
        let mut flags = 0;
        let mut bit = 0;
        while bit < 8 && at < src.len() {
            let n = usize::from(lengths[at][bit]);
            if n >= 3 {
                let token = ((matches[at].0 << 4) | (n - 3)) as u16;
                out.extend_from_slice(&token.to_le_bytes());
                at += n;
            } else {
                flags |= 1 << bit;
                out.push(src[at]);
                at += 1;
            }
            bit += 1;
        }
        out[flag_at] = flags;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literals_matches_and_consumed() {
        let bytes = [9, 0, 7, b'A', b'B', b'C', 0x33, 0];
        let d = decode(&bytes, 9).unwrap();
        assert_eq!(d.bytes, b"ABCABCABC");
        assert_eq!(d.consumed, 8);
        assert!(crate::lz_rle_7f::decode(&bytes, 9).is_err());
        assert!(crate::lz_bits_16::decode(&bytes, 9).is_err());
    }
    #[test]
    fn zero_prefix_wrap_and_overlap() {
        assert_eq!(decode(&[6, 0, 0, 0x23, 0], 6).unwrap().bytes, [0; 6]);
        assert_eq!(decode(&[3, 0, 0, 0x00, 0x02], 3).unwrap().bytes, [0; 3]);
        let d = decode(&[7, 0, 1, b'X', 0x13, 0], 7).unwrap();
        assert_eq!(d.bytes, b"XXXXXXX");
    }
    #[test]
    fn bounded_and_truncated() {
        for v in [vec![], vec![3], vec![3, 0], vec![3, 0, 0], vec![3, 0, 0, 0]] {
            assert!(decode(&v, 65536).is_err());
        }
        assert!(decode(&[1, 0, 0, 0x10, 0], 10).is_err());
        assert!(decode(&[3, 0, 0, 0, 0], 3).is_err());
        assert!(decode(&[3, 0, 0, 0x10, 0], 2).is_err());
    }
    #[test]
    fn round_trips() {
        for n in [0, 1, 7, 8, 9, 32, 511, 4096] {
            let src: Vec<_> = (0..n).map(|i| ((i / 7 + i % 11) & 255) as u8).collect();
            let e = encode(&src).unwrap();
            let d = decode(&e, n).unwrap();
            assert_eq!(d.bytes, src);
            assert_eq!(d.consumed, e.len());
        }
    }

    #[test]
    fn encoder_fits_a_slot_that_requires_a_shorter_match() {
        // Five literals followed by a four-byte match fit in ten bytes. Taking
        // the earlier three-byte match needs another token and eleven bytes.
        let raw = [0, 0, 0, 1, 0, 0, 0, 0, 0];
        let stored = encode(&raw).unwrap();
        assert_eq!(stored.len(), 10);
        assert_eq!(decode(&stored, raw.len()).unwrap().bytes, raw);
    }
}

#[cfg(test)]
mod dialect_tests {
    use super::*;
    #[test]
    fn prehistory_dialects_and_unaligned_destination() {
        // Literal A then a distance-34 match of length 18: five-bit prefix
        // copies one zero then resets to the beginning; other policies zero-fill.
        let stream = [19, 0, 1, b'A', 0x2f, 0x02];
        let low5 = decode_with_policy(&stream, 100, 0x2000, Prehistory::Low5Prefix).unwrap();
        assert_eq!(&low5.bytes[..4], &[b'A', 0, b'A', 0]);
        for policy in [Prehistory::Low8Prefix, Prehistory::ZeroUntilStart] {
            let d = decode_with_policy(&stream, 100, 0x2000, policy).unwrap();
            assert_eq!(d.bytes, [vec![b'A'], vec![0; 18]].concat());
        }
        // Only the eight-bit and fill-until-start routines test whether the
        // source reaches the original destination before the counter ends.
        let stream = [4, 0, 1, b'A', 0x20, 0];
        let low5 = decode_with_policy(&stream, 100, 0x2001, Prehistory::Low5Prefix).unwrap();
        assert_eq!(low5.bytes, [b'A', 0, 0, 0]);
        for policy in [Prehistory::Low8Prefix, Prehistory::ZeroUntilStart] {
            let d = decode_with_policy(&stream, 100, 0x2001, policy).unwrap();
            assert_eq!(d.bytes, [b'A', 0, b'A', 0]);
        }
    }
}
