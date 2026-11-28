//! Plane-strided RLE stream with per-plane `0x00` terminators.
//!
//! For each plane `p` in `0..planes` (default `planes = 4` for 4bpp planar tiles,
//! or `planes = 2` for 2bpp planar tiles), output writes advance by `stride = planes`
//! starting at byte offset `p`:
//! - `ctrl == 0x00`: end of current plane.
//! - `ctrl & 0x80 != 0`: literal run of `(ctrl - 0x80) + 1` bytes (`1..=128`).
//! - `ctrl & 0x80 == 0`: RLE repeat of `ctrl + 1` copies (`2..=128`) of the next byte.
use crate::{grow, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

/// Decode a self-terminated plane-strided RLE stream.
pub fn decode(src: &[u8], planes: u8, max_output: usize) -> Result<DecodeOutcome> {
    if !(1..=8).contains(&planes) {
        return Err(Error::InvalidField {
            what: "strided-rle planes must be 1..=8",
            value: u64::from(planes),
        });
    }
    let stride = usize::from(planes);
    let limit = max_output.min(MAX_DECODED);
    let mut r = Reader::new(src);
    let mut out = Vec::new();

    for plane in 0..stride {
        let mut bx = plane;
        loop {
            let ctrl = r.u8()?;
            if ctrl == 0 {
                break;
            }
            let is_literal = (ctrl & 0x80) != 0;
            let count = if is_literal {
                usize::from(ctrl - 0x80) + 1
            } else {
                usize::from(ctrl) + 1
            };
            let span = (count - 1)
                .checked_mul(stride)
                .and_then(|d| bx.checked_add(d))
                .and_then(|last| last.checked_add(1))
                .ok_or(Error::ResourceLimit {
                    resource: "decoded bytes",
                    requested: u64::MAX,
                    limit: limit as u64,
                })?;
            if span > out.len() {
                grow(&out, span - out.len(), limit)?;
                out.resize(span, 0);
            }
            if is_literal {
                for _ in 0..count {
                    out[bx] = r.u8()?;
                    bx += stride;
                }
            } else {
                let val = r.u8()?;
                for _ in 0..count {
                    out[bx] = val;
                    bx += stride;
                }
            }
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

/// Deterministic encoder for plane-strided RLE (`src.len()` should be a multiple of `planes`).
pub fn encode(src: &[u8], planes: u8) -> Result<Vec<u8>> {
    if !(1..=8).contains(&planes) {
        return Err(Error::InvalidField {
            what: "strided-rle planes must be 1..=8",
            value: u64::from(planes),
        });
    }
    grow(&[], src.len(), MAX_DECODED)?;
    let stride = usize::from(planes);
    let mut out = Vec::new();

    for plane in 0..stride {
        let lane: Vec<u8> = src.iter().skip(plane).step_by(stride).copied().collect();
        let mut i = 0usize;
        while i < lane.len() {
            let mut run = 1usize;
            while i + run < lane.len() && run < 128 && lane[i + run] == lane[i] {
                run += 1;
            }
            if run >= 2 {
                out.push((run - 1) as u8);
                out.push(lane[i]);
                i += run;
            } else {
                let start = i;
                i += 1;
                while i < lane.len() && (i - start) < 128 {
                    if i + 1 < lane.len() && lane[i] == lane[i + 1] {
                        break;
                    }
                    i += 1;
                }
                let lit_len = i - start;
                out.push(0x80 | ((lit_len - 1) as u8));
                out.extend_from_slice(&lane[start..i]);
            }
        }
        out.push(0x00);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_2plane_and_4plane_streams() {
        let mut tile4 = vec![0u8; 64];
        for (i, b) in tile4.iter_mut().enumerate() {
            *b = match i % 4 {
                0 => 0xaa,
                1 => (i / 4) as u8,
                2 => if i < 32 { 0x11 } else { 0x22 },
                _ => ((i * 7) & 0xff) as u8,
            };
        }
        let enc4 = encode(&tile4, 4).unwrap();
        let mut padded = enc4.clone();
        padded.extend_from_slice(&[0xde, 0xad]);
        let dec4 = decode(&padded, 4, 256).unwrap();
        assert_eq!(dec4.bytes, tile4);
        assert_eq!(dec4.consumed, enc4.len());

        let tile2: Vec<u8> = (0..32).map(|i| if i % 2 == 0 { 0x55 } else { i as u8 }).collect();
        let enc2 = encode(&tile2, 2).unwrap();
        let dec2 = decode(&enc2, 2, 256).unwrap();
        assert_eq!(dec2.bytes, tile2);
        assert_eq!(dec2.consumed, enc2.len());
    }

    #[test]
    fn rejects_invalid_planes_truncation_and_limit() {
        assert!(decode(&[0], 0, 16).is_err());
        assert!(decode(&[0], 9, 16).is_err());
        assert!(decode(&[0, 0, 0], 4, 16).is_err());
        let enc = encode(&[0x42; 32], 4).unwrap();
        assert!(decode(&enc, 4, 16).is_err());
    }
}
