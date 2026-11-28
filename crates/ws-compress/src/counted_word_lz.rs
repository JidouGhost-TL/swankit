//! Word-aligned counted-command LZ stream.
//!
//! Layout: `u16_le cmd_count`, followed by `cmd_count` little-endian command
//! words, followed by the literal word payload stream.
//! - `cmd & 0x8000 != 0`: long back-reference, `dist = (cmd & 0x03ff) * 2`,
//!   `len = (((cmd >> 10) & 0x1f) + 2) * 2`.
//! - `cmd & 0x4000 != 0`: combined literal + short back-reference: copy
//!   `((cmd >> 10) & 0x0f) * 2` literal bytes, then copy
//!   `(((cmd >> 8) & 0x03) + 2) * 2` bytes from `dist = (cmd & 0x00ff) * 2`.
//! - Otherwise: literal run of `cmd * 2` bytes.
use crate::{copy_match, grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result, Writer};

/// Decode a self-delimiting counted-command word-aligned LZ stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let mut header = Reader::new(src);
    let cmd_count = usize::from(header.u16_le()?);
    let cmd_bytes = cmd_count.checked_mul(2).ok_or(Error::InvalidField {
        what: "counted-word-lz command count",
        value: cmd_count as u64,
    })?;
    let cmds = header.bytes(cmd_bytes)?;
    let data_start = header.pos();
    let mut data = Reader::new(&src[data_start..]);
    let mut cmd_reader = Reader::new(cmds);
    let mut out = Vec::new();

    for _ in 0..cmd_count {
        let cmd = cmd_reader.u16_le()?;
        if cmd & 0x8000 != 0 {
            let dist = usize::from(cmd & 0x03ff) * 2;
            let len = (usize::from((cmd >> 10) & 0x1f) + 2) * 2;
            if dist == 0 {
                return Err(Error::InvalidField {
                    what: "counted-word-lz long match distance",
                    value: 0,
                });
            }
            copy_match(&mut out, &[], dist, len, limit)?;
        } else if cmd & 0x4000 != 0 {
            let lit_len = usize::from((cmd >> 10) & 0x0f) * 2;
            if lit_len > 0 {
                grow(&out, lit_len, limit)?;
                out.extend_from_slice(data.bytes(lit_len)?);
            }
            let dist = usize::from(cmd & 0x00ff) * 2;
            let lz_len = (usize::from((cmd >> 8) & 0x03) + 2) * 2;
            if dist == 0 {
                return Err(Error::InvalidField {
                    what: "counted-word-lz short match distance",
                    value: 0,
                });
            }
            copy_match(&mut out, &[], dist, lz_len, limit)?;
        } else {
            let lit_len = usize::from(cmd) * 2;
            if lit_len > 0 {
                grow(&out, lit_len, limit)?;
                out.extend_from_slice(data.bytes(lit_len)?);
            }
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: data_start + data.pos(),
    })
}

fn find_word_match(src: &[u8], pos: usize) -> Option<(usize, usize)> {
    let max_dist_words = (pos / 2).min(0x03ff);
    let max_len_words = ((src.len() - pos) / 2).min(33);
    if max_dist_words == 0 || max_len_words < 2 {
        return None;
    }
    let mut best_len = 0usize;
    let mut best_dist = 0usize;
    for d_words in 1..=max_dist_words {
        let d_bytes = d_words * 2;
        let mut l_words = 0usize;
        while l_words < max_len_words
            && src[pos + l_words * 2] == src[pos + l_words * 2 - d_bytes]
            && src[pos + l_words * 2 + 1] == src[pos + l_words * 2 + 1 - d_bytes]
        {
            l_words += 1;
        }
        if l_words >= 2 && l_words > best_len {
            best_len = l_words;
            best_dist = d_words;
            if best_len == max_len_words {
                break;
            }
        }
    }
    (best_len >= 2).then_some((best_dist, best_len))
}

