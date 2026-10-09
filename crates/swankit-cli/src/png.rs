//! PNG at the CLI edge: RGBA8, filter 0, zlib with stored DEFLATE blocks.
//! Specification: https://www.w3.org/TR/png-3/ (sections 5, 7, 10 and 11).
use crate::Result;
use formatkit_core::Image;

fn crc(bytes: impl Iterator<Item = u8>) -> u32 {
    let mut c = u32::MAX;
    for b in bytes {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = (c >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(c & 1)));
        }
    }
    !c
}
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], bytes: &[u8]) -> Result<()> {
    let length = u32::try_from(bytes.len())?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(bytes);
    out.extend_from_slice(&crc(kind.iter().chain(bytes).copied()).to_be_bytes());
    Ok(())
}
pub fn encode(image: &Image) -> Result<Vec<u8>> {
    let required = Image::rgba_len(image.width, image.height)?;
    if image.width == 0
        || image.height == 0
        || image.rgba.len() != required
        || required > swankit::extract::MAX_IMAGE_BYTES
    {
        return Err("invalid or oversized PNG image".into());
    }
    let stride = image.width as usize * 4;
    let mut raw = Vec::with_capacity(required + image.height as usize);
    for row in image.rgba.chunks_exact(stride) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks = raw.chunks(u16::MAX as usize);
    let count = blocks.len();
    for (i, block) in blocks.enumerate() {
        zlib.push(u8::from(i + 1 == count));
        let n = u16::try_from(block.len())?;
        zlib.extend_from_slice(&n.to_le_bytes());
        zlib.extend_from_slice(&(!n).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&image.width.to_be_bytes());
    header.extend_from_slice(&image.height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header)?;
    chunk(&mut out, b"IDAT", &zlib)?;
    chunk(&mut out, b"IEND", &[])?;
    Ok(out)
}
