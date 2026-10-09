//! Owner declaration for later product-level catalog composition.
use crate::{Header, Mapper, SaveDeclaration};
use formatkit_catalog::{
    Category, Confidence, DecoderRequirement, FormatCapabilities, FormatDescriptor, FormatId, Probe,
};

/// WonderSwan cartridge image identity (mono and color share the same grammar).
pub const ROM_ID: FormatId = FormatId::new("ws-rom");

/// Bounded footer-only recognition. A checksum or size-declaration mismatch does
/// not hide an otherwise recognizable image. Unknown declarations are parsed by
/// the facts API but do not contribute a global catalog match.
pub fn probe(bytes: &[u8]) -> bool {
    Header::parse(bytes).is_ok_and(|header| {
        header.maintenance & 0xf == 0
            && header.declared_rom_size().is_some()
            && !matches!(header.save(), SaveDeclaration::Unknown(_))
            && !matches!(header.mapper(), Mapper::Unknown(_))
    })
}

/// Compose this descriptor explicitly into the product's FormatCatalog.
pub const ROM_FORMAT: FormatDescriptor = FormatDescriptor {
    id: ROM_ID,
    category: Category::Executable,
    decoder: Some("ws-rom"),
    precedence: 100,
    probes: &[Probe::Structural {
        name: "ws-rom-footer",
        check: probe,
    }],
    extension_hints: &["ws", "wsc"],
    requirement: DecoderRequirement::None,
    ambiguity_group: None,
    capabilities: FormatCapabilities {
        parse: true,
        decode: false,
        edit: false,
        write: false,
        round_trip: false,
        corpus: true,
        bindings: false,
        confidence: Confidence::CorpusBacked,
    },
};
