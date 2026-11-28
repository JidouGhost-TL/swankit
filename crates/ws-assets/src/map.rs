//! `swankit-asset-map/1`: one JSON document describing located assets in one ROM.
//! Numbers accept JSON integers or `0x` hexadecimal strings; maps written by
//! this crate use hexadecimal strings for offsets.
use crate::malformed;
use formatkit_core::Result;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "swankit-asset-map/1";
/// Largest accepted map document.
pub const MAX_MAP_BYTES: usize = 64 * 1024 * 1024;
/// Largest accepted asset population.
pub const MAX_ASSETS: usize = 65_536;

/// Integer that serializes as `0x` hexadecimal and accepts decimal too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Num(pub u64);

impl Num {
    pub fn usize(self) -> Result<usize> {
        usize::try_from(self.0).map_err(|_| malformed("number exceeds address width"))
    }
}

impl Serialize for Num {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&format!("0x{:x}", self.0))
    }
}

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Unsigned;
        impl<'de> de::Visitor<'de> for Unsigned {
            type Value = Num;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an unsigned 64-bit integer or decimal/0x hexadecimal string")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Num, E> {
                Ok(Num(v))
            }

            fn visit_str<E: de::Error>(self, s: &str) -> std::result::Result<Num, E> {
                parse_num(s).map(Num).map_err(|_| {
                    E::custom(format!(
                        "invalid unsigned 64-bit integer {s:?}; use decimal or 0x hexadecimal"
                    ))
                })
            }
        }
        d.deserialize_any(Unsigned)
    }
}

fn parse_num(s: &str) -> std::result::Result<u64, String> {
    let parsed = if let Some(hex) = s.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)
    } else {
        s.parse()
    };
    parsed.map_err(|_| format!("invalid integer {s:?}"))
}

/// Signed addend: an integer or a `0x`/`-0x` string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Signed(pub i64);

impl Serialize for Signed {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        if self.0 < 0 {
            s.serialize_str(&format!("-0x{:x}", self.0.unsigned_abs()))
        } else {
            s.serialize_str(&format!("0x{:x}", self.0))
        }
    }
}

