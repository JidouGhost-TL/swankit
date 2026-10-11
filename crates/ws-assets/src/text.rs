//! Named character maps resolved through `ws-text`.
use crate::malformed;
use crate::map::{Asset, AssetMap, CharmapSource};
use formatkit_core::Result;
use std::collections::BTreeMap;
use ws_text::{
    builtin_definition, CharacterMap, CodeUnit, DecodedText, TextDefinition, TextLimits,
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

    /// Bytes for an edited text that keep every unchanged source token
    /// exactly (hidden control operands, aliases, attribute bits): the
    /// longest unchanged token prefix and suffix are copied, and only the
    /// edited span between them is canonically encoded.
    pub fn encode_edit(&self, asset: &Asset, original: &[u8], text: &str) -> Result<Vec<u8>> {
        let map = self.for_asset(asset)?;
        let old = self.decode(asset, original)?;
        let tokens = &old.tokens;
        // The terminating control never joins the copied prefix.
        let limit = tokens.len() - usize::from(old.terminated && !tokens.is_empty());
        let mut head = 0;
        let mut used = 0;
        while head < limit && text[used..].starts_with(tokens[head].text.as_str()) {
            used += tokens[head].text.len();
            head += 1;
        }
        let mut tail = tokens.len();
        let mut rest = text.len();
        while tail > head
            && rest >= used + tokens[tail - 1].text.len()
            && text[..rest].ends_with(tokens[tail - 1].text.as_str())
        {
            rest -= tokens[tail - 1].text.len();
            tail -= 1;
        }
        let middle = &text[used..rest];
        let mut bytes: Vec<u8> = tokens[..head]
            .iter()
            .flat_map(|t| t.bytes.clone())
            .collect();
        if !middle.is_empty() {
            let encoded = map.encode(middle, MAX_TEXT_BYTES)?;
            let decoded = map.decode(&encoded, Self::limits())?;
            let mut keep = encoded.len();
            if decoded.terminated && tail < tokens.len() {
                // Drop the canonical end appended to the fragment; the source suffix ends it.
                if let Some(last) = decoded.tokens.last().filter(|t| t.text.is_empty()) {
                    keep -= last.bytes.len();
                }
            }
            bytes.extend_from_slice(&encoded[..keep]);
        }
        bytes.extend(tokens[tail..].iter().flat_map(|t| t.bytes.clone()));
        let check = map.decode(&bytes, Self::limits())?;
        if check.text != text || check.consumed != bytes.len() || !check.terminated {
            return Err(malformed(format!(
                "{}: edited text does not re-decode to the requested text",
                asset.id
            )));
        }
        Ok(bytes)
    }
}
