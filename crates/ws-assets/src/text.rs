//! Named character maps resolved through `ws-text`.
use crate::malformed;
use crate::map::{Asset, AssetMap, CharmapSource};
use formatkit_core::Result;
use std::collections::BTreeMap;
use ws_text::{
    builtin_definition, CharacterMap, CodeUnit, DecodedText, TextDefinition, TextLimits, TextToken,
};

/// Longest string accepted for a codec-less text asset.
pub const MAX_TEXT_BYTES: usize = 65_536;

/// Character maps of one asset map, compiled once.
#[derive(Debug, Default)]
pub struct Charmaps {
    maps: BTreeMap<String, CharacterMap>,
}

fn unit(name: Option<&str>) -> Result<CodeUnit> {
    match name.unwrap_or("byte") {
        "byte" => Ok(CodeUnit::Byte),
        "word-le" => Ok(CodeUnit::WordLe),
        "word-be" => Ok(CodeUnit::WordBe),
        other => Err(malformed(format!("unknown code unit {other:?}"))),
    }
}

fn compile(
    source: &CharmapSource,
    load: &mut dyn FnMut(&str) -> Result<Vec<u8>>,
) -> Result<CharacterMap> {
    let chosen = [
        source.builtin.is_some(),
        source.file.is_some(),
        source.inline.is_some(),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    if chosen != 1 {
        return Err(malformed(
            "charmap needs exactly one of builtin, file, inline",
        ));
    }
    if let Some(id) = &source.builtin {
        return builtin_definition(id)?.character_map();
    }
    if let Some(value) = &source.inline {
        let bytes = serde_json::to_vec(value).map_err(|e| malformed(e.to_string()))?;
        return CharacterMap::from_json(&bytes);
    }
    let path = source.file.as_deref().unwrap_or_default();
    let bytes = load(path)?;
    if path.ends_with(".tsv") {
        let text =
            std::str::from_utf8(&bytes).map_err(|_| malformed("charmap TSV is not UTF-8"))?;
        return CharacterMap::from_tsv(text, unit(source.unit.as_deref())?);
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| malformed(e.to_string()))?;
    if value.get("schema").and_then(|s| s.as_str()) == Some("swankit-text-def/1") {
        TextDefinition::from_json(&bytes)?.character_map()
    } else {
        CharacterMap::from_json(&bytes)
    }
}

impl Charmaps {
    /// Compile every named map. `load` reads files named relative to the map.
    pub fn load(map: &AssetMap, mut load: impl FnMut(&str) -> Result<Vec<u8>>) -> Result<Self> {
        let mut maps = BTreeMap::new();
        for (name, source) in &map.charmaps {
            let compiled = compile(source, &mut load)
                .map_err(|e| malformed(format!("charmap {name}: {e}")))?;
            maps.insert(name.clone(), compiled);
        }
        Ok(Self { maps })
    }

    /// Make file-backed character maps independent of the map's directory.
    /// The compiled definition preserves TSV units and full text definitions'
    /// character-map semantics in the existing inline representation.
    pub(crate) fn inline_files(&self, map: &mut AssetMap) -> Result<()> {
        for (name, source) in &mut map.charmaps {
            if source.file.is_some() {
                let compiled = self.maps.get(name).ok_or_else(|| {
                    malformed(format!("charmap {name:?} not loaded for returned map"))
                })?;
                source.inline = Some(
                    serde_json::to_value(compiled.definition())
                        .map_err(|e| malformed(e.to_string()))?,
                );
                source.file = None;
                source.unit = None;
            }
        }
        Ok(())
    }

    pub fn for_asset(&self, asset: &Asset) -> Result<&CharacterMap> {
        let name = asset
            .relations
            .charmap
            .as_deref()
            .ok_or_else(|| malformed(format!("{}: text needs relations.charmap", asset.id)))?;
        self.maps
            .get(name)
            .ok_or_else(|| malformed(format!("{}: charmap {name:?} not loaded", asset.id)))
    }

    fn limits() -> TextLimits {
        TextLimits {
            input_bytes: MAX_TEXT_BYTES,
            ..TextLimits::default()
        }
    }

    /// Decode a text payload; a declared length must be consumed up to its end
    /// control (trailing slot padding after the end is allowed).
    pub fn decode(&self, asset: &Asset, bytes: &[u8]) -> Result<DecodedText> {
        let window = &bytes[..bytes.len().min(MAX_TEXT_BYTES)];
        let decoded = self.for_asset(asset)?.decode(window, Self::limits())?;
        if !decoded.terminated && decoded.consumed != bytes.len() {
            return Err(malformed(format!("{}: text not terminated", asset.id)));
        }
        Ok(decoded)
    }

    /// Stored length of an unterminated-by-length text: through its end control.
    pub fn terminated_length(&self, asset: &Asset, window: &[u8]) -> Result<usize> {
        let window = &window[..window.len().min(MAX_TEXT_BYTES)];
        let decoded = self.for_asset(asset)?.decode(window, Self::limits())?;
        if !decoded.terminated {
            return Err(malformed(format!("{}: no end control found", asset.id)));
        }
        Ok(decoded.consumed)
    }

    /// Canonical bytes for replacement text, including the end control.
    pub fn encode(&self, asset: &Asset, text: &str) -> Result<Vec<u8>> {
        self.for_asset(asset)?.encode(text, MAX_TEXT_BYTES)
    }

    /// Bytes for an edited text: `ws_text::CharacterMap::encode_edit` keeps
    /// unchanged tokens and the edited span's hidden attribute bits.
    pub fn encode_edit(&self, asset: &Asset, original: &[u8], text: &str) -> Result<Vec<u8>> {
        let old = self.decode(asset, original)?;
        self.for_asset(asset)?
            .encode_edit(&original[..old.consumed], text, Self::limits())
            .map_err(|e| malformed(format!("{}: {e}", asset.id)))
    }

    /// Displayed lines of a text payload that break its declared limits
    /// (`line_width`, `lines`), measured with the glyph widths of its font.
    pub fn limit_violations(
        &self,
        map: &AssetMap,
        asset: &Asset,
        bytes: &[u8],
    ) -> Result<Vec<String>> {
        let f = &asset.format;
        if f.line_width.is_none() && f.lines.is_none() {
            return Ok(Vec::new());
        }
        let font =
            asset.relations.font.as_deref().ok_or_else(|| {
                malformed(format!("{}: line limits need relations.font", asset.id))
            })?;
        let font = map.asset(font)?;
        let advance = font
            .format
            .advance
            .unwrap_or(font.format.tiles_wide.unwrap_or(1) * 8);
        let widths = font.format.glyph_widths.clone().unwrap_or_default();
        let width = |t: &TextToken| widths.get(&t.text).copied().unwrap_or(advance);
        let decoded = self.decode(asset, bytes)?;
        let lines =
            self.for_asset(asset)?
                .measure_lines(&decoded.tokens, f.page_break.as_deref(), &width);
        let mut out = Vec::new();
        for l in &lines {
            if let Some(max) = f.line_width.filter(|max| l.width > *max) {
                out.push(format!(
                    "{}: page {} line {} is {} pixels wide ({} glyphs), the limit is {max}: {:?}",
                    asset.id,
                    l.page + 1,
                    l.line + 1,
                    l.width,
                    l.glyphs,
                    l.text
                ));
            }
        }
        if let Some(max) = f.lines {
            let pages = lines.iter().map(|l| l.page).max().map_or(0, |p| p + 1);
            for page in 0..pages {
                let n = lines.iter().filter(|l| l.page == page).count();
                if n > max as usize {
                    out.push(format!(
                        "{}: page {} has {n} lines, the limit is {max}",
                        asset.id,
                        page + 1
                    ));
                }
            }
        }
        Ok(out)
    }
}
