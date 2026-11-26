//! `formatkit.asset-map.v1`: universal declarative asset-map schema support,
//! lossless conversion from `swankit-asset-map/1`, verification, and Markdown
//! reporting.
use crate::data::decode_asset;
use crate::map::{
    Address, Asset, AssetMap, CharmapSource, Codec, CodecParams, Confidence, Evidence, Format,
    InlinePalette, Kind, Num, Reference, Relations, RomIdentity, MAX_ASSETS, MAX_MAP_BYTES, SCHEMA,
};
use crate::text::Charmaps;
use crate::verify::{verify, Finding, VerifyReport};
use crate::{check_identity, malformed, sha256_hex};
use formatkit_core::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

pub const FORMATKIT_SCHEMA: &str = "formatkit.asset-map.v1";
pub const FORMATKIT_SCHEMA_VERSION: u32 = 1;
pub const KIT_NAME: &str = "swankit";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormatkitMediaKind {
    FlatRom,
    FilesystemDisc,
    FilesystemRom,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormatkitFingerprint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rom_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pvd_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_exe_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormatkitMedia {
    pub kind: FormatkitMediaKind,
    pub fingerprint: FormatkitFingerprint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_count: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormatkitStatus {
    Identified,
    Mapped,
    Opaque,
    Unknown,
}

impl FormatkitStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identified => "identified",
            Self::Mapped => "mapped",
            Self::Opaque => "opaque",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormatkitMappingKind {
    RawImage,
    RawAudio,
    Compressed,
    ExeIndexed,
    Companion,
    Opaque,
}

impl FormatkitMappingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RawImage => "raw-image",
            Self::RawAudio => "raw-audio",
            Self::Compressed => "compressed",
            Self::ExeIndexed => "exe-indexed",
            Self::Companion => "companion",
            Self::Opaque => "opaque",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormatkitOpaqueClass {
    Script,
    Collision,
    Table,
    Padding,
    Bytecode,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatkitMapping {
    pub kind: FormatkitMappingKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired_with: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opaque_class: Option<FormatkitOpaqueClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormatkitEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lba: Option<u64>,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub status: FormatkitStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entropy: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_refs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapping: Option<FormatkitMapping>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormatkitAssetMapV1 {
    pub schema: String,
    pub schema_version: u32,
    pub kit: String,
    pub media: FormatkitMedia,
    pub entries: Vec<FormatkitEntry>,
}

impl FormatkitAssetMapV1 {
    /// Parse and validate a `formatkit.asset-map.v1` JSON document.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_MAP_BYTES {
            return Err(malformed("asset map exceeds 64 MiB"));
        }
        let doc: Self = serde_json::from_slice(bytes).map_err(|e| malformed(e.to_string()))?;
        doc.check_structure()?;
        Ok(doc)
    }

    pub fn to_json(&self) -> Result<Vec<u8>> {
        let mut out = serde_json::to_vec_pretty(self).map_err(|e| malformed(e.to_string()))?;
        out.push(b'\n');
        Ok(out)
    }

    pub fn check_structure(&self) -> Result<()> {
        if self.schema != FORMATKIT_SCHEMA {
            return Err(malformed(format!("unsupported schema {:?}", self.schema)));
        }
        if self.schema_version != FORMATKIT_SCHEMA_VERSION {
            return Err(malformed(format!(
                "unsupported schema_version {}",
                self.schema_version
            )));
        }
        if self.kit.is_empty() {
            return Err(malformed("kit must not be empty"));
        }
        if let Some(sha) = &self.media.fingerprint.rom_sha256 {
            check_sha256("media.fingerprint.rom_sha256", sha)?;
        }
        if let Some(sha) = &self.media.fingerprint.pvd_sha256 {
            check_sha256("media.fingerprint.pvd_sha256", sha)?;
        }
        if let Some(sha) = &self.media.fingerprint.main_exe_sha256 {
            check_sha256("media.fingerprint.main_exe_sha256", sha)?;
        }
        if self.entries.len() > MAX_ASSETS {
            return Err(malformed("too many entries"));
        }
        let mut ids = BTreeSet::new();
        for entry in &self.entries {
            if entry.id.is_empty() {
                return Err(malformed("entry id must not be empty"));
            }
            if !ids.insert(entry.id.as_str()) {
                return Err(malformed(format!("duplicate entry id {:?}", entry.id)));
            }
            if let Some(sha) = &entry.sha256 {
                check_sha256(&format!("entry {} sha256", entry.id), sha)?;
            }
            if let Some(h) = entry.entropy {
                if !h.is_finite() || !(0.0..=8.0).contains(&h) {
                    return Err(malformed(format!(
                        "entry {} entropy {} must be in 0.0..=8.0",
                        entry.id, h
                    )));
                }
            }
        }
        Ok(())
    }

    /// Convert all `mapped` entries into an [`AssetMap`] for verification,
    /// extraction, and replacement.
    pub fn to_asset_map(&self) -> Result<AssetMap> {
        self.check_structure()?;
        let sha256 =
            self.media.fingerprint.rom_sha256.clone().ok_or_else(|| {
                malformed("media.fingerprint.rom_sha256 is required for flat-rom")
            })?;
        let total_bytes = self
            .media
            .total_bytes
            .ok_or_else(|| malformed("media.total_bytes is required for flat-rom"))?;

        let mut charmaps: BTreeMap<String, CharmapSource> = BTreeMap::new();
        let mut palettes: BTreeMap<String, InlinePalette> = BTreeMap::new();
        let mut assets = Vec::new();

        for entry in &self.entries {
            if entry.status != FormatkitStatus::Mapped {
                continue;
            }
            let offset = entry.offset.ok_or_else(|| {
                malformed(format!(
                    "mapped entry {:?} is missing required offset",
                    entry.id
                ))
            })?;
            let asset = entry_to_asset(entry, offset, &mut charmaps, &mut palettes)?;
            assets.push(asset);
        }

        let map = AssetMap {
            schema: SCHEMA.into(),
            rom: RomIdentity {
                sha256,
                size: Num(total_bytes),
                label: None,
            },
            charmaps,
            palettes,
            free_space: Vec::new(),
            coverage: None,
            assets,
            screens: Vec::new(),
        };
        map.check_structure()?;
        Ok(map)
    }
}

impl std::str::FromStr for FormatkitAssetMapV1 {
    type Err = formatkit_core::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::from_json(s.as_bytes())
    }
}

/// Either a legacy `swankit-asset-map/1` document or a universal
/// `formatkit.asset-map.v1` document.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadedAssetMap {
    SwankitV1(AssetMap),
    FormatkitV1(FormatkitAssetMapV1),
}

