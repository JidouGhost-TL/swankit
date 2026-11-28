//! WebAssembly bindings for Swankit WonderSwan ROM, asset-map, save-data,
//! and IPS patch operations. All processing runs locally on caller-supplied
//! byte buffers with zero network or filesystem access.
#![forbid(unsafe_code)]

use formatkit_core::{Error, Image};
use formatkit_pixel::grayscale_palette;
use js_sys::{Object, Reflect, Uint8Array};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;
use ws_assets::formatkit::format_id_for;
use ws_assets::map::{Asset, AssetMap, Codec, Kind};
use ws_assets::{
    decode_asset, palette_colors, render_sprites, render_tilemap, replace, sha256_hex, Charmaps,
    LoadedAssetMap, ReplaceInput, Replacement,
};
use ws_gfx::{decode_tile, ColorPalettes, TileFormat};
use ws_patch::{ApplyOptions, Patch};
use ws_rom::{publisher_name, Orientation, RomFacts};
use ws_saves::{DecodeContext, Format as SaveFormat};
use ws_sound::render::{signed_pcm, unsigned_pcm};
use ws_text::{FontDefinition, FontFormat, GlyphOrder};

const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_PCM_SAMPLE_RATE: u32 = 12_000;

/// Internal preview representation converted to a JS object at the WASM boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewOutput {
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Wav {
        sample_rate: u32,
        wav: Vec<u8>,
    },
    Bytes {
        data: Vec<u8>,
        format_id: String,
    },
    Text {
        text: String,
    },
}

/// Internal replacement output converted to a JS object at the WASM boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceDiffOutput {
    pub patched_rom: Vec<u8>,
    pub ips_patch: Vec<u8>,
}

#[derive(Debug, Serialize)]
struct RomIdentityJson {
    sha256: String,
    size: usize,
    size_pow2: bool,
    publisher_id: u8,
    publisher: Option<&'static str>,
    mapper_code: u8,
    mapper: String,
    save_code: u8,
    save_type: String,
    save_bytes: Option<u32>,
    color: bool,
    system: &'static str,
    orientation: &'static str,
    game_id: u8,
    version: u8,
    rom_size_code: u8,
    declared_rom_bytes: Option<usize>,
    declared_size_matches: bool,
    bus_width: u8,
    rom_speed_cycles: u8,
    reset_segment: u16,
    reset_offset: u16,
    reset_linear: u32,
    checksum_footer: u16,
    checksum_computed: u16,
    checksum_ok: bool,
}

fn load_map_and_charmaps(map_json: &str) -> Result<(LoadedAssetMap, AssetMap, Charmaps), String> {
    let loaded = LoadedAssetMap::from_json(map_json.as_bytes()).map_err(|e| e.to_string())?;
    let asset_map = loaded.to_asset_map().map_err(|e| e.to_string())?;
    let charmaps = Charmaps::load(&asset_map, |path| {
        Err(Error::Malformed(format!(
            "external charmap file {path:?} is not available in WebAssembly; use inline or builtin charmaps"
        )))
    })
    .map_err(|e| e.to_string())?;
    Ok((loaded, asset_map, charmaps))
}

fn parse_palette_override(bytes: &[u8]) -> Result<[[u8; 4]; 16], String> {
    match bytes.len() {
        32 => {
            let mut padded = vec![0u8; 512];
            padded[..32].copy_from_slice(bytes);
            let parsed = ColorPalettes::parse(&padded).map_err(|e| e.to_string())?;
            parsed.palette(0).copied().map_err(|e| e.to_string())
        }
        512 => {
            let parsed = ColorPalettes::parse(bytes).map_err(|e| e.to_string())?;
            parsed.palette(0).copied().map_err(|e| e.to_string())
        }
        64 => {
            let mut out = [[0, 0, 0, 255]; 16];
            for (i, chunk) in bytes.chunks_exact(4).enumerate() {
                out[i] = [chunk[0], chunk[1], chunk[2], chunk[3]];
            }
            Ok(out)
        }
        16 => {
            let mut out = [[0, 0, 0, 255]; 16];
            for (i, chunk) in bytes.chunks_exact(4).enumerate() {
                out[i] = [chunk[0], chunk[1], chunk[2], chunk[3]];
            }
            Ok(out)
        }
        2 => {
            let mut out = [[0, 0, 0, 255]; 16];
            for (c, color) in out.iter_mut().enumerate().take(4) {
                let shade = (bytes[c / 2] >> (4 * (c % 2))) & 7;
                let level = usize::from(shade) * 15 / 7;
                let g = (255 - level * 17) as u8;
                *color = [g, g, g, 255];
            }
            Ok(out)
        }
        other => Err(format!(
            "palette_override must be 32 (rgb444), 512 (16x rgb444), 64 (16x rgba), 16 (4x rgba), or 2 (mono-lut) bytes; got {other}"
        )),
    }
}

fn grayscale_16(colors: usize) -> [[u8; 4]; 16] {
    let mut out = [[0, 0, 0, 255]; 16];
    for (dst, color) in out
        .iter_mut()
        .zip(grayscale_palette(colors).into_iter().rev())
    {
        *dst = color;
    }
    out
}

fn tile_format_for(asset: &Asset) -> Result<TileFormat, String> {
    let f = &asset.format;
    match (f.bpp.unwrap_or(2), f.layout.as_deref().unwrap_or("planar")) {
        (2, "planar") => Ok(TileFormat::Bpp2),
        (4, "planar") => Ok(TileFormat::Bpp4Planar),
        (4, "packed") => Ok(TileFormat::Bpp4Packed),
        (bpp, layout) => Err(format!(
            "{}: unsupported tile format {bpp}bpp {layout}",
            asset.id
        )),
    }
}

