//! Explicit identities for headerless streams; selection belongs to a carrier.
use formatkit_catalog::{
    Category, Confidence, DecoderRequirement, FormatCapabilities, FormatDescriptor, FormatId,
};

pub const LZ_RLE_7F_ID: FormatId = FormatId::new("ws-lz-rle-7f");
pub const LZ_BITS_16_ID: FormatId = FormatId::new("ws-lz-bits-16");
pub const LZ_U8_ZERO_ID: FormatId = FormatId::new("ws-lz-u8-zero");
pub const LZ_WORD16_MSB_ID: FormatId = FormatId::new("ws-lz-word16-msb");
pub const LZ_WORD_COUNTED_ID: FormatId = FormatId::new("ws-lz-word-counted");
pub const LZSS_12_ID: FormatId = FormatId::new("ws-lzss-12");
pub const LZSS_12_EXT_ID: FormatId = FormatId::new("ws-lzss-12-ext");
pub const FLAG16_DISTANCE11_ID: FormatId = FormatId::new("ws-flag16-distance11");
pub const MASK16_FILL_ID: FormatId = FormatId::new("ws-mask16-fill");
pub const LZ_COMMAND16_LSB_ID: FormatId = FormatId::new("ws-lz-command16-lsb");

/// Explicit decode-only grammar; no recognition probe or encoder.
pub const LZSS_12_EXT_FORMAT: FormatDescriptor = {
    let mut format = descriptor(LZSS_12_EXT_ID, "ws-lzss-12-ext", Confidence::CorpusBacked);
    format.capabilities.write = false;
    format
};

/// Explicit decode-only grammar, selected by an enclosing definition.
pub const FLAG16_DISTANCE11_FORMAT: FormatDescriptor = {
    let mut format = descriptor(
        FLAG16_DISTANCE11_ID,
        "ws-flag16-distance11",
        Confidence::CorpusBacked,
    );
    format.capabilities.write = false;
    format
};

/// Explicit decode-only grammar selected by an enclosing definition.
pub const MASK16_FILL_FORMAT: FormatDescriptor = {
    let mut format = descriptor(MASK16_FILL_ID, "ws-mask16-fill", Confidence::CorpusBacked);
    format.capabilities.write = false;
    format
};

/// Explicit decode-only grammar selected by an enclosing definition.
pub const LZ_COMMAND16_LSB_FORMAT: FormatDescriptor = {
    let mut format = descriptor(
        LZ_COMMAND16_LSB_ID,
        "ws-lz-command16-lsb",
        Confidence::CorpusBacked,
    );
    format.capabilities.write = false;
    format
};

pub const LZSS_U16_ZERO_ID: FormatId = FormatId::new("ws-lzss-u16-zero");

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
    descriptor(
        LZ_WORD_COUNTED_ID,
        "ws-lz-word-counted",
        Confidence::CorpusBacked,
    ),
    descriptor(LZSS_12_ID, "ws-lzss-12", Confidence::CorpusBacked),
    descriptor(
        LZSS_U16_ZERO_ID,
        "ws-lzss-u16-zero",
        Confidence::CorpusBacked,
    ),
    descriptor(LZ_RLE_7F_ID, "ws-lz-rle-7f", Confidence::CorpusBacked),
    descriptor(LZ_BITS_16_ID, "ws-lz-bits-16", Confidence::CorpusBacked),
    descriptor(OKUMURA_ID, "ws-okumura-lzss", Confidence::Structural),
    descriptor(LZ_U8_ZERO_ID, "ws-lz-u8-zero", Confidence::CorpusBacked),
    descriptor(
        LZ_WORD16_MSB_ID,
        "ws-lz-word16-msb",
        Confidence::CorpusBacked,
    ),
    LZSS_12_EXT_FORMAT,
    FLAG16_DISTANCE11_FORMAT,
    MASK16_FILL_FORMAT,
    LZ_COMMAND16_LSB_FORMAT,
];

/// Decode-only external-length support, with separately scoped corpus evidence.
pub const LZSS_12_EXT_SUPPORT: EmbeddedFormatSupport = EmbeddedFormatSupport {
    id: LZSS_12_EXT_ID,
    detectable_carrier: None,
    family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
    decoder: "ws-compress",
    context: ContextRequirement::ExplicitSelection,
    local_discriminator: "lzss-12-ext",
    dialect_group: None,
    typed_context: true,
    capabilities: LZSS_12_EXT_FORMAT.capabilities,
    corpus_evidence: Some(CorpusEvidence {
        handler: "ws-compress-lzss-12-ext",
        sample_floor: 1,
    }),
};

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
    EmbeddedFormatSupport {
        id: LZSS_U16_ZERO_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lzss-u16-zero",
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
            handler: "ws-compress-extra",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZSS_12_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lzss-12",
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
            handler: "ws-compress-extra",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZ_U8_ZERO_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lz-u8-zero",
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
            handler: "ws-compress-u8-zero",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZ_WORD16_MSB_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lz-word16-msb",
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
            handler: "ws-compress-word16-msb",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZ_WORD_COUNTED_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lz-word-counted",
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
            handler: "ws-compress-word-counted",
            sample_floor: 1,
        }),
    },
    LZSS_12_EXT_SUPPORT,
    EmbeddedFormatSupport {
        id: FLAG16_DISTANCE11_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "flag16-distance11",
        dialect_group: None,
        typed_context: true,
        capabilities: FLAG16_DISTANCE11_FORMAT.capabilities,
        corpus_evidence: Some(CorpusEvidence {
            handler: "ws-compress-flag16-distance11",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: MASK16_FILL_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "mask16-fill",
        dialect_group: None,
        typed_context: true,
        capabilities: MASK16_FILL_FORMAT.capabilities,
        corpus_evidence: Some(CorpusEvidence {
            handler: "ws-compress-mask16-fill",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZ_COMMAND16_LSB_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lz-command16-lsb",
        dialect_group: None,
        typed_context: true,
        capabilities: LZ_COMMAND16_LSB_FORMAT.capabilities,
        corpus_evidence: Some(CorpusEvidence {
            handler: "ws-compress-lz-command16-lsb",
            sample_floor: 1,
        }),
    },
];
