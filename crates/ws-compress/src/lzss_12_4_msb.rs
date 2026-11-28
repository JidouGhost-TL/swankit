//! MSB-first bitstream 12-bit ring position / 4-bit length LZSS with 0-position EOF.
//!
//! Ring buffer: 4096 bytes zero-initialized, write cursor `bp` starts at `1`
//! (wraps modulo 4096 via `& 0x0FFF`).
//!
//! Bitstream is read MSB-first (bit 7 down to bit 0 per input byte):
//! - `1` bit: read 8-bit literal `b`, emit `b`, store `ring[bp] = b`, advance `bp`.
//! - `0` bit: read 12-bit ring position `rpos`:
//!   - `rpos == 0`: end of stream.
//!   - `rpos != 0`: read 4-bit length `len_raw`, `count = len_raw + 2` (`2..=17`);
//!     copy `count` bytes sequentially from `ring[rpos]`, updating `ring[bp]` and
//!     advancing both `rpos` and `bp` modulo 4096.
use crate::{grow, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Reader, Result};

struct MsbBitReader<'a> {
    r: Reader<'a>,
    cur: u8,
    bits_left: u8,
}

impl<'a> MsbBitReader<'a> {
    fn new(src: &'a [u8]) -> Self {
        Self {
            r: Reader::new(src),
            cur: 0,
            bits_left: 0,
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
    fn read_bits(&mut self, n: u8) -> Result<u16> {
        let mut v = 0u16;
        for _ in 0..n {
            v = (v << 1) | u16::from(self.read_bit()?);
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

    fn write_bits(&mut self, value: u16, n: u8) {
        for shift in (0..n).rev() {
            self.cur = (self.cur << 1) | (((value >> shift) & 1) as u8);
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
        self.out
    }
}

/// Decode a self-terminated 12-bit/4-bit MSB-first ring LZSS stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let mut br = MsbBitReader::new(src);
    let mut ring = [0u8; 4096];
    let mut bp = 1usize;
    let mut out = Vec::new();

    loop {
        if br.read_bit()? == 1 {
            let b = br.read_bits(8)? as u8;
            grow(&out, 1, limit)?;
            out.push(b);
            ring[bp] = b;
            bp = (bp + 1) & 0x0fff;
        } else {
            let mut rpos = usize::from(br.read_bits(12)?);
            if rpos == 0 {
                break;
            }
            let count = usize::from(br.read_bits(4)?) + 2;
            grow(&out, count, limit)?;
            for _ in 0..count {
                let b = ring[rpos];
                out.push(b);
                ring[bp] = b;
                rpos = (rpos + 1) & 0x0fff;
                bp = (bp + 1) & 0x0fff;
            }
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: br.consumed(),
    })
}

/// Deterministic greedy encoder for the 12-bit/4-bit MSB-first ring LZSS format.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    grow(&[], src.len(), MAX_DECODED)?;
    let mut bw = MsbBitWriter::new();
    let mut ring = [0u8; 4096];
    let mut bp = 1usize;
    let mut i = 0usize;

    while i < src.len() {
        let mut best_len = 0usize;
        let mut best_pos = 0usize;
        let max_len = (src.len() - i).min(17);
        if max_len >= 2 {
            // Search non-zero ring positions; simulate sequential ring read/write for overlap.
            let search_back = i.min(4095);
            for dist in 1..=search_back {
                let rpos = (bp + 4096 - dist) & 0x0fff;
                if rpos == 0 {
                    continue;
                }
                let mut matched = 0usize;
                while matched < max_len {
                    let expected = if matched < dist {
                        ring[(rpos + matched) & 0x0fff]
                    } else {
                        src[i + matched - dist]
                    };
                    if expected != src[i + matched] {
                        break;
                    }
                    matched += 1;
                }
                if matched > best_len {
                    best_len = matched;
                    best_pos = rpos;
                    if best_len == max_len {
                        break;
                    }
                }
            }
        }

        if best_len >= 2 {
            bw.write_bits(0, 1);
            bw.write_bits(best_pos as u16, 12);
            bw.write_bits((best_len - 2) as u16, 4);
            for k in 0..best_len {
                ring[bp] = src[i + k];
                bp = (bp + 1) & 0x0fff;
            }
            i += best_len;
        } else {
            let b = src[i];
            bw.write_bits(1, 1);
            bw.write_bits(u16::from(b), 8);
            ring[bp] = b;
            bp = (bp + 1) & 0x0fff;
            i += 1;
        }
    }

    // EOF marker: flag 0 + 12-bit position 0.
    bw.write_bits(0, 1);
    bw.write_bits(0, 12);
    Ok(bw.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_literals_matches_overlap_and_empty() {
        for sample in [
            b"".as_slice(),
            b"A",
            b"AAAAAAAAAAAAAAAAAAAA",
            b"ABCDABCDABCD0123456789ABCDABCD",
        ] {
            let enc = encode(sample).unwrap();
            let mut padded = enc.clone();
            padded.extend_from_slice(&[0xff, 0xfe]);
            let dec = decode(&padded, 1024).unwrap();
            assert_eq!(dec.bytes, sample);
            assert_eq!(dec.consumed, enc.len());
        }
    }

    #[test]
    fn rejects_truncation_and_output_limit() {
        let enc = encode(b"hello hello hello").unwrap();
        assert!(decode(&enc[..enc.len() - 1], 64).is_err());
        assert!(decode(&enc, 8).is_err());
        assert!(decode(&[], 64).is_err());
    }
}
