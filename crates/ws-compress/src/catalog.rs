//! Explicit identities for headerless streams; selection belongs to a carrier.
use formatkit_catalog::{
    Category, Confidence, DecoderRequirement, FormatCapabilities, FormatDescriptor, FormatId,
};

pub const LZ_RLE_7F_ID: FormatId = FormatId::new("ws-lz-rle-7f");
pub const LZ_BITS_16_ID: FormatId = FormatId::new("ws-lz-bits-16");
pub const OKUMURA_ID: FormatId = FormatId::new("ws-okumura-lzss");

const fn descriptor(
    id: FormatId,
    decoder: &'static str,
    confidence: Confidence,
) -> FormatDescriptor {
    FormatDescriptor {
        id,
        category: Category::Compression,
        decoder: Some(decoder),
        precedence: 0,
        probes: &[],
        extension_hints: &[],
        requirement: DecoderRequirement::None,
        ambiguity_group: None,
        capabilities: FormatCapabilities {
            parse: true,
            decode: true,
            edit: false,
            write: true,
            round_trip: true,
            corpus: true,
            bindings: false,
            confidence,
        },
    }
}

pub const FORMATS: &[FormatDescriptor] = &[
    descriptor(LZ_RLE_7F_ID, "ws-lz-rle-7f", Confidence::CorpusBacked),
    descriptor(LZ_BITS_16_ID, "ws-lz-bits-16", Confidence::CorpusBacked),
    descriptor(OKUMURA_ID, "ws-okumura-lzss", Confidence::Structural),
];