impl LoadedAssetMap {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_MAP_BYTES {
            return Err(malformed("asset map exceeds 64 MiB"));
        }
        let peek: Value = serde_json::from_slice(bytes).map_err(|e| malformed(e.to_string()))?;
        let schema = peek
            .get("schema")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("asset map missing string \"schema\" field"))?;
        match schema {
            SCHEMA => Ok(Self::SwankitV1(AssetMap::from_json(bytes)?)),
            FORMATKIT_SCHEMA => Ok(Self::FormatkitV1(FormatkitAssetMapV1::from_json(bytes)?)),
            other => Err(malformed(format!("unsupported schema {other:?}"))),
        }
    }

    pub fn schema(&self) -> &str {
        match self {
            Self::SwankitV1(_) => SCHEMA,
            Self::FormatkitV1(_) => FORMATKIT_SCHEMA,
        }
    }

    pub fn to_asset_map(&self) -> Result<AssetMap> {
        match self {
            Self::SwankitV1(map) => Ok(map.clone()),
            Self::FormatkitV1(doc) => doc.to_asset_map(),
        }
    }

    /// Verify ROM identity, per-entry slice bounds & SHA-256 (when present),
    /// and test-decode every `mapped` entry via `ws-assets`.
    pub fn verify(&self, charmaps: &Charmaps, rom: &[u8]) -> Result<VerifyReport> {
        match self {
            Self::SwankitV1(map) => verify(map, charmaps, rom),
            Self::FormatkitV1(doc) => {
                doc.check_structure()?;
                let asset_map = doc.to_asset_map()?;
                let mut report = verify(&asset_map, charmaps, rom)?;
                for entry in &doc.entries {
                    let Some(offset) = entry.offset else {
                        continue;
                    };
                    let Ok(start) = usize::try_from(offset) else {
                        report.errors.push(Finding {
                            id: entry.id.clone(),
                            message: "entry offset exceeds address width".into(),
                        });
                        continue;
                    };
                    let Ok(size) = usize::try_from(entry.size) else {
                        report.errors.push(Finding {
                            id: entry.id.clone(),
                            message: "entry size exceeds address width".into(),
                        });
                        continue;
                    };
                    let Some(end) = start.checked_add(size) else {
                        report.errors.push(Finding {
                            id: entry.id.clone(),
                            message: "entry range overflows address width".into(),
                        });
                        continue;
                    };
                    if end > rom.len() {
                        report.errors.push(Finding {
                            id: entry.id.clone(),
                            message: format!(
                                "entry range {start:#x}..{end:#x} exceeds ROM length {}",
                                rom.len()
                            ),
                        });
                        continue;
                    }
                    if let Some(expected_sha) = &entry.sha256 {
                        let actual = sha256_hex(&rom[start..end]);
                        if !actual.eq_ignore_ascii_case(expected_sha) {
                            report.errors.push(Finding {
                                id: entry.id.clone(),
                                message: format!(
                                    "slice SHA-256 {actual} differs from declared {expected_sha}"
                                ),
                            });
                        }
                    }
                }
                Ok(report)
            }
        }
    }

    /// Render a Markdown summary table of the map's entries.
    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# Asset Map Summary\n");
        match self {
            Self::FormatkitV1(doc) => {
                let _ = writeln!(out, "- **Schema:** `{}`", doc.schema);
                let _ = writeln!(out, "- **Kit:** `{}`", doc.kit);
                if let Some(sha) = &doc.media.fingerprint.rom_sha256 {
                    let _ = writeln!(out, "- **ROM SHA-256:** `{sha}`");
                }
                if let Some(total) = doc.media.total_bytes {
                    let _ = writeln!(out, "- **ROM Size:** `{total}` bytes");
                }
                let _ = writeln!(out, "- **Entries:** `{}`\n", doc.entries.len());
                let _ = writeln!(
                    out,
                    "| ID | Offset | Size | Format ID | Kind | Status | Verified |"
                );
                let _ = writeln!(out, "| --- | --- | --- | --- | --- | --- | --- |");
                for e in &doc.entries {
                    let offset = e
                        .offset
                        .map_or_else(|| "-".to_owned(), |o| format!("`0x{o:06x}`"));
                    let format_id = e
                        .format_id
                        .as_deref()
                        .map_or_else(|| "-".to_owned(), |f| format!("`{f}`"));
                    let kind = e
                        .mapping
                        .as_ref()
                        .map_or_else(|| "-".to_owned(), |m| format!("`{}`", m.kind.as_str()));
                    let verified = e
                        .verified
                        .map_or_else(|| "-".to_owned(), |v| format!("`{v}`"));
                    let _ = writeln!(
                        out,
                        "| `{}` | {} | `{}` | {} | {} | `{}` | {} |",
                        e.id,
                        offset,
                        e.size,
                        format_id,
                        kind,
                        e.status.as_str(),
                        verified
                    );
                }
            }
            Self::SwankitV1(map) => {
                let _ = writeln!(out, "- **Schema:** `{}`", map.schema);
                let _ = writeln!(out, "- **Kit:** `{KIT_NAME}`");
                let _ = writeln!(out, "- **ROM SHA-256:** `{}`", map.rom.sha256);
                let _ = writeln!(out, "- **ROM Size:** `{}` bytes", map.rom.size.0);
                let _ = writeln!(out, "- **Entries:** `{}`\n", map.assets.len());
                let _ = writeln!(
                    out,
                    "| ID | Offset | Size | Format ID | Kind | Status | Verified |"
                );
                let _ = writeln!(out, "| --- | --- | --- | --- | --- | --- | --- |");
                for a in &map.assets {
                    let offset = format!("`0x{:06x}`", a.offset.0);
                    let size = a
                        .length
                        .map_or_else(|| "-".to_owned(), |n| format!("`{}`", n.0));
                    let format_id = format!("`{}`", format_id_for(a));
                    let (mkind, _) = mapping_kind_for(a);
                    let _ = writeln!(
                        out,
                        "| `{}` | {} | {} | {} | `{}` | `mapped` | - |",
                        a.id,
                        offset,
                        size,
                        format_id,
                        mkind.as_str()
                    );
                }
            }
        }
        out
    }
}

