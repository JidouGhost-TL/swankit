//! Replacement by asset id: re-encode with the asset's codec, write in place
//! when the new stored bytes fit, otherwise relocate into declared free space
//! when every reference is known. Nothing is written outside the asset's own
//! range, a declared free range or a declared reference.
use crate::data::{allocation_capacity, decode_asset, encode_payload, validate_reservations};
use crate::gfx::{unrender_tilemap_with, unrender_tiles, Allocation};
use crate::map::{AssetMap, Codec, FreeRange, Kind, Num, TileRange};
use crate::refs::{encode_reference, references_for};
use crate::text::Charmaps;
use crate::{check_identity, malformed, sha256_hex};
use formatkit_core::{Image, Result};
use std::collections::BTreeMap;

/// Replacement content in the asset's decoded (editable) form.
#[derive(Debug, Clone)]
pub enum ReplaceInput {
    /// Tiles sheet, tilemap rendering or palette swatch, as extracted.
    Image(Image),
    /// UTF-8 text in the charmap's cell spelling.
    Text(String),
    /// Decoded payload bytes (any kind).
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct Replacement {
    pub id: String,
    pub input: ReplaceInput,
}

/// Policy switches for [`replace_with`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ReplaceOptions {
    /// Write text that breaks its declared line limits, reporting warnings.
    pub force: bool,
    /// Give differently edited cells of a shared tile new tiles from the
    /// declared free tile space and rewrite those tilemap cells.
    pub allocate: bool,
}

#[derive(Debug)]
pub struct ReplaceOutcome {
    pub rom: Vec<u8>,
    /// The map rewritten for the patched ROM (identity, moved offsets, free
    /// space), with file-backed character maps inlined for portable reuse.
    pub map: AssetMap,
    /// Human-readable actions taken.
    pub actions: Vec<String>,
    /// Pixels mapped to the nearest palette colour rather than an exact one.
    pub approximated_pixels: usize,
    /// Limits broken under `force`.
    pub warnings: Vec<String>,
}

/// Recompute the footer checksum (sum of all bytes but the last two).
pub fn fix_checksum(rom: &mut [u8]) -> Result<u16> {
    if rom.len() < ws_rom::MIN_ROM_SIZE {
        return Err(malformed("ROM too small for a footer"));
    }
    let n = rom.len() - 2;
    let sum = formatkit_core::checksum::sum_bytes_u16(&rom[..n], 0);
    rom[n..].copy_from_slice(&sum.to_le_bytes());
    Ok(sum)
}

fn palette_from_swatch(image: &Image, old: &[u8]) -> Result<Vec<u8>> {
    let rows = old.len() / 32;
    if image.width != 128 || image.height as usize != rows * 8 {
        return Err(malformed(format!(
            "palette image must be 128x{} like the extracted swatch",
            rows * 8
        )));
    }
    let mut out = Vec::with_capacity(old.len());
    for r in 0..rows {
        for c in 0..16 {
            let at = ((r * 8 + 4) * 128 + c * 8 + 4) * 4;
            let px = &image.rgba[at..at + 3];
            let q = |v: u8| ((u16::from(v) + 8) / 17).min(15);
            let word = (q(px[0]) << 8) | (q(px[1]) << 4) | q(px[2]);
            let old_word = u16::from_le_bytes([old[(r * 16 + c) * 2], old[(r * 16 + c) * 2 + 1]]);
            out.extend_from_slice(&((old_word & 0xf000) | word).to_le_bytes());
        }
    }
    Ok(out)
}

