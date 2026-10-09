//! Byte commands: backward matches, literals, zero-prefixed words and runs.
//! `7f ff` terminates; the matching length-33/distance-1 token is reserved.
use crate::{copy_match, grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Reader, Result, Writer};

/// Decode one terminated stream, reporting the extent through `7f ff`.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    decode_with_history(src, &[], max_output)
}

/// Decode with known destination bytes preceding the output (last 1024 used).
/// A missing prehistory reference is an error, never an implicit zero fill.
pub fn decode_with_history(src: &[u8], history: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let history = &history[history.len().saturating_sub(1024)..];
    let limit = max_output.min(MAX_DECODED);
    let mut r = Reader::new(src);
    let mut out = Vec::new();
    loop {
        let c = r.u8()?;
        match c {
            0x00..=0x7f => {
                let b = r.u8()?;
                if c == 0x7f && b == 0xff {
                    return Ok(DecodeOutcome {
                        bytes: out,
                        consumed: r.pos(),
                    });
                }
                let len = usize::from((c >> 2) & 31) + 2;
                let distance = 1024 - ((usize::from(c & 3) << 8) | usize::from(b));
                copy_match(&mut out, history, distance, len, limit)?;
            }
            0x80..=0x9f => {
                let len = usize::from(c & 31) + 1;
                grow(&out, len, limit)?;
                out.extend_from_slice(r.bytes(len)?);
            }
            0xa0..=0xbf => {
                let pairs = usize::from(c & 31) + 1;
                grow(&out, pairs * 2, limit)?;
                for &b in r.bytes(pairs)? {
                    out.extend_from_slice(&[0, b]);
                }
            }
            0xc0..=0xdf => {
                let len = usize::from(c & 31) + 2;
                grow(&out, len, limit)?;
                let value = r.u8()?;
                out.resize(out.len() + len, value);
            }
            0xe0..=0xff => {
                let len = if c == 0xff {
                    usize::from(r.u8()?) + 32
                } else {
                    usize::from(c & 31) + 1
                };
                grow(&out, len, limit)?;
                out.resize(out.len() + len, 0);
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
enum Token {
    #[default]
    End,
    Match(usize, usize),
    Literal(usize),
    Words(usize),
    Run(usize),
    Zero(usize),
}

/// Minimum-byte dynamic-programming parse, including the terminating token.
/// Ties follow the reference packer: match, literal, words, zeros, run; then
/// shortest length, and nearest distance attaining the longest match.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    let n = src.len();
    let mut cost = vec![0usize; n + 1];
    let mut tokens = vec![Token::End; n + 1];
    let mut run = vec![0usize; n + 1];
    let mut words = vec![0usize; n + 2];
    let mut matches = [0usize; 1025];
    cost[n] = 2;
    for i in (0..n).rev() {
        run[i] = if i + 1 < n && src[i] == src[i + 1] {
            run[i + 1] + 1
        } else {
            1
        };
        words[i] = if i + 1 < n && src[i] == 0 {
            words[i + 2] + 1
        } else {
            0
        };
    }
    for i in (0..n).rev() {
        let mut best = usize::MAX;
        let mut token = Token::End;
        let mut choose = |price: usize, candidate: Token| {
            if price < best {
                best = price;
                token = candidate;
            }
        };
        let (mut longest, mut distance) = (0, 0);
        for (d, m) in matches.iter_mut().enumerate().skip(1) {
            *m = if i >= d && src[i] == src[i - d] {
                (*m + 1).min(33)
            } else {
                0
            };
            let usable = if d == 1 { (*m).min(32) } else { *m };
            if usable > longest {
                longest = usable;
                distance = d;
            }
        }
        for len in 2..=longest {
            choose(2 + cost[i + len], Token::Match(len, distance));
        }
        for len in 1..=32.min(n - i) {
            choose(1 + len + cost[i + len], Token::Literal(len));
        }
        for pairs in 1..=32.min(words[i]) {
            choose(1 + pairs + cost[i + pairs * 2], Token::Words(pairs));
        }
        if src[i] == 0 {
            for len in 1..=287.min(run[i]) {
                choose(usize::from(len > 31) + 1 + cost[i + len], Token::Zero(len));
            }
        }
        for len in 2..=33.min(run[i]) {
            choose(2 + cost[i + len], Token::Run(len));
        }
        cost[i] = best;
        tokens[i] = token;
    }
    let mut w = Writer::new();
    let mut i = 0;
    while i < n {
        let used = match tokens[i] {
            Token::Match(len, distance) => {
                let field = 1024 - distance;
                w.u8((((len - 2) << 2) | (field >> 8)) as u8);
                w.u8(field as u8);
                len
            }
            Token::Literal(len) => {
                w.u8(0x80 | (len - 1) as u8);
                w.bytes(&src[i..i + len]);
                len
            }
            Token::Words(pairs) => {
                w.u8(0xa0 | (pairs - 1) as u8);
                for p in 0..pairs {
                    w.u8(src[i + 2 * p + 1]);
                }
                pairs * 2
            }
            Token::Run(len) => {
                w.u8(0xc0 | (len - 2) as u8);
                w.u8(src[i]);
                len
            }
            Token::Zero(len) => {
                if len <= 31 {
                    w.u8(0xe0 | (len - 1) as u8);
                } else {
                    w.u8(0xff);
                    w.u8((len - 32) as u8);
                }
                len
            }
            Token::End => unreachable!("every nonempty suffix has a literal parse"),
        };
        i += used;
    }
    w.bytes(&[0x7f, 0xff]);
    Ok(w.into_vec())
}