/// Compute Shannon entropy in bits per byte (`0.0..=8.0`).
pub fn shannon_entropy(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0u64; 256];
    for &b in bytes {
        counts[usize::from(b)] += 1;
    }
    let len = bytes.len() as f64;
    let mut entropy = 0.0f64;
    for &c in &counts {
        if c > 0 {
            let p = c as f64 / len;
            entropy -= p * p.log2();
        }
    }
    let clamped = entropy.clamp(0.0, 8.0);
    let rounded = (clamped * 1_000_000.0).round() / 1_000_000.0;
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}

/// Neutral `format_id` for a WonderSwan asset.
pub fn format_id_for(asset: &Asset) -> String {
    match asset.codec {
        Codec::Raw => match asset.kind {
            Kind::Tiles => {
                let bpp = asset.format.bpp.unwrap_or(2);
                let layout = asset.format.layout.as_deref().unwrap_or("planar");
                format!("ws.tiles.{bpp}bpp-{layout}")
            }
            Kind::Tilemap => {
                let entry = asset.format.entry.as_deref().unwrap_or("ws-word");
                format!("ws.tilemap.{entry}")
            }
            Kind::Palette => {
                let pf = asset.format.palette_format.as_deref().unwrap_or("rgb444");
                format!("ws.palette.{pf}")
            }
            Kind::Font => {
                let bpp = asset.format.bpp.unwrap_or(1);
                format!("ws.font.{bpp}bpp")
            }
            Kind::SpriteTable => {
                let entry = asset.format.entry.as_deref().unwrap_or("ws-oam");
                format!("ws.sprite-table.{entry}")
            }
            Kind::Text => "ws.text.charmap".into(),
            Kind::PointerTable => "ws.pointer-table".into(),
            Kind::Pcm => "ws.pcm.u8".into(),
            Kind::Music => "ws.audio.music".into(),
            Kind::SoundEffect => "ws.audio.sfx".into(),
            Kind::DataTable => "ws.data.table".into(),
            Kind::Other => "ws.data.raw".into(),
        },
        Codec::LzRle7f => "ws.codec.lz-rle-7f".into(),
        Codec::LzBits16 => "ws.codec.lz-bits-16".into(),
        Codec::LzU8Zero => "ws.codec.lz-u8-zero".into(),
        Codec::LzWord16Msb => "ws.codec.lz-word16-msb".into(),
        Codec::LzWordCounted => "ws.codec.lz-word-counted".into(),
        Codec::Okumura => "ws.codec.okumura-lzss".into(),
        Codec::LzssU16Zero => "ws.codec.lzss-u16-zero".into(),
        Codec::Lzss12 => "ws.codec.lzss-12".into(),
        Codec::Lzs2 => "ws.codec.lzs2".into(),
    }
}

