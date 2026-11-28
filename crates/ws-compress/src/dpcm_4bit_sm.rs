//! 4-bit sign-magnitude DPCM audio stream with scale multiplier and saturating carry/borrow revert.
//!
//! Stream header (2 bytes):
//! - Byte `0`: `(0x1 << 4) | rate_code`, where `rate_code` selects playback rate
//!   (`1` = 12000 Hz, `2` = 6000 Hz, `3` = 4000 Hz, `4` = 3000 Hz).
//! - Byte `1`: unsigned 8-bit step `scale` multiplier.
//!
//! Payload (`src[2..]`):
//! - Accumulator initializes to `0x80`, phase counter initializes to `0`, byte cursor `pos = 0`.
//! - Each sample tick increments `phase` modulo 256:
//!   - Even `phase`: read low nibble (`raw & 0x0F`), keep `pos`.
//!   - Odd `phase`: read high nibble (`raw >> 4`), advance `pos += 1`.
//!   *(Byte 0 emits only its high nibble; each subsequent byte emits low nibble then high nibble.)*
//! - `delta = ((nib & 0x07) * scale) & 0xFF`.
//! - If `nib < 8`: add `delta` to accumulator if `acc + delta <= 255`, else leave unchanged.
//! - If `nib >= 8`: subtract `delta` from accumulator if `acc >= delta`, else leave unchanged.
//! - Emit accumulator as unsigned 8-bit PCM.
//!
//! Stream extent is supplied by the enclosing carrier.
use crate::{grow, DecodeOutcome, MAX_STREAM_DECODED};
use formatkit_core::{Error, Result};

/// Default unsigned 8-bit PCM midpoint accumulator.
pub const DEFAULT_INIT: u8 = 0x80;

/// Decode playback sample rate from the 2-byte header, if valid.
pub fn header_sample_rate(src: &[u8]) -> Option<u32> {
    let &b0 = src.first()?;
    if (b0 >> 4) != 1 {
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

/// Decode a carrier-bounded 4-bit sign-magnitude DPCM audio stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    if src.len() < 2 {
        return Err(Error::Truncated {
            offset: 0,
            needed: 2,
            available: src.len(),
        });
    }
    let b0 = src[0];
    if (b0 >> 4) != 1 {
        return Err(Error::InvalidField {
            what: "dpcm-4bit-sm header codec tag must be 0x1",
            value: u64::from(b0 >> 4),
        });
    }
    let scale = src[1];
    let data = &src[2..];
    let limit = max_output.min(MAX_STREAM_DECODED);
    let out_len = if data.is_empty() {
        0
    } else {
        data.len()
            .checked_mul(2)
            .map(|n| n - 1)
            .ok_or(Error::ResourceLimit {
                resource: "decoded bytes",
                requested: u64::MAX,
                limit: limit as u64,
            })?
    };
    grow(&[], out_len, limit)?;

    let mut out = Vec::with_capacity(out_len);
    let mut acc: u8 = DEFAULT_INIT;
    let mut phase: u8 = 0;
    let mut pos: usize = 0;

    while pos < data.len() {
        let raw = data[pos];
        phase = phase.wrapping_add(1);
        let nib = if (!phase) & 1 == 1 {
            raw & 0x0f
        } else {
            pos += 1;
            raw >> 4
        };
        let delta = (nib & 0x07).wrapping_mul(scale);
        if nib < 8 {
            if let Some(next) = acc.checked_add(delta) {
                acc = next;
            }
        } else if let Some(next) = acc.checked_sub(delta) {
            acc = next;
        }
        out.push(acc);
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: src.len(),
    })
}

/// Deterministic encoder for unsigned 8-bit PCM (default 6000 Hz, scale 3).
pub fn encode(pcm: &[u8]) -> Result<Vec<u8>> {
    encode_with_params(pcm, 2, 3)
}

