//! Tile glyphs use the display layout documented by WSdev Tile Data.
use crate::{limit, malformed};
use formatkit_core::{bytes_at, Image, Result};
use serde::{Deserialize, Serialize};
use ws_gfx::{render_atlas, render_mono_atlas, Arrangement, AtlasLayout, Order, TileFormat};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FontFormat {
    Bpp1,
    Bpp2,
    Bpp4Planar,
    Bpp4Packed,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GlyphOrder {
    #[default]
    Row,
    Column,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FontDefinition {
    pub offset: usize,
    pub count: usize,
    pub stride: usize,
    pub format: FontFormat,
    pub tiles_wide: u32,
    pub tiles_high: u32,
    #[serde(default)]
    pub order: GlyphOrder,
    /// Explicit tile byte offsets within a glyph (e.g. a padded half-width font).
    #[serde(default)]
    pub tile_offsets: Vec<usize>,
    /// Per-cell advances; does not crop or alter the source bitmap.
    #[serde(default)]
    pub widths: Vec<u32>,
    /// Optional one-byte-per-glyph resident advance table.
    #[serde(default)]
    pub width_table: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphImage {
    pub image: Image,
    pub advance: u32,
}

impl FontDefinition {
    pub fn glyph(
        &self,
        bytes: &[u8],
        index: usize,
        palette: &[[u8; 4]],
        maximum_bytes: usize,
    ) -> Result<GlyphImage> {
        if index >= self.count || self.tiles_wide == 0 || self.tiles_high == 0 || self.stride == 0 {
            return Err(malformed("invalid font dimensions, stride or glyph index"));
        }
        let width = self
            .tiles_wide
            .checked_mul(8)
            .ok_or_else(|| malformed("font width overflow"))?;
        let height = self
            .tiles_high
            .checked_mul(8)
            .ok_or_else(|| malformed("font height overflow"))?;
        let needed = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| malformed("font image size overflow"))?;
        limit("font RGBA bytes", needed, maximum_bytes)?;
        let offset = index
            .checked_mul(self.stride)
            .and_then(|n| n.checked_add(self.offset))
            .ok_or_else(|| malformed("font source offset overflow"))?;
        let source = bytes_at(bytes, offset, self.stride)?;
        let (tile_bytes, format) = match self.format {
            FontFormat::Bpp1 => (8, None),
            FontFormat::Bpp2 => (16, Some(TileFormat::Bpp2)),
            FontFormat::Bpp4Planar => (32, Some(TileFormat::Bpp4Planar)),
            FontFormat::Bpp4Packed => (32, Some(TileFormat::Bpp4Packed)),
        };
        let tiles = (self.tiles_wide as usize)
            .checked_mul(self.tiles_high as usize)
            .ok_or_else(|| malformed("font tile count overflow"))?;
        if !self.tile_offsets.is_empty() && self.tile_offsets.len() != tiles {
            return Err(malformed("font tile offset count differs from dimensions"));
        }
        if !self.widths.is_empty() && self.widths.len() != self.count {
            return Err(malformed("font advance count differs from glyph count"));
        }
        let mut data = Vec::new();
        for tile in 0..tiles {
            let at = self.tile_offsets.get(tile).copied().map_or_else(
                || {
                    tile.checked_mul(tile_bytes)
                        .ok_or_else(|| malformed("font tile offset overflow"))
                },
                Ok,
            )?;
            data.extend_from_slice(bytes_at(source, at, tile_bytes)?);
        }
        let layout = AtlasLayout {
            arrangement: Arrangement {
                tiles_wide: self.tiles_wide,
                tiles_high: self.tiles_high,
                order: match self.order {
                    GlyphOrder::Row => Order::RowMajor,
                    GlyphOrder::Column => Order::ColumnMajor,
                },
            },
            columns: 1,
            scale: 1,
            max_bytes: maximum_bytes,
        };
        let image = match format {
            Some(format) => render_atlas(&data, format, palette, layout),
            None => render_mono_atlas(&data, palette, layout),
        }?;
        let advance = if let Some(table) = self.width_table {
            let at = table
                .checked_add(index)
                .ok_or_else(|| malformed("font width table overflow"))?;
            u32::from(formatkit_layout::U8Field::new(at).read(bytes)?)
        } else {
            self.widths.get(index).copied().unwrap_or(width)
        };
        if advance > width {
            return Err(malformed("font advance exceeds bitmap width"));
        }
        Ok(GlyphImage { image, advance })
    }
}
