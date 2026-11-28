//! `formatkit.asset-map.v1`: universal declarative asset-map schema support,
//! lossless conversion from `swankit-asset-map/1`, verification, and Markdown
//! reporting.
use crate::data::decode_asset;
use crate::map::{
    Address, Anchor, Asset, AssetMap, CharmapSource, Codec, CodecParams, Confidence, Endian,
    Evidence, Form, Format, FreeRange, InlinePalette, Kind, Num, Reference, Relations, RomIdentity,
    Screen, Signed, TileRef, MAX_ASSETS, MAX_MAP_BYTES, SCHEMA,
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

/// A single non-empty string or a non-empty list of non-empty strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrVec {
    One(String),
    Many(Vec<String>),
}

impl StringOrVec {
    pub fn from_vec(mut items: Vec<String>) -> Option<Self> {
        match items.len() {
            0 => None,
            1 => Some(Self::One(items.pop().unwrap())),
            _ => Some(Self::Many(items)),
        }
    }

    pub fn first(&self) -> Option<&str> {
        match self {
            Self::One(s) => Some(s.as_str()),
            Self::Many(v) => v.first().map(String::as_str),
        }
    }

    pub fn as_vec(&self) -> Vec<&str> {
        match self {
            Self::One(s) => vec![s.as_str()],
            Self::Many(v) => v.iter().map(String::as_str).collect(),
        }
    }

