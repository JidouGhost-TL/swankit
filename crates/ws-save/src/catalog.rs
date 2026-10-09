//! Save image identity is a size candidate, not proof of game or physical medium.
//! Geometry is delegated to Media; see the hardware sources in the workspace.
use crate::{EepromSize, Media};
use formatkit_catalog::{
    Category, Confidence, DecoderRequirement, FormatCapabilities, FormatDescriptor, FormatId, Probe,
};
pub const SAVE_IMAGE: FormatId = FormatId::new("ws-save-image");
pub fn possible_media(length: usize) -> Vec<Media> {
    let mut out = Vec::new();
    for media in [
        Media::CartridgeEeprom(EepromSize::Kbit1),
        Media::CartridgeEeprom(EepromSize::Kbit8),
        Media::CartridgeEeprom(EepromSize::Kbit16),
        Media::InternalMono,
        Media::InternalColor,
        Media::CartridgeSram { bytes: length },
    ] {
        if media.size().is_ok_and(|n| n == length) {
            out.push(media);
        }
    }
    out
}
pub fn probe(bytes: &[u8]) -> bool {
    !possible_media(bytes.len()).is_empty()
}
pub const SAVE_FORMAT: FormatDescriptor = FormatDescriptor {
    id: SAVE_IMAGE,
    category: Category::Data,
    decoder: Some("ws-save"),
    precedence: 10,
    probes: &[Probe::Structural {
        name: "save-media-size",
        check: probe,
    }],
    extension_hints: &["sav", "sram", "eeprom", "ieep"],
    requirement: DecoderRequirement::None,
    ambiguity_group: None,
    capabilities: FormatCapabilities {
        parse: true,
        decode: true,
        edit: true,
        write: false,
        round_trip: false,
        corpus: false,
        bindings: false,
        confidence: Confidence::Structural,
    },
};
