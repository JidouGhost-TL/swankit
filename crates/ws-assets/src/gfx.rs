//! Rendering and its inverse for located graphics. Tile, map-word and palette
//! grammars come from `ws-gfx` (WSdev Display/Tile_Data, Display/Palette,
//! Display/Screen). The tile encoder here is the exact inverse of
//! `ws_gfx::decode_tile` and is checked against it.
use crate::data::decode_asset;
use crate::malformed;
use crate::map::{parse_rgb, Asset, AssetMap, Kind};
use crate::text::Charmaps;
use formatkit_core::{Image, Result};
use std::collections::BTreeMap;
use ws_gfx::{decode_tile, transparent, ColorPalettes, MapEntry, TileFormat, HEIGHT, WIDTH};

/// Largest rendered image.
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// Hardware palettes.
const PALETTES: usize = 16;
/// Tile slots: a colour-mode map word addresses 1024; engine maps with
/// 12-bit indices address up to 4096 tiles of their own blob.
const TILE_SLOTS: usize = 4096;

pub type Rgba = [u8; 4];
pub type PaletteSet = Vec<Option<[Rgba; 16]>>;

pub(crate) fn tile_format(asset: &Asset) -> Result<TileFormat> {
    let f = &asset.format;
    match (f.bpp.unwrap_or(2), f.layout.as_deref().unwrap_or("planar")) {
        (2, "planar") => Ok(TileFormat::Bpp2),
        (4, "planar") => Ok(TileFormat::Bpp4Planar),
        (4, "packed") => Ok(TileFormat::Bpp4Packed),
        (bpp, layout) => Err(malformed(format!(
            "{}: unsupported tile format {bpp}bpp {layout}",
            asset.id
        ))),
    }
}

/// Inverse of `ws_gfx::decode_tile`.
pub fn encode_tile(indices: &[u8; 64], format: TileFormat) -> Vec<u8> {
    let mut out = vec![0u8; format.bytes_per_tile()];
    for y in 0..8 {
        for x in 0..8 {
            let v = indices[y * 8 + x];
            match format {
                TileFormat::Bpp4Packed => out[y * 4 + x / 2] |= (v & 15) << (4 * (1 - x % 2)),
                TileFormat::Bpp2 | TileFormat::Bpp4Planar => {
                    let planes = if format == TileFormat::Bpp2 { 2 } else { 4 };
                    for p in 0..planes {
                        out[y * planes + p] |= ((v >> p) & 1) << (7 - x);
                    }
                }
            }
        }
    }
    out
}

fn grayscale(colors: usize) -> [Rgba; 16] {
    let mut out = [[0, 0, 0, 255]; 16];
    for (i, c) in out.iter_mut().enumerate().take(colors) {
        let g = (255 - i * 255 / (colors - 1)) as u8;
        *c = [g, g, g, 255];
    }
    out
}