/// Map a `ws-assets` asset's codec and kind to `formatkit.asset-map.v1`
/// `mapping.kind` and optional `mapping.opaque_class`.
pub fn mapping_kind_for(asset: &Asset) -> (FormatkitMappingKind, Option<FormatkitOpaqueClass>) {
    if asset.codec != Codec::Raw {
        return (FormatkitMappingKind::Compressed, None);
    }
    match asset.kind {
        Kind::Tiles | Kind::Tilemap | Kind::Palette | Kind::Font | Kind::SpriteTable => {
            (FormatkitMappingKind::RawImage, None)
        }
        Kind::Pcm | Kind::Music | Kind::SoundEffect => (FormatkitMappingKind::RawAudio, None),
        Kind::PointerTable => (FormatkitMappingKind::ExeIndexed, None),
        Kind::Text => (
            FormatkitMappingKind::Opaque,
            Some(FormatkitOpaqueClass::Script),
        ),
        Kind::DataTable => (
            FormatkitMappingKind::Opaque,
            Some(FormatkitOpaqueClass::Table),
        ),
        Kind::Other => (
            FormatkitMappingKind::Opaque,
            Some(FormatkitOpaqueClass::Other),
        ),
    }
}

/// Verify `map` against `rom` and convert it losslessly into a verified
/// `formatkit.asset-map.v1` document (using builtin/inline charmaps only).
pub fn to_formatkit_v1(map: &AssetMap, rom: &[u8]) -> Result<FormatkitAssetMapV1> {
    let charmaps = Charmaps::load(map, |name| {
        Err(malformed(format!(
            "charmap file {name:?} requires to_formatkit_v1_with_charmaps"
        )))
    })?;
    to_formatkit_v1_with_charmaps(map, &charmaps, rom)
}

