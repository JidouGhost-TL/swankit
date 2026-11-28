//! Extraction: every mapped asset to a file, plus an index.
use crate::data::decode_asset;
use crate::gfx::{
    palette_colors, render_screen, render_sprites, render_tilemap, render_tiles, MAX_IMAGE_BYTES,
};
use crate::map::{Asset, AssetMap, Codec, Kind};
use crate::refs::{read_reference, references_for};
use crate::text::Charmaps;
use crate::{check_identity, malformed, sha256_hex};
use formatkit_core::{Image, Result};
use serde_json::{json, Value};
use ws_text::{FontDefinition, FontFormat, GlyphOrder};

/// File content: images are encoded by the caller (PNG at the CLI edge).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractedFile {
    Image(Image),
    Bytes(Vec<u8>),
}

#[derive(Debug)]
pub struct Extraction {
    /// Relative file names and contents.
    pub files: Vec<(String, ExtractedFile)>,
    /// `index.json` content.
    pub index: Value,
    /// Assets or screens that failed; their errors are also in the index.
    pub failures: usize,
}

fn swatch(set: &[Option<[[u8; 4]; 16]>]) -> Result<Image> {
    let rows: Vec<_> = set.iter().flatten().collect();
    if rows.is_empty() {
        return Err(malformed("palette has no colours"));
    }
    let (w, h) = (16 * 8, rows.len() * 8);
    let mut rgba = Image::try_rgba_buffer(w as u32, h as u32, MAX_IMAGE_BYTES, "swatch")?;
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&rows[y / 8][x / 8]);
        }
    }
    Image::try_new(w as u32, h as u32, rgba)
}

pub(crate) fn font_definition(asset: &Asset) -> Result<FontDefinition> {
    let f = &asset.format;
    let format = match (f.bpp.unwrap_or(1), f.layout.as_deref().unwrap_or("planar")) {
        (1, _) => FontFormat::Bpp1,
        (2, _) => FontFormat::Bpp2,
        (4, "planar") => FontFormat::Bpp4Planar,
        (4, "packed") => FontFormat::Bpp4Packed,
        _ => return Err(malformed(format!("{}: font bpp", asset.id))),
    };
    let order = match f.order.as_deref().unwrap_or("row") {
        "row" => GlyphOrder::Row,
        "column" => GlyphOrder::Column,
        other => return Err(malformed(format!("{}: glyph order {other:?}", asset.id))),
    };
    let tiles_wide = f.tiles_wide.unwrap_or(1);
    let tiles_high = f.tiles_high.unwrap_or(1);
    let tile_bytes = match format {
        FontFormat::Bpp1 => 8,
        FontFormat::Bpp2 => 16,
        _ => 32,
    };
    let natural = tile_bytes * (tiles_wide * tiles_high) as usize;
    Ok(FontDefinition {
        offset: 0,
        count: f
            .count
            .ok_or_else(|| malformed(format!("{}: font count", asset.id)))?
            .usize()?,
        stride: f.stride.map_or(Ok(natural), |s| s.usize())?,
        format,
        tiles_wide,
        tiles_high,
        order,
        tile_offsets: f
            .tile_offsets
            .iter()
            .flatten()
            .map(|n| n.usize())
            .collect::<Result<_>>()?,
        widths: vec![],
        width_table: None,
    })
}

pub(crate) fn font_palette(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    asset: &Asset,
) -> Result<Vec<[u8; 4]>> {
    let def = font_definition(asset)?;
    let colors = match def.format {
        FontFormat::Bpp1 => 2,
        FontFormat::Bpp2 => 4,
        _ => 16,
    };
    let palette: Vec<[u8; 4]> = match asset.relations.palette.as_deref() {
        Some(name) => {
            let index = asset.format.palette_index.unwrap_or_default().usize()?;
            palette_colors(map, charmaps, rom, &[name])?
                .get(index)
                .copied()
                .flatten()
                .ok_or_else(|| malformed(format!("{}: preview palette missing", asset.id)))?
                .to_vec()
        }
        None => (0..16)
            .map(|i| {
                let g = 255 - (i.min(colors - 1) * 255 / (colors - 1)) as u8;
                [g, g, g, 255]
            })
            .collect(),
    };
    Ok(palette)
}