fn render_tiles_preview(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    asset: &Asset,
    decoded: &[u8],
    palette_override: Option<&[u8]>,
    tiles_per_row: Option<u32>,
) -> Result<Image, String> {
    let format = tile_format_for(asset)?;
    let bpt = format.bytes_per_tile();
    if decoded.is_empty() || !decoded.len().is_multiple_of(bpt) {
        return Err(format!("{}: not a whole number of tiles", asset.id));
    }
    let tile_count = format.tile_count(decoded.len());
    let tiles = (0..tile_count)
        .map(|i| decode_tile(decoded, i * bpt, format))
        .collect::<formatkit_core::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;

    let palette = if let Some(ov) = palette_override.filter(|b| !b.is_empty()) {
        parse_palette_override(ov)?
    } else if let Some(name) = asset.relations.palette.as_deref() {
        let idx = asset
            .format
            .palette_index
            .unwrap_or_default()
            .usize()
            .map_err(|e| e.to_string())?;
        let set = palette_colors(map, charmaps, rom, &[name]).map_err(|e| e.to_string())?;
        set.get(idx)
            .copied()
            .flatten()
            .unwrap_or_else(|| grayscale_16(format.colors()))
    } else {
        grayscale_16(format.colors())
    };

    let columns = match tiles_per_row {
        Some(n) if n > 0 => n as usize,
        _ => asset
            .format
            .width
            .map_or(Ok(16), |w| w.usize())
            .map_err(|e| e.to_string())?
            .max(1),
    };
    let rows = tiles.len().div_ceil(columns);
    let width = columns * 8;
    let height = rows * 8;
    let mut rgba = Image::try_rgba_buffer(width as u32, height as u32, MAX_IMAGE_BYTES, "tiles")
        .map_err(|e| e.to_string())?;
    rgba.resize(width * height * 4, 0);
    for (i, tile) in tiles.iter().enumerate() {
        let (tx, ty) = ((i % columns) * 8, (i / columns) * 8);
        for (p, &v) in tile.iter().enumerate() {
            let x = tx + p % 8;
            let y = ty + p / 8;
            let at = (y * width + x) * 4;
            rgba[at..at + 4].copy_from_slice(&palette[usize::from(v) & 0x0f]);
        }
    }
    Image::try_new(width as u32, height as u32, rgba).map_err(|e| e.to_string())
}

fn render_palette_swatch(set: &[Option<[[u8; 4]; 16]>]) -> Result<Image, String> {
    let rows: Vec<_> = set.iter().flatten().collect();
    if rows.is_empty() {
        return Err("palette has no colours".into());
    }
    let (w, h) = (16 * 8, rows.len() * 8);
    let mut rgba = Image::try_rgba_buffer(w as u32, h as u32, MAX_IMAGE_BYTES, "swatch")
        .map_err(|e| e.to_string())?;
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&rows[y / 8][x / 8]);
        }
    }
    Image::try_new(w as u32, h as u32, rgba).map_err(|e| e.to_string())
}

fn render_font_preview(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    asset: &Asset,
    decoded: &[u8],
    palette_override: Option<&[u8]>,
    tiles_per_row: Option<u32>,
) -> Result<Image, String> {
    let f = &asset.format;
    let format = match (f.bpp.unwrap_or(1), f.layout.as_deref().unwrap_or("planar")) {
        (1, _) => FontFormat::Bpp1,
        (2, _) => FontFormat::Bpp2,
        (4, "planar") => FontFormat::Bpp4Planar,
        (4, "packed") => FontFormat::Bpp4Packed,
        _ => return Err(format!("{}: unsupported font bpp/layout", asset.id)),
    };
    let order = match f.order.as_deref().unwrap_or("row") {
        "row" => GlyphOrder::Row,
        "column" => GlyphOrder::Column,
        other => return Err(format!("{}: unsupported glyph order {other:?}", asset.id)),
    };
    let tiles_wide = f.tiles_wide.unwrap_or(1);
    let tiles_high = f.tiles_high.unwrap_or(1);
    let tile_bytes = match format {
        FontFormat::Bpp1 => 8,
        FontFormat::Bpp2 => 16,
        _ => 32,
    };
    let natural = tile_bytes * (tiles_wide as usize) * (tiles_high as usize);
    let count = f
        .count
        .ok_or_else(|| format!("{}: font missing count", asset.id))?
        .usize()
        .map_err(|e| e.to_string())?;
    let stride = f
        .stride
        .map_or(Ok(natural), |s| s.usize())
        .map_err(|e| e.to_string())?;
    let tile_offsets = f
        .tile_offsets
        .iter()
        .flatten()
        .map(|n| n.usize())
        .collect::<formatkit_core::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let def = FontDefinition {
        offset: 0,
        count,
        stride,
        format,
        tiles_wide,
        tiles_high,
        order,
        tile_offsets,
        widths: vec![],
        width_table: None,
    };

    let colors = match format {
        FontFormat::Bpp1 => 2,
        FontFormat::Bpp2 => 4,
        _ => 16,
    };
    let palette: Vec<[u8; 4]> = if let Some(ov) = palette_override.filter(|b| !b.is_empty()) {
        parse_palette_override(ov)?.to_vec()
    } else if let Some(name) = asset.relations.palette.as_deref() {
        let idx = f
            .palette_index
            .unwrap_or_default()
            .usize()
            .map_err(|e| e.to_string())?;
        palette_colors(map, charmaps, rom, &[name])
            .map_err(|e| e.to_string())?
            .get(idx)
            .copied()
            .flatten()
            .ok_or_else(|| format!("{}: preview palette missing", asset.id))?
            .to_vec()
    } else {
        (0..16)
            .map(|i| {
                let g = 255 - (i.min(colors - 1) * 255 / (colors - 1)) as u8;
                [g, g, g, 255]
            })
            .collect()
    };

    let columns = match tiles_per_row {
        Some(n) if n > 0 => n as usize,
        _ => f
            .width
            .map_or(Ok(16), |w| w.usize())
            .map_err(|e| e.to_string())?
            .max(1),
    };
    let (gw, gh) = ((tiles_wide as usize) * 8, (tiles_high as usize) * 8);
    let rows = def.count.div_ceil(columns);
    let (w, h) = (columns * gw, rows * gh);
    let mut rgba = Image::try_rgba_buffer(w as u32, h as u32, MAX_IMAGE_BYTES, "font sheet")
        .map_err(|e| e.to_string())?;
    rgba.resize(w * h * 4, 0);
    for g in 0..def.count {
        let glyph = def
            .glyph(decoded, g, &palette, MAX_IMAGE_BYTES)
            .map_err(|e| e.to_string())?
            .image;
        let (ox, oy) = ((g % columns) * gw, (g / columns) * gh);
        formatkit_pixel::blit_rgba(
            &mut rgba,
            w as u32,
            h as u32,
            ox as u32,
            oy as u32,
            &glyph.rgba,
            gw as u32,
            gh as u32,
            false,
            false,
            1,
            false,
        )
        .map_err(|e| e.to_string())?;
    }
    Image::try_new(w as u32, h as u32, rgba).map_err(|e| e.to_string())
}