impl<'de> Deserialize<'de> for Signed {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(i64),
            Text(String),
        }
        match Raw::deserialize(d)? {
            Raw::Int(v) => Ok(Signed(v)),
            Raw::Text(s) => {
                let (neg, body) = s
                    .strip_prefix('-')
                    .map_or((false, s.as_str()), |b| (true, b));
                let v = parse_num(body).map_err(de::Error::custom)?;
                let v = i64::try_from(v).map_err(de::Error::custom)?;
                Ok(Signed(if neg { -v } else { v }))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Tiles,
    Tilemap,
    Palette,
    Font,
    Text,
    PointerTable,
    Music,
    SoundEffect,
    Pcm,
    DataTable,
    /// Hardware sprite attribute records (WSdev Display/Sprites): 4 bytes
    /// each, attribute word (tile, palette, window, priority, flips), Y, X.
    SpriteTable,
    Other,
}

impl Kind {
    pub const ALL: [Kind; 12] = [
        Kind::Tiles,
        Kind::Tilemap,
        Kind::Palette,
        Kind::Font,
        Kind::Text,
        Kind::PointerTable,
        Kind::Music,
        Kind::SoundEffect,
        Kind::Pcm,
        Kind::DataTable,
        Kind::SpriteTable,
        Kind::Other,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Kind::Tiles => "tiles",
            Kind::Tilemap => "tilemap",
            Kind::Palette => "palette",
            Kind::Font => "font",
            Kind::Text => "text",
            Kind::PointerTable => "pointer-table",
            Kind::Music => "music",
            Kind::SoundEffect => "sound-effect",
            Kind::Pcm => "pcm",
            Kind::DataTable => "data-table",
            Kind::SpriteTable => "sprite-table",
            Kind::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    High,
    #[default]
    Medium,
    Low,
}

/// Storage codec. Identifiers equal the owning crate's format identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Codec {
    #[default]
    #[serde(rename = "raw")]
    Raw,
    #[serde(rename = "ws-lz-rle-7f")]
    LzRle7f,
    #[serde(rename = "ws-lz-bits-16")]
    LzBits16,
    #[serde(rename = "ws-lz-u8-zero")]
    LzU8Zero,
    #[serde(rename = "ws-lz-word16-msb")]
    LzWord16Msb,
    #[serde(rename = "ws-lz-word-counted")]
    LzWordCounted,
    #[serde(rename = "ws-okumura-lzss")]
    Okumura,
    #[serde(rename = "ws-lzss-u16-zero")]
    LzssU16Zero,
    #[serde(rename = "ws-lzss-12")]
    Lzss12,
    #[serde(rename = "ws-lzs2")]
    Lzs2,
    #[serde(rename = "ws-rle-77ee")]
    Rle77ee,
    #[serde(rename = "ws-rle-pcm-00")]
    RlePcm00,
    #[serde(rename = "ws-counted-word-lz")]
    CountedWordLz,
    #[serde(rename = "ws-dpcm-6bit")]
    Dpcm6bit,
    #[serde(rename = "ws-lzss-11-5")]
    Lzss11_5,
    #[serde(rename = "ws-flag16-distance11")]
    Flag16Distance11,
    #[serde(rename = "ws-planar-rle")]
    PlanarRle,
    #[serde(rename = "ws-lzss-token-be")]
    LzssTokenBe,
    #[serde(rename = "ws-lzss-be-units")]
    LzssBeUnits,
    #[serde(rename = "ws-lzss-12-ext")]
    Lzss12Ext,
    #[serde(rename = "ws-nibble-lzss-ring8")]
    NibbleLzssRing8,
    #[serde(rename = "ws-byte-lzss-ring8")]
    ByteLzssRing8,
    #[serde(rename = "ws-zero-rle16")]
    ZeroRle16,
    #[serde(rename = "ws-lz-tile-7b")]
    LzTile7b,
    #[serde(rename = "ws-lz-command16-lsb")]
    LzCommand16Lsb,
    #[serde(rename = "ws-lzss-msb-distance12")]
    LzssMsbDistance12,
    #[serde(rename = "ws-mask16-fill")]
    Mask16Fill,
    #[serde(rename = "ws-dpcm-4bit-nibble")]
    Dpcm4bitNibble,
    #[serde(rename = "ws-lz-rle-pat")]
    LzRlePat,
    #[serde(rename = "ws-lzss-bandai-g")]
    LzssBandaiG,
    #[serde(rename = "ws-rle16-bit15")]
    Rle16Bit15,
    #[serde(rename = "ws-tile-xor-pack")]
    TileXorPack,
    #[serde(rename = "ws-lzss-11-5-direct")]
    Lzss11_5Direct,
    #[serde(rename = "ws-huffman-static")]
    HuffmanStatic,
    #[serde(rename = "ws-rle-lz4-cmd")]
    RleLz4Cmd,
    #[serde(rename = "ws-word-rle-00")]
    WordRle00,
}

impl Codec {
    /// Whether the stream carries its own end marker or decoded size.
    pub fn terminated(self) -> bool {
        matches!(
            self,
            Codec::LzRle7f
                | Codec::LzBits16
                | Codec::LzU8Zero
                | Codec::LzWord16Msb
                | Codec::LzWordCounted
                | Codec::LzssU16Zero
                | Codec::Lzs2
                | Codec::Rle77ee
                | Codec::CountedWordLz
                | Codec::Lzss11_5
                | Codec::Flag16Distance11
                | Codec::PlanarRle
                | Codec::LzssTokenBe
                | Codec::LzssBeUnits
                | Codec::NibbleLzssRing8
                | Codec::ByteLzssRing8
                | Codec::LzTile7b
                | Codec::LzCommand16Lsb
                | Codec::Mask16Fill
                | Codec::LzssBandaiG
                | Codec::TileXorPack
                | Codec::Lzss11_5Direct
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodecParams {
    /// Decoded size for codecs without a stored output size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_length: Option<Num>,
    /// Okumura flag bit order: `lsb` (default) or `msb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit_order: Option<String>,
    /// Okumura ring fill byte, default 0x20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ring_fill: Option<Num>,
    /// Original destination offset, used by destination-relative prehistory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<Num>,
    /// `low5-prefix`, `low8-prefix`, or `zero-until-start`; no implicit policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prehistory: Option<String>,
    /// Explicit bytes immediately preceding the destination, oldest first.
    /// An empty array declares that no prior bytes are available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<u8>>,
    /// Unit size in bytes for unit-framed codecs (`16` or `32`, default `16`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_size: Option<Num>,
    /// Source address width in bits for unit-framed codecs (`16` or `32`, default `16`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_width: Option<Num>,
    /// Plane count for planar-interleaved LZSS codecs (`1` or `2`, default `1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planes: Option<Num>,
    /// Optional prefix header bytes skipped before stream framing (e.g. `2` for 2-plane LZSS).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_bytes: Option<Num>,
    /// Inline static Huffman decode tree table bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<Vec<u8>>,
    /// ROM byte offset of an external static Huffman decode tree table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_offset: Option<Num>,
    /// Optional byte length of the static Huffman tree at `tree_offset` (default `1024`, max `4096`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_length: Option<Num>,
}

/// CPU view of the asset when loaded: purely descriptive.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Address {
    /// Window: `c0` (linear 40000-FFFFF), `c2` or `c3` (64 KiB data windows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    /// Bank register value selecting the window contents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bank: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Num>,
}

/// How the bytes stored at a reference encode an asset location.
/// `stored = ((anchor(target) + add) >> shift) mod 2^(8*width)`.
/// Bits above the stored width are implied by the referencing code (a bank or
/// segment held elsewhere); relocation must preserve them. Bits below `shift`
/// are supplied by a companion reference (an offset beside a bank number) or,
/// with `align`, must be zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Encoding {
    pub width: u8,
    #[serde(default)]
    pub endian: Endian,
    /// `file` (file offset) or `chip` (offset in the power-of-two chip, which
    /// equals the CPU address in the linear window for a full 1 MiB group).
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default)]
    pub add: Signed,
    #[serde(default)]
    pub shift: u8,
    /// Required target alignment (a segment-only pointer needs 16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Num>,
    /// Exclusive upper bound on the stored value that the referencing code
    /// accepts (a loader that switches banks only below some bank).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below: Option<Num>,
    /// The bits above `width` come from another reference of the same asset
    /// (a bank number beside this offset) that is rewritten with it, so they
    /// may change on relocation instead of being fixed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub companion: bool,
    /// `linear` (default) or `far`: a 4-byte offset16+segment16 pointer into a
    /// 64 KiB window starting at file offset `window` and seen at CPU segment
    /// `segment_base`; `normalize` is `paragraph` (offset < 16) or `segment`
    /// (segment fixed at `segment_base`).
    #[serde(default, skip_serializing_if = "is_linear")]
    pub form: Form,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_base: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalize: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Form {
    #[default]
    Linear,
    Far,
}

fn is_linear(f: &Form) -> bool {
    *f == Form::Linear
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Endian {
    #[default]
    Little,
    Big,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    #[default]
    File,
    Chip,
}

/// One place in the ROM that points at an asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    /// File offset of the stored value.
    pub at: Num,
    pub width: u8,
    #[serde(default)]
    pub endian: Endian,
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default)]
    pub add: Signed,
    #[serde(default)]
    pub shift: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Num>,
    /// See [`Encoding`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below: Option<Num>,
    /// The bits above `width` come from another reference of the same asset
    /// (a bank number beside this offset) that is rewritten with it, so they
    /// may change on relocation instead of being fixed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub companion: bool,
    /// Far form: see [`Encoding`].
    #[serde(default, skip_serializing_if = "is_linear")]
    pub form: Form,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_base: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalize: Option<String>,
    /// `pointer-table-entry`, `code-immediate`, `bank-number`, `segment` or `other`.
    #[serde(default = "default_ref_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Reference {
    pub fn encoding(&self) -> Encoding {
        Encoding {
            width: self.width,
            endian: self.endian,
            anchor: self.anchor,
            add: self.add,
            shift: self.shift,
            align: self.align,
            below: self.below,
            companion: self.companion,
            form: self.form,
            segment_base: self.segment_base,
            window: self.window,
            normalize: self.normalize.clone(),
        }
    }
}

