//! PNG reading at the CLI edge (PNG 3rd edition, sections 7-11): non-interlaced
//! greyscale, truecolour, indexed (with tRNS) and alpha variants, depths 1-8.
use crate::Result;
use formatkit_core::Image;

const MAX_PIXELS: u64 = 16 * 1024 * 1024;

pub fn decode(bytes: &[u8]) -> Result<Image> {
    Ok(formatkit_codec::decode_png_rgba(bytes, MAX_PIXELS)?)
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