/// New decoded payloads keyed by asset id.
fn payloads(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    r: &Replacement,
    options: &ReplaceOptions,
    allocated: &mut Vec<Allocation>,
) -> Result<(BTreeMap<String, Vec<u8>>, usize)> {
    let asset = map.asset(&r.id)?;
    if asset.codec == Codec::Lzs2 {
        return Err(malformed(format!(
            "{}: LZS2 assets are decode-only",
            asset.id
        )));
    }
    Ok(match (&r.input, asset.kind) {
        (ReplaceInput::Image(img), Kind::Font) => {
            crate::font::unrender_font(map, charmaps, rom, &r.id, img)?
        }
        (ReplaceInput::Image(img), Kind::Tiles) => unrender_tiles(map, charmaps, rom, &r.id, img)?,
        (ReplaceInput::Image(img), Kind::Tilemap) => {
            let edits = unrender_tilemap_with(map, charmaps, rom, &r.id, img, options.allocate)?;
            *allocated = edits.allocated;
            (edits.payloads, edits.approximated)
        }
        (ReplaceInput::Image(img), Kind::Palette) => {
            if asset.format.palette_format.as_deref().unwrap_or("rgb444") != "rgb444" {
                return Err(malformed(format!(
                    "{}: only rgb444 palettes take an image",
                    r.id
                )));
            }
            let old = decode_asset(map, charmaps, rom, &r.id)?.decoded;
            (
                BTreeMap::from([(r.id.clone(), palette_from_swatch(img, &old)?)]),
                0,
            )
        }
        (ReplaceInput::Text(text), Kind::Text) => {
            let old = decode_asset(map, charmaps, rom, &r.id)?.decoded;
            let bytes = charmaps.encode_edit(asset, &old, text)?;
            (BTreeMap::from([(r.id.clone(), bytes)]), 0)
        }
        (ReplaceInput::Bytes(bytes), _) => (BTreeMap::from([(r.id.clone(), bytes.clone())]), 0),
        (_, kind) => {
            return Err(malformed(format!(
                "{}: this input type cannot replace a {} asset",
                r.id,
                kind.name()
            )))
        }
    })
}

/// Remove one tile index from a tiles asset's declared free tiles.
fn take_free_tile(map: &mut AssetMap, id: &str, index: usize) -> Result<()> {
    let asset = map
        .assets
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| malformed(format!("unknown asset id {id:?}")))?;
    let mut ranges = Vec::new();
    for r in asset.format.free_tiles.take().unwrap_or_default() {
        let (first, count) = (r.first.usize()?, r.count.usize()?);
        if (first..first + count).contains(&index) {
            for (f, c) in [
                (first, index - first),
                (index + 1, first + count - index - 1),
            ] {
                if c > 0 {
                    ranges.push(TileRange {
                        first: Num(f as u64),
                        count: Num(c as u64),
                    });
                }
            }
        } else {
            ranges.push(r);
        }
    }
    asset.format.free_tiles = (!ranges.is_empty()).then_some(ranges);
    Ok(())
}

fn bank(offset: usize) -> usize {
    offset >> 16
}

/// Apply one replacement with default options and fix the checksum.
pub fn replace(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    replacement: &Replacement,
) -> Result<ReplaceOutcome> {
    replace_with(map, charmaps, rom, replacement, &ReplaceOptions::default())
}