fn format_save_title(id: &str) -> String {
    id.replace('_', " ")
}

/// Pure-Rust core implementation of [`identify_rom`].
pub fn identify_rom_impl(rom: &[u8]) -> Result<String, String> {
    let facts = RomFacts::parse(rom).map_err(|e| e.to_string())?;
    let h = facts.header;
    let info = RomIdentityJson {
        sha256: sha256_hex(rom),
        size: rom.len(),
        size_pow2: facts.geometry.is_power_of_two(),
        publisher_id: h.publisher_id,
        publisher: publisher_name(h.publisher_id),
        mapper_code: h.mapper_code,
        mapper: format!("{:?}", h.mapper()),
        save_code: h.save_code,
        save_type: format!("{:?}", h.save()),
        save_bytes: if h.save_code == 1 {
            Some(32_768)
        } else {
            h.save().bytes()
        },
        color: h.color(),
        system: if h.color() { "color" } else { "mono" },
        orientation: match h.orientation() {
            Orientation::Horizontal => "horizontal",
            Orientation::Vertical => "vertical",
        },
        game_id: h.game_id,
        version: h.version,
        rom_size_code: h.rom_size_code,
        declared_rom_bytes: h.declared_rom_size(),
        declared_size_matches: facts.declared_size_matches(),
        bus_width: h.bus_width(),
        rom_speed_cycles: h.rom_speed_cycles(),
        reset_segment: h.reset_segment,
        reset_offset: h.reset_offset,
        reset_linear: h.reset_linear(),
        checksum_footer: h.checksum,
        checksum_computed: facts.checksum_computed,
        checksum_ok: facts.checksum_ok(),
    };
    serde_json::to_string(&info).map_err(|e| e.to_string())
}

/// Pure-Rust core implementation of [`verify_asset_map`].
pub fn verify_asset_map_impl(rom: &[u8], map_json: &str) -> Result<String, String> {
    let (loaded, asset_map, charmaps) = load_map_and_charmaps(map_json)?;
    let report = loaded
        .verify(&charmaps, rom)
        .map_err(|e| e.to_string())?;

    let mut errors_by_id: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for finding in &report.errors {
        errors_by_id
            .entry(finding.id.as_str())
            .or_default()
            .push(finding.message.as_str());
    }

    let entries: Vec<Value> = match &loaded {
        LoadedAssetMap::FormatkitV1(doc) => doc
            .entries
            .iter()
            .map(|e| {
                let errs = errors_by_id.get(e.id.as_str()).cloned().unwrap_or_default();
                json!({
                    "id": e.id,
                    "offset": e.offset,
                    "size": e.size,
                    "status": e.status.as_str(),
                    "format_id": e.format_id,
                    "ok": errs.is_empty(),
                    "errors": errs,
                })
            })
            .collect(),
        LoadedAssetMap::SwankitV1(_) => asset_map
            .assets
            .iter()
            .map(|a| {
                let errs = errors_by_id.get(a.id.as_str()).cloned().unwrap_or_default();
                json!({
                    "id": a.id,
                    "offset": a.offset.0,
                    "size": a.length.map(|n| n.0),
                    "kind": a.kind.name(),
                    "format_id": format_id_for(a),
                    "ok": errs.is_empty(),
                    "errors": errs,
                })
            })
            .collect(),
    };

    let summary = json!({
        "ok": report.ok(),
        "schema": loaded.schema(),
        "assets": report.assets,
        "references": report.references,
        "mapped_bytes": report.mapped_bytes,
        "free_bytes": report.free_bytes,
        "rom_bytes": report.rom_bytes,
        "coverage": report.coverage(),
        "error_count": report.errors.len(),
        "errors": report.errors,
        "by_kind": report.by_kind,
        "by_confidence": report.by_confidence,
        "entries": entries,
    });
    serde_json::to_string(&summary).map_err(|e| e.to_string())
}

/// Pure-Rust core implementation of [`preview_asset`].
pub fn preview_asset_impl(
    rom: &[u8],
    map_json: &str,
    entry_id: &str,
    palette_override: Option<&[u8]>,
    tiles_per_row: Option<u32>,
    sample_rate: Option<u32>,
) -> Result<PreviewOutput, String> {
    let (loaded, asset_map, charmaps) = load_map_and_charmaps(map_json)?;
    ws_assets::check_identity(&asset_map, rom).map_err(|e| e.to_string())?;

    let asset = match asset_map.asset(entry_id) {
        Ok(asset) => asset,
        Err(err) => {
            if let LoadedAssetMap::FormatkitV1(doc) = &loaded {
                if let Some(entry) = doc.entries.iter().find(|e| e.id == entry_id) {
                    let start = entry
                        .offset
                        .and_then(|o| usize::try_from(o).ok())
                        .ok_or_else(|| format!("{entry_id}: entry has no valid offset"))?;
                    let size = usize::try_from(entry.size)
                        .map_err(|_| format!("{entry_id}: entry size overflows usize"))?;
                    let end = start
                        .checked_add(size)
                        .filter(|&end| end <= rom.len())
                        .ok_or_else(|| format!("{entry_id}: entry range exceeds ROM length"))?;
                    return Ok(PreviewOutput::Bytes {
                        data: rom[start..end].to_vec(),
                        format_id: entry
                            .format_id
                            .clone()
                            .unwrap_or_else(|| "ws.data.raw".to_owned()),
                    });
                }
            }
            return Err(err.to_string());
        }
    };

    let stored =
        decode_asset(&asset_map, &charmaps, rom, entry_id).map_err(|e| e.to_string())?;
    let format_id = format_id_for(asset);

    if asset.kind == Kind::Pcm
        || matches!(asset.codec, Codec::RlePcm00 | Codec::Dpcm6bit)
    {
        let rate = sample_rate
            .filter(|&r| r > 0)
            .or(asset.format.sample_rate)
            .unwrap_or(DEFAULT_PCM_SAMPLE_RATE);
        let pcm = match asset.format.sample_format.as_deref().unwrap_or("u8") {
            "s8" => signed_pcm(&stored.decoded, rate, stored.decoded.len()),
            "u8" => unsigned_pcm(&stored.decoded, rate, stored.decoded.len()),
            other => {
                return Err(format!(
                    "{entry_id}: unsupported pcm sample_format {other:?}"
                ))
            }
        }
        .map_err(|e| e.to_string())?;
        let wav = pcm.to_wav_bytes().map_err(|e| e.to_string())?;
        return Ok(PreviewOutput::Wav {
            sample_rate: rate,
            wav,
        });
    }

    match asset.kind {
        Kind::Tiles => {
            let img = render_tiles_preview(
                &asset_map,
                &charmaps,
                rom,
                asset,
                &stored.decoded,
                palette_override,
                tiles_per_row,
            )?;
            Ok(PreviewOutput::Image {
                width: img.width,
                height: img.height,
                rgba: img.rgba,
            })
        }
        Kind::Tilemap if !asset.relations.tiles.is_empty() => {
            let img =
                render_tilemap(&asset_map, &charmaps, rom, entry_id).map_err(|e| e.to_string())?;
            Ok(PreviewOutput::Image {
                width: img.width,
                height: img.height,
                rgba: img.rgba,
            })
        }
        Kind::SpriteTable if !asset.relations.tiles.is_empty() => {
            let img =
                render_sprites(&asset_map, &charmaps, rom, entry_id).map_err(|e| e.to_string())?;
            Ok(PreviewOutput::Image {
                width: img.width,
                height: img.height,
                rgba: img.rgba,
            })
        }
        Kind::Palette => {
            let set =
                palette_colors(&asset_map, &charmaps, rom, &[entry_id]).map_err(|e| e.to_string())?;
            let img = render_palette_swatch(&set)?;
            Ok(PreviewOutput::Image {
                width: img.width,
                height: img.height,
                rgba: img.rgba,
            })
        }
        Kind::Font => {
            let img = render_font_preview(
                &asset_map,
                &charmaps,
                rom,
                asset,
                &stored.decoded,
                palette_override,
                tiles_per_row,
            )?;
            Ok(PreviewOutput::Image {
                width: img.width,
                height: img.height,
                rgba: img.rgba,
            })
        }
        Kind::Text => {
            let decoded = charmaps
                .decode(asset, &stored.decoded)
                .map_err(|e| e.to_string())?;
            Ok(PreviewOutput::Text { text: decoded.text })
        }
        _ => Ok(PreviewOutput::Bytes {
            data: stored.decoded,
            format_id,
        }),
    }
}

