//! Zero-terminated byte commands: literal blocks and 8-bit backward distances.
//! Select this grammar from its carrier; a zero byte alone is not a probe.
use crate::{copy_match, grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Reader, Result, Writer};

/// Decode through the zero command, leaving trailing carrier bytes unconsumed.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    decode_with_history(src, &[], max_output)
}

/// Decode with known bytes immediately before the destination (last 256 used).
/// A backward reference may overlap output, but unknown history is refused.
pub fn decode_with_history(src: &[u8], history: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let history = &history[history.len().saturating_sub(256)..];
    let limit = max_output.min(MAX_DECODED);
    let mut r = Reader::new(src);
    let mut out = Vec::new();
    loop {
        let command = r.u8()?;
        match command {
            0 => {
                return Ok(DecodeOutcome {
                    bytes: out,
                    consumed: r.pos(),
                });
            }
            1..=127 => {
                let count = usize::from(command);
                grow(&out, count, limit)?;
                out.extend_from_slice(r.bytes(count)?);
            }
            _ => {
                let distance = usize::from(r.u8()?) + 1;
                let count = usize::from(command & 127) + 3;
                copy_match(&mut out, history, distance, count, limit)?;
            }
        }
    }
}

/// Minimum-byte parse over literal blocks and produced-byte backward matches.
/// Ties prefer matches, shorter commands, then the nearest longest match.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    let n = src.len();
    let mut cost = vec![0usize; n + 1];
    let mut lengths = vec![0usize; n];
    let mut distances = vec![0usize; n];
    cost[n] = 1; // terminating zero command
    for at in (0..n).rev() {
        let (mut distance, mut longest) = (0, 0);
        for d in 1..=at.min(256) {
            let mut count = 0;
            while count < 130 && at + count < n && src[at + count] == src[at + count - d] {
                count += 1;
            }
            if count > longest {
                distance = d;
                longest = count;
                if count == 130 {
                    break;
                }
            }
        }
        let mut best = usize::MAX;
        for count in 3..=longest {
            let price = 2 + cost[at + count];
            if price < best {
                best = price;
                lengths[at] = count;
                distances[at] = distance;
            }
        }
        for count in 1..=127.min(n - at) {
            let price = 1 + count + cost[at + count];
            if price < best {
                best = price;
                lengths[at] = count;
                distances[at] = 0;
            }
        }
        cost[at] = best;
    }
    let mut w = Writer::new();
    let mut at = 0;
    while at < n {
        let count = lengths[at];
        if distances[at] == 0 {
            w.u8(count as u8);
            w.bytes(&src[at..at + count]);
        } else {
            w.u8(0x80 | (count - 3) as u8);
            w.u8((distances[at] - 1) as u8);
        }
        at += count;
    }
    w.u8(0);
    Ok(w.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_commands_terminator_and_extent() {
        let source = [2, b'A', b'B', 3, 0, 128, 255, 0, 0xaa];
        let got = decode(&source, 5).unwrap();
        assert_eq!(got.bytes, [b'A', b'B', 0, 128, 255]);
        assert_eq!(got.consumed, 8);
        assert_eq!(decode(&[0, 42], 0).unwrap().consumed, 1);
        assert_eq!(encode(&[]).unwrap(), [0]);
        let mut source = vec![127];
        source.extend(0..127);
        source.push(0);
        assert_eq!(
            decode(&source, 127).unwrap().bytes,
            (0..127).collect::<Vec<_>>()
        );
    }

    #[test]
    fn match_limits_overlap_and_explicit_history() {
        assert_eq!(decode(&[1, 42, 0x80, 0, 0], 4).unwrap().bytes, [42; 4]);
        assert_eq!(decode(&[1, 42, 0xff, 0, 0], 131).unwrap().bytes, [42; 131]);
        assert_eq!(
            decode(&[2, b'A', b'B', 0x83, 1, 0], 8).unwrap().bytes,
            b"ABABABAB"
        );
        let history: Vec<u8> = (0..=255).collect();
        assert_eq!(
            decode_with_history(&[0x80, 255, 0], &history, 3)
                .unwrap()
                .bytes,
            [0, 1, 2]
        );
        assert_eq!(
            decode_with_history(&[0x83, 2, 0], b"ABC", 6).unwrap().bytes,
            b"ABCABC"
        );
        assert!(decode(&[0x80, 0, 0], 3).is_err());
        assert!(decode_with_history(&[0x80, 255, 0], &history[1..], 3).is_err());
    }

    #[test]
    fn rejects_truncation_and_complete_token_overflow() {
        let source = [2, b'A', b'B', 0x83, 1, 0];
        for end in 0..source.len() {
            assert!(decode(&source[..end], 8).is_err(), "prefix {end}");
        }
        assert!(decode(&source, 7).is_err());
        assert!(decode(&[2, 0, 0, 0], 1).is_err());
        assert!(encode(&vec![0; MAX_DECODED + 1]).is_err());
        let mut too_large = vec![1, 0];
        for _ in 0..505 {
            too_large.extend_from_slice(&[0xff, 0]);
        }
        too_large.push(0);
        assert!(decode(&too_large, usize::MAX).is_err());
    }

    #[test]
    fn round_trips_and_minimum_literal_block_cost() {
        for n in [0, 1, 2, 3, 127, 128, 130, 256, 8192, MAX_DECODED] {
            let raw: Vec<_> = (0..n).map(|i| ((i / 7 + i % 11) & 255) as u8).collect();
            let encoded = encode(&raw).unwrap();
            let got = decode(&encoded, n).unwrap();
            assert_eq!(got.bytes, raw);
            assert_eq!(got.consumed, encoded.len());
            assert_eq!(encode(&raw).unwrap(), encoded);
        }
        assert_eq!(
            encode(b"ABCABCABC").unwrap(),
            [3, b'A', b'B', b'C', 0x83, 2, 0]
        );
    }
}