fn default_ref_kind() -> String {
    "other".into()
}

/// One decoded asset's bytes may hold several tile sets placed in VRAM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileRef {
    pub id: String,
    /// VRAM tile index of the asset's first tile.
    #[serde(default)]
    pub at: Num,
    /// First tile of the asset to use (skips leading tiles).
    #[serde(default)]
    pub skip: Num,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relations {
    /// Tilemap: tile sets in VRAM order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiles: Vec<TileRef>,
    /// Tiles / font preview colours: a palette asset id or an inline palette name
    /// (with `format.palette_index` choosing the hardware palette).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<String>,
    /// Tilemap: palette assets (or inline palettes) loaded for the scene, each
    /// placed at its own `format.palette_index`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub palettes: Vec<String>,
    /// Text: named character map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charmap: Option<String>,
    /// Text and data: the pointer table indexing this asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer_table: Option<String>,
    /// Text: the font asset that renders it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Pointer table: target asset ids by entry (null for unmapped targets).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<Option<String>>,
}

/// Kind-specific format parameters; unused fields must be absent.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Format {
    /// Tiles / font: bits per pixel (1, 2 or 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpp: Option<u8>,
    /// Tiles: `planar` (default) or `packed` for 4bpp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// Tiles preview / tilemap / font sheet: columns or cells wide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Num>,
    /// Tilemap: cells high.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Num>,
    /// Tilemap: `ws-word` (hardware screen word, default), `byte` (tile index)
    /// or `index12-flip` (12-bit tile index, bit 14/15 flips, palette from
    /// `palette_index`; an engine's ROM map before VRAM slot assignment).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// Tilemap: added to each entry's tile number (`byte` entries) or OR-ed attributes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile_base: Option<Num>,
    /// Tilemap byte entries: attribute bits (palette/bank/flip) of every cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<Num>,
    /// Tilemap: honour the colour-mode tile bank bit (default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<bool>,
    /// Palette `mono-lut`: the eight shade levels (ports 1C-1F, 0 = white,
    /// 15 = black); default linear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shades: Option<Vec<u8>>,
    /// Palette: `rgb444` (colour RAM words) or `mono-lut` (port 20-3F pairs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette_format: Option<String>,
    /// Palette / tilemap: hardware palette index of the asset's first palette,
    /// or the palette used for previews of tiles and fonts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette_index: Option<Num>,
    /// Font: glyph cell geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiles_wide: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiles_high: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    /// Font: explicit tile byte offsets inside a glyph record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile_offsets: Option<Vec<Num>>,
    /// Font / pointer table / data table: record count and stride.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stride: Option<Num>,
    /// Pointer table: entry encoding (relative to entry start: `at` is ignored).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<Encoding>,
    /// Pointer table: byte offset of the pointer inside each record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer_offset: Option<Num>,
    /// PCM: sample rate in hertz (8-bit samples).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    /// PCM: sample encoding (`u8` default, or `s8` for signed 8-bit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_format: Option<String>,
    /// In-place replacement padding byte for the unused tail of the slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<Num>,
    /// Text: widest displayed line in pixels (glyph widths from the font).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_width: Option<u32>,
    /// Text: most lines on one page (box).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u32>,
    /// Text: decoded spelling of the control fragment that starts a new page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_break: Option<String>,
    /// Font: pixels a glyph advances (default `tiles_wide` * 8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advance: Option<u32>,
    /// Tiles / tilemap: a size header stored before the payload and part of
    /// the asset: `u16-count` (little-endian tile count) or
    /// `u16-width-height` (little-endian cells wide, cells high). Decoding
    /// checks it; replacement rewrites it, so tiles may change count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// Tiles: tile indices of the decoded payload the game never draws
    /// (declared with evidence); `replace --allocate` fills them with new tiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_tiles: Option<Vec<TileRange>>,
    /// Font: advances of glyphs that differ from `advance`, by decoded spelling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph_widths: Option<BTreeMap<String, u32>>,
}