    fn check(&self, field: &str) -> Result<()> {
        match self {
            Self::One(s) => {
                if s.is_empty() {
                    return Err(malformed(format!("{field} must not be empty")));
                }
            }
            Self::Many(v) => {
                if v.is_empty() {
                    return Err(malformed(format!("{field} must not be an empty array")));
                }
                if v.iter().any(String::is_empty) {
                    return Err(malformed(format!("{field} must not contain empty strings")));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormatkitFreeSpace {
    pub offset: u64,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bank: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FormatkitShared {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charmaps: Option<BTreeMap<String, CharmapSource>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palettes: Option<BTreeMap<String, InlinePalette>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screens: Option<Vec<Screen>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl FormatkitShared {
    pub fn is_empty(&self) -> bool {
        self.charmaps.as_ref().is_none_or(BTreeMap::is_empty)
            && self.palettes.as_ref().is_none_or(BTreeMap::is_empty)
            && self.screens.as_ref().is_none_or(Vec::is_empty)
            && self.extra.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormatkitEndian {
    Le,
    Be,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormatkitReference {
    pub at: u64,
    pub width: u8,
    pub endian: FormatkitEndian,
    pub form: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bank: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatkitMapping {
    pub kind: FormatkitMappingKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired_with: Option<StringOrVec>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub relations: BTreeMap<String, StringOrVec>,
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
    pub reserved_capacity: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub status: FormatkitStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entropy: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_refs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<FormatkitReference>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub free_space: Vec<FormatkitFreeSpace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<FormatkitShared>,
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
            for (idx, r) in entry.references.iter().enumerate() {
                if r.width < 1 {
                    return Err(malformed(format!(
                        "entry {} references[{idx}] width must be >= 1",
                        entry.id
                    )));
                }
                if r.form.is_empty() {
                    return Err(malformed(format!(
                        "entry {} references[{idx}] form must not be empty",
                        entry.id
                    )));
                }
            }
            if let Some(mapping) = &entry.mapping {
                if let Some(paired) = &mapping.paired_with {
                    paired.check(&format!("entry {} mapping.paired_with", entry.id))?;
                }
                for (rel_key, rel_val) in &mapping.relations {
                    rel_val.check(&format!("entry {} mapping.relations.{rel_key}", entry.id))?;
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

        let mut charmaps: BTreeMap<String, CharmapSource> = self
            .shared
            .as_ref()
            .and_then(|s| s.charmaps.clone())
            .unwrap_or_default();
        let mut palettes: BTreeMap<String, InlinePalette> = self
            .shared
            .as_ref()
            .and_then(|s| s.palettes.clone())
            .unwrap_or_default();
        let screens: Vec<Screen> = self
            .shared
            .as_ref()
            .and_then(|s| s.screens.clone())
            .unwrap_or_default();

        let free_space_fills: Vec<Option<u64>> = self
            .shared
            .as_ref()
            .and_then(|s| s.extra.get("free_space_fills"))
            .map(|v| {
                serde_json::from_value::<Vec<Option<u64>>>(v.clone())
                    .map_err(|e| malformed(format!("invalid shared.free_space_fills: {e}")))
            })
            .transpose()?
            .unwrap_or_default();

        let free_space: Vec<FreeRange> = self
            .free_space
            .iter()
            .enumerate()
            .map(|(i, f)| FreeRange {
                offset: Num(f.offset),
                length: Num(f.size),
                fill: free_space_fills.get(i).copied().flatten().map(Num),
                note: f.note.clone(),
            })
            .collect();

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
            free_space,
            coverage: None,
            assets,
            screens,
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
            Kind::Pcm => match asset.format.sample_format.as_deref().unwrap_or("u8") {
                "s8" => "ws.pcm.s8".into(),
                _ => "ws.pcm.u8".into(),
            },
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
        Codec::Rle77ee => "ws.codec.rle-77ee".into(),
        Codec::RlePcm00 => "ws.codec.rle-pcm-00".into(),
        Codec::CountedWordLz => "ws.codec.counted-word-lz".into(),
        Codec::Dpcm6bit => "ws.codec.dpcm-6bit".into(),
        Codec::Lzss11_5 => "ws.codec.lzss-11-5".into(),
        Codec::Flag16Distance11 => "ws.codec.flag16-distance11".into(),
        Codec::PlanarRle => "ws.codec.planar-rle".into(),
        Codec::LzssTokenBe => "ws.codec.lzss-token-be".into(),
        Codec::LzssBeUnits => "ws.codec.lzss-be-units".into(),
        Codec::Lzss12Ext => "ws.codec.lzss-12-ext".into(),
        Codec::NibbleLzssRing8 => "ws.codec.nibble-lzss-ring8".into(),
        Codec::ByteLzssRing8 => "ws.codec.byte-lzss-ring8".into(),
        Codec::ZeroRle16 => "ws.codec.zero-rle16".into(),
        Codec::LzTile7b => "ws.codec.lz-tile-7b".into(),
        Codec::LzCommand16Lsb => "ws.codec.lz-command16-lsb".into(),
        Codec::LzssMsbDistance12 => "ws.codec.lzss-msb-distance12".into(),
        Codec::Mask16Fill => "ws.codec.mask16-fill".into(),
        Codec::Dpcm4bitNibble => "ws.codec.dpcm-4bit-nibble".into(),
        Codec::LzRlePat => "ws.codec.lz-rle-pat".into(),
        Codec::LzssBandaiG => "ws.codec.lzss-bandai-g".into(),
        Codec::Rle16Bit15 => "ws.codec.rle16-bit15".into(),
        Codec::TileXorPack => "ws.codec.tile-xor-pack".into(),
        Codec::Lzss11_5Direct => "ws.codec.lzss-11-5-direct".into(),
        Codec::HuffmanStatic => "ws.codec.huffman-static".into(),
        Codec::RleLz4Cmd => "ws.codec.rle-lz4-cmd".into(),
        Codec::WordRle00 => "ws.codec.word-rle-00".into(),
        Codec::Dpcm4bitSm => "ws.codec.dpcm-4bit-sm".into(),
        Codec::Wave4bitPacked => "ws.codec.wave-4bit-packed".into(),
        Codec::Tilemap2bit => "ws.codec.tilemap-2bit".into(),
        Codec::Lzss12_4Msb => "ws.codec.lzss-12-4-msb".into(),
        Codec::StridedRle => "ws.codec.strided-rle".into(),
        Codec::Dict16Rle => "ws.codec.dict16-rle".into(),
        Codec::Lz44 => "ws.codec.lz44".into(),
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

        let paired_with = build_paired_with(&asset.relations);
        let mapping_relations = build_mapping_relations(&asset.relations);
        let fk_references: Vec<FormatkitReference> = asset
            .references
            .iter()
            .map(reference_to_formatkit)
            .collect();

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
        if relations_need_params(&asset.relations) {
            params.insert(
                "relations".into(),
                serde_json::to_value(&asset.relations).map_err(|e| malformed(e.to_string()))?,
            );
        }
        if asset.references.iter().any(reference_needs_params) {
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

        let evidence_str = summarize_evidence(&asset.evidence);
        let code_refs = asset.evidence.loader.as_ref().map(|l| vec![l.clone()]);

        entries.push(FormatkitEntry {
            id: asset.id.clone(),
            path: None,
            offset: Some(stored.offset as u64),
            lba: None,
            size: stored.length as u64,
            reserved_capacity: asset.reserved_capacity.map(|n| n.0),
            sha256: Some(sha256_hex(slice)),
            status: FormatkitStatus::Mapped,
            format_id: Some(format_id_for(asset)),
            entropy: Some(shannon_entropy(slice)),
            code_refs,
            references: fk_references,
            mapping: Some(FormatkitMapping {
                kind: mkind,
                params: if params.is_empty() {
                    None
                } else {
                    Some(params)
                },
                paired_with,
                relations: mapping_relations,
                inner_format: Some(asset.kind.name().into()),
                opaque_class,
                evidence: evidence_str,
            }),
            verified: Some(true),
        });
    }

    let free_space: Vec<FormatkitFreeSpace> = map
        .free_space
        .iter()
        .map(|f| FormatkitFreeSpace {
            offset: f.offset.0,
            size: f.length.0,
            bank: None,
            note: f.note.clone(),
        })
        .collect();

    let mut shared = FormatkitShared {
        charmaps: (!map.charmaps.is_empty()).then(|| map.charmaps.clone()),
        palettes: (!map.palettes.is_empty()).then(|| map.palettes.clone()),
        screens: (!map.screens.is_empty()).then(|| map.screens.clone()),
        extra: Map::new(),
    };
    if map.free_space.iter().any(|f| f.fill.is_some()) {
        let fills: Vec<Option<u64>> = map.free_space.iter().map(|f| f.fill.map(|n| n.0)).collect();
        shared.extra.insert(
            "free_space_fills".into(),
            serde_json::to_value(fills).map_err(|e| malformed(e.to_string()))?,
        );
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
        free_space,
        shared: (!shared.is_empty()).then_some(shared),
        entries,
    };
    doc.check_structure()?;
    Ok(doc)
}

fn build_paired_with(rel: &Relations) -> Option<StringOrVec> {
    let mut ids = Vec::new();
    let mut push_unique = |id: &str| {
        if !id.is_empty() && !ids.iter().any(|existing: &String| existing == id) {
            ids.push(id.to_owned());
        }
    };
    if let Some(p) = &rel.palette {
        push_unique(p);
    }
    for p in &rel.palettes {
        push_unique(p);
    }
    for t in &rel.tiles {
        push_unique(&t.id);
    }
    if let Some(f) = &rel.font {
        push_unique(f);
    }
    if let Some(pt) = &rel.pointer_table {
        push_unique(pt);
    }
    StringOrVec::from_vec(ids)
}

fn build_mapping_relations(rel: &Relations) -> BTreeMap<String, StringOrVec> {
    let mut out = BTreeMap::new();
    if let Some(v) = StringOrVec::from_vec(rel.tiles.iter().map(|t| t.id.clone()).collect()) {
        out.insert("tiles".into(), v);
    }
    if let Some(p) = &rel.palette {
        if !p.is_empty() {
            out.insert("palette".into(), StringOrVec::One(p.clone()));
        }
    }
    if let Some(v) = StringOrVec::from_vec(rel.palettes.clone()) {
        out.insert("palettes".into(), v);
    }
    if let Some(cm) = &rel.charmap {
        if !cm.is_empty() {
            out.insert("charmap".into(), StringOrVec::One(cm.clone()));
        }
    }
    if let Some(pt) = &rel.pointer_table {
        if !pt.is_empty() {
            out.insert("pointer_table".into(), StringOrVec::One(pt.clone()));
        }
    }
    if let Some(f) = &rel.font {
        if !f.is_empty() {
            out.insert("font".into(), StringOrVec::One(f.clone()));
        }
    }
    if let Some(v) = StringOrVec::from_vec(rel.targets.iter().flatten().cloned().collect()) {
        out.insert("targets".into(), v);
    }
    out
}

fn relations_need_params(rel: &Relations) -> bool {
    rel.tiles.iter().any(|t| t.at.0 != 0 || t.skip.0 != 0)
        || rel.targets.iter().any(Option::is_none)
}

fn reference_to_formatkit(r: &Reference) -> FormatkitReference {
    FormatkitReference {
        at: r.at.0,
        width: r.width,
        endian: match r.endian {
            Endian::Little => FormatkitEndian::Le,
            Endian::Big => FormatkitEndian::Be,
        },
        form: match r.form {
            Form::Linear => "linear".into(),
            Form::Far => "far".into(),
        },
        shift: (r.shift != 0).then_some(i32::from(r.shift)),
        add: (r.add.0 != 0).then_some(r.add.0),
        bank: None,
        note: r.note.clone(),
    }
}

fn reference_needs_params(r: &Reference) -> bool {
    r.anchor != Anchor::File
        || r.align.is_some()
        || r.below.is_some()
        || r.companion
        || r.form == Form::Far
        || r.segment_base.is_some()
        || r.window.is_some()
        || r.normalize.is_some()
        || r.kind != "other"
}

fn formatkit_to_reference(r: &FormatkitReference, entry_id: &str) -> Result<Reference> {
    let shift = match r.shift {
        Some(s) => u8::try_from(s).map_err(|_| {
            malformed(format!(
                "{entry_id}: reference shift {s} must be in 0..=255"
            ))
        })?,
        None => 0,
    };
    let form = match r.form.as_str() {
        "linear" => Form::Linear,
        "far" => Form::Far,
        other => {
            return Err(malformed(format!(
                "{entry_id}: unsupported reference form {other:?}"
            )))
        }
    };
    Ok(Reference {
        at: Num(r.at),
        width: r.width,
        endian: match r.endian {
            FormatkitEndian::Le => Endian::Little,
            FormatkitEndian::Be => Endian::Big,
        },
        anchor: Anchor::File,
        add: Signed(r.add.unwrap_or(0)),
        shift,
        align: None,
        below: None,
        companion: false,
        form,
        segment_base: None,
        window: None,
        normalize: None,
        kind: "other".into(),
        note: r.note.clone(),
    })
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
        "ws.codec.rle-77ee" => Codec::Rle77ee,
        "ws.codec.rle-pcm-00" => Codec::RlePcm00,
        "ws.codec.counted-word-lz" => Codec::CountedWordLz,
        "ws.codec.dpcm-6bit" => Codec::Dpcm6bit,
        "ws.codec.lzss-11-5" => Codec::Lzss11_5,
        "ws.codec.flag16-distance11" => Codec::Flag16Distance11,
        "ws.codec.planar-rle" => Codec::PlanarRle,
        "ws.codec.lzss-token-be" => Codec::LzssTokenBe,
        "ws.codec.lzss-be-units" => Codec::LzssBeUnits,
        "ws.codec.lzss-12-ext" => Codec::Lzss12Ext,
        "ws.codec.nibble-lzss-ring8" => Codec::NibbleLzssRing8,
        "ws.codec.byte-lzss-ring8" => Codec::ByteLzssRing8,
        "ws.codec.zero-rle16" => Codec::ZeroRle16,
        "ws.codec.lz-tile-7b" => Codec::LzTile7b,
        "ws.codec.lz-command16-lsb" => Codec::LzCommand16Lsb,
        "ws.codec.lzss-msb-distance12" => Codec::LzssMsbDistance12,
        "ws.codec.mask16-fill" => Codec::Mask16Fill,
        "ws.codec.dpcm-4bit-nibble" => Codec::Dpcm4bitNibble,
        "ws.codec.lz-rle-pat" => Codec::LzRlePat,
        "ws.codec.lzss-bandai-g" => Codec::LzssBandaiG,
        "ws.codec.rle16-bit15" => Codec::Rle16Bit15,
        "ws.codec.tile-xor-pack" => Codec::TileXorPack,
        "ws.codec.lzss-11-5-direct" => Codec::Lzss11_5Direct,
        "ws.codec.huffman-static" => Codec::HuffmanStatic,
        "ws.codec.rle-lz4-cmd" => Codec::RleLz4Cmd,
        "ws.codec.word-rle-00" => Codec::WordRle00,
        "ws.codec.dpcm-4bit-sm" => Codec::Dpcm4bitSm,
        "ws.codec.wave-4bit-packed" => Codec::Wave4bitPacked,
        "ws.codec.tilemap-2bit" => Codec::Tilemap2bit,
        "ws.codec.lzss-12-4-msb" => Codec::Lzss12_4Msb,
        "ws.codec.strided-rle" => Codec::StridedRle,
        "ws.codec.dict16-rle" => Codec::Dict16Rle,
        "ws.codec.lz44" => Codec::Lz44,
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
        } else if let Some(sf) = fid.strip_prefix("ws.pcm.") {
            if sf == "s8" {
                format.sample_format = Some("s8".to_owned());
            }
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
    let mut format =
        parse_param::<Format>(params, "format", &entry.id)?.unwrap_or_else(|| infer_format(entry));
    if format.sample_format.is_none() && entry.format_id.as_deref() == Some("ws.pcm.s8") {
        format.sample_format = Some("s8".to_owned());
    }
    let mut relations =
        parse_param::<Relations>(params, "relations", &entry.id)?.unwrap_or_default();
    if relations == Relations::default() {
        if let Some(mrel) = entry
            .mapping
            .as_ref()
            .map(|m| &m.relations)
            .filter(|r| !r.is_empty())
        {
            if let Some(tiles) = mrel.get("tiles") {
                for id in tiles.as_vec() {
                    relations.tiles.push(TileRef {
                        id: id.to_owned(),
                        at: Num(0),
                        skip: Num(0),
                    });
                }
            }
            if let Some(p) = mrel.get("palette").and_then(StringOrVec::first) {
                relations.palette = Some(p.to_owned());
            }
            if let Some(pals) = mrel.get("palettes") {
                relations.palettes = pals.as_vec().into_iter().map(str::to_owned).collect();
            }
            if let Some(cm) = mrel.get("charmap").and_then(StringOrVec::first) {
                relations.charmap = Some(cm.to_owned());
            }
            if let Some(pt) = mrel.get("pointer_table").and_then(StringOrVec::first) {
                relations.pointer_table = Some(pt.to_owned());
            }
            if let Some(f) = mrel.get("font").and_then(StringOrVec::first) {
                relations.font = Some(f.to_owned());
            }
            if let Some(tgts) = mrel.get("targets") {
                relations.targets = tgts
                    .as_vec()
                    .into_iter()
                    .map(|s| Some(s.to_owned()))
                    .collect();
            }
        } else if let Some(paired) = entry.mapping.as_ref().and_then(|m| m.paired_with.as_ref()) {
            match kind {
                Kind::Tiles | Kind::Font => {
                    relations.palette = paired.first().map(str::to_owned);
                }
                Kind::Tilemap | Kind::SpriteTable => {
                    for id in paired.as_vec() {
                        relations.tiles.push(TileRef {
                            id: id.to_owned(),
                            at: Num(0),
                            skip: Num(0),
                        });
                    }
                }
                Kind::Text => {
                    relations.font = paired.first().map(str::to_owned);
                }
                _ => {}
            }
        }
    }
    let references =
        if let Some(refs) = parse_param::<Vec<Reference>>(params, "references", &entry.id)? {
            refs
        } else if !entry.references.is_empty() {
            entry
                .references
                .iter()
                .map(|r| formatkit_to_reference(r, &entry.id))
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
    let references_complete =
        parse_param::<bool>(params, "references_complete", &entry.id)?.unwrap_or(false);
    let overlaps = parse_param::<Vec<String>>(params, "overlaps", &entry.id)?.unwrap_or_default();
    let reserved_capacity = match entry.reserved_capacity {
        Some(cap) => Some(Num(cap)),
        None => parse_param::<Num>(params, "reserved_capacity", &entry.id)?,
    };
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