/// Hardware palettes provided by palette assets and inline palettes.
pub fn palette_colors(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    names: &[&str],
) -> Result<PaletteSet> {
    let mut set: PaletteSet = vec![None; PALETTES];
    for name in names {
        if let Some(inline) = map.palettes.get(*name) {
            let colors = inline
                .colors
                .iter()
                .map(|c| parse_rgb(c))
                .collect::<Result<Vec<_>>>()?;
            for (i, chunk) in colors.chunks(16).enumerate() {
                let mut p = [[0, 0, 0, 255]; 16];
                p[..chunk.len()].copy_from_slice(chunk);
                *set.get_mut(i)
                    .ok_or_else(|| malformed("too many inline palettes"))? = Some(p);
            }
            continue;
        }
        let asset = map.asset(name)?;
        if asset.kind != Kind::Palette {
            return Err(malformed(format!("{name} is not a palette")));
        }
        let first = asset.format.palette_index.unwrap_or_default().usize()?;
        let bytes = decode_asset(map, charmaps, rom, name)?.decoded;
        match asset.format.palette_format.as_deref().unwrap_or("rgb444") {
            "rgb444" => {
                if bytes.is_empty() || bytes.len() % 32 != 0 || bytes.len() / 32 + first > PALETTES
                {
                    return Err(malformed(format!(
                        "{name}: rgb444 palettes are 32-byte records"
                    )));
                }
                let mut padded = vec![0u8; 512];
                padded[..bytes.len()].copy_from_slice(&bytes);
                let parsed = ColorPalettes::parse(&padded)?;
                for i in 0..bytes.len() / 32 {
                    set[first + i] = Some(*parsed.palette(i as u8)?);
                }
            }
            "mono-lut" => {
                // Two bytes per palette: four 3-bit shade selections (port 20+2p).
                if bytes.is_empty() || bytes.len() % 2 != 0 || bytes.len() / 2 + first > PALETTES {
                    return Err(malformed(format!("{name}: mono LUTs are 2-byte records")));
                }
                let levels: Vec<usize> = match &asset.format.shades {
                    Some(l) if l.len() == 8 && l.iter().all(|v| *v < 16) => {
                        l.iter().map(|v| usize::from(*v)).collect()
                    }
                    Some(_) => return Err(malformed(format!("{name}: shades are 8 levels 0-15"))),
                    None => (0..8).map(|i| i * 15 / 7).collect(),
                };
                for (i, pair) in bytes.chunks(2).enumerate() {
                    let mut p = [[0, 0, 0, 255]; 16];
                    for (c, color) in p.iter_mut().enumerate().take(4) {
                        let shade = (pair[c / 2] >> (4 * (c % 2))) & 7;
                        let g = (255 - levels[usize::from(shade)] * 17) as u8;
                        *color = [g, g, g, 255];
                    }
                    set[first + i] = Some(p);
                }
            }
            other => return Err(malformed(format!("{name}: palette format {other:?}"))),
        }
    }
    Ok(set)
}

fn preview_palette(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    asset: &Asset,
    colors: usize,
) -> Result<[Rgba; 16]> {
    let Some(name) = asset.relations.palette.as_deref() else {
        return Ok(grayscale(colors));
    };
    let index = asset.format.palette_index.unwrap_or_default().usize()?;
    let set = palette_colors(map, charmaps, rom, &[name])?;
    set.get(index).copied().flatten().ok_or_else(|| {
        malformed(format!(
            "{}: preview palette {index} not provided",
            asset.id
        ))
    })
}

/// A tile set's decoded tiles plus the geometry used to draw it as a sheet.
pub struct TileSheet {
    pub format: TileFormat,
    pub columns: usize,
    pub palette: [Rgba; 16],
    pub tiles: Vec<[u8; 64]>,
}

fn tile_sheet(map: &AssetMap, charmaps: &Charmaps, rom: &[u8], asset: &Asset) -> Result<TileSheet> {
    let format = tile_format(asset)?;
    let data = decode_asset(map, charmaps, rom, &asset.id)?.decoded;
    if data.is_empty() || data.len() % format.bytes_per_tile() != 0 {
        return Err(malformed(format!(
            "{}: not a whole number of tiles",
            asset.id
        )));
    }
    let tiles = (0..format.tile_count(data.len()))
        .map(|i| decode_tile(&data, i * format.bytes_per_tile(), format))
        .collect::<Result<Vec<_>>>()?;
    Ok(TileSheet {
        format,
        columns: asset.format.width.map_or(Ok(16), |w| w.usize())?.max(1),
        palette: preview_palette(map, charmaps, rom, asset, format.colors())?,
        tiles,
    })
}

fn blank(width: usize, height: usize, fill: Rgba) -> Result<Image> {
    let mut rgba =
        Image::try_rgba_buffer(width as u32, height as u32, MAX_IMAGE_BYTES, "asset image")?;
    for _ in 0..width * height {
        rgba.extend_from_slice(&fill);
    }
    Image::try_new(width as u32, height as u32, rgba)
}

fn put(image: &mut Image, x: usize, y: usize, c: Rgba) {
    let at = (y * image.width as usize + x) * 4;
    image.rgba[at..at + 4].copy_from_slice(&c);
}

fn get(image: &Image, x: usize, y: usize) -> Rgba {
    let at = (y * image.width as usize + x) * 4;
    [
        image.rgba[at],
        image.rgba[at + 1],
        image.rgba[at + 2],
        image.rgba[at + 3],
    ]
}