/// Verify `map` against `rom` with pre-loaded `charmaps` and convert it
/// losslessly into a verified `formatkit.asset-map.v1` document.
pub fn to_formatkit_v1_with_charmaps(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
) -> Result<FormatkitAssetMapV1> {
    check_identity(map, rom)?;
    let report = verify(map, charmaps, rom)?;
    if let Some(first) = report.errors.first() {
        return Err(malformed(format!(
            "cannot convert unverified map ({} error(s), first in {}): {}",
            report.errors.len(),
            first.id,
            first.message
        )));
    }

    let mut entries = Vec::with_capacity(map.assets.len());
    for asset in &map.assets {
        let stored = decode_asset(map, charmaps, rom, &asset.id)?;
        let slice = &rom[stored.offset..stored.offset + stored.length];
        let (mkind, opaque_class) = mapping_kind_for(asset);

        let paired_with = asset
            .relations
            .palette
            .clone()
            .or_else(|| asset.relations.palettes.first().cloned())
            .or_else(|| asset.relations.tiles.first().map(|t| t.id.clone()))
            .or_else(|| asset.relations.font.clone())
            .or_else(|| asset.relations.pointer_table.clone());

        let mut params = Map::new();
        if asset.codec != Codec::Raw {
            params.insert(
                "codec".into(),
                serde_json::to_value(asset.codec).map_err(|e| malformed(e.to_string()))?,
            );
        }
        if asset.codec_params != CodecParams::default() {
            params.insert(
                "codec_params".into(),
                serde_json::to_value(&asset.codec_params).map_err(|e| malformed(e.to_string()))?,
            );
        }
        if asset.format != Format::default() {
            params.insert(
                "format".into(),
                serde_json::to_value(&asset.format).map_err(|e| malformed(e.to_string()))?,
            );
        }
        if asset.relations != Relations::default() {
            params.insert(
                "relations".into(),
                serde_json::to_value(&asset.relations).map_err(|e| malformed(e.to_string()))?,
            );
        }
        if !asset.references.is_empty() {
            params.insert(
                "references".into(),
                serde_json::to_value(&asset.references).map_err(|e| malformed(e.to_string()))?,
            );
        }
        if asset.references_complete {
            params.insert("references_complete".into(), Value::Bool(true));
        }
        if !asset.overlaps.is_empty() {
            params.insert(
                "overlaps".into(),
                serde_json::to_value(&asset.overlaps).map_err(|e| malformed(e.to_string()))?,
            );
        }
        if let Some(cap) = asset.reserved_capacity {
            params.insert(
                "reserved_capacity".into(),
                serde_json::to_value(cap).map_err(|e| malformed(e.to_string()))?,
            );
        }
        if let Some(addr) = &asset.address {
            params.insert(
                "address".into(),
                serde_json::to_value(addr).map_err(|e| malformed(e.to_string()))?,
            );
        }
        if asset.confidence != Confidence::default() {
            params.insert(
                "confidence".into(),
                serde_json::to_value(asset.confidence).map_err(|e| malformed(e.to_string()))?,
            );
        }
        if asset.evidence != Evidence::default() {
            params.insert(
                "evidence".into(),
                serde_json::to_value(&asset.evidence).map_err(|e| malformed(e.to_string()))?,
            );
        }
        if let Some(cm_name) = &asset.relations.charmap {
            if let Some(cm_source) = map.charmaps.get(cm_name) {
                params.insert(
                    "charmap_source".into(),
                    serde_json::to_value(cm_source).map_err(|e| malformed(e.to_string()))?,
                );
            }
        }
        let mut inline_pals = Map::new();
        for pal_name in asset
            .relations
            .palette
            .iter()
            .chain(&asset.relations.palettes)
        {
            if let Some(inline_pal) = map.palettes.get(pal_name) {
                inline_pals.insert(
                    pal_name.clone(),
                    serde_json::to_value(inline_pal).map_err(|e| malformed(e.to_string()))?,
                );
            }
        }
        if !inline_pals.is_empty() {
            params.insert("inline_palettes".into(), Value::Object(inline_pals));
        }

        let evidence_str = summarize_evidence(&asset.evidence);
        let code_refs = asset.evidence.loader.as_ref().map(|l| vec![l.clone()]);

        entries.push(FormatkitEntry {
            id: asset.id.clone(),
            path: None,
            offset: Some(stored.offset as u64),
            lba: None,
            size: stored.length as u64,
            sha256: Some(sha256_hex(slice)),
            status: FormatkitStatus::Mapped,
            format_id: Some(format_id_for(asset)),
            entropy: Some(shannon_entropy(slice)),
            code_refs,
            mapping: Some(FormatkitMapping {
                kind: mkind,
                params: if params.is_empty() {
                    None
                } else {
                    Some(params)
                },
                paired_with,
                inner_format: Some(asset.kind.name().into()),
                opaque_class,
                evidence: evidence_str,
            }),
            verified: Some(true),
        });
    }

    let doc = FormatkitAssetMapV1 {
        schema: FORMATKIT_SCHEMA.into(),
        schema_version: FORMATKIT_SCHEMA_VERSION,
        kit: KIT_NAME.into(),
        media: FormatkitMedia {
            kind: FormatkitMediaKind::FlatRom,
            fingerprint: FormatkitFingerprint {
                rom_sha256: Some(sha256_hex(rom)),
                pvd_sha256: None,
                main_exe_sha256: None,
            },
            total_bytes: Some(rom.len() as u64),
            file_count: None,
        },
        entries,
    };
    doc.check_structure()?;
    Ok(doc)
}

