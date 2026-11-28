//! 16-byte FIFO dictionary + repeat-last-byte MSB-first bitstream codec.
//!
//! Header (20 bytes):
//! - `+0..+1`: `u16_le out_len` — exact uncompressed byte count (`>= 1`).
//! - `+2..+17`: `u8[16]` — initial 16-byte FIFO dictionary.
//! - `+18`: `u8 init_byte` — emitted unconditionally as `out[0]` and stored as `last_byte`.
//! - `+19`: `u8 first_bits` — initial MSB-first bitstream byte.
//!
//! Decode loop while `out.len() < out_len`:
//! - Read 1 bit `b0`:
//!   - `1`: repeat `last_byte`.
//!   - `0`: read 1 bit `b1`:
//!     - `1`: read 4-bit MSB index `idx`, `last_byte = dict[idx]` (dictionary unchanged).
//!     - `0`: read 8-bit MSB literal `val`, `last_byte = val`, `dict[dict_idx] = val`,
//!       `dict_idx = (dict_idx + 1) & 0x0F`.
//! - Emit `last_byte`.
use crate::{grow, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

struct LazyMsbReader<'a> {
    r: Reader<'a>,
    cur: u8,
    bits_left: u8,
}

impl<'a> LazyMsbReader<'a> {
    fn new(r: Reader<'a>, first_byte: u8) -> Self {
        Self {
            r,
            cur: first_byte,
            bits_left: 8,
        }
    }

    #[inline]
    fn read_bit(&mut self) -> Result<u8> {
        if self.bits_left == 0 {
            self.cur = self.r.u8()?;
            self.bits_left = 8;
        }
        let bit = (self.cur >> 7) & 1;
        self.cur <<= 1;
        self.bits_left -= 1;
        Ok(bit)
    }

    #[inline]
    fn read_bits(&mut self, n: u8) -> Result<u8> {
        let mut v = 0u8;
        for _ in 0..n {
            v = (v << 1) | self.read_bit()?;
        }
        Ok(v)
    }

    fn consumed(&self) -> usize {
        self.r.pos()
    }
}

struct MsbBitWriter {
    out: Vec<u8>,
    cur: u8,
    bits_used: u8,
}

impl MsbBitWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            cur: 0,
            bits_used: 0,
        }
    }

    fn write_bits(&mut self, value: u8, n: u8) {
        for shift in (0..n).rev() {
            self.cur = (self.cur << 1) | ((value >> shift) & 1);
            self.bits_used += 1;
            if self.bits_used == 8 {
                self.out.push(self.cur);
                self.cur = 0;
                self.bits_used = 0;
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bits_used > 0 {
            self.out.push(self.cur << (8 - self.bits_used));
        }
        if self.out.is_empty() {
            self.out.push(0);
        }
        self.out
    }
}

/// Decode a self-terminated 16-byte FIFO dictionary + repeat-last-byte stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    if src.len() < 20 {
        return Err(Error::Truncated {
            offset: 0,
            needed: 20,
            available: src.len(),
        });
    }
    let mut r = Reader::new(src);
    let out_len = usize::from(r.u16_le()?);
    if out_len == 0 {
        return Err(Error::InvalidField {
            what: "dict16-rle decoded length must be positive",
            value: 0,
        });
    }
    let limit = max_output.min(MAX_DECODED);
    grow(&[], out_len, limit)?;

    let mut dict = [0u8; 16];
    dict.copy_from_slice(r.bytes(16)?);
    let mut last_byte = r.u8()?;
    let first_bits = r.u8()?;

    let mut br = LazyMsbReader::new(r, first_bits);
    let mut dict_idx = 0usize;
    let mut out = Vec::with_capacity(out_len);
    out.push(last_byte);

    while out.len() < out_len {
        if br.read_bit()? == 1 {
            out.push(last_byte);
        } else if br.read_bit()? == 1 {
            let idx = usize::from(br.read_bits(4)?);
            last_byte = dict[idx];
            out.push(last_byte);
        } else {
            let val = br.read_bits(8)?;
            last_byte = val;
            dict[dict_idx] = val;
            dict_idx = (dict_idx + 1) & 0x0f;
            out.push(last_byte);
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: br.consumed(),
    })
}

/// Deterministic encoder for the 16-byte FIFO dictionary + repeat-last-byte format.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    if src.is_empty() || src.len() > usize::from(u16::MAX) {
        return Err(Error::InvalidField {
            what: "dict16-rle input length must be 1..=65535",
            value: src.len() as u64,
        });
    }
    // Seed initial dictionary with up to 16 most frequent bytes (0x00 first if present).
    let mut counts = [0usize; 256];
    for &b in src {
        counts[usize::from(b)] += 1;
    }
    let mut ranked: Vec<(usize, u8)> = counts
        .iter()
        .enumerate()
        .filter(|(_, &c)| c > 0)
        .map(|(b, &c)| (c, b as u8))
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut dict = [0u8; 16];
    for (slot, &(_, byte)) in dict.iter_mut().zip(ranked.iter()) {
        *slot = byte;
    }

    let mut out = Vec::with_capacity(20 + src.len());
    out.extend_from_slice(&(src.len() as u16).to_le_bytes());
    out.extend_from_slice(&dict);
    out.push(src[0]);

    let mut bw = MsbBitWriter::new();
    let mut last_byte = src[0];
    let mut dict_idx = 0usize;

    for &b in &src[1..] {
        if b == last_byte {
            bw.write_bits(1, 1);
        } else if let Some(idx) = dict.iter().position(|&d| d == b) {
            bw.write_bits(0b01, 2);
            bw.write_bits(idx as u8, 4);
            last_byte = b;
        } else {
            bw.write_bits(0b00, 2);
            bw.write_bits(b, 8);
            last_byte = b;
            dict[dict_idx] = b;
            dict_idx = (dict_idx + 1) & 0x0f;
        }
    }

    out.extend_from_slice(&bw.finish());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_repeat_dict_and_fifo_updates() {
        let mut sample = Vec::new();
        sample.extend_from_slice(&[0x00; 12]);
        for i in 0u8..40 {
            sample.push(i);
            sample.push(i);
        }
        sample.extend_from_slice(&[0xff, 0x00, 0xaa, 0xaa, 0x55, 0x01, 0x02, 0x03]);

        let enc = encode(&sample).unwrap();
        let mut padded = enc.clone();
        padded.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        let dec = decode(&padded, 1024).unwrap();
        assert_eq!(dec.bytes, sample);
        assert_eq!(dec.consumed, enc.len());

        // Single-byte edge case
        let one = encode(&[0x42]).unwrap();
        assert_eq!(one.len(), 20);
        let dec1 = decode(&one, 16).unwrap();
        assert_eq!(dec1.bytes, vec![0x42]);
        assert_eq!(dec1.consumed, 20);
    }

    #[test]
    fn rejects_zero_length_truncation_and_limit() {
        assert!(decode(&[0; 19], 64).is_err());
        assert!(decode(&[0; 20], 64).is_err());
        let enc = encode(b"0123456789abcdefghij").unwrap();
        assert!(decode(&enc[..enc.len() - 1], 64).is_err());
        assert!(decode(&enc, 10).is_err());
    }
}
