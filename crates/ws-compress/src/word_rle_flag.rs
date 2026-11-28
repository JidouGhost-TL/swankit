//! Flag-prefixed 16-bit word RLE stream with repeat, increment, and decrement runs.
//!
//! Header:
//! - `flag: u8` (must be `0x02`)
//! - `raw_len: u16_le` (exact decoded byte length; `even_len = raw_len & !1`)
//! - If `raw_len & 1 != 0`: one trailing odd literal byte immediately follows the header
//!   and is appended at the end of the decoded output.
//!
//! Token stream (while emitted even bytes `< even_len`):
//! - Read `op: u8` (`mode = op & 0xc0`, `cnt = op & 0x3f`):
//!   - `mode == 0x00`: copy `cnt + 1` literal `u16_le` words verbatim.
//!   - `mode == 0x40`: read `w: u16_le`, emit `w` repeated `cnt + 2` times.
//!   - `mode == 0x80`: read `w: u16_le`, emit `cnt + 2` incrementing words `w, w+1, ...` (mod `2^16`).
//!   - `mode == 0xc0`: read `w: u16_le`, emit `cnt + 2` decrementing words `w, w-1, ...` (mod `2^16`).
use crate::{grow, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

const HEADER_FLAG: u8 = 0x02;

/// Decode a flag-prefixed (`0x02`) 16-bit word-RLE stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let mut r = Reader::new(src);
    let flag = r.u8()?;
    if flag != HEADER_FLAG {
        return Err(Error::InvalidField {
            what: "word-rle-flag header flag must be 0x02",
            value: u64::from(flag),
        });
    }
    let raw_len = usize::from(r.u16_le()?);
    grow(&[], raw_len, limit)?;
    let even_len = raw_len & !1;
    let odd_byte = if raw_len & 1 != 0 {
        Some(r.u8()?)
    } else {
        None
    };

    let mut out = Vec::with_capacity(raw_len);
    while out.len() < even_len {
        let op = r.u8()?;
        let mode = op & 0xc0;
        let cnt = usize::from(op & 0x3f);
        match mode {
            0x00 => {
                let byte_len = (cnt + 1) * 2;
                if out.len().saturating_add(byte_len) > even_len {
                    return Err(Error::InvalidField {
                        what: "word-rle-flag literal run exceeds declared output length",
                        value: out.len().saturating_add(byte_len) as u64,
                    });
                }
                out.extend_from_slice(r.bytes(byte_len)?);
            }
            0x40 => {
                let n_words = cnt + 2;
                let byte_len = n_words * 2;
                if out.len().saturating_add(byte_len) > even_len {
                    return Err(Error::InvalidField {
                        what: "word-rle-flag repeat run exceeds declared output length",
                        value: out.len().saturating_add(byte_len) as u64,
                    });
                }
                let word = r.bytes(2)?;
                for _ in 0..n_words {
                    out.extend_from_slice(word);
                }
            }
            0x80 => {
                let n_words = cnt + 2;
                let byte_len = n_words * 2;
                if out.len().saturating_add(byte_len) > even_len {
                    return Err(Error::InvalidField {
                        what: "word-rle-flag increment run exceeds declared output length",
                        value: out.len().saturating_add(byte_len) as u64,
                    });
                }
                let mut w = r.u16_le()?;
                for _ in 0..n_words {
                    out.extend_from_slice(&w.to_le_bytes());
                    w = w.wrapping_add(1);
                }
            }
            0xc0 => {
                let n_words = cnt + 2;
                let byte_len = n_words * 2;
                if out.len().saturating_add(byte_len) > even_len {
                    return Err(Error::InvalidField {
                        what: "word-rle-flag decrement run exceeds declared output length",
                        value: out.len().saturating_add(byte_len) as u64,
                    });
                }
                let mut w = r.u16_le()?;
                for _ in 0..n_words {
                    out.extend_from_slice(&w.to_le_bytes());
                    w = w.wrapping_sub(1);
                }
            }
            _ => unreachable!(),
        }
    }

    if let Some(b) = odd_byte {
        out.push(b);
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

fn best_run(words: &[u16], start: usize) -> (u8, usize) {
    let base = words[start];
    let max_run = (words.len() - start).min(65);

    let mut rep = 1usize;
    while rep < max_run && words[start + rep] == base {
        rep += 1;
    }

    let mut inc = 1usize;
    while inc < max_run && words[start + inc] == base.wrapping_add(inc as u16) {
        inc += 1;
    }

    let mut dec = 1usize;
    while dec < max_run && words[start + dec] == base.wrapping_sub(dec as u16) {
        dec += 1;
    }

    if rep >= inc && rep >= dec {
        (0x40, rep)
    } else if inc >= dec {
        (0x80, inc)
    } else {
        (0xc0, dec)
    }
}

/// Deterministic encoder for `word-rle-flag`.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    let raw_len = u16::try_from(src.len()).map_err(|_| Error::ResourceLimit {
        resource: "word-rle-flag input length",
        requested: src.len() as u64,
        limit: u64::from(u16::MAX),
    })?;
    let mut out = Vec::with_capacity(4 + src.len());
    out.push(HEADER_FLAG);
    out.extend_from_slice(&raw_len.to_le_bytes());
    if let Some(&last) = src.last().filter(|_| src.len() & 1 != 0) {
        out.push(last);
    }

    let even_len = src.len() & !1;
    let words: Vec<u16> = src[..even_len]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    let mut i = 0usize;
    while i < words.len() {
        let (mode, run) = best_run(&words, i);
        if run >= 2 {
            out.push(mode | ((run - 2) as u8));
            out.extend_from_slice(&words[i].to_le_bytes());
            i += run;
        } else {
            let start = i;
            i += 1;
            while i < words.len() && (i - start) < 64 {
                let (_, next_run) = best_run(&words, i);
                if next_run >= 2 {
                    break;
                }
                i += 1;
            }
            let lit_len = i - start;
            out.push((lit_len - 1) as u8);
            for &w in &words[start..i] {
                out.extend_from_slice(&w.to_le_bytes());
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_all_four_modes_and_odd_length() {
        let mut raw = Vec::new();
        // Literals
        for w in [0x0100u16, 0x0500, 0x0900] {
            raw.extend_from_slice(&w.to_le_bytes());
        }
        // Repeat run
        for _ in 0..8 {
            raw.extend_from_slice(&0x1234u16.to_le_bytes());
        }
        // Incrementing run with wraparound
        for i in 0u16..6 {
            raw.extend_from_slice(&0xfffeu16.wrapping_add(i).to_le_bytes());
        }
        // Decrementing run with wraparound
        for i in 0u16..5 {
            raw.extend_from_slice(&0x0002u16.wrapping_sub(i).to_le_bytes());
        }
        // Odd trailing byte
        raw.push(0xab);

        let enc = encode(&raw).unwrap();
        let mut padded = enc.clone();
        padded.extend_from_slice(&[0xde, 0xad]);
        let dec = decode(&padded, MAX_DECODED).unwrap();
        assert_eq!(dec.bytes, raw);
        assert_eq!(dec.consumed, enc.len());
    }

    #[test]
    fn rejects_wrong_flag_overshoot_and_truncation() {
        assert!(matches!(
            decode(&[0x01, 0x02, 0x00, 0x00, 0x11, 0x22], 64),
            Err(Error::InvalidField { .. })
        ));
        // Even len = 2, repeat run of 2 words = 4 bytes -> overshoot
        assert!(matches!(
            decode(&[0x02, 0x02, 0x00, 0x40, 0x11, 0x22], 64),
            Err(Error::InvalidField { .. })
        ));
        // Truncated odd byte
        assert!(matches!(
            decode(&[0x02, 0x01, 0x00], 64),
            Err(Error::Truncated { .. })
        ));
    }
}
