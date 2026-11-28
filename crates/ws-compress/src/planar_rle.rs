//! Bitplane-split tile RLE stream with `0xFF 0xFF` per-plane sentinels.
//!
//! Layout:
//! - 6-byte header: `u16_le flags`, `u8 width_tiles` (`1..=64`),
//!   `u8 height_tiles` (`1..=64`), `u16_le word_count`.
//!   - `word_count == width_tiles * height_tiles * 8` selects 2bpp (2 planes,
//!     `plane_len = word_count`; pass 0 fills odd bytes, pass 1 fills even bytes).
//!   - `word_count == width_tiles * height_tiles * 16` selects 4bpp (4 planes,
//!     `plane_len = word_count / 2`; pass `p` fills `p::4`).
//! - Each plane is an RLE stream terminated by `0xFF 0xFF`:
//!   - `0x00 <count> <lit[count]>`: emit `count` literal bytes.
//!   - `<count> <value>` (`count != 0`, not `0xFF 0xFF`): repeat `value` `count` times.
use crate::{grow, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result, Writer};

/// Decode a self-verifying bitplane-split RLE tile block.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let mut r = Reader::new(src);
    let _flags = r.u16_le()?;
    let width = usize::from(r.u8()?);
    let height = usize::from(r.u8()?);
    let word_count = usize::from(r.u16_le()?);

    if !(1..=64).contains(&width) {
        return Err(Error::InvalidField {
            what: "planar-rle width_tiles",
            value: width as u64,
        });
    }
    if !(1..=64).contains(&height) {
        return Err(Error::InvalidField {
            what: "planar-rle height_tiles",
            value: height as u64,
        });
    }
    let tiles = width * height;
    let (n_planes, plane_len) = if word_count == tiles * 8 {
        (2usize, word_count)
    } else if word_count == tiles * 16 {
        (4usize, word_count / 2)
    } else {
        return Err(Error::InvalidField {
            what: "planar-rle word_count",
            value: word_count as u64,
        });
    };

    let total_out = word_count * 2;
    grow(&[], total_out, limit)?;

    let mut planes = Vec::with_capacity(n_planes);
    for _ in 0..n_planes {
        let mut plane = Vec::with_capacity(plane_len);
        loop {
            let b0 = r.u8()?;
            let b1 = r.u8()?;
            if b0 == 0xff && b1 == 0xff {
                break;
            }
            if b0 == 0x00 {
                let cnt = usize::from(b1);
                if plane.len() + cnt > plane_len {
                    return Err(Error::Malformed("planar-rle plane length overflow".into()));
                }
                plane.extend_from_slice(r.bytes(cnt)?);
            } else {
                let cnt = usize::from(b0);
                if plane.len() + cnt > plane_len {
                    return Err(Error::Malformed("planar-rle plane length overflow".into()));
                }
                plane.resize(plane.len() + cnt, b1);
            }
        }
        if plane.len() != plane_len {
            return Err(Error::Malformed(format!(
                "planar-rle plane produced {} bytes, expected {plane_len}",
                plane.len()
            )));
        }
        planes.push(plane);
    }

    let mut out = vec![0u8; total_out];
    if n_planes == 2 {
        for i in 0..plane_len {
            out[i * 2 + 1] = planes[0][i];
            out[i * 2] = planes[1][i];
        }
    } else {
        for p in 0..4 {
            for i in 0..plane_len {
                out[i * 4 + p] = planes[p][i];
            }
        }
    }

    Ok(DecodeOutcome {
        bytes: out,
        consumed: r.pos(),
    })
}

fn encode_plane(plane: &[u8], w: &mut Writer) {
    let n = plane.len();
    let mut i = 0usize;
    while i < n {
        let b = plane[i];
        let mut run = 1usize;
        let max_run = if b == 0xff { 254 } else { 255 };
        while i + run < n && run < max_run && plane[i + run] == b {
            run += 1;
        }
        if run >= 3 || (run == 2 && b != 0xff) {
            w.u8(run as u8);
            w.u8(b);
            i += run;
        } else {
            let lit_start = i;
            let mut lit_len = 0usize;
            while i < n && lit_len < 255 {
                let cur = plane[i];
                let mut r = 1usize;
                while i + r < n && r < 3 && plane[i + r] == cur {
                    r += 1;
                }
                if r >= 3 {
                    break;
                }
                i += 1;
                lit_len += 1;
            }
            w.u8(0x00);
            w.u8(lit_len as u8);
            w.bytes(&plane[lit_start..lit_start + lit_len]);
        }
    }
    w.u8(0xff);
    w.u8(0xff);
}