/// Deterministic encoder for even-length counted-command word-aligned LZ streams.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    if !src.len().is_multiple_of(2) {
        return Err(Error::InvalidField {
            what: "counted-word-lz input length must be even",
            value: src.len() as u64,
        });
    }
    let mut cmds: Vec<u16> = Vec::new();
    let mut lits: Vec<u8> = Vec::new();
    let mut pending_start = 0usize;
    let mut pending_words = 0usize;
    let mut pos = 0usize;

    let flush_literals =
        |cmds: &mut Vec<u16>, lits: &mut Vec<u8>, start: usize, mut words: usize| {
            let mut off = start;
            while words > 0 {
                let chunk = words.min(0x3fff);
                cmds.push(chunk as u16);
                lits.extend_from_slice(&src[off..off + chunk * 2]);
                off += chunk * 2;
                words -= chunk;
            }
        };

    while pos < src.len() {
        if let Some((dist_w, len_w)) = find_word_match(src, pos) {
            if pending_words <= 0x0f && dist_w <= 0xff && len_w <= 5 {
                let cmd = 0x4000u16
                    | ((pending_words as u16) << 10)
                    | (((len_w - 2) as u16) << 8)
                    | (dist_w as u16);
                cmds.push(cmd);
                lits.extend_from_slice(&src[pending_start..pending_start + pending_words * 2]);
                pending_words = 0;
                pos += len_w * 2;
                pending_start = pos;
                continue;
            }
            if pending_words > 0 {
                flush_literals(&mut cmds, &mut lits, pending_start, pending_words);
                pending_words = 0;
            }
            let cmd = 0x8000u16 | (((len_w - 2) as u16) << 10) | (dist_w as u16);
            cmds.push(cmd);
            pos += len_w * 2;
            pending_start = pos;
        } else {
            if pending_words == 0 {
                pending_start = pos;
            }
            pending_words += 1;
            pos += 2;
        }
    }
    if pending_words > 0 {
        flush_literals(&mut cmds, &mut lits, pending_start, pending_words);
    }

    let cmd_count = u16::try_from(cmds.len()).map_err(|_| Error::ResourceLimit {
        resource: "counted-word-lz commands",
        requested: cmds.len() as u64,
        limit: u64::from(u16::MAX),
    })?;
    let mut w = Writer::new();
    w.u16_le(cmd_count);
    for cmd in cmds {
        w.u16_le(cmd);
    }
    w.bytes(&lits);
    Ok(w.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_all_command_types_and_leaves_trailer() {
        // 3 commands:
        // 1) pure literal: 2 words -> [0x11, 0x22, 0x33, 0x44]
        // 2) combined lit+short LZ: 1 literal word [0x55, 0x66], then dist=1 word (2B), len=3 words (6B)
        //    -> repeats [0x55, 0x66] 3 times
        // 3) long LZ: dist=5 words (10B), len=2 words (4B) -> copies [0x33, 0x44, 0x55, 0x66]
        let mut stream = Vec::new();
        stream.extend_from_slice(&3u16.to_le_bytes());
        stream.extend_from_slice(&2u16.to_le_bytes());
        let combined: u16 = 0x4000 | (1 << 10) | ((3 - 2) << 8) | 1;
        stream.extend_from_slice(&combined.to_le_bytes());
        let long_lz: u16 = 0x8000 | ((2 - 2) << 10) | 5;
        stream.extend_from_slice(&long_lz.to_le_bytes());
        stream.extend_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
        let payload_len = stream.len();
        stream.extend_from_slice(&[0xde, 0xad]);

        let got = decode(&stream, 64).unwrap();
        assert_eq!(
            got.bytes,
            vec![
                0x11, 0x22, 0x33, 0x44, //
                0x55, 0x66, 0x55, 0x66, 0x55, 0x66, 0x55, 0x66, //
                0x33, 0x44, 0x55, 0x66,
            ]
        );
        assert_eq!(got.consumed, payload_len);
        assert_eq!(decode(&[0, 0, 0xaa], 0).unwrap().consumed, 2);
    }

    #[test]
    fn rejects_invalid_distances_truncation_odd_encode_and_overflow() {
        assert!(decode(&[], 64).is_err());
        assert!(decode(&[1, 0], 64).is_err());
        // Zero distance in long LZ
        assert!(decode(&[1, 0, 0x00, 0x80], 64).is_err());
        // Zero distance in combined LZ
        assert!(decode(&[1, 0, 0x00, 0x40], 64).is_err());
        // Distance beyond history
        assert!(decode(&[1, 0, 0x01, 0x80], 64).is_err());
        // Truncated literal data
        assert!(decode(&[1, 0, 0x02, 0x00, 0xaa, 0xbb], 64).is_err());
        // Output overflow
        assert!(decode(&[1, 0, 0x02, 0x00, 1, 2, 3, 4], 2).is_err());
        // Odd-length encode rejected
        assert!(encode(&[1, 2, 3]).is_err());
        assert!(encode(&vec![0; MAX_DECODED + 2]).is_err());
    }

    #[test]
    fn round_trips_word_patterns_and_limits() {
        for n in [0, 2, 4, 16, 64, 512, 4096] {
            let raw: Vec<u8> = (0..n).map(|i| ((i / 6 + i % 11) & 0xff) as u8).collect();
            let encoded = encode(&raw).unwrap();
            let got = decode(&encoded, n).unwrap();
            assert_eq!(got.bytes, raw);
            assert_eq!(got.consumed, encoded.len());
            assert_eq!(encode(&raw).unwrap(), encoded);
        }
    }
}