pub(crate) fn font_sheet(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    asset: &Asset,
) -> Result<Image> {
    let def = font_definition(asset)?;
    let data = decode_asset(map, charmaps, rom, &asset.id)?.decoded;
    let palette = font_palette(map, charmaps, rom, asset)?;
    let columns = asset.format.width.map_or(Ok(16), |w| w.usize())?.max(1);
    let (gw, gh) = (def.tiles_wide as usize * 8, def.tiles_high as usize * 8);
    let rows = def.count.div_ceil(columns);
    let (w, h) = (columns * gw, rows * gh);
    let mut rgba = Image::try_rgba_buffer(w as u32, h as u32, MAX_IMAGE_BYTES, "font sheet")?;
    rgba.resize(w * h * 4, 0);
    for g in 0..def.count {
        let glyph = def.glyph(&data, g, &palette, MAX_IMAGE_BYTES)?.image;
        let (ox, oy) = (g % columns * gw, g / columns * gh);
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
        .map_err(|e| malformed(e.to_string()))?;
    }
    Image::try_new(w as u32, h as u32, rgba)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn extract_one(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    asset: &Asset,
    files: &mut Vec<(String, ExtractedFile)>,
) -> Result<Value> {
    let stored = decode_asset(map, charmaps, rom, &asset.id)?;
    let id = &asset.id;
    let mut names = Vec::new();
    let mut add = |name: String, file: ExtractedFile, files: &mut Vec<_>| {
        names.push(name.clone());
        files.push((name, file));
    };
    let mut extra = json!({});
    match asset.kind {
        Kind::Tiles => add(
            format!("{id}.png"),
            ExtractedFile::Image(render_tiles(map, charmaps, rom, id)?),
            files,
        ),
        Kind::Tilemap if asset.relations.tiles.is_empty() => {
            // No tile relation declared: the words are extracted, not rendered.
            extra = json!({"rendered": false, "reason": "no tiles relation"});
            add(
                format!("{id}.bin"),
                ExtractedFile::Bytes(stored.decoded.clone()),
                files,
            );
        }
        Kind::Tilemap => add(
            format!("{id}.png"),
            ExtractedFile::Image(render_tilemap(map, charmaps, rom, id)?),
            files,
        ),
        Kind::SpriteTable if asset.relations.tiles.is_empty() => {
            extra = json!({"rendered": false, "reason": "no tiles relation"});
            add(
                format!("{id}.bin"),
                ExtractedFile::Bytes(stored.decoded.clone()),
                files,
            );
        }
        Kind::SpriteTable => {
            add(
                format!("{id}.png"),
                ExtractedFile::Image(render_sprites(map, charmaps, rom, id)?),
                files,
            );
            add(
                format!("{id}.bin"),
                ExtractedFile::Bytes(stored.decoded.clone()),
                files,
            );
        }
        Kind::Palette => {
            let set = palette_colors(map, charmaps, rom, &[id])?;
            add(
                format!("{id}.png"),
                ExtractedFile::Image(swatch(&set)?),
                files,
            );
        }
        Kind::Font => add(
            format!("{id}.png"),
            ExtractedFile::Image(font_sheet(map, charmaps, rom, asset)?),
            files,
        ),
        Kind::Text => {
            let decoded = charmaps.decode(asset, &stored.decoded)?;
            let mut codes = String::new();
            for t in &decoded.tokens {
                codes.push_str(&format!(
                    "{}\t{}\n",
                    hex(&t.bytes),
                    serde_json::to_string(&t.text).unwrap_or_default()
                ));
            }
            extra = json!({"text": decoded.text, "terminated": decoded.terminated, "consumed": decoded.consumed});
            add(
                format!("{id}.txt"),
                ExtractedFile::Bytes(format!("{}\n", decoded.text).into_bytes()),
                files,
            );
            add(
                format!("{id}.codes.tsv"),
                ExtractedFile::Bytes(codes.into_bytes()),
                files,
            );
        }
        Kind::PointerTable => {
            let mut entries = Vec::new();
            if let Some(enc) = &asset.format.pointer {
                let stride = asset
                    .format
                    .stride
                    .map_or(Ok(usize::from(enc.width)), |s| s.usize())?;
                let inner = asset.format.pointer_offset.unwrap_or_default().usize()?;
                let count = asset
                    .format
                    .count
                    .map_or(Ok(stored.length / stride.max(1)), |c| c.usize())?;
                for i in 0..count {
                    let at = stored.offset + i * stride + inner;
                    let value = read_reference(rom, at, enc)?;
                    let target = asset.relations.targets.get(i).cloned().flatten();
                    entries.push(json!({"index": i, "at": at, "value": format!("0x{value:x}"), "target": target}));
                }
            }
            add(
                format!("{id}.json"),
                ExtractedFile::Bytes(
                    serde_json::to_vec_pretty(&entries).map_err(|e| malformed(e.to_string()))?,
                ),
                files,
            );
            add(
                format!("{id}.bin"),
                ExtractedFile::Bytes(stored.decoded.clone()),
                files,
            );
        }
        Kind::Pcm => {
            let raw_header = rom.get(stored.offset..stored.offset.saturating_add(2)).unwrap_or(&[]);
            let header_rate = match asset.codec {
                Codec::Dpcm4bitSm => ws_compress::dpcm_4bit_sm::header_sample_rate(raw_header),
                Codec::Wave4bitPacked => ws_compress::wave_4bit_packed::header_sample_rate(raw_header),
                _ => None,
            };
            let rate = asset
                .format
                .sample_rate
                .or(header_rate)
                .ok_or_else(|| malformed(format!("{id}: pcm sample_rate")))?;
            let pcm = match asset.format.sample_format.as_deref().unwrap_or("u8") {
                "u8" => ws_sound::render::unsigned_pcm(&stored.decoded, rate, stored.decoded.len())?,
                "s8" => ws_sound::render::signed_pcm(&stored.decoded, rate, stored.decoded.len())?,
                other => {
                    return Err(malformed(format!(
                        "{id}: unsupported pcm sample_format {other:?}"
                    )))
                }
            };
            add(
                format!("{id}.wav"),
                ExtractedFile::Bytes(pcm.to_wav_bytes()?),
                files,
            );
        }
        Kind::Music | Kind::SoundEffect | Kind::DataTable | Kind::Other => {
            add(
                format!("{id}.bin"),
                ExtractedFile::Bytes(stored.decoded.clone()),
                files,
            );
        }
    }
    if asset.codec != Codec::Raw
        && matches!(
            asset.kind,
            Kind::Tiles | Kind::Tilemap | Kind::Palette | Kind::Font
        )
    {
        add(
            format!("{id}.decoded.bin"),
            ExtractedFile::Bytes(stored.decoded.clone()),
            files,
        );
    }
    let refs = references_for(map, id)?;
    Ok(json!({
        "id": id,
        "kind": asset.kind.name(),
        "offset": stored.offset,
        "stored_length": stored.length,
        "decoded_length": stored.decoded.len(),
        "decoded_sha256": sha256_hex(&stored.decoded),
        "codec": asset.codec,
        "confidence": asset.confidence,
        "references": refs.len(),
        "files": names,
        "details": extra,
    }))
}

/// Extract every asset and screen. Per-asset failures are recorded, not hidden.
pub fn extract(map: &AssetMap, charmaps: &Charmaps, rom: &[u8]) -> Result<Extraction> {
    check_identity(map, rom)?;
    let mut files = Vec::new();
    let mut assets = Vec::new();
    let mut failures = 0;
    for asset in &map.assets {
        match extract_one(map, charmaps, rom, asset, &mut files) {
            Ok(v) => assets.push(v),
            Err(e) => {
                failures += 1;
                assets.push(
                    json!({"id": asset.id, "kind": asset.kind.name(), "error": e.to_string()}),
                );
            }
        }
    }
    let mut screens = Vec::new();
    for screen in &map.screens {
        match render_screen(map, charmaps, rom, &screen.id) {
            Ok(image) => {
                let name = format!("{}.png", screen.id);
                files.push((name.clone(), ExtractedFile::Image(image)));
                screens.push(json!({"id": screen.id, "file": name}));
            }
            Err(e) => {
                failures += 1;
                screens.push(json!({"id": screen.id, "error": e.to_string()}));
            }
        }
    }
    let index = json!({
        "schema": "swankit-asset-extract/1",
        "rom_sha256": map.rom.sha256,
        "rom_size": rom.len(),
        "assets": assets,
        "screens": screens,
        "failures": failures,
    });
    Ok(Extraction {
        files,
        index,
        failures,
    })
}