/// A run of tile indices inside a tiles asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileRange {
    pub first: Num,
    pub count: Num,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// Loading routine (CPU address or file offset) that reads the asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
    /// Trace or capture showing the bytes reaching VRAM, RAM or sound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub id: String,
    pub kind: Kind,
    /// File offset of the stored (possibly compressed) bytes.
    pub offset: Num,
    /// Stored length; absent means codec-terminated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<Num>,
    /// Proven total allocation from offset, independent of consumed length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved_capacity: Option<Num>,
    #[serde(default)]
    pub codec: Codec,
    #[serde(default, skip_serializing_if = "is_default")]
    pub codec_params: CodecParams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub format: Format,
    #[serde(default, skip_serializing_if = "is_default")]
    pub relations: Relations,
    /// Every place pointing at this asset that is known (pointer table entries
    /// listed through a pointer-table asset are added automatically).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Reference>,
    /// Declares that `references` (plus pointer tables) list every pointer:
    /// required before relocation.
    #[serde(default)]
    pub references_complete: bool,
    /// Assets whose byte ranges may overlap this one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlaps: Vec<String>,
    #[serde(default)]
    pub confidence: Confidence,
    #[serde(default, skip_serializing_if = "is_default")]
    pub evidence: Evidence,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomIdentity {
    pub sha256: String,
    pub size: Num,
    /// Free label (title, revision) for people; never interpreted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreeRange {
    pub offset: Num,
    pub length: Num,
    /// Byte every free position holds in the original ROM, if uniform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Num>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Named character map: a builtin text definition's map, a file, or inline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharmapSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<String>,
    /// Path relative to the map document: charmap JSON, text-def JSON or TSV.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<serde_json::Value>,
    /// TSV code unit: `byte`, `word-le` or `word-be`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// A composed screen: tilemap layers drawn back to front over a back colour,
/// with sprite tables between and above them (WSdev Display: screen 1,
/// low-priority sprites, screen 2, high-priority sprites).
/// Positions are pixels on the 224x144 display (after scrolling).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Screen {
    pub id: String,
    pub layers: Vec<Layer>,
    /// Sprite tables in sprite-RAM order (earlier records win overlaps).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sprites: Vec<SpriteLayer>,
    /// Sprite window (port 0C-0F, inclusive) when the display enables it:
    /// each sprite draws only inside or only outside it, per its attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite_window: Option<Window>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub tilemap: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    /// Hardware screen (1 or 2): low-priority sprites draw above screen 1
    /// and below screen 2. Absent means screen 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<u8>,
    /// Positions wrap around the 256x256 screen plane, as hardware scrolling
    /// does: give `x`/`y` as the map position minus the scroll register.
    #[serde(default, skip_serializing_if = "is_false")]
    pub wrap: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One sprite table placed in a screen; `x`/`y` are added to every record
/// (modulo 256, like the hardware coordinates).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpriteLayer {
    pub table: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
}