/// Tiles as a sheet (`format.width` columns, default 16) in the preview palette.
pub fn render_tiles(map: &AssetMap, charmaps: &Charmaps, rom: &[u8], id: &str) -> Result<Image> {
    let sheet = tile_sheet(map, charmaps, rom, map.asset(id)?)?;
    let rows = sheet.tiles.len().div_ceil(sheet.columns);
    let mut image = blank(sheet.columns * 8, rows * 8, [0, 0, 0, 255])?;
    for (i, tile) in sheet.tiles.iter().enumerate() {
        let (tx, ty) = (i % sheet.columns * 8, i / sheet.columns * 8);
        for (p, &v) in tile.iter().enumerate() {
            put(
                &mut image,
                tx + p % 8,
                ty + p / 8,
                sheet.palette[v as usize],
            );
        }
    }
    Ok(image)
}

/// VRAM tile slots populated by a tilemap's tile relations.
struct Vram {
    format: TileFormat,
    slots: BTreeMap<usize, [u8; 64]>,
}

fn vram(map: &AssetMap, charmaps: &Charmaps, rom: &[u8], asset: &Asset) -> Result<Vram> {
    let mut slots = BTreeMap::new();
    let mut format = None;
    for t in &asset.relations.tiles {
        let tiles = map.asset(&t.id)?;
        let sheet = tile_sheet(map, charmaps, rom, tiles)?;
        if format.is_some_and(|f| f != sheet.format) {
            return Err(malformed(format!("{}: mixed tile formats", asset.id)));
        }
        format = Some(sheet.format);
        let (at, skip) = (t.at.usize()?, t.skip.usize()?);
        for (i, tile) in sheet.tiles.into_iter().enumerate().skip(skip) {
            let slot = at + i - skip;
            if slot >= TILE_SLOTS {
                return Err(malformed(format!("{}: tile slot {slot} beyond VRAM", t.id)));
            }
            slots.insert(slot, tile);
        }
    }
    Ok(Vram {
        format: format.ok_or_else(|| malformed(format!("{}: tilemap needs tiles", asset.id)))?,
        slots,
    })
}

struct Cells {
    width: usize,
    height: usize,
    entries: Vec<MapEntry>,
}

fn cells(map: &AssetMap, charmaps: &Charmaps, rom: &[u8], asset: &Asset) -> Result<Cells> {
    let f = &asset.format;
    let width = f
        .width
        .ok_or_else(|| malformed(format!("{}: tilemap width", asset.id)))?
        .usize()?;
    let height = f
        .height
        .ok_or_else(|| malformed(format!("{}: tilemap height", asset.id)))?
        .usize()?;
    if width == 0 || height == 0 || width > 256 || height > 256 {
        return Err(malformed(format!("{}: tilemap dimensions", asset.id)));
    }
    let data = decode_asset(map, charmaps, rom, &asset.id)?.decoded;
    let color = f.color.unwrap_or(true);
    let base = f.tile_base.unwrap_or_default().0 as u16;
    let n = width * height;
    let entries = match f.entry.as_deref().unwrap_or("ws-word") {
        "ws-word" => {
            if data.len() < n * 2 {
                return Err(malformed(format!(
                    "{}: map shorter than {n} words",
                    asset.id
                )));
            }
            data.chunks_exact(2)
                .take(n)
                .map(|w| {
                    MapEntry::from_word(u16::from_le_bytes([w[0], w[1]]).wrapping_add(base), color)
                })
                .collect()
        }
        "byte" => {
            if data.len() < n {
                return Err(malformed(format!(
                    "{}: map shorter than {n} bytes",
                    asset.id
                )));
            }
            let attributes = f.attributes.unwrap_or_default().0 as u16;
            data.iter()
                .take(n)
                .map(|&b| MapEntry::from_word((u16::from(b) + base) | attributes, color))
                .collect()
        }
        "index12-flip" => {
            if data.len() < n * 2 {
                return Err(malformed(format!(
                    "{}: map shorter than {n} words",
                    asset.id
                )));
            }
            let palette = f.palette_index.unwrap_or_default().0 as u8 & 15;
            data.chunks_exact(2)
                .take(n)
                .map(|w| {
                    let w = u16::from_le_bytes([w[0], w[1]]);
                    MapEntry {
                        tile: (w & 0xfff).wrapping_add(base) & 0xfff,
                        palette,
                        bank: false,
                        h_flip: w & 0x4000 != 0,
                        v_flip: w & 0x8000 != 0,
                    }
                })
                .collect()
        }
        other => return Err(malformed(format!("{}: map entry {other:?}", asset.id))),
    };
    Ok(Cells {
        width,
        height,
        entries,
    })
}

