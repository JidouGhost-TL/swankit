//! PNG reading at the CLI edge (PNG 3rd edition, sections 7-11): non-interlaced
//! greyscale, truecolour, indexed (with tRNS) and alpha variants, depths 1-8.
use crate::Result;
use formatkit_core::Image;

const MAX_PIXELS: u64 = 16 * 1024 * 1024;

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

pub fn decode(bytes: &[u8]) -> Result<Image> {
    if bytes.len() < 8 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("not a PNG file".into());
    }
    let mut at = 8;
    let (mut header, mut plte, mut trns, mut idat) = (None, Vec::new(), Vec::new(), Vec::new());
    while at + 12 <= bytes.len() {
        let len = be32(&bytes[at..]) as usize;
        let kind = &bytes[at + 4..at + 8];
        let body = bytes
            .get(at + 8..at + 8 + len)
            .ok_or("truncated PNG chunk")?;
        match kind {
            b"IHDR" if len == 13 => header = Some(body.to_vec()),
            b"PLTE" => plte = body.to_vec(),
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    let h = header.ok_or("PNG without IHDR")?;
    let (width, height, depth, color, interlace) =
        (be32(&h[0..]), be32(&h[4..]), h[8], h[9], h[12]);
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("PNG dimensions out of range".into());
    }
    if interlace != 0 {
        return Err("interlaced PNG is not supported".into());
    }
    let channels = match color {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return Err("invalid PNG colour type".into()),
    };
    let ok_depth = match color {
        0 => matches!(depth, 1 | 2 | 4 | 8),
        3 => matches!(depth, 1 | 2 | 4 | 8),
        _ => depth == 8,
    };
    if !ok_depth {
        return Err(
            format!("PNG bit depth {depth} for colour type {color} is not supported").into(),
        );
    }
    let bits = usize::from(depth) * channels;
    let stride = (width as usize * bits).div_ceil(8);
    let bpp = bits.div_ceil(8).max(1);
    let expected = (stride + 1) * height as usize;
    let raw = formatkit_codec::zlib_inflate_with_limit(&idat, expected)
        .ok_or("invalid PNG image data")?;
    if raw.len() != expected {
        return Err("PNG image data has the wrong size".into());
    }
    let mut prev = vec![0u8; stride];
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for row in raw.chunks_exact(stride + 1) {
        let (filter, src) = (row[0], &row[1..]);
        let mut cur = vec![0u8; stride];
        for i in 0..stride {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let (b, c) = (prev[i], if i >= bpp { prev[i - bpp] } else { 0 });
            cur[i] = src[i].wrapping_add(match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => paeth(a, b, c),
                _ => return Err("invalid PNG filter".into()),
            });
        }
        for x in 0..width as usize {
            let sample = |k: usize| -> u8 {
                if depth == 8 {
                    cur[x * channels + k]
                } else {
                    let bit = x * usize::from(depth);
                    (cur[bit / 8] >> (8 - usize::from(depth) - bit % 8)) & ((1 << depth) - 1)
                }
            };
            let scale = |v: u8| {
                if depth == 8 {
                    v
                } else {
                    (u16::from(v) * 255 / ((1 << depth) - 1)) as u8
                }
            };
            let px = match color {
                0 => {
                    let g = scale(sample(0));
                    [g, g, g, 255]
                }
                2 => [sample(0), sample(1), sample(2), 255],
                3 => {
                    let i = usize::from(sample(0));
                    let p = plte
                        .get(i * 3..i * 3 + 3)
                        .ok_or("PNG palette index out of range")?;
                    [p[0], p[1], p[2], trns.get(i).copied().unwrap_or(255)]
                }
                4 => [sample(0), sample(0), sample(0), sample(1)],
                _ => [sample(0), sample(1), sample(2), sample(3)],
            };
            rgba.extend_from_slice(&px);
        }
        prev = cur;
    }
    Ok(Image::try_new(width, height, rgba)?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trips_the_cli_encoder() {
        let rgba: Vec<u8> = (0..3 * 2 * 4).map(|i| (i * 7) as u8).collect();
        let image = formatkit_core::Image::try_new(3, 2, rgba).unwrap();
        let png = crate::png::encode(&image).unwrap();
        assert_eq!(super::decode(&png).unwrap(), image);
    }
}
