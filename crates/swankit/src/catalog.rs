//! Explicit composition. Ownership and probes stay in each format crate.
use formatkit_catalog::{catalog::EmbeddedFormatSupport, FormatDescriptor, FormatId};
use formatkit_runtime::{FormatModule, ModuleCatalog, ModuleCatalogError, ModuleState};

const fn row(
    id: FormatId,
    owner: &'static str,
    descriptor: Option<&'static FormatDescriptor>,
    embedded: Option<&'static EmbeddedFormatSupport>,
) -> FormatModule {
    FormatModule {
        format: id,
        owner,
        state: ModuleState::Legacy,
        descriptor,
        embedded_support: embedded,
        writer_contract: None,
        namespace_semantics: None,
        operations: &[],
    }
}

/// Runtime rows carry owner metadata. Typed APIs dispatch Rust values directly;
/// these rows do not claim strict runtime callbacks or writer proof contracts.
pub static MODULES: &[FormatModule] = &[
    #[cfg(feature = "saves")]
    row(
        crate::save_image::catalog::SAVE_IMAGE,
        "ws-save",
        Some(&crate::save_image::catalog::SAVE_FORMAT),
        None,
    ),
    #[cfg(feature = "saves")]
    row(
        crate::saves::catalog::DEFINED_SAVE,
        "ws-saves",
        None,
        Some(&crate::saves::catalog::SAVE_SUPPORT),
    ),
    row(
        ws_rom::catalog::ROM_ID,
        "ws-rom",
        Some(&ws_rom::catalog::ROM_FORMAT),
        None,
    ),
    row(
        ws_patch::catalog::IPS_ID,
        "ws-patch",
        Some(&ws_patch::catalog::IPS_FORMAT),
        None,
    ),
    row(
        ws_compress::catalog::LZ_RLE_7F_ID,
        "ws-compress",
        None,
        Some(&ws_compress::catalog::EMBEDDED[0]),
    ),
    row(
        ws_compress::catalog::LZ_BITS_16_ID,
        "ws-compress",
        None,
        Some(&ws_compress::catalog::EMBEDDED[1]),
    ),
    row(
        ws_compress::catalog::LZSS_12_ID,
        "ws-compress",
        None,
        Some(&ws_compress::catalog::EMBEDDED[4]),
    ),
    row(
        ws_compress::catalog::LZSS_U16_ZERO_ID,
        "ws-compress",
        None,
        Some(&ws_compress::catalog::EMBEDDED[3]),
    ),
    row(
        ws_compress::catalog::OKUMURA_ID,
        "ws-compress",
        None,
        Some(&ws_compress::catalog::EMBEDDED[2]),
    ),
    row(
        ws_compress::catalog::LZ_U8_ZERO_ID,
        "ws-compress",
        None,
        Some(&ws_compress::catalog::EMBEDDED[5]),
    ),
    row(
        ws_compress::catalog::LZ_WORD16_MSB_ID,
        "ws-compress",
        None,
        Some(&ws_compress::catalog::EMBEDDED[6]),
    ),
    row(
        ws_gfx::catalog::TILE_2BPP,
        "ws-gfx",
        None,
        Some(&ws_gfx::catalog::EMBEDDED[0]),
    ),
    row(
        ws_gfx::catalog::TILE_4BPP_PLANAR,
        "ws-gfx",
        None,
        Some(&ws_gfx::catalog::EMBEDDED[1]),
    ),
    row(
        ws_gfx::catalog::TILE_4BPP_PACKED,
        "ws-gfx",
        None,
        Some(&ws_gfx::catalog::EMBEDDED[2]),
    ),
    row(
        ws_gfx::catalog::MONO_PALETTES,
        "ws-gfx",
        None,
        Some(&ws_gfx::catalog::EMBEDDED[3]),
    ),
    row(
        ws_gfx::catalog::COLOR_PALETTES,
        "ws-gfx",
        None,
        Some(&ws_gfx::catalog::EMBEDDED[4]),
    ),
    row(
        ws_gfx::catalog::SCREEN_MAP,
        "ws-gfx",
        None,
        Some(&ws_gfx::catalog::EMBEDDED[5]),
    ),
    row(
        ws_gfx::catalog::SPRITE_TABLE,
        "ws-gfx",
        None,
        Some(&ws_gfx::catalog::EMBEDDED[6]),
    ),
    row(
        ws_text::catalog::CHARACTER_MAP,
        "ws-text",
        None,
        Some(&ws_text::catalog::EMBEDDED[0]),
    ),
    row(
        ws_text::catalog::TEXT_TABLE,
        "ws-text",
        None,
        Some(&ws_text::catalog::EMBEDDED[1]),
    ),
    row(
        ws_text::catalog::FONT_STRIP,
        "ws-text",
        None,
        Some(&ws_text::catalog::EMBEDDED[2]),
    ),
    row(
        ws_sound::catalog::WAVE,
        "ws-sound",
        None,
        Some(&ws_sound::catalog::EMBEDDED[0]),
    ),
    row(
        ws_sound::catalog::BYTE_SEQUENCE,
        "ws-sound",
        None,
        Some(&ws_sound::catalog::EMBEDDED[1]),
    ),
    row(
        ws_sound::catalog::CELL_SEQUENCE,
        "ws-sound",
        None,
        Some(&ws_sound::catalog::EMBEDDED[2]),
    ),
    row(
        ws_sound::catalog::SDK_SEQUENCE,
        "ws-sound",
        None,
        Some(&ws_sound::catalog::EMBEDDED[3]),
    ),
    row(
        ws_sound::catalog::UNSIGNED_PCM,
        "ws-sound",
        None,
        Some(&ws_sound::catalog::EMBEDDED[4]),
    ),
];

pub fn compose() -> std::result::Result<ModuleCatalog, ModuleCatalogError> {
    ModuleCatalog::new(MODULES)
}
