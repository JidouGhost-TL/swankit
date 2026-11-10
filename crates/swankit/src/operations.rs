//! Typed product dispatch. Metadata is composed explicitly; owner APIs parse bytes.
use crate::{compress, extract, gfx, patch, text, Toolkit};
use formatkit_catalog::FormatId;
use formatkit_core::{Error, Image, Result};
use serde_json::Value;
impl Toolkit {
    fn available(&self, id: FormatId) -> Result<()> {
        self.catalog()
            .module(id)
            .map(|_| ())
            .ok_or_else(|| Error::Unsupported(format!("format module is unavailable: {id}")))
    }
    pub fn tiles(
        &self,
        bytes: &[u8],
        format: gfx::TileFormat,
        palette: &[[u8; 4]],
        layout: gfx::AtlasLayout,
    ) -> Result<Image> {
        self.available(format.id())?;
        gfx::render_atlas(bytes, format, palette, layout)
    }
    pub fn snapshot(
        &self,
        bytes: &[u8],
        color: bool,
        profile: gfx::RenderProfile,
    ) -> Result<Image> {
        self.available(gfx::catalog::SCREEN_MAP)?;
        extract::snapshot(bytes, color, profile)
    }
    /// Compose per-scanline display ports over one captured RAM image.
    /// Changes to palette RAM or tiles within the frame require more evidence.
    pub fn snapshot_with_line_ports(
        &self,
        bytes: &[u8],
        color: bool,
        profile: gfx::RenderProfile,
        rows: &[[u8; 256]],
    ) -> Result<Image> {
        self.available(gfx::catalog::SCREEN_MAP)?;
        extract::snapshot_with_line_ports(bytes, color, profile, Some(rows))
    }
    pub fn extract_text(
        &self,
        bytes: &[u8],
        definition: &text::TextDefinition,
        table: &str,
    ) -> Result<Value> {
        self.available(text::catalog::TEXT_TABLE)?;
        extract::text_table(bytes, definition, table)
    }
    pub fn extract_font(
        &self,
        bytes: &[u8],
        definition: &text::TextDefinition,
        font: &str,
        columns: u32,
    ) -> Result<Image> {
        self.available(text::catalog::FONT_STRIP)?;
        extract::font_atlas(bytes, definition, font, columns)
    }
    pub fn extract_codec(
        &self,
        bytes: &[u8],
        definition: &extract::CodecDefinition,
    ) -> Result<compress::DecodeOutcome> {
        self.available(match definition.codec {
            extract::Codec::LzRle7f => compress::catalog::LZ_RLE_7F_ID,
            extract::Codec::LzBits16 => compress::catalog::LZ_BITS_16_ID,
            extract::Codec::LzU8Zero => compress::catalog::LZ_U8_ZERO_ID,
            extract::Codec::LzWord16Msb => compress::catalog::LZ_WORD16_MSB_ID,
            extract::Codec::LzWordCounted => compress::catalog::LZ_WORD_COUNTED_ID,
            extract::Codec::OkumuraLzss => compress::catalog::OKUMURA_ID,
            extract::Codec::LzssU16Zero => compress::catalog::LZSS_U16_ZERO_ID,
            extract::Codec::Lzss12 => compress::catalog::LZSS_12_ID,
            extract::Codec::Lzss12Ext => compress::catalog::LZSS_12_EXT_ID,
            extract::Codec::Flag16Distance11 => compress::catalog::FLAG16_DISTANCE11_ID,
            extract::Codec::Mask16Fill => compress::catalog::MASK16_FILL_ID,
        })?;
        definition.decode(bytes)
    }
    pub fn extract_sound(
        &self,
        bytes: &[u8],
        definition: &extract::SoundDefinition,
    ) -> Result<Value> {
        self.available(match definition.format {
            extract::SoundFormat::UnsignedPcm => crate::sound::catalog::UNSIGNED_PCM,
            extract::SoundFormat::Wave => crate::sound::catalog::WAVE,
            extract::SoundFormat::SdkSequence => crate::sound::catalog::SDK_SEQUENCE,
            extract::SoundFormat::CellSequence => crate::sound::catalog::CELL_SEQUENCE,
            extract::SoundFormat::ByteSequence => crate::sound::catalog::BYTE_SEQUENCE,
        })?;
        definition.extract(bytes)
    }
    pub fn apply_patch(
        &self,
        bytes: &[u8],
        base: &[u8],
        options: patch::ApplyOptions,
    ) -> Result<Vec<u8>> {
        self.available(patch::catalog::IPS_ID)?;
        patch::Patch::parse(bytes)?.apply(base, options)
    }
    pub fn create_patch(&self, base: &[u8], target: &[u8]) -> Result<Vec<u8>> {
        self.available(patch::catalog::IPS_ID)?;
        patch::create(base, target)
    }
}
