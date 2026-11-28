//! 4-bit packed unsigned wavetable-streamed PCM audio stream.
//!
//! Stream header (1 byte):
//! - Byte `0`: `(0x2 << 4) | rate_code`, where `rate_code` selects playback rate
//!   (`1` = 12000 Hz, `2` = 6000 Hz, `3` = 4000 Hz, `4` = 3000 Hz).
//!
//! Payload (`src[1..]`):
//! - Packed unsigned 4-bit PCM samples, low nibble first then high nibble.
//! - Each 4-bit sample (`0..=15`) is scaled to unsigned 8-bit PCM (`<< 4`).
//!
//! Stream extent is supplied by the enclosing carrier.
use crate::{grow, DecodeOutcome, MAX_STREAM_DECODED};
use formatkit_core::{Error, Result};

/// Decode playback sample rate from the 1-byte header, if valid.
pub fn header_sample_rate(src: &[u8]) -> Option<u32> {
    let &b0 = src.first()?;
    if (b0 >> 4) != 2 {
        return None;
    }
    Some(rate_from_code(b0 & 0x0f))
}

#[inline]
fn rate_from_code(code: u8) -> u32 {
    match code {
        1 => 12_000,
        2 => 6_000,
        3 => 4_000,
        4 => 3_000,
        _ => 6_000,
    }
}

/// Decode a carrier-bounded 4-bit packed unsigned PCM audio stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    if src.is_empty() {
        return Err(Error::Truncated {
            offset: 0,
            needed: 1,
            available: 0,
        });
    }
    let b0 = src[0];
    if (b0 >> 4) != 2 {
        return Err(Error::InvalidField {
            what: "wave-4bit-packed header codec tag must be 0x2",
            value: u64::from(b0 >> 4),
        });
    }
    let data = &src[1..];
    let limit = max_output.min(MAX_STREAM_DECODED);
    let out_len = data.len().checked_mul(2).ok_or(Error::ResourceLimit {
        resource: "decoded bytes",
        requested: u64::MAX,
        limit: limit as u64,
    })?;
    grow(&[], out_len, limit)?;

    let mut out = Vec::with_capacity(out_len);
    for &b in data {
        out.push((b & 0x0f) << 4);
        out.push(b & 0xf0);
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: src.len(),
    })
}

/// Deterministic encoder for unsigned 8-bit PCM (default 6000 Hz, top 4 bits per sample).
pub fn encode(pcm: &[u8]) -> Result<Vec<u8>> {
    encode_with_rate(pcm, 2)
}

/// Deterministic encoder with explicit rate code (`1..=4`).
pub fn encode_with_rate(pcm: &[u8], rate_code: u8) -> Result<Vec<u8>> {
    grow(&[], pcm.len(), MAX_STREAM_DECODED)?;
    if !(1..=4).contains(&rate_code) {
        return Err(Error::InvalidField {
            what: "wave-4bit-packed rate_code must be 1..=4",
            value: u64::from(rate_code),
        });
    }
    if pcm.len() & 1 != 0 {
        return Err(Error::InvalidField {
            what: "wave-4bit-packed requires an even sample count",
            value: pcm.len() as u64,
        });
    }
    let mut out = Vec::with_capacity(1 + pcm.len() / 2);
    out.push(0x20 | (rate_code & 0x0f));
    for pair in pcm.chunks_exact(2) {
        let lo = (pair[0] >> 4) & 0x0f;
        let hi = pair[1] & 0xf0;
        out.push(hi | lo);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_low_then_high_nibbles_and_round_trips() {
        let stream = [0x21, 0x84, 0xf0];
        assert_eq!(header_sample_rate(&stream), Some(12_000));
        let got = decode(&stream, 16).unwrap();
        assert_eq!(got.bytes, vec![0x40, 0x80, 0x00, 0xf0]);
        assert_eq!(got.consumed, 3);

        let enc = encode_with_rate(&got.bytes, 1).unwrap();
        assert_eq!(enc, stream);
    }

    #[test]
    fn validates_header_limits_and_large_streams() {
        assert!(decode(&[], 16).is_err());
        assert!(decode(&[0x11, 0x88], 16).is_err());
        assert_eq!(decode(&[0x22], 16).unwrap().bytes, Vec::<u8>::new());
        assert!(encode(&[0x80]).is_err());

        let n = 80 * 1024;
        let large: Vec<u8> = (0..n).map(|i| ((i & 0x0f) << 4) as u8).collect();
        let enc = encode(&large).unwrap();
        let dec = decode(&enc, n).unwrap();
        assert_eq!(dec.bytes, large);
        assert!(matches!(
            decode(&enc, n - 1),
            Err(Error::ResourceLimit { .. })
        ));
    }
}
