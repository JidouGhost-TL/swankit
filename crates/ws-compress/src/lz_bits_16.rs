//! Interleaved little-endian flag words, consumed least-significant bit first.
//! The next word is fetched immediately after bit 16, before token operands.
use crate::{copy_match, grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, LsbBitCursor, Reader, Result};

/// Decode one terminated stream, including eagerly read flag words in extent.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    decode_with_history(src, &[], max_output)
}

/// Decode with known preceding destination bytes (last 8192 used).
pub fn decode_with_history(src: &[u8], history: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let history = &history[history.len().saturating_sub(8192)..];
    let limit = max_output.min(MAX_DECODED);
    let mut r = Reader::new(src);
    let mut flags = LsbBitCursor::new(r.bytes(2)?);
    let mut out = Vec::new();
    loop {
        if bit(&mut r, &mut flags)? != 0 {
            grow(&out, 1, limit)?;
            out.push(r.u8()?);
            continue;
        }
        let (distance, len) = if bit(&mut r, &mut flags)? != 0 {
            let word = r.u16_le()?;
            let distance = 8192 - usize::from(word >> 3 & 0x1f00 | word & 0xff);
            let len = if word & 0x700 != 0 {
                usize::from((word >> 8) & 7) + 2
            } else {
                let extra = r.u8()?;
                if extra == 0 {
                    return Ok(DecodeOutcome {
                        bytes: out,
                        consumed: r.pos(),
                    });
                }
                usize::from(extra) + 1
            };
            (distance, len)
        } else {
            let a = bit(&mut r, &mut flags)?;
            let b = bit(&mut r, &mut flags)?;
            (256 - usize::from(r.u8()?), ((a << 1) | b) as usize + 2)
        };
        copy_match(&mut out, history, distance, len, limit)?;
    }
}

// Grammar-level eager word refill composed from Formatkit's checked cursors.
fn bit<'a>(r: &mut Reader<'a>, flags: &mut LsbBitCursor<'a>) -> Result<u32> {
    let value = flags
        .read(1)
        .ok_or_else(|| Error::Malformed("missing buffered flag word".into()))?;
    if flags.remaining_bits() == 0 {
        *flags = LsbBitCursor::new(r.bytes(2)?);
    }
    Ok(value)
}

struct FlagWriter {
    bytes: Vec<u8>,
    slot: usize,
    used: u32,
}
impl FlagWriter {
    fn new() -> Self {
        Self {
            bytes: vec![0, 0],
            slot: 0,
            used: 0,
        }
    }
    fn bit(&mut self, value: u32) {
        self.bytes[self.slot + (self.used / 8) as usize] |= ((value & 1) << (self.used % 8)) as u8;
        self.used += 1;
        if self.used == 16 {
            self.slot = self.bytes.len();
            self.bytes.extend_from_slice(&[0, 0]);
            self.used = 0;
        }
    }
    fn data(&mut self, value: u8) {
        self.bytes.push(value);
    }
}

/// Reference-compatible minimum-bit parse with a 512-candidate prefix chain.
/// Ties retain literals, then shortest short match, then shortest long match;
/// equal-length matches retain the most recent candidate. Match operands and
/// the terminator have the same placement as the game's eager flag refill.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    let n = src.len();
    let mut cost = vec![0usize; n + 1];
    let mut lengths = vec![1usize; n];
    let mut distances = vec![0isize; n];
    let mut head = vec![None; 65536];
    let mut previous = vec![None; n];
    let mut long = vec![(0usize, 0usize); n];
    let mut short = vec![(0usize, 0usize); n];
    for i in 0..n.saturating_sub(1) {
        let key = usize::from(src[i]) << 8 | usize::from(src[i + 1]);
        let mut candidate = head[key];
        let mut visited = 0;
        while let Some(j) = candidate {
            if i - j > 8192 || visited == 512 {
                break;
            }
            let mut len = 0;
            while len < 256 && i + len < n && src[j + len] == src[i + len] {
                len += 1;
            }
            if len > long[i].0 {
                long[i] = (len, i - j);
            }
            if i - j <= 256 && len.min(5) > short[i].0 {
                short[i] = (len.min(5), i - j);
            }
            if long[i].0 == 256 && short[i].0 == 5 {
                break;
            }
            visited += 1;
            candidate = previous[j];
        }
        previous[i] = head[key];
        head[key] = Some(i);
    }
    for i in (0..n).rev() {
        let mut best = cost[i + 1] + 9;
        for len in 2..=short[i].0 {
            let price = cost[i + len] + 12;
            if price < best {
                best = price;
                lengths[i] = len;
                distances[i] = -(short[i].1 as isize);
            }
        }
        for len in 2..=long[i].0 {
            let price = cost[i + len] + if (3..=9).contains(&len) { 18 } else { 26 };
            if price < best {
                best = price;
                lengths[i] = len;
                distances[i] = long[i].1 as isize;
            }
        }
        cost[i] = best;
    }
    let mut w = FlagWriter::new();
    let mut i = 0;
    while i < n {
        let len = lengths[i];
        if len == 1 {
            w.bit(1);
            w.data(src[i]);
        } else if distances[i] < 0 {
            w.bit(0);
            w.bit(0);
            w.bit(((len - 2) >> 1) as u32);
            w.bit(((len - 2) & 1) as u32);
            w.data((256 - distances[i].unsigned_abs()) as u8);
        } else {
            let field = 8192 - distances[i] as usize;
            w.bit(0);
            w.bit(1);
            w.data(field as u8);
            if (3..=9).contains(&len) {
                w.data((((field >> 8) << 3) | (len - 2)) as u8);
            } else {
                w.data(((field >> 8) << 3) as u8);
                w.data((len - 1) as u8);
            }
        }
        i += len;
    }
    w.bit(0);
    w.bit(1);
    w.data(0);
    w.data(0);
    w.data(0);
    Ok(w.bytes)
}