/// Deterministic encoder for bitplane-split RLE tile blocks.
pub fn encode(src: &[u8], flags: u16, width_tiles: u8, height_tiles: u8, bpp: u8) -> Result<Vec<u8>> {
    if !(1..=64).contains(&width_tiles) {
        return Err(Error::InvalidField {
            what: "planar-rle width_tiles",
            value: u64::from(width_tiles),
        });
    }
    if !(1..=64).contains(&height_tiles) {
        return Err(Error::InvalidField {
            what: "planar-rle height_tiles",
            value: u64::from(height_tiles),
        });
    }
    let tiles = usize::from(width_tiles) * usize::from(height_tiles);
    let (n_planes, word_count, plane_len) = match bpp {
        2 => (2usize, tiles * 8, tiles * 8),
        4 => (4usize, tiles * 16, tiles * 8),
        other => {
            return Err(Error::InvalidField {
                what: "planar-rle bpp (must be 2 or 4)",
                value: u64::from(other),
            })
        }
    };
    let expected_len = word_count * 2;
    if src.len() != expected_len {
        return Err(Error::InvalidField {
            what: "planar-rle payload byte length",
            value: src.len() as u64,
        });
    }
    grow(&[], expected_len, MAX_DECODED)?;

    let mut w = Writer::new();
    w.u16_le(flags);
    w.u8(width_tiles);
    w.u8(height_tiles);
    w.u16_le(word_count as u16);

    if n_planes == 2 {
        let p0: Vec<u8> = (0..plane_len).map(|i| src[i * 2 + 1]).collect();
        let p1: Vec<u8> = (0..plane_len).map(|i| src[i * 2]).collect();
        encode_plane(&p0, &mut w);
        encode_plane(&p1, &mut w);
    } else {
        for p in 0..4 {
            let plane: Vec<u8> = (0..plane_len).map(|i| src[i * 4 + p]).collect();
            encode_plane(&plane, &mut w);
        }
    }
    Ok(w.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_and_round_trips_2bpp_and_4bpp() {
        let raw_2bpp: Vec<u8> = (0..64)
            .map(|i| if i % 8 < 4 { 0xff } else { (i * 7) as u8 })
            .collect();
        let enc2 = encode(&raw_2bpp, 0x0001, 2, 2, 2).unwrap();
        let dec2 = decode(&enc2, 1024).unwrap();
        assert_eq!(dec2.bytes, raw_2bpp);
        assert_eq!(dec2.consumed, enc2.len());

        let raw_4bpp: Vec<u8> = (0..128)
            .map(|i| if i % 12 < 6 { 0x00 } else { (i * 13) as u8 })
            .collect();
        let enc4 = encode(&raw_4bpp, 0x0002, 2, 2, 4).unwrap();
        let dec4 = decode(&enc4, 1024).unwrap();
        assert_eq!(dec4.bytes, raw_4bpp);
        assert_eq!(dec4.consumed, enc4.len());
    }

    #[test]
    fn rejects_malformed_headers_short_planes_and_overflow() {
        assert!(decode(&[], 64).is_err());
        // width = 0
        assert!(decode(&[0, 0, 0, 1, 8, 0, 0xff, 0xff, 0xff, 0xff], 64).is_err());
        // mismatched word_count
        assert!(decode(&[0, 0, 1, 1, 9, 0, 0xff, 0xff, 0xff, 0xff], 64).is_err());
        // plane too short before 0xff 0xff
        assert!(decode(&[0, 0, 1, 1, 8, 0, 4, 0xaa, 0xff, 0xff, 8, 0xbb, 0xff, 0xff], 64).is_err());
        // output limit too small
        assert!(decode(&[0, 0, 1, 1, 8, 0, 8, 0xaa, 0xff, 0xff, 8, 0xbb, 0xff, 0xff], 8).is_err());
    }
}
