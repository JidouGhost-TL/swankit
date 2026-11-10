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
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodecDefinition {
    pub schema: String,
    pub codec: Codec,
    #[serde(default)]
    pub offset: usize,
    pub length: usize,
    pub max_output: usize,
    #[serde(default)]
    pub decoded_size: Option<usize>,
    #[serde(default)]
    pub bit_order: BitOrder,
    #[serde(default)]
    pub fill: u8,
    #[serde(default)]
    pub history: Vec<u8>,
    #[serde(default = "default_destination")]
    pub destination: u16,
    #[serde(default)]
    pub prehistory: Prehistory,
}
impl CodecDefinition {
    pub fn decode(&self, bytes: &[u8]) -> Result<compress::DecodeOutcome> {
        if self.schema != "swankit-codec-def/1" {
            return Err(bad("unknown codec definition schema"));
        }
        if self.max_output > compress::MAX_DECODED || self.history.len() > compress::MAX_DECODED {
            return Err(bad("codec output/history exceeds 65536 bytes"));
        }
        let source = bytes_at(bytes, self.offset, self.length)?;
        match self.codec {
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
            Codec::Lzss12 => compress::lzss_12::decode_with_history(
                source,
                &self.history,
                self.decoded_size
                    .ok_or_else(|| bad("external-length LZSS requires decoded_size"))?,
                self.max_output,
            ),
        }
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
    Ok((0..colors)
        .map(|i| {
            let v = (255 - i * 255 / (colors - 1)) as u8;
            [v, v, v, 255]
        })
        .collect())
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
