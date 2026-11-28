//! 6-bit packed signed DPCM audio stream (4 samples per 3 bytes).
//!
//! Each 3-byte group `(b0, b1, b2)` encodes four 6-bit two's-complement deltas:
//! - `d0 = sign_extend_6(b0 & 0x3f)`
//! - `d1 = sign_extend_6(b1 & 0x3f)`
//! - `d2 = sign_extend_6(b2 & 0x3f)`
//! - `d3 = sign_extend_6(((b0 >> 6) & 3) | (((b1 >> 6) & 3) << 2) | (((b2 >> 6) & 3) << 4))`
//!
//! Deltas are accumulated modulo 256 onto an unsigned 8-bit PCM sample
//! initialized to `0x80` (or caller-supplied `init`). Trailing 1 or 2 bytes
//! emit 1 or 2 samples from their low 6 bits. Extent is supplied by the carrier.
use crate::{grow, DecodeOutcome, MAX_STREAM_DECODED};
use formatkit_core::{Error, Result};

/// Default unsigned 8-bit PCM midpoint accumulator.
pub const DEFAULT_INIT: u8 = 0x80;

#[inline]
fn sign_extend_6(v: u8) -> i8 {
    let masked = v & 0x3f;
    if masked & 0x20 != 0 {
        (masked | 0xc0) as i8
    } else {
        masked as i8
    }
}

/// Decode a carrier-bounded 6-bit packed DPCM stream starting from `0x80`.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    decode_with_init(src, DEFAULT_INIT, max_output)
}

/// Decode a carrier-bounded 6-bit packed DPCM stream with explicit initial accumulator.
pub fn decode_with_init(src: &[u8], init: u8, max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_STREAM_DECODED);
    let full_groups = src.len() / 3;
    let rem = src.len() % 3;
    let out_len = full_groups
        .checked_mul(4)
        .and_then(|n| n.checked_add(rem))
        .ok_or(Error::ResourceLimit {
            resource: "decoded bytes",
            requested: u64::MAX,
            limit: limit as u64,
        })?;
    grow(&[], out_len, limit)?;

    let mut out = Vec::with_capacity(out_len);
    let mut acc = init;
    let chunks = src.chunks_exact(3);
    let remainder = chunks.remainder();
    for g in chunks {
        let (b0, b1, b2) = (g[0], g[1], g[2]);
        let d0 = sign_extend_6(b0);
        let d1 = sign_extend_6(b1);
        let d2 = sign_extend_6(b2);
        let raw3 = ((b0 >> 6) & 0x03) | (((b1 >> 6) & 0x03) << 2) | (((b2 >> 6) & 0x03) << 4);
        let d3 = sign_extend_6(raw3);
        for d in [d0, d1, d2, d3] {
            acc = acc.wrapping_add(d as u8);
            out.push(acc);
        }
    }
    for &b in remainder {
        let d = sign_extend_6(b);
        acc = acc.wrapping_add(d as u8);
        out.push(acc);
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: src.len(),
    })
}

/// Deterministic encoder for unsigned 8-bit PCM into 6-bit packed DPCM (initial `0x80`).
///
/// Sample-to-sample deltas are clamped to `-32..=31` while tracking the
/// reconstructed accumulator so signals with `|delta| <= 31` round-trip
/// bit-for-bit. Input lengths with `len % 4 == 3` are rejected because 3 packed
/// bytes always decode to 4 samples.
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    encode_with_init(src, DEFAULT_INIT)
}

/// Deterministic encoder with explicit initial accumulator.
pub fn encode_with_init(src: &[u8], init: u8) -> Result<Vec<u8>> {
    grow(&[], src.len(), MAX_STREAM_DECODED)?;
    if src.len() % 4 == 3 {
        return Err(Error::InvalidField {
            what: "dpcm-6bit sample count cannot have remainder 3 mod 4",
            value: src.len() as u64,
        });
    }
    let mut out = Vec::with_capacity((src.len() / 4) * 3 + (src.len() % 4));
    let mut acc = init;
    let mut step = |target: u8| -> u8 {
        let diff = i16::from(target) - i16::from(acc);
        let clamped = diff.clamp(-32, 31) as i8;
        acc = acc.wrapping_add(clamped as u8);
        (clamped as u8) & 0x3f
    };

    let chunks = src.chunks_exact(4);
    let remainder = chunks.remainder();
    for g in chunks {
        let d0 = step(g[0]);
        let d1 = step(g[1]);
        let d2 = step(g[2]);
        let d3 = step(g[3]);
        out.push(d0 | ((d3 & 0x03) << 6));
        out.push(d1 | (((d3 >> 2) & 0x03) << 6));
        out.push(d2 | (((d3 >> 4) & 0x03) << 6));
    }
    for &s in remainder {
        out.push(step(s));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_packed_groups_and_remainder() {
        // Deltas: +1, -1 (0x3f), +31 (0x1f), -32 (0x20 = 0b10_00_00)
        // d3 bits: lo2=00, mid2=00, hi2=10
        let b0 = 0x01 | (0x00 << 6);
        let b1 = 0x3f | (0x00 << 6);
        let b2 = 0x1f | (0x02 << 6);
        let got = decode(&[b0, b1, b2], 16).unwrap();
        assert_eq!(got.bytes, vec![0x81, 0x80, 0x9f, 0x7f]);
        assert_eq!(got.consumed, 3);

        // Remainder of 2 bytes: +2, -2 (0x3e)
        let rem = decode(&[0x02, 0x3e], 16).unwrap();
        assert_eq!(rem.bytes, vec![0x82, 0x80]);
        assert_eq!(rem.consumed, 2);

        assert_eq!(decode(&[], 0).unwrap().bytes, Vec::<u8>::new());
    }

    #[test]
    fn supports_oversize_streams_and_rejects_overflow() {
        let n = 96 * 1024;
        let wave: Vec<u8> = (0..n)
            .map(|i| (128i32 + ((i as i32 % 30) - 15)) as u8)
            .collect();
        let enc = encode(&wave).unwrap();
        assert_eq!(enc.len(), (n / 4) * 3);
        let dec = decode(&enc, n).unwrap();
        assert_eq!(dec.bytes, wave);
        assert_eq!(dec.consumed, enc.len());

        assert!(decode(&enc, n - 1).is_err());
        assert!(encode(&[0x80, 0x81, 0x82]).is_err());
    }
}