fn summarize_evidence(e: &Evidence) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(loader) = &e.loader {
        parts.push(format!("loader={loader}"));
    }
    if let Some(trace) = &e.trace {
        parts.push(format!("trace={trace}"));
    }
    if let Some(notes) = &e.notes {
        parts.push(notes.clone());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

fn check_sha256(field: &str, s: &str) -> Result<()> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(malformed(format!("{field} must be 64 hexadecimal digits")));
    }
    Ok(())
}

fn kind_from_name(name: &str) -> Option<Kind> {
    match name {
        "tiles" => Some(Kind::Tiles),
        "tilemap" => Some(Kind::Tilemap),
        "palette" => Some(Kind::Palette),
        "font" => Some(Kind::Font),
        "text" => Some(Kind::Text),
        "pointer-table" => Some(Kind::PointerTable),
        "music" => Some(Kind::Music),
        "sound-effect" => Some(Kind::SoundEffect),
        "pcm" => Some(Kind::Pcm),
        "data-table" => Some(Kind::DataTable),
        "sprite-table" => Some(Kind::SpriteTable),
        "other" => Some(Kind::Other),
        _ => None,
    }
}

fn infer_kind(entry: &FormatkitEntry) -> Kind {
    if let Some(inner) = entry
        .mapping
        .as_ref()
        .and_then(|m| m.inner_format.as_deref())
    {
        if let Some(k) = kind_from_name(inner) {
            return k;
        }
    }
    if let Some(fid) = entry.format_id.as_deref() {
        if fid.starts_with("ws.tiles.") {
            return Kind::Tiles;
        }
        if fid.starts_with("ws.tilemap.") {
            return Kind::Tilemap;
        }
        if fid.starts_with("ws.palette.") {
            return Kind::Palette;
        }
        if fid.starts_with("ws.font.") {
            return Kind::Font;
        }
        if fid.starts_with("ws.sprite-table.") {
            return Kind::SpriteTable;
        }
        if fid.starts_with("ws.text.") {
            return Kind::Text;
        }
        if fid.starts_with("ws.pointer-table") {
            return Kind::PointerTable;
        }
        if fid.starts_with("ws.pcm.") {
            return Kind::Pcm;
        }
        if fid.starts_with("ws.audio.music") {
            return Kind::Music;
        }
        if fid.starts_with("ws.audio.sfx") {
            return Kind::SoundEffect;
        }
        if fid.starts_with("ws.data.table") {
            return Kind::DataTable;
        }
    }
    match entry.mapping.as_ref().map(|m| m.kind) {
        Some(FormatkitMappingKind::RawImage) => Kind::Tiles,
        Some(FormatkitMappingKind::RawAudio) => Kind::Pcm,
        Some(FormatkitMappingKind::ExeIndexed) => Kind::PointerTable,
        _ => Kind::Other,
    }
}

