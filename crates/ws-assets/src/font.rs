//! Inverse of the reviewed font sheet layout, retaining glyph padding.
use crate::data::decode_asset;
use crate::extract::{font_definition, font_palette, font_sheet};
use crate::gfx::nearest;
use crate::{malformed, AssetMap, Charmaps};
use formatkit_core::{bytes_at, Image, Result};
use std::collections::BTreeMap;
use ws_gfx::{decode_tile, encode_tile, TileFormat};
use ws_text::{FontFormat, GlyphOrder};

pub(crate) fn unrender_font(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    id: &str,
    image: &Image,
) -> Result<(BTreeMap<String, Vec<u8>>, usize)> {
    let asset = map.asset(id)?;
    let definition = font_definition(asset)?;
    let original_sheet = font_sheet(map, charmaps, rom, asset)?;
    if (image.width, image.height) != (original_sheet.width, original_sheet.height) {
        return Err(malformed(
            "font image dimensions differ from extracted sheet",
        ));
    }
    let palette = font_palette(map, charmaps, rom, asset)?;
    let mut data = decode_asset(map, charmaps, rom, id)?.decoded;
    let (tile_bytes, format, colors) = match definition.format {
        FontFormat::Bpp1 => (8, TileFormat::Bpp2, 2),
        FontFormat::Bpp2 => (16, TileFormat::Bpp2, 4),
        FontFormat::Bpp4Planar => (32, TileFormat::Bpp4Planar, 16),
        FontFormat::Bpp4Packed => (32, TileFormat::Bpp4Packed, 16),
    };
    let wide = definition.tiles_wide as usize;
    let high = definition.tiles_high as usize;
    let columns = image.width as usize / (wide * 8);
    let mut changed = BTreeMap::new();
    let mut approximated = 0;
    for glyph in 0..definition.count {
        for tile in 0..wide * high {
            let offset = glyph * definition.stride
                + definition
                    .tile_offsets
                    .get(tile)
                    .copied()
                    .unwrap_or(tile * tile_bytes);
            let raw = bytes_at(&data, offset, tile_bytes)?;
            let expanded = if matches!(definition.format, FontFormat::Bpp1) {
                raw.iter().flat_map(|b| [*b, 0]).collect::<Vec<_>>()
            } else {
                raw.to_vec()
            };
            let original = decode_tile(&expanded, 0, format)?;
            let (tx, ty) = match definition.order {
                GlyphOrder::Row => (tile % wide, tile / wide),
                GlyphOrder::Column => (tile / high, tile % high),
            };
            let x0 = (glyph % columns * wide + tx) * 8;
            let y0 = (glyph / columns * high + ty) * 8;
            let mut indices = [0; 64];
            for y in 0..8 {
                for x in 0..8 {
                    let pixel = ((y0 + y) * image.width as usize + x0 + x) * 4;
                    let rgba = image.rgba[pixel..pixel + 4].try_into().unwrap();
                    let (value, exact) = nearest(&palette, colors, rgba, original[y * 8 + x]);
                    indices[y * 8 + x] = value;
                    approximated += usize::from(!exact);
                }
            }
            let encoded = encode_tile(&indices, format);
            let stored = if matches!(definition.format, FontFormat::Bpp1) {
                encoded.iter().step_by(2).copied().collect::<Vec<_>>()
            } else {
                encoded
            };
            for (i, b) in stored.into_iter().enumerate() {
                if changed
                    .insert(offset + i, b)
                    .is_some_and(|prior| prior != b)
                {
                    return Err(malformed("font edits conflict at an aliased source byte"));
                }
            }
        }
    }
    for (at, b) in changed {
        data[at] = b;
    }
    Ok((BTreeMap::from([(id.into(), data)]), approximated))
}
