//! Product extraction definitions select owner grammars without byte guessing.
use crate::{compress, gfx, sound, text};
use formatkit_core::{bytes_at, Error, Image, Reader, Result};
use serde::Deserialize;
use serde_json::{json, Value};

pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_DEFINITION_BYTES: usize = 4 * 1024 * 1024;

pub fn definition<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.len() > MAX_DEFINITION_BYTES {
        return Err(bad("definition exceeds 4 MiB"));
    }
    serde_json::from_slice(bytes).map_err(|e| bad(format!("definition JSON: {e}")))
}
fn bad(s: impl Into<String>) -> Error {
    Error::Malformed(s.into())
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Codec {
    #[serde(rename = "lz-rle-7f")]
    LzRle7f,
    #[serde(rename = "lz-bits-16")]
    LzBits16,
    LzU8Zero,
    LzWord16Msb,
    LzWordCounted,
    OkumuraLzss,
    #[serde(rename = "lzss-u16-zero")]
    LzssU16Zero,
    #[serde(rename = "lzss-12")]
    Lzss12,
    #[serde(rename = "lzss-12-ext")]
    Lzss12Ext,
    #[serde(rename = "flag16-distance11")]
    Flag16Distance11,
    #[serde(rename = "mask16-fill")]
    Mask16Fill,
    #[serde(rename = "lz-command16-lsb")]
    LzCommand16Lsb,
    #[serde(rename = "lzss-msb-distance12")]
    LzssMsbDistance12,
    #[serde(rename = "rle-77ee")]
    Rle77ee,
    #[serde(rename = "rle-pcm-00")]
    RlePcm00,
    #[serde(rename = "counted-word-lz")]
    CountedWordLz,
    #[serde(rename = "dpcm-6bit")]
    Dpcm6bit,
    #[serde(rename = "lzss-11-5")]
    Lzss11_5,
    #[serde(rename = "planar-rle")]
    PlanarRle,
    #[serde(rename = "lzss-token-be")]
    LzssTokenBe,
    #[serde(rename = "nibble-lzss-ring8")]
    NibbleLzssRing8,
    #[serde(rename = "byte-lzss-ring8")]
    ByteLzssRing8,
    #[serde(rename = "zero-rle16")]
    ZeroRle16,
    #[serde(rename = "lz-tile-7b")]
    LzTile7b,
    #[serde(rename = "dpcm-4bit-nibble")]
    Dpcm4bitNibble,
    #[serde(rename = "lz-rle-pat")]
    LzRlePat,
    #[serde(rename = "lzss-bandai-g")]
    LzssBandaiG,
    #[serde(rename = "rle16-bit15")]
    Rle16Bit15,
    #[serde(rename = "tile-xor-pack")]
    TileXorPack,
    #[serde(rename = "lzss-11-5-direct")]
    Lzss11_5Direct,
    #[serde(rename = "huffman-static")]
    HuffmanStatic,
    #[serde(rename = "rle-lz4-cmd")]
    RleLz4Cmd,
    #[serde(rename = "word-rle-00")]
    WordRle00,
    #[serde(skip_deserializing)]
    LzssBeUnits(TokenParameters),
}
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Prehistory {
    #[default]
    Low5Prefix,
    Low8Prefix,
    ZeroUntilStart,
}
fn default_destination() -> u16 {
    0x2000
}
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BitOrder {
    #[default]
    LsbFirst,
    MsbFirst,
}
impl BitOrder {
    fn owner(self) -> compress::BitOrder {
        match self {
            Self::LsbFirst => compress::BitOrder::LsbFirst,
            Self::MsbFirst => compress::BitOrder::MsbFirst,
        }
    }
}
#[derive(Debug)]
pub struct CodecDefinition {
    pub schema: String,
    pub codec: Codec,
    pub offset: usize,
    pub length: usize,
    pub max_output: usize,
    pub decoded_size: Option<usize>,
    pub bit_order: BitOrder,
    pub fill: u8,
    pub history: Vec<u8>,
    pub destination: u16,
    pub prehistory: Prehistory,
    pub tree: Vec<u8>,
    pub tree_offset: Option<usize>,
    pub tree_length: Option<usize>,
}

/// Parameters are carried by the new codec variant to preserve legacy Rust literals.
#[derive(Debug, Clone, Copy)]
pub struct TokenParameters {
    pub unit_size: u8,
    pub source_width: u8,
    pub original_sha256: [u8; 32],
}
#[derive(Default)]
struct Parameter<T> {
    present: bool,
    value: Option<T>,
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Parameter<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(Self {
            present: true,
            value: Option::<T>::deserialize(d)?,
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodecWire {
    schema: String,
    codec: String,
    #[serde(default)]
    offset: usize,
    length: usize,
    max_output: usize,
    #[serde(default)]
    decoded_size: Option<usize>,
    #[serde(default)]
    bit_order: BitOrder,
    #[serde(default)]
    fill: u8,
    #[serde(default)]
    history: Vec<u8>,
    #[serde(default = "default_destination")]
    destination: u16,
    #[serde(default)]
    prehistory: Prehistory,
    #[serde(default)]
    tree: Vec<u8>,
    #[serde(default)]
    tree_offset: Option<usize>,
    #[serde(default)]
    tree_length: Option<usize>,
    #[serde(default)]
    unit_size: Parameter<u8>,
    #[serde(default)]
    source_width: Parameter<u8>,
    #[serde(default)]
    original_sha256: Parameter<String>,
}
impl<'de> Deserialize<'de> for CodecDefinition {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;
        let w = CodecWire::deserialize(deserializer)?;
        let has_parameters =
            w.unit_size.present || w.source_width.present || w.original_sha256.present;
        let codec = if w.codec == "lzss-be-units" {
            let unit_size = w
                .unit_size
                .value
                .ok_or_else(|| D::Error::custom("token.missing-unit"))?;
            let source_width = w
                .source_width
                .value
                .ok_or_else(|| D::Error::custom("token.missing-width"))?;
            if !matches!(unit_size, 16 | 32) {
                return Err(D::Error::custom("token.invalid-unit"));
            }
            if !matches!(source_width, 16 | 32) {
                return Err(D::Error::custom("token.invalid-width"));
            }
            let sha = w
                .original_sha256
                .value
                .as_deref()
                .ok_or_else(|| D::Error::custom("token.missing-identity"))?;
            if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(D::Error::custom("token.invalid-identity"));
            }
            let mut original_sha256 = [0; 32];
            for (i, b) in original_sha256.iter_mut().enumerate() {
                *b = u8::from_str_radix(&sha[2 * i..2 * i + 2], 16).map_err(D::Error::custom)?;
            }
            Codec::LzssBeUnits(TokenParameters {
                unit_size,
                source_width,
                original_sha256,
            })
        } else {
            if has_parameters {
                return Err(D::Error::custom("token.parameters-on-other-codec"));
            }
            Codec::deserialize(Value::String(w.codec)).map_err(D::Error::custom)?
        };
        Ok(Self {
            schema: w.schema,
            codec,
            offset: w.offset,
            length: w.length,
            max_output: w.max_output,
            decoded_size: w.decoded_size,
            bit_order: w.bit_order,
            fill: w.fill,
            history: w.history,
            destination: w.destination,
            prehistory: w.prehistory,
            tree: w.tree,
            tree_offset: w.tree_offset,
            tree_length: w.tree_length,
        })
    }
}
/// Stable refusal codes for the selected unit-carrier operation.
pub fn token_refusal_reason(error: &Error) -> &'static str {
    match error {
        Error::Truncated { .. } => "truncated-input",
        Error::ResourceLimit { .. } => "output-limit",
        Error::Malformed(s) => {
            for (text, code) in [
                ("identity-mismatch", "identity-mismatch"),
                ("source-width-overflow", "source-width-overflow"),
                ("zero-header", "zero-header"),
                ("zero-distance", "zero-distance"),
                ("unknown-history", "unknown-history"),
                ("token-overshoot", "token-overshoot"),
            ] {
                if s.contains(text) {
                    return code;
                }
            }
            "invalid-parameters"
        }
        _ => "invalid-input",
    }
}
impl CodecDefinition {
    pub fn decode(&self, bytes: &[u8]) -> Result<compress::DecodeOutcome> {
        if self.schema != "swankit-codec-def/1" {
            return Err(bad("unknown codec definition schema"));
        }
        let output_limit = match self.codec {
            Codec::Rle77ee
            | Codec::RlePcm00
            | Codec::Dpcm6bit
            | Codec::ZeroRle16
            | Codec::Dpcm4bitNibble
            | Codec::Rle16Bit15
            | Codec::RleLz4Cmd
            | Codec::WordRle00 => compress::MAX_STREAM_DECODED,
            _ => compress::MAX_DECODED,
        };
        if self.max_output > output_limit || self.history.len() > compress::MAX_DECODED {
            return Err(bad("codec output/history exceeds limit"));
        }
        let source = bytes_at(bytes, self.offset, self.length)?;
        match self.codec {
            Codec::LzssBeUnits(p) => {
                if !self.history.is_empty()
                    || self.destination != default_destination()
                    || !matches!(self.prehistory, Prehistory::Low5Prefix)
                    || self.fill != 0
                    || !matches!(self.bit_order, BitOrder::LsbFirst)
                {
                    return Err(bad("token.unsupported-history-or-flags"));
                }
                let expected = p
                    .original_sha256
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>();
                if ws_assets::sha256_hex(bytes) != expected {
                    return Err(bad("token.identity-mismatch"));
                }
                let got = compress::lzss_be_units::decode(
                    source,
                    p.unit_size,
                    p.source_width,
                    self.max_output,
                )?;
                if self.decoded_size.is_some_and(|n| n != got.bytes.len()) {
                    return Err(bad("token.decoded-size-mismatch"));
                }
                Ok(got)
            }
            Codec::LzWordCounted => {
                if !self.history.is_empty() {
                    return Err(bad("counted word streams require produced history"));
                }
                compress::lz_word_counted::decode_at(source, self.max_output, self.offset)
            }
            Codec::LzWord16Msb => {
                if !self.history.is_empty() {
                    return Err(bad("word-flag streams require produced-byte history"));
                }
                compress::lz_word16_msb::decode(source, self.max_output)
            }
            Codec::LzRle7f => {
                compress::lz_rle_7f::decode_with_history(source, &self.history, self.max_output)
            }
            Codec::LzBits16 => {
                compress::lz_bits_16::decode_with_history(source, &self.history, self.max_output)
            }
            Codec::LzU8Zero => {
                compress::lz_u8_zero::decode_with_history(source, &self.history, self.max_output)
            }
            Codec::OkumuraLzss => {
                if !self.history.is_empty() {
                    return Err(bad("Okumura uses ring fill, not external history"));
                }
                compress::okumura::decode(
                    source,
                    self.decoded_size
                        .ok_or_else(|| bad("Okumura requires decoded_size"))?,
                    self.bit_order.owner(),
                    self.fill,
                    self.max_output,
                )
            }
            Codec::LzssU16Zero => {
                if !self.history.is_empty() {
                    return Err(bad("length-prefixed LZSS uses its prehistory policy"));
                }
                let policy = match self.prehistory {
                    Prehistory::Low5Prefix => compress::lzss_u16_zero::Prehistory::Low5Prefix,
                    Prehistory::Low8Prefix => compress::lzss_u16_zero::Prehistory::Low8Prefix,
                    Prehistory::ZeroUntilStart => {
                        compress::lzss_u16_zero::Prehistory::ZeroUntilStart
                    }
                };
                compress::lzss_u16_zero::decode_with_policy(
                    source,
                    self.max_output,
                    self.destination,
                    policy,
                )
            }
            Codec::Lzss12Ext => compress::lzss_12_ext::decode_with_history(
                source,
                &self.history,
                self.decoded_size
                    .ok_or_else(|| bad("extended-length LZSS requires decoded_size"))?,
                self.max_output,
            ),
            Codec::Flag16Distance11 => {
                if let Some(size) = self.decoded_size {
                    let mut header = Reader::new(source);
                    header.u8()?;
                    let declared = usize::from(header.u16_le()?);
                    if size != declared {
                        return Err(bad("declared output size disagrees with codec header"));
                    }
                }
                compress::flag16_distance11::decode_with_history(
                    source,
                    &self.history,
                    self.max_output,
                )
            }
            Codec::Mask16Fill => {
                if !self.history.is_empty() {
                    return Err(bad("mask16-fill has no history input"));
                }
                let declared = compress::mask16_fill::decoded_size(source)?;
                if self.decoded_size.is_some_and(|size| size != declared) {
                    return Err(bad(
                        "declared output size disagrees with mask16-fill header",
                    ));
                }
                compress::mask16_fill::decode(source, self.max_output)
            }
            Codec::LzCommand16Lsb => {
                let decoded = compress::lz_command16_lsb::decode_with_history(
                    source,
                    &self.history,
                    self.max_output,
                )?;
                if self
                    .decoded_size
                    .is_some_and(|size| size != decoded.bytes.len())
                {
                    return Err(bad(
                        "declared output size disagrees with decoded command stream",
                    ));
                }
                Ok(decoded)
            }
            Codec::LzssMsbDistance12 => compress::lzss_msb_distance12::decode_with_history(
                source,
                &self.history,
                self.decoded_size
                    .ok_or_else(|| bad("MSB distance-12 LZSS requires decoded_size"))?,
                self.max_output,
            ),
            Codec::Lzss12 => compress::lzss_12::decode_with_history(
                source,
                &self.history,
                self.decoded_size
                    .ok_or_else(|| bad("external-length LZSS requires decoded_size"))?,
                self.max_output,
            ),
            Codec::Rle77ee => {
                if !self.history.is_empty() {
                    return Err(bad("rle-77ee has no history input"));
                }
                compress::rle_77ee::decode(source, self.max_output)
            }
            Codec::RlePcm00 => {
                if !self.history.is_empty() {
                    return Err(bad("rle-pcm-00 has no history input"));
                }
                compress::rle_pcm_00::decode(source, self.max_output)
            }
            Codec::CountedWordLz => {
                if !self.history.is_empty() {
                    return Err(bad("counted-word-lz has no history input"));
                }
                compress::counted_word_lz::decode(source, self.max_output)
            }
            Codec::Dpcm6bit => {
                if !self.history.is_empty() {
                    return Err(bad("dpcm-6bit has no history input"));
                }
                compress::dpcm_6bit::decode(source, self.max_output)
            }
            Codec::Lzss11_5 => {
                if !self.history.is_empty() {
                    return Err(bad("lzss-11-5 has no history input"));
                }
                compress::lzss_11_5::decode(source, self.max_output)
            }
            Codec::PlanarRle => {
                if !self.history.is_empty() {
                    return Err(bad("planar-rle has no history input"));
                }
                compress::planar_rle::decode(source, self.max_output)
            }
            Codec::LzssTokenBe => {
                if !self.history.is_empty() {
                    return Err(bad("lzss-token-be has no history input"));
                }
                compress::lzss_token_be::decode(source, self.max_output)
            }
            Codec::NibbleLzssRing8 => {
                if !self.history.is_empty() {
                    return Err(bad("nibble-lzss-ring8 has no history input"));
                }
                compress::nibble_lzss_ring8::decode(source, self.max_output)
            }
            Codec::ByteLzssRing8 => {
                if !self.history.is_empty() {
                    return Err(bad("byte-lzss-ring8 has no history input"));
                }
                compress::byte_lzss_ring8::decode(source, self.max_output)
            }
            Codec::ZeroRle16 => {
                if !self.history.is_empty() {
                    return Err(bad("zero-rle16 has no history input"));
                }
                compress::zero_rle16::decode(source, self.max_output)
            }
            Codec::LzTile7b => {
                if !self.history.is_empty() {
                    return Err(bad("lz-tile-7b has no history input"));
                }
                compress::lz_tile_7b::decode(source, self.max_output)
            }
            Codec::Dpcm4bitNibble => {
                if !self.history.is_empty() {
                    return Err(bad("dpcm-4bit-nibble has no history input"));
                }
                compress::dpcm_4bit_nibble::decode(source, self.max_output)
            }
            Codec::LzRlePat => {
                if !self.history.is_empty() {
                    return Err(bad("lz-rle-pat has no history input"));
                }
                compress::lz_rle_pat::decode(
                    source,
                    self.decoded_size
                        .ok_or_else(|| bad("lz-rle-pat requires decoded_size"))?,
                    self.max_output,
                )
            }
            Codec::LzssBandaiG => {
                if !self.history.is_empty() {
                    return Err(bad("lzss-bandai-g has no history input"));
                }
                compress::lzss_bandai_g::decode(source, self.max_output)
            }
            Codec::Rle16Bit15 => {
                if !self.history.is_empty() {
                    return Err(bad("rle16-bit15 has no history input"));
                }
                compress::rle16_bit15::decode(source, self.max_output)
            }
            Codec::TileXorPack => {
                if !self.history.is_empty() {
                    return Err(bad("tile-xor-pack has no history input"));
                }
                compress::tile_xor_pack::decode(source, self.max_output)
            }
            Codec::Lzss11_5Direct => {
                if !self.history.is_empty() {
                    return Err(bad("lzss-11-5-direct has no history input"));
                }
                compress::lzss_11_5_direct::decode(source, self.max_output)
            }
            Codec::HuffmanStatic => {
                if !self.history.is_empty() {
                    return Err(bad("huffman-static has no history input"));
                }
                let tree_slice: &[u8] = if !self.tree.is_empty() {
                    &self.tree
                } else if let Some(off) = self.tree_offset {
                    let len = self.tree_length.unwrap_or(1024);
                    if off + 2 > bytes.len() {
                        return Err(bad("huffman-static tree_offset out of bounds"));
                    }
                    let end = off.saturating_add(len).min(bytes.len());
                    &bytes[off..end]
                } else {
                    return Err(bad("huffman-static requires tree or tree_offset"));
                };
                compress::huffman_static::decode(
                    source,
                    tree_slice,
                    self.decoded_size,
                    self.max_output,
                )
            }
            Codec::RleLz4Cmd => {
                if !self.history.is_empty() {
                    return Err(bad("rle-lz4-cmd has no history input"));
                }
                compress::rle_lz4_cmd::decode(source, self.max_output)
            }
            Codec::WordRle00 => {
                if !self.history.is_empty() {
                    return Err(bad("word-rle-00 has no history input"));
                }
                compress::word_rle_00::decode(source, self.decoded_size, self.max_output)
            }
        }
    }
}

impl CodecDefinition {
    pub(crate) fn provenance(
        &self,
        bytes: &[u8],
        decoded: &compress::DecodeOutcome,
    ) -> Result<Option<Value>> {
        let Codec::LzssBeUnits(p) = self.codec else {
            return Ok(None);
        };
        let source = bytes_at(bytes, self.offset, self.length)?;
        let n = compress::lzss_be_units::normalize(
            source,
            p.unit_size,
            p.source_width,
            self.max_output,
        )?;
        if decoded.consumed != n.stored_consumed {
            return Err(bad("token.extent-mismatch"));
        }
        Ok(Some(
            json!({"schema":"swankit-codec-extraction/1","status":"decoded","stored_grammar":"lzss-be-units","adapter":"be-direct-distance-to-lzss12/1","normalized_codec":"lzss-12","original":{"bytes":bytes.len(),"sha256":ws_assets::sha256_hex(bytes)},"parameters":{"offset":self.offset,"input_cap":self.length,"unit_size":p.unit_size,"source_width":p.source_width,"max_output":self.max_output,"history":"empty","final_token":"reject-overshoot"},"header_count":n.header_count,"stored":{"consumed":decoded.consumed,"sha256":ws_assets::sha256_hex(&source[..decoded.consumed])},"normalized":{"consumed":n.bytes.len(),"sha256":ws_assets::sha256_hex(&n.bytes)},"decoded":{"bytes":decoded.bytes.len(),"sha256":ws_assets::sha256_hex(&decoded.bytes)}}),
        ))
    }
}

/// Dump tables without losing source tokens, controls or normalization details.
pub fn text_table(bytes: &[u8], def: &text::TextDefinition, table: &str) -> Result<Value> {
    let selected = def
        .tables
        .get(table)
        .ok_or_else(|| bad("unknown text table"))?;
    let strings = selected.dump(
        bytes,
        &def.character_map()?,
        text::TextLimits::default(),
        65536,
    )?;
    Ok(
        json!({"definition": def.id, "table": table, "strings": strings.iter().map(|s| json!({
        "offset": s.offset, "text": s.decoded.text, "normalized": s.decoded.normalized,
        "consumed": s.decoded.consumed, "terminated": s.decoded.terminated,
        "final_page": s.decoded.final_page,
        "tokens": s.decoded.tokens.iter().map(|t| json!({"bytes": t.bytes, "text": t.text})).collect::<Vec<_>>()
    })).collect::<Vec<_>>() }),
    )
}

pub fn grayscale(colors: usize) -> Result<Vec<[u8; 4]>> {
    if !matches!(colors, 2 | 4 | 16) {
        return Err(bad("grayscale palette requires 2, 4 or 16 colors"));
    }
    let mut palette = formatkit_pixel::grayscale_palette(colors);
    palette.reverse();
    Ok(palette)
}

pub fn font_atlas(
    bytes: &[u8],
    def: &text::TextDefinition,
    font: &str,
    columns: u32,
) -> Result<Image> {
    let font = def.fonts.get(font).ok_or_else(|| bad("unknown font"))?;
    if font.count == 0 || columns == 0 {
        return Err(bad("font count and columns must be positive"));
    }
    let width = font
        .tiles_wide
        .checked_mul(8)
        .and_then(|n| n.checked_mul(columns))
        .ok_or_else(|| bad("font width overflow"))?;
    let rows = u32::try_from(font.count.div_ceil(columns as usize))
        .map_err(|_| bad("font row overflow"))?;
    let height = font
        .tiles_high
        .checked_mul(8)
        .and_then(|n| n.checked_mul(rows))
        .ok_or_else(|| bad("font height overflow"))?;
    let mut rgba = Image::try_rgba_buffer(width, height, MAX_IMAGE_BYTES, "font atlas RGBA bytes")?;
    rgba.resize(Image::rgba_len(width, height)?, 0);
    let palette = grayscale(match font.format {
        text::FontFormat::Bpp1 => 2,
        text::FontFormat::Bpp2 => 4,
        _ => 16,
    })?;
    for i in 0..font.count {
        let glyph = font.glyph(bytes, i, &palette, MAX_IMAGE_BYTES)?;
        let x = (i % columns as usize) * glyph.image.width as usize;
        let y = (i / columns as usize) * glyph.image.height as usize;
        let row_bytes = glyph.image.width as usize * 4;
        for row in 0..glyph.image.height as usize {
            let at = ((y + row) * width as usize + x) * 4;
            rgba[at..at + row_bytes]
                .copy_from_slice(&glyph.image.rgba[row * row_bytes..(row + 1) * row_bytes]);
        }
    }
    Image::try_new(width, height, rgba)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SoundFormat {
    Wave,
    UnsignedPcm,
    SignedPcm,
    SdkSequence,
    CellSequence,
    ByteSequence,
}
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ByteDialect {
    #[default]
    A,
    B,
    C,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundDefinition {
    #[serde(default)]
    pub sample_rate: Option<u32>,
    pub schema: String,
    pub format: SoundFormat,
    #[serde(default)]
    pub offset: usize,
    pub length: usize,
    #[serde(default)]
    pub dialect: ByteDialect,
    #[serde(default)]
    pub direct: bool,
    #[serde(default)]
    pub direct_divisor: bool,
    #[serde(default)]
    pub channel: u8,
    #[serde(default)]
    pub call_depth_bytes: u8,
}
impl SoundDefinition {
    pub fn extract(&self, bytes: &[u8]) -> Result<Value> {
        if self.schema != "swankit-sound-def/1" {
            return Err(bad("unknown sound definition schema"));
        }
        let source = bytes_at(bytes, self.offset, self.length)?;
        let mut r = Reader::new(source);
        let mut records = Vec::new();
        match self.format {
            SoundFormat::Wave => {
                if source.is_empty() || !source.len().is_multiple_of(sound::WAVE_BYTES) {
                    return Err(bad("wave extent must contain complete 16-byte waves"));
                }
                let waves = sound::instrument::wave_bank(source, source.len() / sound::WAVE_BYTES)?;
                return Ok(
                    json!({"format": "wave", "waves": waves.iter().map(|w| w.samples().to_vec()).collect::<Vec<_>>() }),
                );
            }
            SoundFormat::UnsignedPcm => {
                if source.len() > sound::render::MAX_FRAMES {
                    return Err(bad("PCM sample limit exceeded"));
                }
                let pcm = sound::render::unsigned_pcm(
                    source,
                    self.sample_rate
                        .ok_or_else(|| bad("unsigned PCM requires sample_rate"))?,
                    sound::render::MAX_FRAMES,
                )?;
                return Ok(
                    json!({"format": "unsigned-pcm", "sample_rate": pcm.sample_rate(), "channels": pcm.channels(), "samples": pcm.samples()}),
                );
            }
            SoundFormat::SignedPcm => {
                if source.len() > sound::render::MAX_FRAMES {
                    return Err(bad("PCM sample limit exceeded"));
                }
                let pcm = sound::render::signed_pcm(
                    source,
                    self.sample_rate
                        .ok_or_else(|| bad("signed PCM requires sample_rate"))?,
                    sound::render::MAX_FRAMES,
                )?;
                return Ok(
                    json!({"format": "signed-pcm", "sample_rate": pcm.sample_rate(), "channels": pcm.channels(), "samples": pcm.samples()}),
                );
            }
            _ => {}
        }
        while r.remaining() > 0 {
            if records.len() >= sound::drivers::MAX_RECORDS {
                return Err(bad("sound record limit exceeded"));
            }
            let at = r.pos();
            let (record, ended) = match self.format {
                SoundFormat::SdkSequence => {
                    let c = sound::drivers::sdk::Cell::parse(r.bytes(2)?)?;
                    let ended = matches!(c, sound::drivers::sdk::Cell::End);
                    (
                        serde_json::to_value(c).map_err(|e| bad(e.to_string()))?,
                        ended,
                    )
                }
                SoundFormat::CellSequence => {
                    let c = sound::drivers::cell::Cell::parse(r.bytes(4)?)?;
                    let ended = matches!(c, sound::drivers::cell::Cell::End);
                    (
                        serde_json::to_value(c).map_err(|e| bad(e.to_string()))?,
                        ended,
                    )
                }
                SoundFormat::ByteSequence => {
                    let dialect = match self.dialect {
                        ByteDialect::A => sound::drivers::byte::Dialect::A,
                        ByteDialect::B => sound::drivers::byte::Dialect::B,
                        ByteDialect::C => sound::drivers::byte::Dialect::C,
                    };
                    let c = sound::drivers::byte::instruction(
                        &mut r,
                        dialect,
                        sound::drivers::byte::Mode {
                            direct: self.direct,
                            direct_divisor: self.direct_divisor,
                            channel: self.channel,
                            call_depth_bytes: self.call_depth_bytes,
                        },
                    )?;
                    let ended = matches!(
                        c,
                        sound::drivers::byte::Instruction::End
                            | sound::drivers::byte::Instruction::DirectEnd { .. }
                    );
                    (
                        serde_json::to_value(c).map_err(|e| bad(e.to_string()))?,
                        ended,
                    )
                }
                _ => return Err(bad("invalid sequence selection")),
            };
            records.push(
                json!({"offset": self.offset + at, "consumed": r.pos() - at, "record": record}),
            );
            if ended {
                return Ok(json!({"records": records, "consumed": r.pos(), "terminated": true}));
            }
        }
        Err(bad(
            "sound sequence has no terminator in the selected extent",
        ))
    }
}

pub fn snapshot(bytes: &[u8], color: bool, profile: gfx::RenderProfile) -> Result<Image> {
    snapshot_with_line_ports(bytes, color, profile, None)
}

pub fn snapshot_with_line_ports(
    bytes: &[u8],
    color: bool,
    profile: gfx::RenderProfile,
    rows: Option<&[[u8; 256]]>,
) -> Result<Image> {
    let mut r = Reader::new(bytes);
    let ram = r.bytes(65536)?;
    let ports = r.bytes(256)?;
    if r.remaining() != 0 {
        return Err(bad(
            "snapshot must contain exactly 65536 RAM bytes and 256 ports",
        ));
    }
    let snapshot = gfx::Snapshot::new(ram, ports, color)?;
    if let Some(rows) = rows {
        snapshot.with_line_ports(rows)?.render(profile)
    } else {
        snapshot.render(profile)
    }
}