/// Deterministic encoder with explicit rate code (`1..=4`) and step `scale`.
pub fn encode_with_params(pcm: &[u8], rate_code: u8, scale: u8) -> Result<Vec<u8>> {
    grow(&[], pcm.len(), MAX_STREAM_DECODED)?;
    if !(1..=4).contains(&rate_code) {
        return Err(Error::InvalidField {
            what: "dpcm-4bit-sm rate_code must be 1..=4",
            value: u64::from(rate_code),
        });
    }
    let mut out = Vec::with_capacity(2 + pcm.len() / 2 + 1);
    out.push(0x10 | (rate_code & 0x0f));
    out.push(scale);
    if pcm.is_empty() {
        return Ok(out);
    }

    let mut acc: u8 = DEFAULT_INIT;
    let mut pick_nibble = |target: u8| -> u8 {
        let mut best_nib = 0u8;
        let mut best_acc = acc;
        let mut best_err = u16::MAX;
        for nib in 0u8..16 {
            let delta = (nib & 0x07).wrapping_mul(scale);
            let cand = if nib < 8 {
                acc.checked_add(delta).unwrap_or(acc)
            } else {
                acc.checked_sub(delta).unwrap_or(acc)
            };
            let err = cand.abs_diff(target) as u16;
            if err < best_err {
                best_err = err;
                best_nib = nib;
                best_acc = cand;
            }
        }
        acc = best_acc;
        best_nib
    };

    // Sample 0 occupies the high nibble of payload byte 0.
    let first = pick_nibble(pcm[0]);
    out.push(first << 4);

    // Subsequent samples pack low nibble then high nibble per byte.
    let rest = &pcm[1..];
    let mut i = 0;
    while i < rest.len() {
        let lo = pick_nibble(rest[i]);
        let hi = if i + 1 < rest.len() {
            pick_nibble(rest[i + 1])
        } else {
            0
        };
        out.push((hi << 4) | (lo & 0x0f));
        i += 2;
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_phase_order_and_carry_borrow_revert() {
        // Header: codec 0x1, rate 2 (6 kHz), scale 3
        // Byte 0: high nibble 0x2 (+6 => 0x86); low nibble ignored
        // Byte 1: low nibble 0x9 (-3 => 0x83), high nibble 0x7 (+21 => 0x98)
        let stream = [0x12, 0x03, 0x2f, 0x79];
        assert_eq!(header_sample_rate(&stream), Some(6_000));
        let got = decode(&stream, 16).unwrap();
        assert_eq!(got.bytes, vec![0x86, 0x83, 0x98]);
        assert_eq!(got.consumed, stream.len());

        // Carry/borrow revert: scale = 100
        // nib 0x2 -> +200 from 0x80 (328 > 255 -> revert to 0x80)
        // nib 0xa -> -200 from 0x80 (128 < 200 -> revert to 0x80)
        // nib 0x1 -> +100 from 0x80 (228)
        let revert = [0x11, 100, 0x20, 0x1a];
        assert_eq!(header_sample_rate(&revert), Some(12_000));
        let got = decode(&revert, 16).unwrap();
        assert_eq!(got.bytes, vec![0x80, 0x80, 228]);
    }

    #[test]
    fn validates_header_and_supports_large_streams() {
        assert!(decode(&[], 16).is_err());
        assert!(decode(&[0x12], 16).is_err());
        assert!(decode(&[0x22, 0x03, 0x00], 16).is_err());
        assert_eq!(decode(&[0x12, 0x03], 16).unwrap().bytes, Vec::<u8>::new());

        // Round-trip odd sample count with step multiples of scale=2
        let pcm: Vec<u8> = vec![128, 130, 134, 132, 126, 128, 130];
        let enc = encode_with_params(&pcm, 2, 2).unwrap();
        let dec = decode(&enc, pcm.len()).unwrap();
        assert_eq!(dec.bytes, pcm);

        // > 64 KiB stream
        let n = 80 * 1024 + 1;
        let large: Vec<u8> = (0..n).map(|i| 128u8.wrapping_add(((i % 5) * 2) as u8)).collect();
        let enc_large = encode_with_params(&large, 1, 2).unwrap();
        let dec_large = decode(&enc_large, n).unwrap();
        assert_eq!(dec_large.bytes.len(), n);
        assert!(matches!(
            decode(&enc_large, n - 1),
            Err(Error::ResourceLimit { .. })
        ));
    }
}