/// Pure-Rust core implementation of [`replace_asset_and_diff`].
pub fn replace_asset_and_diff_impl(
    rom: &[u8],
    map_json: &str,
    entry_id: &str,
    new_bytes: &[u8],
) -> Result<ReplaceDiffOutput, String> {
    let (_loaded, asset_map, charmaps) = load_map_and_charmaps(map_json)?;
    let replacement = Replacement {
        id: entry_id.to_owned(),
        input: ReplaceInput::Bytes(new_bytes.to_vec()),
    };
    let outcome =
        replace(&asset_map, &charmaps, rom, &replacement).map_err(|e| e.to_string())?;
    let ips_patch = ws_patch::create(rom, &outcome.rom).map_err(|e| e.to_string())?;
    Ok(ReplaceDiffOutput {
        patched_rom: outcome.rom,
        ips_patch,
    })
}

/// Pure-Rust core implementation of [`apply_ips`].
pub fn apply_ips_impl(rom: &[u8], ips: &[u8]) -> Result<Vec<u8>, String> {
    let patch = Patch::parse(ips).map_err(|e| e.to_string())?;
    patch
        .apply(rom, ApplyOptions::default())
        .map_err(|e| e.to_string())
}

/// Pure-Rust core implementation of [`list_save_defs`].
pub fn list_save_defs_impl() -> String {
    let formats = match ws_saves::formats() {
        Ok(f) => f,
        Err(_) => return "[]".to_owned(),
    };
    let items: Vec<Value> = formats
        .iter()
        .map(|f| {
            let media_arr = f.definition()["media"].as_array();
            let primary_media = media_arr
                .and_then(|arr| arr.first())
                .and_then(|m| m["kind"].as_str())
                .unwrap_or("unknown");
            let primary_size = media_arr
                .and_then(|arr| arr.first())
                .and_then(|m| m["sizes"].as_array())
                .and_then(|s| s.first())
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let all_sizes: Vec<u64> = media_arr
                .into_iter()
                .flatten()
                .filter_map(|m| m["sizes"].as_array())
                .flatten()
                .filter_map(Value::as_u64)
                .collect();
            json!({
                "id": f.id,
                "title": format_save_title(&f.id),
                "media": primary_media,
                "size": primary_size,
                "sizes": all_sizes,
                "media_list": f.definition()["media"],
                "field_count": f.fields.len(),
            })
        })
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_owned())
}

