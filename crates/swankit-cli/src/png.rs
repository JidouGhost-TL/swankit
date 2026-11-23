//! PNG at the CLI edge: RGBA8, filter 0, zlib with stored DEFLATE blocks.
//! Specification: https://www.w3.org/TR/png-3/ (sections 5, 7, 10 and 11).
use crate::Result;
use formatkit_core::Image;

pub fn encode(image: &Image) -> Result<Vec<u8>> {
    let required = Image::rgba_len(image.width, image.height)?;
    if image.width == 0
        || image.height == 0
        || image.rgba.len() != required
        || required > swankit::extract::MAX_IMAGE_BYTES
    {
        return Err("invalid or oversized PNG image".into());
    }
    let raw_len = required.saturating_add(image.height as usize);
    let max_png_bytes = raw_len
        .saturating_add(raw_len.div_ceil(u16::MAX as usize).saturating_mul(5))
        .saturating_add(63);
    Ok(formatkit_codec::encode_rgba_png(image, max_png_bytes)?)
}
