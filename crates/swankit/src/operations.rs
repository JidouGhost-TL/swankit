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
            extract::Codec::OkumuraLzss => compress::catalog::OKUMURA_ID,
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