fn infer_codec(entry: &FormatkitEntry) -> Codec {
    let Some(fid) = entry.format_id.as_deref() else {
        return Codec::Raw;
    };
    match fid {
        "ws.codec.lz-rle-7f" => Codec::LzRle7f,
        "ws.codec.lz-bits-16" => Codec::LzBits16,
        "ws.codec.lz-u8-zero" => Codec::LzU8Zero,
        "ws.codec.lz-word16-msb" => Codec::LzWord16Msb,
        "ws.codec.lz-word-counted" => Codec::LzWordCounted,
        "ws.codec.okumura-lzss" => Codec::Okumura,
        "ws.codec.lzss-u16-zero" => Codec::LzssU16Zero,
        "ws.codec.lzss-12" => Codec::Lzss12,
        "ws.codec.lzs2" => Codec::Lzs2,
        _ => Codec::Raw,
    }
}

fn infer_format(entry: &FormatkitEntry) -> Format {
    let mut format = Format::default();
    if let Some(fid) = entry.format_id.as_deref() {
        if let Some(rest) = fid.strip_prefix("ws.tiles.") {
            if let Some((bpp_str, layout)) = rest.split_once("bpp-") {
                if let Ok(bpp) = bpp_str.parse::<u8>() {
                    format.bpp = Some(bpp);
                    format.layout = Some(layout.to_owned());
                }
            }
        } else if let Some(entry_fmt) = fid.strip_prefix("ws.tilemap.") {
            format.entry = Some(entry_fmt.to_owned());
        } else if let Some(pf) = fid.strip_prefix("ws.palette.") {
            format.palette_format = Some(pf.to_owned());
        } else if let Some(rest) = fid.strip_prefix("ws.font.") {
            if let Some(bpp_str) = rest.strip_suffix("bpp") {
                if let Ok(bpp) = bpp_str.parse::<u8>() {
                    format.bpp = Some(bpp);
                }
            }
        } else if let Some(entry_fmt) = fid.strip_prefix("ws.sprite-table.") {
            format.entry = Some(entry_fmt.to_owned());
        }
    }
    format
}

