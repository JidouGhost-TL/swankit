//! Big-endian token-counted bit-flag LZSS stream.
//!
//! Layout:
//! - 3-byte header: `u8 flags`, `u16_be token_count`.
//! - LSB-first 8-bit flag stream:
//!   - `0`: 1 literal byte.
//!   - `1, 0`: literal run of `next_byte + 11` bytes.
//!   - `1, 1`: `u16_le w` with `dist = (w & 0x0fff) + 1`, `len = (w >> 12) & 0x0f`;
//!     if `len == 0`, `len = next_byte + 16`; copy `len + 2` bytes from `out[-dist]`.
use crate::{copy_match, grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Reader, Result, Writer};

/// Decode a big-endian token-counted bit-flag LZSS stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let mut r = Reader::new(src);
    let _header = r.u8()?;
    let tokens = usize::from(r.u16_be()?);
    let mut out = Vec::new();
    let mut flags: u16 = 0;

    let mut next_bit = |r: &mut Reader<'_>| -> Result<bool> {
        let hi = (flags >> 8) as u8;
        flags >>= 1;
        if (hi >> 1) == 0 {
            flags = 0xff00 | u16::from(r.u8()?);
        }
        Ok((flags & 1) != 0)
    };

    for _ in 0..tokens {
        if !next_bit(&mut r)? {
            grow(&out, 1, limit)?;
            out.push(r.u8()?);
        } else if !next_bit(&mut r)? {
            let cnt = usize::from(r.u8()?) + 11;
            grow(&out, cnt, limit)?;
            out.extend_from_slice(r.bytes(cnt)?);
        } else {
            let w = r.u16_le()?;
            let dist = usize::from(w & 0x0fff) + 1;
            let mut len = usize::from((w >> 12) & 0x0f);
            if len == 0 {
                len = usize::from(r.u8()?) + 16;
            }
            len += 2;
            copy_match(&mut out, &[], dist, len, limit)?;
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

fn find_match(src: &[u8], pos: usize) -> Option<(usize, usize)> {
    let max_dist = pos.min(4096);
    let max_len = (src.len() - pos).min(273);
    if max_dist == 0 || max_len < 3 {
        return None;
    }
    let mut best_len = 0usize;
    let mut best_dist = 0usize;
    for d in 1..=max_dist {
        let mut l = 0usize;
        while l < max_len && src[pos + l] == src[pos + l - d] {
            l += 1;
        }
        if l >= 3 && l > best_len {
            best_len = l;
            best_dist = d;
            if best_len == max_len {
                break;
            }
        }
    }
    (best_len >= 3).then_some((best_dist, best_len))
}

/// Deterministic encoder for big-endian token-counted bit-flag LZSS streams.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    input_size(src)?;
    let mut body = Writer::new();
    let mut tokens: u16 = 0;

    // Buffer flag bits and payload bytes in interleaved flag-byte order:
    // the decoder refills a flag byte on demand at the exact moment a bit is needed.
    struct BitEncoder {
        out: Vec<u8>,
        flag_pos: Option<usize>,
        bit_idx: u8,
    }
    impl BitEncoder {
        fn new() -> Self {
            Self {
                out: Vec::new(),
                flag_pos: None,
                bit_idx: 0,
            }
        }
        fn push_bit(&mut self, bit: bool) {
            if self.flag_pos.is_none() || self.bit_idx == 8 {
                self.flag_pos = Some(self.out.len());
                self.out.push(0);
                self.bit_idx = 0;
            }
            if bit {
                let idx = self.flag_pos.unwrap();
                self.out[idx] |= 1 << self.bit_idx;
            }
            self.bit_idx += 1;
        }
        fn push_byte(&mut self, b: u8) {
            self.out.push(b);
        }
        fn push_bytes(&mut self, bs: &[u8]) {
            self.out.extend_from_slice(bs);
        }
    }

    let mut enc = BitEncoder::new();
    let mut pos = 0usize;
    let mut pending_lits = 0usize;
    let mut pending_start = 0usize;

    let flush_lits = |enc: &mut BitEncoder, tokens: &mut u16, start: usize, mut count: usize| {
        let mut off = start;
        while count >= 11 {
            let chunk = count.min(266);
            enc.push_bit(true);
            enc.push_bit(false);
            enc.push_byte((chunk - 11) as u8);
            enc.push_bytes(&src[off..off + chunk]);
            *tokens = tokens.wrapping_add(1);
            off += chunk;
            count -= chunk;
        }
        while count > 0 {
            enc.push_bit(false);
            enc.push_byte(src[off]);
            *tokens = tokens.wrapping_add(1);
            off += 1;
            count -= 1;
        }
    };

    while pos < src.len() {
        if let Some((dist, len)) = find_match(src, pos) {
            if pending_lits > 0 {
                flush_lits(&mut enc, &mut tokens, pending_start, pending_lits);
                pending_lits = 0;
            }
            enc.push_bit(true);
            enc.push_bit(true);
            let adj = len - 2;
            if adj <= 15 {
                let w = ((adj as u16) << 12) | ((dist - 1) as u16);
                enc.push_bytes(&w.to_le_bytes());
            } else {
                let w = (dist - 1) as u16;
                enc.push_bytes(&w.to_le_bytes());
                enc.push_byte((adj - 16) as u8);
            }
            tokens = tokens.wrapping_add(1);
            pos += len;
            pending_start = pos;
        } else {
            if pending_lits == 0 {
                pending_start = pos;
            }
            pending_lits += 1;
            pos += 1;
        }
    }
    if pending_lits > 0 {
        flush_lits(&mut enc, &mut tokens, pending_start, pending_lits);
    }

    body.u8(0);
    body.u16_be(tokens);
    body.bytes(&enc.out);
    Ok(body.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_literals_runs_short_and_long_matches() {
        // Token 0: literal 0x41 (bit 0)
        // Token 1: short match dist=1, len=4 (bits 1,1; w = (2 << 12) | 0 = 0x2000)
        // Token 2: literal run of 11 bytes (bits 1,0; next=0)
        // Token 3: long match dist=1, len=20 (bits 1,1; w = 0x0000, extra = 20-2-16 = 2)
        // Flag bits LSB-first: [0, 1, 1, 1, 0, 1, 1] = 0b0110_1110 = 0x6e
        let mut stream = vec![0x00, 0x00, 0x04, 0x6e, 0x41, 0x00, 0x20, 0x00];
        stream.extend(0u8..11);
        stream.extend_from_slice(&[0x00, 0x00, 0x02]);
        let payload_len = stream.len();
        stream.extend_from_slice(&[0xde, 0xad]);

        let got = decode(&stream, 128).unwrap();
        let mut expected = vec![0x41; 5];
        expected.extend(0u8..11);
        expected.extend(std::iter::repeat_n(10u8, 20));
        assert_eq!(got.bytes, expected);
        assert_eq!(got.consumed, payload_len);
        assert_eq!(decode(&[0, 0, 0, 0xaa], 0).unwrap().consumed, 3);
    }

    #[test]
    fn rejects_truncation_bad_distance_and_overflow() {
        assert!(decode(&[], 64).is_err());
        assert!(decode(&[0, 0], 64).is_err());
        assert!(decode(&[0, 0, 1], 64).is_err());
        // Match with empty history
        assert!(decode(&[0, 0, 1, 0x03, 0x00, 0x10], 64).is_err());
        // Output overflow on literal run
        assert!(decode(&[0, 0, 1, 0x01, 0x00, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11], 8).is_err());
        assert!(encode(&vec![0; MAX_DECODED + 1]).is_err());
    }

    #[test]
    fn round_trips_all_token_forms() {
        for n in [0, 1, 11, 25, 128, 2048] {
            let raw: Vec<u8> = (0..n).map(|i| ((i / 5 + i % 17) & 0xff) as u8).collect();
            let enc = encode(&raw).unwrap();
            let dec = decode(&enc, n).unwrap();
            assert_eq!(dec.bytes, raw);
            assert_eq!(dec.consumed, enc.len());
            assert_eq!(encode(&raw).unwrap(), enc);
        }
    }
}