fn scene_palettes(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    asset: &Asset,
) -> Result<PaletteSet> {
    let names: Vec<&str> = asset
        .relations
        .palette
        .iter()
        .chain(&asset.relations.palettes)
        .map(String::as_str)
        .collect();
    if names.is_empty() {
        let mut set: PaletteSet = vec![None; PALETTES];
        set.iter_mut().for_each(|p| *p = Some(grayscale(4)));
        return Ok(set);
    }
    palette_colors(map, charmaps, rom, &names)
}

fn cell_pixel(vram: &Vram, e: MapEntry, x: usize, y: usize) -> Option<u8> {
    let tile = vram.slots.get(&usize::from(e.tile))?;
    let sx = if e.h_flip { 7 - x } else { x };
    let sy = if e.v_flip { 7 - y } else { y };
    Some(tile[sy * 8 + sx])
}

/// One tilemap rendered opaquely at its own size (missing tiles are magenta).
pub fn render_tilemap(map: &AssetMap, charmaps: &Charmaps, rom: &[u8], id: &str) -> Result<Image> {
    let asset = map.asset(id)?;
    let c = cells(map, charmaps, rom, asset)?;
    let v = vram(map, charmaps, rom, asset)?;
    let pals = scene_palettes(map, charmaps, rom, asset)?;
    let mut image = blank(c.width * 8, c.height * 8, [255, 0, 255, 255])?;
    for (i, e) in c.entries.iter().enumerate() {
        let pal = pals[usize::from(e.palette)];
        for y in 0..8 {
            for x in 0..8 {
                if let (Some(v), Some(p)) = (cell_pixel(&v, *e, x, y), pal) {
                    put(
                        &mut image,
                        i % c.width * 8 + x,
                        i / c.width * 8 + y,
                        p[usize::from(v)],
                    );
                }
            }
        }
    }
    Ok(image)
}

/// A composed 224x144 screen: layers back to front with hardware transparency.
pub fn render_screen(map: &AssetMap, charmaps: &Charmaps, rom: &[u8], id: &str) -> Result<Image> {
    let screen = map
        .screens
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| malformed(format!("unknown screen {id:?}")))?;
    let back = screen
        .back_color
        .as_deref()
        .map_or(Ok([0, 0, 0, 255]), parse_rgb)?;
    let mut image = blank(WIDTH as usize, HEIGHT as usize, back)?;
    for layer in &screen.layers {
        let asset = map.asset(&layer.tilemap)?;
        let c = cells(map, charmaps, rom, asset)?;
        let v = vram(map, charmaps, rom, asset)?;
        let pals = scene_palettes(map, charmaps, rom, asset)?;
        for (i, e) in c.entries.iter().enumerate() {
            let Some(pal) = pals[usize::from(e.palette)] else {
                continue;
            };
            for y in 0..8 {
                for x in 0..8 {
                    let Some(index) = cell_pixel(&v, *e, x, y) else {
                        continue;
                    };
                    if transparent(e.palette, index, v.format) {
                        continue;
                    }
                    let px = layer.x + (i % c.width * 8 + x) as i32;
                    let py = layer.y + (i / c.width * 8 + y) as i32;
                    if (0..WIDTH as i32).contains(&px) && (0..HEIGHT as i32).contains(&py) {
                        put(
                            &mut image,
                            px as usize,
                            py as usize,
                            pal[usize::from(index)],
                        );
                    }
                }
            }
        }
    }
    Ok(image)
}

fn nearest(palette: &[Rgba], colors: usize, c: Rgba, original: u8) -> (u8, bool) {
    if palette[usize::from(original)][..3] == c[..3] {
        return (original, true);
    }
    let mut best = (0u8, u32::MAX);
    for (i, p) in palette.iter().enumerate().take(colors) {
        let d = (0..3)
            .map(|k| (i32::from(p[k]) - i32::from(c[k])).unsigned_abs().pow(2))
            .sum::<u32>();
        if d < best.1 {
            best = (i as u8, d);
        }
    }
    (best.0, best.1 == 0)
}

/// New decoded payloads (by tile asset id) and the count of approximated pixels.
pub type TileEdits = (BTreeMap<String, Vec<u8>>, usize);

