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
            round_trip: false,
            corpus: matches!(confidence, Confidence::CorpusBacked),
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

use formatkit_catalog::catalog::{ContextRequirement, EmbeddedFormatSupport};
use formatkit_catalog::CorpusEvidence;

/// Owner support rows for explicit selection; never global recognition probes.
pub const EMBEDDED: &[EmbeddedFormatSupport] = &[
    EmbeddedFormatSupport {
        id: LZ_RLE_7F_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lz-rle-7f",
        dialect_group: None,
        typed_context: true,
        capabilities: FormatCapabilities {
            parse: true,
            decode: true,
            edit: false,
            write: true,
            round_trip: false,
            corpus: true,
            bindings: false,
            confidence: Confidence::CorpusBacked,
        },
        corpus_evidence: Some(CorpusEvidence {
            handler: "ws-compress",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZ_BITS_16_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lz-bits-16",
        dialect_group: None,
        typed_context: true,
        capabilities: FormatCapabilities {
            parse: true,
            decode: true,
            edit: false,
            write: true,
            round_trip: false,
            corpus: true,
            bindings: false,
            confidence: Confidence::CorpusBacked,
        },
        corpus_evidence: Some(CorpusEvidence {
            handler: "ws-compress",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: OKUMURA_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "okumura",
        dialect_group: None,
        typed_context: true,
        capabilities: FormatCapabilities {
            parse: true,
            decode: true,
            edit: false,
            write: true,
            round_trip: false,
            corpus: false,
            bindings: false,
            confidence: Confidence::Structural,
        },
        corpus_evidence: None,
    },
];
