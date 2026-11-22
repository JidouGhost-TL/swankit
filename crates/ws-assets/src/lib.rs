//! Per-ROM asset maps (`swankit-asset-map/1`): a JSON table of located assets
//! (offset, stored length, codec, format, relations, references, evidence)
//! read against one ROM image identified by SHA-256.
//!
//! The crate composes the owner crates: tile and palette grammars from
//! `ws-gfx`, codecs from `ws-compress`, character maps and fonts from
//! `ws-text`, PCM from `ws-sound`, IPS from `ws-patch`, footer and banking
//! rules from `ws-rom`. It adds placement policy only: checked ranges,
//! declared overlaps, reference arithmetic, in-place replacement and
//! relocation into declared free space.
#![forbid(unsafe_code)]

mod data;
mod extract;
pub mod fixed_table;
mod font;
mod gfx;
pub mod map;
pub mod parameters;
mod refs;
mod replace;
mod text;
mod verify;

pub use data::{decode_asset, encode_payload, stored_range, Stored};
pub use extract::{extract, ExtractedFile, Extraction};
pub use gfx::{
    palette_colors, render_screen, render_sprites, render_tilemap, render_tiles, unrender_tilemap,
    unrender_tiles, TileSheet,
};
pub use map::{AssetMap, Kind, SCHEMA};
pub use refs::{encode_reference, read_reference, references_for, ResolvedReference};
pub use replace::{
    fix_checksum, replace, replace_with, ReplaceInput, ReplaceOptions, ReplaceOutcome, Replacement,
};
pub use text::Charmaps;
pub use verify::{verify, Finding, VerifyReport};

use formatkit_core::{Error, Result};
use sha2::{Digest, Sha256};

pub(crate) fn malformed(message: impl Into<String>) -> Error {
    Error::Malformed(message.into())
}

/// Lower-case hexadecimal SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Refuse a ROM whose identity differs from the map's.
pub fn check_identity(map: &AssetMap, rom: &[u8]) -> Result<()> {
    check_rom_identity(&map.rom, rom)
}

pub(crate) fn check_rom_identity(identity: &map::RomIdentity, rom: &[u8]) -> Result<()> {
    let size = identity.size.usize()?;
    if rom.len() != size {
        return Err(malformed(format!(
            "ROM is {} bytes; the map describes {} bytes",
            rom.len(),
            size
        )));
    }
    let actual = sha256_hex(rom);
    if !actual.eq_ignore_ascii_case(&identity.sha256) {
        return Err(malformed(format!(
            "ROM SHA-256 {actual} differs from the map's {}",
            identity.sha256
        )));
    }
    Ok(())
}