fn select_save_format(
    save: &[u8],
    title_id: Option<&str>,
    ctx: &DecodeContext,
) -> Result<(&'static SaveFormat, Vec<Value>), String> {
    if let Some(id) = title_id.filter(|s| !s.is_empty()) {
        let fmt = ws_saves::format(id).map_err(|e| e.to_string())?;
        return Ok((fmt, Vec::new()));
    }
    let formats = ws_saves::formats().map_err(|e| e.to_string())?;
    let mut matches: Vec<(&'static SaveFormat, ws_saves::ValidityEvidence)> = Vec::new();
    for f in formats {
        if f.accepts_primary_size(save.len()) {
            if let Ok(Some(ev)) = f.recognize_with_context(save, "save.bin", ctx) {
                matches.push((f, ev));
            }
        }
    }
    if matches.is_empty() {
        return Err(
            "no matching save definition found for this image; specify title_id explicitly".into(),
        );
    }
    matches.sort_by(|a, b| {
        b.1.usable_flags
            .cmp(&a.1.usable_flags)
            .then_with(|| b.1.checks_passed.cmp(&a.1.checks_passed))
            .then_with(|| a.1.checks_failed.cmp(&b.1.checks_failed))
    });
    let candidates: Vec<Value> = matches
        .iter()
        .map(|(f, ev)| {
            json!({
                "id": f.id,
                "title": format_save_title(&f.id),
                "evidence": ev,
            })
        })
        .collect();
    Ok((matches[0].0, candidates))
}

/// Pure-Rust core implementation of [`inspect_save`].
pub fn inspect_save_impl(save: &[u8], title_id: Option<&str>) -> Result<String, String> {
    let ctx = DecodeContext::default();
    let (fmt, candidates) = select_save_format(save, title_id, &ctx)?;
    if !fmt.accepts_size(save.len()) {
        return Err(format!(
            "save size {} bytes is not declared by definition {}",
            save.len(),
            fmt.id
        ));
    }

    let evidence = fmt
        .recognize_with_context(save, "save.bin", &ctx)
        .map_err(|e| e.to_string())?;
    let decoded = fmt
        .decode_with_context(save, "save.bin", &ctx)
        .map_err(|e| e.to_string())?;
    let checks = fmt
        .verify_with_context(save, "save.bin", &ctx)
        .map_err(|e| e.to_string())?;

    let raw_fields = fmt.definition()["fields"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let fields: Vec<Value> = fmt
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let meta = raw_fields.get(index);
            let current_value = if field
                .offset
                .checked_add(field.size)
                .is_some_and(|end| end <= save.len())
            {
                field.read(save).ok()
            } else {
                None
            };
            let copy_group = fmt.copy_group_for_field(index).map(|g| g.name.as_str());
            json!({
                "index": index,
                "name": field.name,
                "medium": field.medium,
                "offset": field.offset,
                "size": field.size,
                "encoding": field.encoding,
                "type": meta.and_then(|m| m.get("type")).cloned().unwrap_or(Value::Null),
                "meaning": meta.and_then(|m| m.get("meaning")).cloned().unwrap_or(Value::Null),
                "confidence": meta.and_then(|m| m.get("confidence")).cloned().unwrap_or(Value::Null),
                "constraint": meta.and_then(|m| m.get("constraint")).cloned().unwrap_or(Value::Null),
                "array": meta.and_then(|m| m.get("array")).cloned().unwrap_or(Value::Null),
                "copy_group": copy_group,
                "value": current_value,
            })
        })
        .collect();

    let copy_groups: Vec<Value> = fmt
        .copy_groups()
        .iter()
        .map(|g| {
            json!({
                "name": g.name,
                "fields": g.fields,
            })
        })
        .collect();

    let out = json!({
        "id": fmt.id,
        "title": format_save_title(&fmt.id),
        "size": save.len(),
        "valid": evidence.is_some(),
        "evidence": evidence,
        "checks": checks,
        "decoded": decoded,
        "copy_groups": copy_groups,
        "candidates": candidates,
        "fields": fields,
    });
    serde_json::to_string(&out).map_err(|e| e.to_string())
}

struct ParsedEdit {
    field_index: usize,
    value: Value,
    all_copies: Option<bool>,
}

fn resolve_field_index(fmt: &SaveFormat, name: &str) -> Result<usize, String> {
    if let Ok(idx) = name.parse::<usize>() {
        if idx < fmt.fields.len() {
            return Ok(idx);
        }
    }
    let matches: Vec<usize> = fmt
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| f.name == name)
        .map(|(i, _)| i)
        .collect();
    match matches.len() {
        0 => Err(format!("unknown save field {name:?}")),
        1 => Ok(matches[0]),
        _ => Err(format!(
            "ambiguous save field name {name:?}; specify numeric field index"
        )),
    }
}

fn parse_save_edits(fmt: &SaveFormat, edits_json: &str) -> Result<Vec<ParsedEdit>, String> {
    let root: Value =
        serde_json::from_str(edits_json).map_err(|e| format!("invalid edits_json: {e}"))?;
    match root {
        Value::Array(items) => {
            let mut edits = Vec::with_capacity(items.len());
            for item in items {
                let obj = item
                    .as_object()
                    .ok_or_else(|| "each edit entry must be a JSON object".to_owned())?;
                let field_index = if let Some(idx) = obj.get("field").and_then(Value::as_u64) {
                    usize::try_from(idx).map_err(|_| "field index overflow".to_owned())?
                } else if let Some(idx) = obj.get("index").and_then(Value::as_u64) {
                    usize::try_from(idx).map_err(|_| "field index overflow".to_owned())?
                } else if let Some(name) = obj
                    .get("name")
                    .or_else(|| obj.get("field"))
                    .and_then(Value::as_str)
                {
                    resolve_field_index(fmt, name)?
                } else {
                    return Err("edit entry must specify \"field\", \"index\", or \"name\"".into());
                };
                if field_index >= fmt.fields.len() {
                    return Err(format!("field index {field_index} out of range"));
                }
                let value = obj
                    .get("value")
                    .cloned()
                    .ok_or_else(|| "edit entry missing \"value\"".to_owned())?;
                let all_copies = obj.get("all_copies").and_then(Value::as_bool);
                edits.push(ParsedEdit {
                    field_index,
                    value,
                    all_copies,
                });
            }
            Ok(edits)
        }
        Value::Object(map) => {
            let mut edits = Vec::with_capacity(map.len());
            for (key, value) in map {
                let field_index = resolve_field_index(fmt, &key)?;
                edits.push(ParsedEdit {
                    field_index,
                    value,
                    all_copies: None,
                });
            }
            Ok(edits)
        }
        _ => Err("edits_json must be a JSON array of edit objects or a field-to-value object".into()),
    }
}