fn parse_param<T: serde::de::DeserializeOwned>(
    params: Option<&Map<String, Value>>,
    key: &str,
    entry_id: &str,
) -> Result<Option<T>> {
    let Some(val) = params.and_then(|p| p.get(key)) else {
        return Ok(None);
    };
    serde_json::from_value(val.clone())
        .map(Some)
        .map_err(|e| malformed(format!("{entry_id}: invalid mapping.params.{key}: {e}")))
}

fn entry_to_asset(
    entry: &FormatkitEntry,
    offset: u64,
    charmaps: &mut BTreeMap<String, CharmapSource>,
    palettes: &mut BTreeMap<String, InlinePalette>,
) -> Result<Asset> {
    let params = entry.mapping.as_ref().and_then(|m| m.params.as_ref());
    let kind = infer_kind(entry);
    let codec =
        parse_param::<Codec>(params, "codec", &entry.id)?.unwrap_or_else(|| infer_codec(entry));
    let codec_params =
        parse_param::<CodecParams>(params, "codec_params", &entry.id)?.unwrap_or_default();
    let format =
        parse_param::<Format>(params, "format", &entry.id)?.unwrap_or_else(|| infer_format(entry));
    let mut relations =
        parse_param::<Relations>(params, "relations", &entry.id)?.unwrap_or_default();
    if relations == Relations::default() {
        if let Some(paired) = entry.mapping.as_ref().and_then(|m| m.paired_with.as_ref()) {
            match kind {
                Kind::Tiles | Kind::Font => relations.palette = Some(paired.clone()),
                Kind::Tilemap | Kind::SpriteTable => {
                    relations.tiles.push(crate::map::TileRef {
                        id: paired.clone(),
                        at: Num(0),
                        skip: Num(0),
                    });
                }
                Kind::Text => relations.font = Some(paired.clone()),
                _ => {}
            }
        }
    }
    let references =
        parse_param::<Vec<Reference>>(params, "references", &entry.id)?.unwrap_or_default();
    let references_complete =
        parse_param::<bool>(params, "references_complete", &entry.id)?.unwrap_or(false);
    let overlaps = parse_param::<Vec<String>>(params, "overlaps", &entry.id)?.unwrap_or_default();
    let reserved_capacity = parse_param::<Num>(params, "reserved_capacity", &entry.id)?;
    let address = parse_param::<Address>(params, "address", &entry.id)?;
    let confidence =
        parse_param::<Confidence>(params, "confidence", &entry.id)?.unwrap_or_default();
    let mut evidence = parse_param::<Evidence>(params, "evidence", &entry.id)?.unwrap_or_default();
    if evidence == Evidence::default() {
        if let Some(ev) = entry.mapping.as_ref().and_then(|m| m.evidence.as_ref()) {
            evidence.notes = Some(ev.clone());
        }
    }
    if evidence.loader.is_none() {
        if let Some(loader) = entry.code_refs.as_ref().and_then(|r| r.first()) {
            evidence.loader = Some(loader.clone());
        }
    }

    if let Some(cm_source) = parse_param::<CharmapSource>(params, "charmap_source", &entry.id)? {
        if let Some(cm_name) = &relations.charmap {
            charmaps.entry(cm_name.clone()).or_insert(cm_source);
        }
    }
    if let Some(inline_pals) =
        parse_param::<BTreeMap<String, InlinePalette>>(params, "inline_palettes", &entry.id)?
    {
        for (name, pal) in inline_pals {
            palettes.entry(name).or_insert(pal);
        }
    }

    Ok(Asset {
        id: entry.id.clone(),
        kind,
        offset: Num(offset),
        length: Some(Num(entry.size)),
        reserved_capacity,
        codec,
        codec_params,
        address,
        format,
        relations,
        references,
        references_complete,
        overlaps,
        confidence,
        evidence,
    })
}
