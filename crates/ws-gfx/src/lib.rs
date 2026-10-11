//! WonderSwan graphics grammars and static display rendering.
//! Hardware rules: <https://ws.nesdev.org/wiki/Display> and its subpages.
#![forbid(unsafe_code)]

pub mod catalog;
mod palette;
mod render;
mod tile;

pub use palette::{transparent, ColorPalettes, MonoPalettes};
pub use render::{MapEntry, RenderProfile, Snapshot, Sprite, HEIGHT, WIDTH};
pub use tile::{
    decode_tile, encode_tile, render_atlas, render_tile, Arrangement, AtlasLayout, Order,
    TileFormat,
};

use formatkit_core::{Error, Image, Result};

fn invalid(what: &'static str, value: usize) -> Error {
    Error::InvalidField {
        what,
        value: value as u64,
    }
}

fn image(width: u32, height: u32, max_bytes: usize, fill: [u8; 4]) -> Result<Image> {
    let mut rgba = Image::try_rgba_buffer(width, height, max_bytes, "graphics RGBA bytes")?;
    for _ in 0..Image::rgba_len(width, height)? / 4 {
        rgba.extend_from_slice(&fill);
    }
    Image::try_new(width, height, rgba)
}