/// Inclusive pixel rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub x0: u8,
    pub y0: u8,
    pub x1: u8,
    pub y1: u8,
}

/// Inline palette for monochrome games whose shades are set by code
/// immediates (not by ROM data): RGB triples as `#rrggbb` strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InlinePalette {
    pub colors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetMap {
    pub schema: String,
    pub rom: RomIdentity,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub charmaps: BTreeMap<String, CharmapSource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub palettes: BTreeMap<String, InlinePalette>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub free_space: Vec<FreeRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Coverage>,
    pub assets: Vec<Asset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screens: Vec<Screen>,
}

impl AssetMap {
    /// Parse and check structure that does not need the ROM.
    /// Accepts both `swankit-asset-map/1` and `formatkit.asset-map.v1`.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_MAP_BYTES {
            return Err(malformed("asset map exceeds 64 MiB"));
        }
        let peek: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| malformed(e.to_string()))?;
        let schema = peek
            .get("schema")
            .and_then(|v| v.as_str())
            .ok_or_else(|| malformed("asset map missing string \"schema\" field"))?;
        match schema {
            SCHEMA => {
                let map: Self =
                    serde_json::from_slice(bytes).map_err(|e| malformed(e.to_string()))?;
                map.check_structure()?;
                Ok(map)
            }
            crate::formatkit::FORMATKIT_SCHEMA => {
                let doc = crate::formatkit::FormatkitAssetMapV1::from_json(bytes)?;
                doc.to_asset_map()
            }
            other => Err(malformed(format!("unsupported schema {other:?}"))),
        }
    }

    pub fn to_json(&self) -> Result<Vec<u8>> {
        let mut out = serde_json::to_vec_pretty(self).map_err(|e| malformed(e.to_string()))?;
        out.push(b'\n');
        Ok(out)
    }

    pub fn asset(&self, id: &str) -> Result<&Asset> {
        self.assets
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| malformed(format!("unknown asset id {id:?}")))
    }

    pub(crate) fn check_structure(&self) -> Result<()> {
        if self.schema != SCHEMA {
            return Err(malformed(format!("unsupported schema {:?}", self.schema)));
        }
        if self.rom.sha256.len() != 64 || !self.rom.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(malformed("rom.sha256 must be 64 hexadecimal digits"));
        }
        if self.assets.len() > MAX_ASSETS {
            return Err(malformed("too many assets"));
        }
        let mut ids = BTreeSet::new();
        for a in &self.assets {
            if a.id.is_empty()
                || a.id.len() > 128
                || !a
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            {
                return Err(malformed(format!(
                    "asset id {:?} must be 1-128 of [A-Za-z0-9._-]",
                    a.id
                )));
            }
            if !ids.insert(a.id.as_str()) {
                return Err(malformed(format!("duplicate asset id {:?}", a.id)));
            }
            if a.length.is_none() && !a.codec.terminated() && a.kind != Kind::Text {
                return Err(malformed(format!(
                    "{}: length may be omitted only for terminated codecs or text",
                    a.id
                )));
            }
            if a.codec == Codec::Okumura && a.codec_params.decoded_length.is_none() {
                return Err(malformed(format!("{}: okumura needs decoded_length", a.id)));
            }
            crate::data::validate_codec_params(a)?;
            for r in &a.references {
                check_encoding(&r.encoding(), &a.id)?;
            }
            if a.references.iter().any(|r| r.companion)
                && !a.references.iter().any(|r| !r.companion && r.shift > 0)
            {
                return Err(malformed(format!(
                    "{}: a companion reference needs a shifted reference (the bank) beside it",
                    a.id
                )));
            }
            if let Some(p) = &a.format.pointer {
                check_encoding(p, &a.id)?;
            }
        }
        for a in &self.assets {
            let named = a
                .relations
                .tiles
                .iter()
                .map(|t| Some(t.id.as_str()))
                .chain([
                    a.relations.pointer_table.as_deref(),
                    a.relations.font.as_deref(),
                ])
                .chain(a.relations.targets.iter().map(|t| t.as_deref()))
                .chain(a.overlaps.iter().map(|s| Some(s.as_str())))
                .flatten();
            for id in named {
                if !ids.contains(id) {
                    return Err(malformed(format!(
                        "{}: relation to unknown id {id:?}",
                        a.id
                    )));
                }
            }
            for p in a.relations.palette.iter().chain(&a.relations.palettes) {
                if !ids.contains(p.as_str()) && !self.palettes.contains_key(p) {
                    return Err(malformed(format!("{}: unknown palette {p:?}", a.id)));
                }
            }
            if let Some(c) = &a.relations.charmap {
                if !self.charmaps.contains_key(c) {
                    return Err(malformed(format!("{}: unknown charmap {c:?}", a.id)));
                }
            }
        }
        for screen in &self.screens {
            if ids.contains(screen.id.as_str()) {
                return Err(malformed(format!(
                    "screen id {:?} repeats an asset id",
                    screen.id
                )));
            }
            for layer in &screen.layers {
                if layer.screen.is_some_and(|n| !(1..=2).contains(&n)) {
                    return Err(malformed(format!(
                        "screen {}: layer screen must be 1 or 2",
                        screen.id
                    )));
                }
                if !self
                    .assets
                    .iter()
                    .any(|a| a.id == layer.tilemap && a.kind == Kind::Tilemap)
                {
                    return Err(malformed(format!(
                        "screen {}: {:?} is not a tilemap",
                        screen.id, layer.tilemap
                    )));
                }
            }
            for s in &screen.sprites {
                if !self
                    .assets
                    .iter()
                    .any(|a| a.id == s.table && a.kind == Kind::SpriteTable)
                {
                    return Err(malformed(format!(
                        "screen {}: {:?} is not a sprite table",
                        screen.id, s.table
                    )));
                }
            }
            if let Some(c) = &screen.back_color {
                parse_rgb(c)?;
            }
        }
        for (name, p) in &self.palettes {
            if p.colors.is_empty() || p.colors.len() > 256 {
                return Err(malformed(format!("palette {name}: 1-256 colours")));
            }
            for c in &p.colors {
                parse_rgb(c)?;
            }
        }
        Ok(())
    }
}

impl std::str::FromStr for AssetMap {
    type Err = formatkit_core::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::from_json(s.as_bytes())
    }
}

fn check_encoding(e: &Encoding, id: &str) -> Result<()> {
    if !(1..=4).contains(&e.width) || e.shift > 24 {
        return Err(malformed(format!("{id}: reference width 1-4, shift 0-24")));
    }
    if e.form == Form::Far
        && (e.width != 4
            || e.segment_base.is_none()
            || e.window.is_none()
            || !matches!(e.normalize.as_deref(), Some("paragraph" | "segment")))
    {
        return Err(malformed(format!(
            "{id}: far references are 4 bytes with segment_base, window and normalize paragraph|segment"
        )));
    }
    Ok(())
}

/// Parse `#rrggbb`.
pub fn parse_rgb(s: &str) -> Result<[u8; 4]> {
    let hex = s
        .strip_prefix('#')
        .filter(|h| h.len() == 6)
        .ok_or_else(|| malformed(format!("colour {s:?} must be #rrggbb")))?;
    let v = u32::from_str_radix(hex, 16).map_err(|_| malformed(format!("colour {s:?}")))?;
    Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255])
}