/// Map an edited tile sheet back to tile bytes (same layout as `render_tiles`).
pub fn unrender_tiles(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    id: &str,
    image: &Image,
) -> Result<TileEdits> {
    let asset = map.asset(id)?;
    let sheet = tile_sheet(map, charmaps, rom, asset)?;
    let rows = sheet.tiles.len().div_ceil(sheet.columns);
    if image.width as usize != sheet.columns * 8 || image.height as usize != rows * 8 {
        return Err(malformed(format!(
            "{id}: image must be {}x{} like the extracted sheet",
            sheet.columns * 8,
            rows * 8
        )));
    }
    let mut approx = 0;
    let mut out = Vec::new();
    for (i, tile) in sheet.tiles.iter().enumerate() {
        let (tx, ty) = (i % sheet.columns * 8, i / sheet.columns * 8);
        let mut indices = [0u8; 64];
        for (p, slot) in indices.iter_mut().enumerate() {
            let (v, exact) = nearest(
                &sheet.palette,
                sheet.format.colors(),
                get(image, tx + p % 8, ty + p / 8),
                tile[p],
            );
            approx += usize::from(!exact);
            *slot = v;
        }
        out.extend(encode_tile(&indices, sheet.format));
    }
    Ok((BTreeMap::from([(id.to_owned(), out)]), approx))
}

/// Map an edited tilemap image back onto the tiles it uses. The map words are
/// unchanged; a tile drawn by several cells must receive identical pixels.
pub fn unrender_tilemap(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    id: &str,
    image: &Image,
) -> Result<TileEdits> {
    let asset = map.asset(id)?;
    let c = cells(map, charmaps, rom, asset)?;
    let v = vram(map, charmaps, rom, asset)?;
    let pals = scene_palettes(map, charmaps, rom, asset)?;
    if image.width as usize != c.width * 8 || image.height as usize != c.height * 8 {
        return Err(malformed(format!(
            "{id}: image must be {}x{} like the extracted tilemap",
            c.width * 8,
            c.height * 8
        )));
    }
    let mut edited: BTreeMap<usize, ([u8; 64], usize)> = BTreeMap::new();
    let mut approx = 0;
    for (i, e) in c.entries.iter().enumerate() {
        let slot = usize::from(e.tile);
        let (Some(original), Some(pal)) = (v.slots.get(&slot), pals[usize::from(e.palette)]) else {
            continue;
        };
        let mut tile = *original;
        for y in 0..8 {
            for x in 0..8 {
                let sx = if e.h_flip { 7 - x } else { x };
                let sy = if e.v_flip { 7 - y } else { y };
                let px = get(image, i % c.width * 8 + x, i / c.width * 8 + y);
                let (value, exact) = nearest(&pal, v.format.colors(), px, original[sy * 8 + sx]);
                approx += usize::from(!exact);
                tile[sy * 8 + sx] = value;
            }
        }
        match edited.get(&slot) {
            Some((prior, cell)) if *prior != tile => {
                return Err(malformed(format!(
                    "{id}: tile slot {slot} is drawn by cells {cell} and {i} with different edits; \
                     edit only cells with unique tiles or replace the tiles asset"
                )));
            }
            Some(_) => {}
            None => {
                edited.insert(slot, (tile, i));
            }
        }
    }
    let mut out = BTreeMap::new();
    // Later relations overwrite earlier ones in VRAM: the last loader owns a slot.
    let mut owned = std::collections::BTreeSet::new();
    for t in asset.relations.tiles.iter().rev() {
        let tiles = map.asset(&t.id)?;
        let format = tile_format(tiles)?;
        let mut data = decode_asset(map, charmaps, rom, &t.id)?.decoded;
        let (at, skip) = (t.at.usize()?, t.skip.usize()?);
        let mut changed = false;
        for (slot, (tile, _)) in &edited {
            if *slot < at || owned.contains(slot) {
                continue;
            }
            let index = slot - at + skip;
            let start = index * format.bytes_per_tile();
            if start + format.bytes_per_tile() > data.len() {
                continue;
            }
            owned.insert(*slot);
            let bytes = encode_tile(tile, format);
            if data[start..start + bytes.len()] != bytes[..] {
                data[start..start + bytes.len()].copy_from_slice(&bytes);
                changed = true;
            }
        }
        if changed {
            out.insert(t.id.clone(), data);
        }
    }
    Ok((out, approx))
}
