//! 4-bit nibble-delta DPCM audio stream with hold RLE and 8-bit delta escape.
//!
//! Bytes are unpacked high nibble first (`(b >> 4) & 0x0f`, then `b & 0x0f`).
//! Deltas accumulate modulo 256 onto an unsigned 8-bit PCM sample initialized
//! to `0x20` (or caller-supplied `init`):
//! - `0x0`: hold RLE — read next nibble `rep`, emit `rep + 1` copies of the
//!   current accumulator (`1..=16` samples). Stops cleanly if no next nibble.
//! - `0x8`: 8-bit delta — read next two nibbles `(hi, lo)`, add `(hi << 4) | lo`
//!   modulo 256, and emit the new sample. Stops cleanly if fewer than 2 nibbles remain.
//! - `1..=7`: add `+1..=+7` modulo 256 and emit the new sample.
//! - `9..=15`: add `-7..=-1` (4-bit two's-complement) modulo 256 and emit the new sample.
//!
//! Stream extent is supplied by the enclosing carrier.
use crate::{grow, DecodeOutcome, MAX_STREAM_DECODED};
use formatkit_core::Result;

/// Default initial unsigned 8-bit PCM accumulator.
pub const DEFAULT_INIT: u8 = 0x20;

#[inline]
fn nibble_at(src: &[u8], idx: usize) -> u8 {
    let b = src[idx >> 1];
    if idx & 1 == 0 {
        (b >> 4) & 0x0f
    } else {
        b & 0x0f
    }
}

/// Decode a carrier-bounded 4-bit nibble-delta DPCM stream starting from `0x20`.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    decode_with_init(src, DEFAULT_INIT, max_output)
}

/// Decode a carrier-bounded 4-bit nibble-delta DPCM stream with explicit initial accumulator.
pub fn decode_with_init(src: &[u8], init: u8, max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_STREAM_DECODED);
    let total_nibbles = src.len().saturating_mul(2);
    let mut out = Vec::with_capacity(src.len().min(limit));
    let mut acc = init;
    let mut pos = 0usize;

    while pos < total_nibbles {
        let nib = nibble_at(src, pos);
        pos += 1;
        match nib {
            0x0 => {
                if pos >= total_nibbles {
                    break;
                }
                let rep = usize::from(nibble_at(src, pos)) + 1;
                pos += 1;
                grow(&out, rep, limit)?;
                out.resize(out.len() + rep, acc);
            }
            0x8 => {
                if pos + 2 > total_nibbles {
                    break;
                }
                let hi = nibble_at(src, pos);
                let lo = nibble_at(src, pos + 1);
                pos += 2;
                let delta = (hi << 4) | lo;
                acc = acc.wrapping_add(delta);
                grow(&out, 1, limit)?;
                out.push(acc);
            }
            1..=7 => {
                acc = acc.wrapping_add(nib);
                grow(&out, 1, limit)?;
                out.push(acc);
            }
            _ => {
                // 9..=15 => -7..=-1 in two's complement
                acc = acc.wrapping_add(nib | 0xf0);
                grow(&out, 1, limit)?;
                out.push(acc);
            }
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: src.len(),
    })
}

/// Deterministic lossless encoder for unsigned 8-bit PCM (initial `0x20`).
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    encode_with_init(src, DEFAULT_INIT)
}

/// Deterministic lossless encoder with explicit initial accumulator.
pub fn encode_with_init(src: &[u8], init: u8) -> Result<Vec<u8>> {
    grow(&[], src.len(), MAX_STREAM_DECODED)?;
    let mut nibbles = Vec::with_capacity(src.len());
    let mut acc = init;
    let mut i = 0usize;

    while i < src.len() {
        let target = src[i];
        if target == acc {
            let mut run = 1usize;
            while i + run < src.len() && run < 16 && src[i + run] == acc {
                run += 1;
            }
            nibbles.push(0x0);
            nibbles.push((run - 1) as u8);
            i += run;
            continue;
        }
        let diff = target.wrapping_sub(acc);
        if (1..=7).contains(&diff) {
            nibbles.push(diff);
        } else if diff >= 0xf9 {
            nibbles.push(diff & 0x0f);
        } else {
            nibbles.push(0x8);
            nibbles.push((diff >> 4) & 0x0f);
            nibbles.push(diff & 0x0f);
        }
        acc = target;
        i += 1;
    }

    // Pad odd nibble count with trailing 0x0 (incomplete hold RLE stops cleanly at EOF).
    if nibbles.len() & 1 != 0 {
        nibbles.push(0x0);
    }

    let mut out = Vec::with_capacity(nibbles.len() / 2);
    for pair in nibbles.chunks_exact(2) {
        out.push((pair[0] << 4) | (pair[1] & 0x0f));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use formatkit_core::Error;

    #[test]
    fn decodes_signed_deltas_hold_rle_and_escape() {
        // init = 0x20
        // nibs: 1 (+1 => 0x21), f (-1 => 0x20), 0 2 (hold x3 => 0x20, 0x20, 0x20),
        //       8 4 0 (+0x40 => 0x60), 0 (trailing EOF pad)
        let stream = [0x1f, 0x02, 0x84, 0x00];
        let got = decode(&stream, 32).unwrap();
        assert_eq!(got.bytes, vec![0x21, 0x20, 0x20, 0x20, 0x20, 0x60]);
        assert_eq!(got.consumed, stream.len());

        assert!(decode(&[], 0).unwrap().bytes.is_empty());
    }

    #[test]
    fn round_trips_arbitrary_pcm_and_large_streams() {
        let mut samples = Vec::new();
        samples.extend_from_slice(&[0x20; 19]);
        for i in 0..256u16 {
            samples.push((i.wrapping_mul(37)) as u8);
        }
        samples.extend_from_slice(&[0x80, 0x81, 0x87, 0x80, 0x79, 0x79, 0xff, 0x00]);
        let encoded = encode(&samples).unwrap();
        let decoded = decode(&encoded, samples.len()).unwrap();
        assert_eq!(decoded.bytes, samples);
        assert_eq!(decoded.consumed, encoded.len());

        // > 64 KiB stream
        let n = 80 * 1024;
        let large: Vec<u8> = (0..n).map(|i| ((i / 3) & 0xff) as u8).collect();
        let enc_large = encode(&large).unwrap();
        let dec_large = decode(&enc_large, n).unwrap();
        assert_eq!(dec_large.bytes, large);
        assert!(matches!(
            decode(&enc_large, n - 1),
            Err(Error::ResourceLimit { .. })
        ));
    }
}
