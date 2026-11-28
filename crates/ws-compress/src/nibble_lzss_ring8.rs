//! 4-bit nibble MSB-first bitstream LZSS with a 256-entry ring buffer.
//!
//! Framing:
//! - Little-endian `u16` declared decoded length in bytes.
//! - MSB-first bitstream:
//!   - Flag `1`: 4-bit literal nibble.
//!   - Flag `0`: 8-bit ring index followed by 3-bit length code (`count = code + 3`, `3..=10` nibbles).
//! - Ring buffer has 256 nibble entries initialized to zero, with write cursor starting at `1`.
//! - Output nibbles are packed MSB-first into bytes (`(hi << 4) | lo`).
use crate::{grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

struct MsbBitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bits_left: u8,
}

impl<'a> MsbBitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bits_left: 8,
        }
    }

    fn read(&mut self, n: u8) -> Result<u16> {
        debug_assert!(n >= 1 && n <= 8);
        if self.pos >= self.data.len() {
            return Err(Error::Truncated {
                offset: self.pos,
                needed: 1,
                available: 0,
            });
        }
        if n < self.bits_left {
            let shift = self.bits_left - n;
            let mask = (1u16 << n) - 1;
            let val = (u16::from(self.data[self.pos]) >> shift) & mask;
            self.bits_left -= n;
            return Ok(val);
        }
        let low_mask = (1u16 << self.bits_left) - 1;
        let mut val = u16::from(self.data[self.pos]) & low_mask;
        self.pos += 1;
        if n > self.bits_left {
            let rem = n - self.bits_left;
            if self.pos >= self.data.len() {
                return Err(Error::Truncated {
                    offset: self.pos,
                    needed: 1,
                    available: 0,
                });
            }
            val = (val << rem) | (u16::from(self.data[self.pos]) >> (8 - rem));
        }
        self.bits_left = self.bits_left + 8 - n;
        Ok(val)
    }

    fn bytes_consumed(&self) -> usize {
        self.pos + usize::from(self.bits_left != 8)
    }
}

struct MsbBitWriter {
    bytes: Vec<u8>,
    current: u8,
    bits_used: u8,
}

impl MsbBitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            current: 0,
            bits_used: 0,
        }
    }

    fn write(&mut self, value: u16, n: u8) {
        for i in (0..n).rev() {
            let bit = ((value >> i) & 1) as u8;
            self.current = (self.current << 1) | bit;
            self.bits_used += 1;
            if self.bits_used == 8 {
                self.bytes.push(self.current);
                self.current = 0;
                self.bits_used = 0;
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bits_used > 0 {
            self.current <<= 8 - self.bits_used;
            self.bytes.push(self.current);
        }
        self.bytes
    }
}

/// Decode one nibble-LZSS stream prefixed by its little-endian `u16` byte length.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let mut r = Reader::new(src);
    let out_len = usize::from(r.u16_le()?);
    let limit = max_output.min(MAX_DECODED);
    grow(&[], out_len, limit)?;
    if out_len == 0 {
        return Ok(DecodeOutcome {
            bytes: Vec::new(),
            consumed: 2,
        });
    }

    let mut br = MsbBitReader::new(&src[2..]);
    let mut ring = [0u8; 256];
    let mut bp: usize = 1;
    let mut out = Vec::with_capacity(out_len);
    let mut high_phase = true;
    let mut current_byte = 0u8;

    while out.len() < out_len {
        if br.read(1)? == 1 {
            let nib = br.read(4)? as u8;
            ring[bp] = nib;
            bp = (bp + 1) & 0xFF;
            if high_phase {
                current_byte = nib << 4;
                high_phase = false;
            } else {
                out.push(current_byte | nib);
                high_phase = true;
            }
        } else {
            let mut pos = br.read(8)? as usize;
            let count = usize::from(br.read(3)?) + 3;
            for _ in 0..count {
                let nib = ring[pos];
                pos = (pos + 1) & 0xFF;
                ring[bp] = nib;
                bp = (bp + 1) & 0xFF;
                if out.len() < out_len {
                    if high_phase {
                        current_byte = nib << 4;
                        high_phase = false;
                    } else {
                        out.push(current_byte | nib);
                        high_phase = true;
                    }
                }
            }
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: 2 + br.bytes_consumed(),
    })
}

/// Deterministic encoder for 4-bit nibble MSB-bitstream 256-ring LZSS.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    if src.len() > u16::MAX as usize {
        return Err(Error::InvalidField {
            what: "nibble LZSS output length exceeds u16",
            value: src.len() as u64,
        });
    }
    let mut out = (src.len() as u16).to_le_bytes().to_vec();
    if src.is_empty() {
        return Ok(out);
    }

    let mut nibbles = Vec::with_capacity(src.len() * 2);
    for &b in src {
        nibbles.push(b >> 4);
        nibbles.push(b & 0x0F);
    }

    let mut ring = [0u8; 256];
    let mut bp: usize = 1;
    let mut bw = MsbBitWriter::new();
    let mut i = 0;

    while i < nibbles.len() {
        let max_len = (nibbles.len() - i).min(10);
        let mut best_len = 0usize;
        let mut best_pos = 0usize;

        if max_len >= 3 {
            for start in 0..256 {
                let mut sim_ring = ring;
                let mut sim_bp = bp;
                let mut sim_pos = start;
                let mut matched = 0;
                while matched < max_len {
                    let v = sim_ring[sim_pos];
                    if v != nibbles[i + matched] {
                        break;
                    }
                    sim_pos = (sim_pos + 1) & 0xFF;
                    sim_ring[sim_bp] = v;
                    sim_bp = (sim_bp + 1) & 0xFF;
                    matched += 1;
                }
                if matched >= 3 && matched > best_len {
                    best_len = matched;
                    best_pos = start;
                    if best_len == max_len {
                        break;
                    }
                }
            }
        }

        if best_len >= 3 {
            bw.write(0, 1);
            bw.write(best_pos as u16, 8);
            bw.write((best_len - 3) as u16, 3);
            let mut pos = best_pos;
            for _ in 0..best_len {
                let v = ring[pos];
                pos = (pos + 1) & 0xFF;
                ring[bp] = v;
                bp = (bp + 1) & 0xFF;
            }
            i += best_len;
        } else {
            let nib = nibbles[i];
            bw.write(1, 1);
            bw.write(u16::from(nib), 4);
            ring[bp] = nib;
            bp = (bp + 1) & 0xFF;
            i += 1;
        }
    }

    out.extend_from_slice(&bw.finish());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_patterns_and_zeros() {
        let cases: &[Vec<u8>] = &[
            vec![],
            vec![0x12],
            vec![0x00; 32],
            vec![0xAA; 64],
            (0..256).map(|i| ((i * 7) ^ (i >> 2)) as u8).collect(),
            b"ABABABABABABABAB0123456789abcdef".to_vec(),
        ];
        for src in cases {
            let enc = encode(src).unwrap();
            let dec = decode(&enc, src.len().max(1)).unwrap();
            assert_eq!(dec.bytes, *src);
            assert_eq!(dec.consumed, enc.len());
        }
    }

    #[test]
    fn rejects_truncation_and_limits() {
        assert!(decode(&[], 64).is_err());
        assert!(decode(&[1], 64).is_err());
        assert!(decode(&[4, 0], 64).is_err());
        let enc = encode(&[0x12, 0x34, 0x56, 0x78]).unwrap();
        assert!(decode(&enc, 3).is_err());
    }
}
