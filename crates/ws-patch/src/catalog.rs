//! Owner identity for explicit product catalog composition.
use crate::Patch;
use formatkit_catalog::{
    Category, Confidence, DecoderRequirement, FormatCapabilities, FormatDescriptor, FormatId, Probe,
};

pub const IPS_ID: FormatId = FormatId::new("ws-patch-ips");

pub fn probe(bytes: &[u8]) -> bool {
    Patch::parse(bytes).is_ok()
}

pub const IPS_FORMAT: FormatDescriptor = FormatDescriptor {
    id: IPS_ID,
    category: Category::Data,
    decoder: Some("ws-patch"),
    precedence: 100,
    probes: &[Probe::Structural {
        name: "ips-records",
        check: probe,
    }],
    extension_hints: &["ips"],
    requirement: DecoderRequirement::None,
    ambiguity_group: None,
    capabilities: FormatCapabilities {
        parse: true,
        decode: true,
        edit: true,
        write: true,
        round_trip: true,
        corpus: true,
        bindings: false,
        confidence: Confidence::CorpusBacked,
    },
};