/// Apply one replacement and fix the checksum.
pub fn replace_with(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    replacement: &Replacement,
    options: &ReplaceOptions,
) -> Result<ReplaceOutcome> {
    check_identity(map, rom)?;
    validate_reservations(map, charmaps, rom)?;
    let mut allocated = Vec::new();
    let (decoded, approximated_pixels) =
        payloads(map, charmaps, rom, replacement, options, &mut allocated)?;
    let mut warnings = Vec::new();
    for (id, payload) in &decoded {
        let asset = map.asset(id)?;
        if asset.kind == Kind::Text {
            let broken = charmaps.limit_violations(map, asset, payload)?;
            if !broken.is_empty() && !options.force {
                return Err(malformed(format!(
                    "{}; refused (shorten or break the line, or force)",
                    broken.join("; ")
                )));
            }
            warnings.extend(broken);
        }
    }
    let mut out = rom.to_vec();
    let mut new_map = map.clone();
    let mut actions = Vec::new();
    for (tiles, index, slot, cells) in &allocated {
        take_free_tile(&mut new_map, tiles, *index)?;
        actions.push(format!(
            "{tiles}: free tile {index} (VRAM slot {slot}) allocated for {} cells {cells:?}",
            replacement.id
        ));
    }
    for (id, payload) in &decoded {
        let asset = map.asset(id)?;
        let old = decode_asset(map, charmaps, rom, id)?;
        if *payload == old.decoded {
            actions.push(format!("{id}: unchanged"));
            continue;
        }
        let stored = encode_payload(asset, payload)?;
        let capacity = allocation_capacity(asset, old.offset, old.length, rom.len())?;
        if stored.len() <= capacity {
            out[old.offset..old.offset + stored.len()].copy_from_slice(&stored);
            if let Some(pad) = asset.format.pad {
                let pad = u8::try_from(pad.0).map_err(|_| malformed("pad must be a byte"))?;
                out[old.offset + stored.len()..old.offset + capacity].fill(pad);
            }
            // A self-delimited stream's declared length is its consumed extent.
            if let Some(a) = new_map.assets.iter_mut().find(|a| a.id == *id) {
                if asset.reserved_capacity.is_some()
                    || (a.length.is_some()
                        && (asset.codec.terminated() || asset.codec == Codec::Lzss12))
                {
                    a.length = Some(Num(stored.len() as u64));
                }
                if capacity > stored.len()
                    && (asset.reserved_capacity.is_some()
                        || asset.codec.terminated()
                        || asset.codec == Codec::Lzss12)
                {
                    // The verified original stored extent is an allocation
                    // proof even when its length was inferred from a terminator.
                    a.reserved_capacity = Some(Num(capacity as u64));
                }
            }
            actions.push(format!(
                "{id}: in place at {:#x}, {} of {} stored bytes",
                old.offset,
                stored.len(),
                capacity
            ));
            continue;
        }
        // Relocation: every pointer must be known and expressible at the target.
        let refs = references_for(map, id)?;
        if !asset.references_complete || refs.is_empty() {
            return Err(malformed(format!(
                "{id}: needs {} stored bytes but has {}; relocation requires references_complete \
                 with every reference listed",
                stored.len(),
                capacity
            )));
        }
        let crosses_before = bank(old.offset) != bank(old.offset + old.length - 1);
        let mut chosen = None;
        'ranges: for (ri, free) in new_map.free_space.iter().enumerate() {
            let (start, len) = (free.offset.usize()?, free.length.usize()?);
            let mut at = start;
            while at + stored.len() <= start + len {
                let crosses = bank(at) != bank(at + stored.len() - 1);
                let fits = (!crosses || crosses_before)
                    && refs
                        .iter()
                        .all(|r| encode_reference(&r.encoding, at, old.offset, rom.len()).is_ok());
                if fits {
                    chosen = Some((ri, at));
                    break 'ranges;
                }
                // Advance to the next alignment or bank that could satisfy the references.
                let align = refs
                    .iter()
                    .map(|r| r.encoding.align.map_or(1, |a| a.0.max(1) as usize))
                    .max()
                    .unwrap_or(1);
                at = if crosses {
                    (bank(at) + 1) << 16
                } else {
                    (at / align + 1) * align
                };
            }
        }
        let (ri, at) = chosen.ok_or_else(|| {
            malformed(format!(
                "{id}: no declared free range holds {} bytes within the references' bank/segment limits",
                stored.len()
            ))
        })?;
        out[at..at + stored.len()].copy_from_slice(&stored);
        for r in &refs {
            let bytes = encode_reference(&r.encoding, at, old.offset, rom.len())?;
            let end = r.at + bytes.len();
            if end > out.len() {
                return Err(malformed(format!(
                    "{id}: reference at {:#x} beyond ROM",
                    r.at
                )));
            }
            out[r.at..end].copy_from_slice(&bytes);
        }
        // Shrink the used free range; the old slot becomes free space.
        let free = new_map.free_space.remove(ri);
        let (fs, fl) = (free.offset.usize()?, free.length.usize()?);
        if at > fs {
            new_map.free_space.push(FreeRange {
                offset: Num(fs as u64),
                length: Num((at - fs) as u64),
                fill: free.fill,
                note: free.note.clone(),
            });
        }
        let tail = at + stored.len();
        if tail < fs + fl {
            new_map.free_space.push(FreeRange {
                offset: Num(tail as u64),
                length: Num((fs + fl - tail) as u64),
                fill: free.fill,
                note: free.note.clone(),
            });
        }
        new_map.free_space.push(FreeRange {
            offset: Num(old.offset as u64),
            length: Num(old.length as u64),
            fill: None,
            note: Some(format!("released by relocating {id}")),
        });
        new_map.free_space.sort_by_key(|f| f.offset);
        if let Some(a) = new_map.assets.iter_mut().find(|a| a.id == *id) {
            a.offset = Num(at as u64);
            // The new allocation proves only the encoded extent. Old spare
            // capacity belongs to the old location and never follows a pointer.
            a.reserved_capacity = None;
            if a.length.is_some() {
                a.length = Some(Num(stored.len() as u64));
            }
        }
        actions.push(format!(
            "{id}: relocated {:#x} -> {at:#x} ({} -> {} stored bytes), {} references rewritten",
            old.offset,
            old.length,
            stored.len(),
            refs.len()
        ));
    }
    let sum = fix_checksum(&mut out)?;
    actions.push(format!("footer checksum {sum:#06x}"));
    new_map.rom.sha256 = sha256_hex(&out);
    validate_reservations(&new_map, charmaps, &out)?;
    charmaps.inline_files(&mut new_map)?;
    Ok(ReplaceOutcome {
        rom: out,
        map: new_map,
        actions,
        approximated_pixels,
        warnings,
    })
}
