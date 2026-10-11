//! Tile formats: <https://ws.nesdev.org/wiki/Display/Tile_Data>.
use crate::{image, invalid};
use formatkit_core::{Error, Image, Reader, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileFormat {
    Bpp2,
    Bpp4Planar,
    Bpp4Packed,
}

impl TileFormat {
    pub const fn id(self) -> formatkit_catalog::FormatId {
        match self {
            Self::Bpp2 => crate::catalog::TILE_2BPP,
            Self::Bpp4Planar => crate::catalog::TILE_4BPP_PLANAR,
            Self::Bpp4Packed => crate::catalog::TILE_4BPP_PACKED,
        }
    }
    pub const fn bytes_per_tile(self) -> usize {
        match self {
            Self::Bpp2 => 16,
            _ => 32,
        }
    }
    pub const fn colors(self) -> usize {
        match self {
            Self::Bpp2 => 4,
            _ => 16,
        }
    }
    pub const fn tile_count(self, bytes: usize) -> usize {
        bytes / self.bytes_per_tile()
    }
}

/// Decode one 8x8 tile. Rows interleave planes; bit 7 is the leftmost pixel.
/// Packed 4bpp stores the left pixel in the high nibble.
pub fn decode_tile(data: &[u8], offset: usize, format: TileFormat) -> Result<[u8; 64]> {
    let mut reader = Reader::new(data);
    reader.skip(offset)?;
    let bytes = reader.bytes(format.bytes_per_tile())?;
    let mut indices = [0; 64];
    let planes = if format == TileFormat::Bpp2 { 2 } else { 4 };
    for y in 0..8 {
        for x in 0..8 {
            indices[y * 8 + x] = if format == TileFormat::Bpp4Packed {
                (bytes[y * 4 + x / 2] >> (4 * (1 - x % 2))) & 15
            } else {
                (0..planes).fold(0, |v, p| {
                    v | (((bytes[y * planes + p] >> (7 - x)) & 1) << p)
                })
            };
        }
    }
    Ok(indices)
}

/// Encode one 8x8 tile of palette indices: the exact inverse of
/// [`decode_tile`] (index bits above the format's depth are dropped).
pub fn encode_tile(indices: &[u8; 64], format: TileFormat) -> Vec<u8> {
    let mut out = vec![0u8; format.bytes_per_tile()];
    let planes = if format == TileFormat::Bpp2 { 2 } else { 4 };
    for y in 0..8 {
        for x in 0..8 {
            let v = indices[y * 8 + x];
            if format == TileFormat::Bpp4Packed {
                out[y * 4 + x / 2] |= (v & 15) << (4 * (1 - x % 2));
            } else {
                for p in 0..planes {
                    out[y * planes + p] |= ((v >> p) & 1) << (7 - x);
                }
            }
        }
    }
    out
}

/// Render an individual tile through an owner-selected RGBA palette.
pub fn render_tile(indices: &[u8; 64], palette: &[[u8; 4]]) -> Result<Image> {
    let mut rgba = Image::try_rgba_buffer(8, 8, 256, "tile RGBA bytes")?;
    formatkit_pixel::apply_palette(indices.iter().map(|&i| i as usize), &mut rgba, |i| {
        palette
            .get(i)
            .copied()
            .ok_or_else(|| invalid("tile palette index", i))
    })?;
    Image::try_new(8, 8, rgba)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    RowMajor,
    ColumnMajor,
}

/// Consecutive tiles grouped into a cell, including 8x16 and 16x16 glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrangement {
    pub tiles_wide: u32,
    pub tiles_high: u32,
    pub order: Order,
}

impl Arrangement {
    pub const PLAIN: Self = Self {
        tiles_wide: 1,
        tiles_high: 1,
        order: Order::RowMajor,
    };
    pub const TALL: Self = Self {
        tiles_wide: 1,
        tiles_high: 2,
        order: Order::RowMajor,
    };
    pub const GLYPH_ROW: Self = Self {
        tiles_wide: 2,
        tiles_high: 2,
        order: Order::RowMajor,
    };
    pub const GLYPH_COLUMN: Self = Self {
        tiles_wide: 2,
        tiles_high: 2,
        order: Order::ColumnMajor,
    };
}

#[derive(Debug, Clone, Copy)]
pub struct AtlasLayout {
    pub arrangement: Arrangement,
    pub columns: u32,
    pub scale: u32,
    pub max_bytes: usize,
}

/// Render complete tiles. An incomplete source tile returns an error;
/// unused cells are opaque black, matching the Java atlas renderer.
pub fn render_atlas(
    data: &[u8],
    format: TileFormat,
    palette: &[[u8; 4]],
    layout: AtlasLayout,
) -> Result<Image> {
    if data.is_empty() || !data.len().is_multiple_of(format.bytes_per_tile()) {
        return Err(invalid("atlas tile bytes", data.len()));
    }
    let a = layout.arrangement;
    if a.tiles_wide == 0 || a.tiles_high == 0 || layout.columns == 0 || layout.scale == 0 {
        return Err(invalid("atlas dimensions", 0));
    }
    let overflow = || Error::ResourceLimit {
        resource: "atlas dimensions",
        requested: u64::MAX,
        limit: layout.max_bytes as u64,
    };
    let tpc = (a.tiles_wide as usize)
        .checked_mul(a.tiles_high as usize)
        .ok_or_else(overflow)?;
    let count = format.tile_count(data.len());
    let rows = count.div_ceil(tpc).div_ceil(layout.columns as usize);
    let cw = a
        .tiles_wide
        .checked_mul(8)
        .and_then(|n| n.checked_mul(layout.scale))
        .ok_or_else(overflow)?;
    let ch = a
        .tiles_high
        .checked_mul(8)
        .and_then(|n| n.checked_mul(layout.scale))
        .ok_or_else(overflow)?;
    let width = layout.columns.checked_mul(cw).ok_or_else(overflow)?;
    let height = u32::try_from(rows)
        .ok()
        .and_then(|n| n.checked_mul(ch))
        .ok_or_else(overflow)?;
    let mut out = image(width, height, layout.max_bytes, [0, 0, 0, 255])?;
    for tile in 0..count {
        let indices = decode_tile(data, tile * format.bytes_per_tile(), format)?;
        let cell = tile / tpc;
        let i = tile % tpc;
        let (cx, cy) = match a.order {
            Order::RowMajor => (i % a.tiles_wide as usize, i / a.tiles_wide as usize),
            Order::ColumnMajor => (i / a.tiles_high as usize, i % a.tiles_high as usize),
        };
        let tx = cell % layout.columns as usize * cw as usize + cx * 8 * layout.scale as usize;
        let ty = cell / layout.columns as usize * ch as usize + cy * 8 * layout.scale as usize;
        let rendered = render_tile(&indices, palette)?;
        for y in 0..8 {
            for x in 0..8 {
                let src = (y * 8 + x) * 4;
                for sy in 0..layout.scale as usize {
                    for sx in 0..layout.scale as usize {
                        let dst = ((ty + y * layout.scale as usize + sy) * width as usize
                            + tx
                            + x * layout.scale as usize
                            + sx)
                            * 4;
                        out.rgba[dst..dst + 4].copy_from_slice(&rendered.rgba[src..src + 4]);
                    }
                }
            }
        }
    }
    Ok(out)
}