/// Pure-Rust core implementation of [`edit_save`].
pub fn edit_save_impl(save: &[u8], title_id: &str, edits_json: &str) -> Result<Vec<u8>, String> {
    let fmt = ws_saves::format(title_id).map_err(|e| e.to_string())?;
    if !fmt.accepts_size(save.len()) {
        return Err(format!(
            "save size {} bytes is not declared by definition {}",
            save.len(),
            fmt.id
        ));
    }
    let edits = parse_save_edits(fmt, edits_json)?;
    let mut out = save.to_vec();

    for edit in edits {
        let targets: Vec<usize> = match edit.all_copies {
            Some(false) => vec![edit.field_index],
            Some(true) => fmt
                .copy_group_for_field(edit.field_index)
                .map(|g| g.fields.clone())
                .unwrap_or_else(|| vec![edit.field_index]),
            None => fmt
                .copy_group_for_field(edit.field_index)
                .map(|g| g.fields.clone())
                .unwrap_or_else(|| vec![edit.field_index]),
        };
        for target in &targets {
            fmt.validate_field_value(*target, &edit.value)
                .map_err(|e| e.to_string())?;
        }
        for target in targets {
            fmt.set_field(&mut out, target, &edit.value)
                .map_err(|e| e.to_string())?;
        }
    }

    let ctx = DecodeContext::default();
    fmt.fix_with_context(&mut out, "save.bin", &ctx)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

fn set_prop(obj: &Object, key: &str, val: &JsValue) -> Result<(), JsValue> {
    Reflect::set(obj, &JsValue::from_str(key), val)?;
    Ok(())
}

/// Identify a WonderSwan ROM image and return its SHA-256 and parsed footer
/// metadata as a JSON string.
#[wasm_bindgen]
pub fn identify_rom(rom: &[u8]) -> Result<String, JsValue> {
    identify_rom_impl(rom).map_err(|e| JsValue::from_str(&e))
}

/// Verify a `formatkit.asset-map.v1` or `swankit-asset-map/1` JSON manifest
/// against a ROM image, returning a JSON summary with per-entry status.
#[wasm_bindgen]
pub fn verify_asset_map(rom: &[u8], map_json: &str) -> Result<String, JsValue> {
    verify_asset_map_impl(rom, map_json).map_err(|e| JsValue::from_str(&e))
}

/// Decode and preview a single mapped asset from a ROM image.
///
/// Returns a JS object discriminated by `kind`:
/// - `{ kind: "image", width, height, rgba: Uint8Array }`
/// - `{ kind: "wav", sample_rate, wav: Uint8Array }`
/// - `{ kind: "bytes", data: Uint8Array, format_id }`
/// - `{ kind: "text", text }`
#[wasm_bindgen]
pub fn preview_asset(
    rom: &[u8],
    map_json: &str,
    entry_id: &str,
    palette_override: Option<Vec<u8>>,
    tiles_per_row: Option<u32>,
    sample_rate: Option<u32>,
) -> Result<JsValue, JsValue> {
    let preview = preview_asset_impl(
        rom,
        map_json,
        entry_id,
        palette_override.as_deref(),
        tiles_per_row,
        sample_rate,
    )
    .map_err(|e| JsValue::from_str(&e))?;

    let obj = Object::new();
    match preview {
        PreviewOutput::Image {
            width,
            height,
            rgba,
        } => {
            set_prop(&obj, "kind", &JsValue::from_str("image"))?;
            set_prop(&obj, "width", &JsValue::from_f64(f64::from(width)))?;
            set_prop(&obj, "height", &JsValue::from_f64(f64::from(height)))?;
            set_prop(&obj, "rgba", &Uint8Array::from(rgba.as_slice()).into())?;
        }
        PreviewOutput::Wav { sample_rate, wav } => {
            set_prop(&obj, "kind", &JsValue::from_str("wav"))?;
            set_prop(
                &obj,
                "sample_rate",
                &JsValue::from_f64(f64::from(sample_rate)),
            )?;
            set_prop(&obj, "wav", &Uint8Array::from(wav.as_slice()).into())?;
        }
        PreviewOutput::Bytes { data, format_id } => {
            set_prop(&obj, "kind", &JsValue::from_str("bytes"))?;
            set_prop(&obj, "data", &Uint8Array::from(data.as_slice()).into())?;
            set_prop(&obj, "format_id", &JsValue::from_str(&format_id))?;
        }
        PreviewOutput::Text { text } => {
            set_prop(&obj, "kind", &JsValue::from_str("text"))?;
            set_prop(&obj, "text", &JsValue::from_str(&text))?;
        }
    }
    Ok(obj.into())
}

/// Replace a mapped asset's decoded payload (recompressing if needed and
/// updating the ROM footer checksum) and return `{ patched_rom, ips_patch }`.
#[wasm_bindgen]
pub fn replace_asset_and_diff(
    rom: &[u8],
    map_json: &str,
    entry_id: &str,
    new_bytes: &[u8],
) -> Result<JsValue, JsValue> {
    let out = replace_asset_and_diff_impl(rom, map_json, entry_id, new_bytes)
        .map_err(|e| JsValue::from_str(&e))?;
    let obj = Object::new();
    set_prop(
        &obj,
        "patched_rom",
        &Uint8Array::from(out.patched_rom.as_slice()).into(),
    )?;
    set_prop(
        &obj,
        "ips_patch",
        &Uint8Array::from(out.ips_patch.as_slice()).into(),
    )?;
    Ok(obj.into())
}

/// Apply an IPS patch to a ROM image and return the patched bytes.
#[wasm_bindgen]
pub fn apply_ips(rom: &[u8], ips: &[u8]) -> Result<Vec<u8>, JsValue> {
    apply_ips_impl(rom, ips).map_err(|e| JsValue::from_str(&e))
}

/// Return a JSON array describing all bundled WonderSwan save definitions.
#[wasm_bindgen]
pub fn list_save_defs() -> String {
    list_save_defs_impl()
}

/// Inspect a WonderSwan save image (auto-detecting its definition when
/// `title_id` is omitted) and return validity, decoded tree, and field metadata.
#[wasm_bindgen]
pub fn inspect_save(save: &[u8], title_id: Option<String>) -> Result<String, JsValue> {
    inspect_save_impl(save, title_id.as_deref()).map_err(|e| JsValue::from_str(&e))
}

/// Apply typed edits to a WonderSwan save image, synchronize any declared
/// `copy_groups`, repair checksums/mirrors, and return the patched save bytes.
#[wasm_bindgen]
pub fn edit_save(save: &[u8], title_id: &str, edits_json: &str) -> Result<Vec<u8>, JsValue> {
    edit_save_impl(save, title_id, edits_json).map_err(|e| JsValue::from_str(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ws_assets::fix_checksum;

    fn synthetic_rom() -> Vec<u8> {
        let mut rom = vec![0u8; ws_rom::MIN_ROM_SIZE];
        let footer = rom.len() - ws_rom::FOOTER_SIZE;
        rom[footer] = 0xea; // far jump opcode
        rom[footer + 1..footer + 3].copy_from_slice(&0x0000u16.to_le_bytes());
        rom[footer + 3..footer + 5].copy_from_slice(&0xf000u16.to_le_bytes());
        rom[footer + 5] = 0x00; // maintenance
        rom[footer + 6] = 0x01; // publisher: Bandai
        rom[footer + 7] = 0x01; // color
        rom[footer + 8] = 0x42; // game_id
        rom[footer + 9] = 0x00; // version
        rom[footer + 10] = 0x00; // rom size code
        rom[footer + 11] = 0x01; // save code: SRAM 32 KiB
        rom[footer + 12] = 0x04; // horizontal, 16-bit bus
        rom[footer + 13] = 0x01; // Bandai2003 mapper

        // Synthetic 2bpp planar tile at 0x0100 (16 bytes)
        for i in 0..16 {
            rom[0x0100 + i] = (i as u8).wrapping_mul(17);
        }
        // Synthetic RGB444 palette at 0x0200 (32 bytes = 16 colors)
        for c in 0..16usize {
            let word = ((c as u16) << 8) | ((15 - c as u16) << 4) | (c as u16);
            rom[0x0200 + c * 2..0x0200 + c * 2 + 2].copy_from_slice(&word.to_le_bytes());
        }
        // Synthetic unsigned 8-bit PCM at 0x0300 (64 bytes)
        for i in 0..64 {
            rom[0x0300 + i] = ((i * 4) & 0xff) as u8;
        }
        // Synthetic ASCII text at 0x0400 ("HI" + 0x00 terminator)
        rom[0x0400..0x0403].copy_from_slice(b"HI\0");
        // Synthetic raw data table at 0x0500 (8 bytes)
        rom[0x0500..0x0508].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);

        fix_checksum(&mut rom).expect("fix checksum");
        rom
    }

    fn synthetic_asset_map(rom: &[u8]) -> String {
        let sha = sha256_hex(rom);
        json!({
            "schema": "swankit-asset-map/1",
            "rom": {
                "sha256": sha,
                "size": rom.len()
            },
            "charmaps": {
                "ascii": {
                    "inline": {
                        "schema": "swankit-charmap/1",
                        "entries": [
                            { "bytes": "48", "text": "H" },
                            { "bytes": "49", "text": "I" }
                        ],
                        "controls": [
                            { "code": 0, "ends": true }
                        ]
                    }
                }
            },
            "assets": [
                {
                    "id": "tiles.sample",
                    "kind": "tiles",
                    "offset": "0x0100",
                    "length": 16,
                    "format": { "bpp": 2, "layout": "planar" },
                    "relations": { "palette": "pal.sample" }
                },
                {
                    "id": "pal.sample",
                    "kind": "palette",
                    "offset": "0x0200",
                    "length": 32,
                    "format": { "palette_format": "rgb444" }
                },
                {
                    "id": "pcm.sample",
                    "kind": "pcm",
                    "offset": "0x0300",
                    "length": 64,
                    "format": { "sample_rate": 8000, "sample_format": "u8" }
                },
                {
                    "id": "text.sample",
                    "kind": "text",
                    "offset": "0x0400",
                    "length": 3,
                    "relations": { "charmap": "ascii" }
                },
                {
                    "id": "table.sample",
                    "kind": "data-table",
                    "offset": "0x0500",
                    "length": 8
                }
            ]
        })
        .to_string()
    }

    #[test]
    fn identify_rom_parses_header_and_sha256() {
        let rom = synthetic_rom();
        let out: Value = serde_json::from_str(&identify_rom_impl(&rom).expect("identify")).unwrap();
        assert_eq!(out["sha256"], sha256_hex(&rom));
        assert_eq!(out["size"], ws_rom::MIN_ROM_SIZE);
        assert_eq!(out["publisher"], "Bandai");
        assert_eq!(out["mapper"], "Bandai2003");
        assert_eq!(out["color"], true);
        assert_eq!(out["system"], "color");
        assert_eq!(out["orientation"], "horizontal");
        assert_eq!(out["save_bytes"], 32_768);
        assert_eq!(out["checksum_ok"], true);
    }

    #[test]
    fn verify_asset_map_reports_ok_and_per_entry_status() {
        let rom = synthetic_rom();
        let map = synthetic_asset_map(&rom);
        let out: Value =
            serde_json::from_str(&verify_asset_map_impl(&rom, &map).expect("verify")).unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["assets"], 5);
        assert_eq!(out["error_count"], 0);
        let entries = out["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 5);
        assert!(entries.iter().all(|e| e["ok"] == true));
    }

    #[test]
    fn preview_asset_decodes_image_wav_text_and_bytes() {
        let rom = synthetic_rom();
        let map = synthetic_asset_map(&rom);

        // Tiles with custom tiles_per_row and RGB444 palette override
        let mut custom_pal = vec![0u8; 32];
        custom_pal[0..2].copy_from_slice(&0x0f00u16.to_le_bytes()); // red
        let tiles = preview_asset_impl(&rom, &map, "tiles.sample", Some(&custom_pal), Some(1), None)
            .expect("tiles preview");
        match tiles {
            PreviewOutput::Image {
                width,
                height,
                rgba,
            } => {
                assert_eq!((width, height), (8, 8));
                assert_eq!(rgba.len(), 8 * 8 * 4);
            }
            other => panic!("expected image, got {other:?}"),
        }

        // Palette swatch
        let pal = preview_asset_impl(&rom, &map, "pal.sample", None, None, None).expect("pal");
        match pal {
            PreviewOutput::Image {
                width,
                height,
                rgba,
            } => {
                assert_eq!((width, height), (128, 8));
                assert_eq!(rgba.len(), 128 * 8 * 4);
            }
            other => panic!("expected palette image, got {other:?}"),
        }

        // PCM -> WAV with sample_rate override
        let wav =
            preview_asset_impl(&rom, &map, "pcm.sample", None, None, Some(16_000)).expect("wav");
        match wav {
            PreviewOutput::Wav { sample_rate, wav } => {
                assert_eq!(sample_rate, 16_000);
                assert_eq!(&wav[0..4], b"RIFF");
                assert_eq!(&wav[8..12], b"WAVE");
            }
            other => panic!("expected wav, got {other:?}"),
        }

        // Text
        let text = preview_asset_impl(&rom, &map, "text.sample", None, None, None).expect("text");
        assert_eq!(
            text,
            PreviewOutput::Text {
                text: "HI".to_owned()
            }
        );

        // Raw table bytes
        let raw = preview_asset_impl(&rom, &map, "table.sample", None, None, None).expect("bytes");
        match raw {
            PreviewOutput::Bytes { data, format_id } => {
                assert_eq!(data, vec![1, 2, 3, 4, 5, 6, 7, 8]);
                assert_eq!(format_id, "ws.data.table");
            }
            other => panic!("expected bytes, got {other:?}"),
        }
    }

    #[test]
    fn replace_asset_and_ips_round_trip() {
        let rom = synthetic_rom();
        let map = synthetic_asset_map(&rom);
        let replacement = [9u8, 8, 7, 6, 5, 4, 3, 2];

        let diff = replace_asset_and_diff_impl(&rom, &map, "table.sample", &replacement)
            .expect("replace and diff");
        assert_eq!(&diff.patched_rom[0x0500..0x0508], &replacement);

        // Footer checksum must remain valid after replacement
        let id_after: Value =
            serde_json::from_str(&identify_rom_impl(&diff.patched_rom).expect("id")).unwrap();
        assert_eq!(id_after["checksum_ok"], true);

        // Applying the generated IPS patch reproduces patched_rom byte-for-byte
        let applied = apply_ips_impl(&rom, &diff.ips_patch).expect("apply ips");
        assert_eq!(applied, diff.patched_rom);
    }

    #[test]
    fn formatkit_v1_and_rle_pcm_00_preview_work() {
        let mut rom = synthetic_rom();
        let pcm_samples = vec![0x80u8; 32];
        let encoded_rle = ws_compress::rle_pcm_00::encode(&pcm_samples).expect("encode rle-pcm-00");
        rom[0x0600..0x0600 + encoded_rle.len()].copy_from_slice(&encoded_rle);
        // Signed 8-bit PCM at 0x0700
        rom[0x0700..0x0720].copy_from_slice(&[0x7fu8; 32]);
        fix_checksum(&mut rom).expect("fix checksum");

        let sha = sha256_hex(&rom);
        let fk_map = json!({
            "schema": "formatkit.asset-map.v1",
            "schema_version": 1,
            "kit": "swankit",
            "media": {
                "kind": "flat-rom",
                "fingerprint": { "rom_sha256": sha },
                "total_bytes": rom.len()
            },
            "entries": [
                {
                    "id": "pcm.rle",
                    "offset": 0x0600,
                    "size": encoded_rle.len(),
                    "status": "mapped",
                    "format_id": "ws.codec.rle-pcm-00",
                    "mapping": { "kind": "compressed" }
                },
                {
                    "id": "pcm.signed",
                    "offset": 0x0700,
                    "size": 32,
                    "status": "mapped",
                    "format_id": "ws.pcm.s8",
                    "mapping": {
                        "kind": "raw-audio",
                        "params": { "format": { "sample_rate": 12000 } }
                    }
                }
            ]
        })
        .to_string();

        let verified: Value =
            serde_json::from_str(&verify_asset_map_impl(&rom, &fk_map).expect("verify fk"))
                .unwrap();
        assert_eq!(verified["ok"], true);
        assert_eq!(verified["schema"], "formatkit.asset-map.v1");

        let rle_wav =
            preview_asset_impl(&rom, &fk_map, "pcm.rle", None, None, Some(11_025)).expect("rle");
        match rle_wav {
            PreviewOutput::Wav { sample_rate, wav } => {
                assert_eq!(sample_rate, 11_025);
                assert_eq!(&wav[0..4], b"RIFF");
            }
            other => panic!("expected wav for rle-pcm-00, got {other:?}"),
        }

        let s8_wav =
            preview_asset_impl(&rom, &fk_map, "pcm.signed", None, None, None).expect("s8");
        match s8_wav {
            PreviewOutput::Wav { sample_rate, wav } => {
                assert_eq!(sample_rate, 12_000);
                assert_eq!(&wav[0..4], b"RIFF");
            }
            other => panic!("expected wav for pcm.s8, got {other:?}"),
        }
    }

    #[test]
    fn list_save_defs_contains_all_bundled_definitions() {
        let defs: Value = serde_json::from_str(&list_save_defs_impl()).expect("json");
        let arr = defs.as_array().expect("array");
        assert_eq!(arr.len(), 142);
        assert!(arr.iter().all(|d| {
            d["id"].is_string()
                && d["title"].is_string()
                && d["media"].is_string()
                && d["size"].is_u64()
                && d["field_count"].is_u64()
        }));
    }

    #[test]
    fn inspect_and_edit_save_repairs_checksums_and_copy_groups() {
        // Pick the first bundled definition that has both a scalar field and a copy group
        let formats = ws_saves::formats().expect("formats");
        let fmt = formats
            .iter()
            .find(|f| {
                !f.copy_groups().is_empty()
                    && f.definition()["media"][0]["sizes"][0]
                        .as_u64()
                        .is_some()
            })
            .expect("bundled format with copy_groups");
        let size = fmt.definition()["media"][0]["sizes"][0].as_u64().unwrap() as usize;

        // Build a synthetic image of the declared size and repair it via fix()
        let mut save = vec![0u8; size];
        let ctx = DecodeContext::default();
        // Ensure fix works or test inspect + edit on a format whose fix() succeeds on zeroed buffer
        let (target_fmt, base_save, group_field_idx, new_val) = formats
            .iter()
            .filter(|f| !f.copy_groups().is_empty())
            .find_map(|f| {
                let sz = f.definition()["media"][0]["sizes"][0].as_u64()? as usize;
                let mut buf = vec![0u8; sz];
                let group = f.copy_groups().first()?;
                let field_idx = *group.fields.first()?;
                let field = &f.fields[field_idx];
                let val = match (field.size, field.encoding.as_str()) {
                    (1, "u8") => json!(7),
                    (2, "u16le") => json!(42),
                    (4, "u32le") => json!(1234),
                    _ => return None,
                };
                if f.validate_field_value(field_idx, &val).is_err() {
                    return None;
                }
                if f.fix_with_context(&mut buf, "save.bin", &ctx).is_err() {
                    return None;
                }
                Some((f, buf, field_idx, val))
            })
            .unwrap_or_else(|| {
                let _ = fmt.fix_with_context(&mut save, "save.bin", &ctx);
                let field_idx = fmt.copy_groups()[0].fields[0];
                (fmt, save, field_idx, json!(1))
            });

        // Inspect with explicit title_id
        let inspected: Value = serde_json::from_str(
            &inspect_save_impl(&base_save, Some(&target_fmt.id)).expect("inspect"),
        )
        .unwrap();
        assert_eq!(inspected["id"], target_fmt.id);
        assert_eq!(
            inspected["fields"].as_array().unwrap().len(),
            target_fmt.fields.len()
        );

        // Edit via field index; all copy_group members should be synchronized
        let edits = json!([{ "field": group_field_idx, "value": new_val }]).to_string();
        let patched =
            edit_save_impl(&base_save, &target_fmt.id, &edits).expect("edit save");
        let group = target_fmt.copy_group_for_field(group_field_idx).unwrap();
        for &member_idx in &group.fields {
            let read_back = target_fmt.fields[member_idx]
                .read(&patched)
                .expect("read member");
            assert_eq!(read_back, new_val);
        }
    }
}
