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
pub const LZSS_BE_UNITS_ID: FormatId = FormatId::new("ws-lzss-be-units");
pub const LZSS_MSB_DISTANCE12_ID: FormatId = FormatId::new("ws-lzss-msb-distance12");
pub const RLE_77EE_ID: FormatId = FormatId::new("ws-rle-77ee");
pub const RLE_PCM_00_ID: FormatId = FormatId::new("ws-rle-pcm-00");
pub const COUNTED_WORD_LZ_ID: FormatId = FormatId::new("ws-counted-word-lz");
pub const DPCM_6BIT_ID: FormatId = FormatId::new("ws-dpcm-6bit");
pub const LZSS_11_5_ID: FormatId = FormatId::new("ws-lzss-11-5");
pub const PLANAR_RLE_ID: FormatId = FormatId::new("ws-planar-rle");
pub const LZSS_TOKEN_BE_ID: FormatId = FormatId::new("ws-lzss-token-be");
pub const NIBBLE_LZSS_RING8_ID: FormatId = FormatId::new("ws-nibble-lzss-ring8");
pub const BYTE_LZSS_RING8_ID: FormatId = FormatId::new("ws-byte-lzss-ring8");
pub const ZERO_RLE16_ID: FormatId = FormatId::new("ws-zero-rle16");
pub const LZ_TILE_7B_ID: FormatId = FormatId::new("ws-lz-tile-7b");
pub const DPCM_4BIT_NIBBLE_ID: FormatId = FormatId::new("ws-dpcm-4bit-nibble");
pub const LZ_RLE_PAT_ID: FormatId = FormatId::new("ws-lz-rle-pat");
pub const LZSS_BANDAI_G_ID: FormatId = FormatId::new("ws-lzss-bandai-g");
pub const RLE16_BIT15_ID: FormatId = FormatId::new("ws-rle16-bit15");
pub const TILE_XOR_PACK_ID: FormatId = FormatId::new("ws-tile-xor-pack");
pub const LZSS_11_5_DIRECT_ID: FormatId = FormatId::new("ws-lzss-11-5-direct");
pub const HUFFMAN_STATIC_ID: FormatId = FormatId::new("ws-huffman-static");
pub const RLE_LZ4_CMD_ID: FormatId = FormatId::new("ws-rle-lz4-cmd");
pub const WORD_RLE_00_ID: FormatId = FormatId::new("ws-word-rle-00");
pub const DPCM_4BIT_SM_ID: FormatId = FormatId::new("ws-dpcm-4bit-sm");
pub const WAVE_4BIT_PACKED_ID: FormatId = FormatId::new("ws-wave-4bit-packed");
pub const TILEMAP_2BIT_ID: FormatId = FormatId::new("ws-tilemap-2bit");
pub const LZSS_12_4_MSB_ID: FormatId = FormatId::new("ws-lzss-12-4-msb");
pub const STRIDED_RLE_ID: FormatId = FormatId::new("ws-strided-rle");
pub const DICT16_RLE_ID: FormatId = FormatId::new("ws-dict16-rle");
pub const LZ44_ID: FormatId = FormatId::new("ws-lz44");
pub const RLE2_DOUBLE_BYTE_ID: FormatId = FormatId::new("ws-rle2-double-byte");
pub const RLE_WORD_7B_ID: FormatId = FormatId::new("ws-rle-word-7b");
pub const WORD_RLE_FLAG_ID: FormatId = FormatId::new("ws-word-rle-flag");

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

/// Explicit decode-only grammar selected by an enclosing definition.
pub const LZSS_MSB_DISTANCE12_FORMAT: FormatDescriptor = {
    let mut format = descriptor(
        LZSS_MSB_DISTANCE12_ID,
        "ws-lzss-msb-distance12",
        Confidence::CorpusBacked,
    );
    format.capabilities.write = false;
    format
};

/// Explicit decode-only unit carrier; no automatic probe or encoder.
pub const LZSS_BE_UNITS_FORMAT: FormatDescriptor = {
    let mut format = descriptor(
        LZSS_BE_UNITS_ID,
        "ws-lzss-be-units",
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
    LZSS_MSB_DISTANCE12_FORMAT,
    LZSS_BE_UNITS_FORMAT,
    descriptor(RLE_77EE_ID, "ws-rle-77ee", Confidence::CorpusBacked),
    descriptor(RLE_PCM_00_ID, "ws-rle-pcm-00", Confidence::CorpusBacked),
    descriptor(
        COUNTED_WORD_LZ_ID,
        "ws-counted-word-lz",
        Confidence::CorpusBacked,
    ),
    descriptor(DPCM_6BIT_ID, "ws-dpcm-6bit", Confidence::CorpusBacked),
    descriptor(LZSS_11_5_ID, "ws-lzss-11-5", Confidence::CorpusBacked),
    descriptor(PLANAR_RLE_ID, "ws-planar-rle", Confidence::CorpusBacked),
    descriptor(
        LZSS_TOKEN_BE_ID,
        "ws-lzss-token-be",
        Confidence::CorpusBacked,
    ),
    descriptor(
        NIBBLE_LZSS_RING8_ID,
        "ws-nibble-lzss-ring8",
        Confidence::CorpusBacked,
    ),
    descriptor(
        BYTE_LZSS_RING8_ID,
        "ws-byte-lzss-ring8",
        Confidence::CorpusBacked,
    ),
    descriptor(ZERO_RLE16_ID, "ws-zero-rle16", Confidence::CorpusBacked),
    descriptor(LZ_TILE_7B_ID, "ws-lz-tile-7b", Confidence::CorpusBacked),
    descriptor(
        DPCM_4BIT_NIBBLE_ID,
        "ws-dpcm-4bit-nibble",
        Confidence::CorpusBacked,
    ),
    descriptor(LZ_RLE_PAT_ID, "ws-lz-rle-pat", Confidence::CorpusBacked),
    descriptor(
        LZSS_BANDAI_G_ID,
        "ws-lzss-bandai-g",
        Confidence::CorpusBacked,
    ),
    descriptor(RLE16_BIT15_ID, "ws-rle16-bit15", Confidence::CorpusBacked),
    descriptor(
        TILE_XOR_PACK_ID,
        "ws-tile-xor-pack",
        Confidence::CorpusBacked,
    ),
    descriptor(
        LZSS_11_5_DIRECT_ID,
        "ws-lzss-11-5-direct",
        Confidence::CorpusBacked,
    ),
    descriptor(
        HUFFMAN_STATIC_ID,
        "ws-huffman-static",
        Confidence::CorpusBacked,
    ),
    descriptor(RLE_LZ4_CMD_ID, "ws-rle-lz4-cmd", Confidence::CorpusBacked),
    descriptor(WORD_RLE_00_ID, "ws-word-rle-00", Confidence::CorpusBacked),
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
    EmbeddedFormatSupport {
        id: LZSS_MSB_DISTANCE12_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lzss-msb-distance12",
        dialect_group: None,
        typed_context: true,
        capabilities: LZSS_MSB_DISTANCE12_FORMAT.capabilities,
        corpus_evidence: Some(CorpusEvidence {
            handler: "ws-compress-lzss-msb-distance12",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZSS_BE_UNITS_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lzss-be-units",
        dialect_group: None,
        typed_context: true,
        capabilities: LZSS_BE_UNITS_FORMAT.capabilities,
        corpus_evidence: Some(CorpusEvidence {
            handler: "ws-compress-lzss-be-units",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: RLE_77EE_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "rle-77ee",
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
            handler: "ws-compress-rle-77ee",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: RLE_PCM_00_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "rle-pcm-00",
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
            handler: "ws-compress-rle-pcm-00",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: COUNTED_WORD_LZ_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "counted-word-lz",
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
            handler: "ws-compress-counted-word-lz",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: DPCM_6BIT_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "dpcm-6bit",
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
            handler: "ws-compress-dpcm-6bit",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZSS_11_5_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lzss-11-5",
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
            handler: "ws-compress-lzss-11-5",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: PLANAR_RLE_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "planar-rle",
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
            handler: "ws-compress-planar-rle",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZSS_TOKEN_BE_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lzss-token-be",
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
            handler: "ws-compress-lzss-token-be",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: NIBBLE_LZSS_RING8_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "nibble-lzss-ring8",
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
            handler: "ws-compress-nibble-lzss-ring8",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: BYTE_LZSS_RING8_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "byte-lzss-ring8",
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
            handler: "ws-compress-byte-lzss-ring8",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: ZERO_RLE16_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "zero-rle16",
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
            handler: "ws-compress-zero-rle16",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZ_TILE_7B_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lz-tile-7b",
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
            handler: "ws-compress-lz-tile-7b",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: DPCM_4BIT_NIBBLE_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "dpcm-4bit-nibble",
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
            handler: "ws-compress-dpcm-4bit-nibble",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZ_RLE_PAT_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lz-rle-pat",
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
            handler: "ws-compress-lz-rle-pat",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZSS_BANDAI_G_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lzss-bandai-g",
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
            handler: "ws-compress-lzss-bandai-g",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: RLE16_BIT15_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "rle16-bit15",
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
            handler: "ws-compress-rle16-bit15",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: TILE_XOR_PACK_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "tile-xor-pack",
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
            handler: "ws-compress-tile-xor-pack",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZSS_11_5_DIRECT_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lzss-11-5-direct",
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
            handler: "ws-compress-lzss-11-5-direct",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: HUFFMAN_STATIC_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "huffman-static",
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
            handler: "ws-compress-huffman-static",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: RLE_LZ4_CMD_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "rle-lz4-cmd",
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
            handler: "ws-compress-rle-lz4-cmd",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: WORD_RLE_00_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "word-rle-00",
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
            handler: "ws-compress-word-rle-00",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: DPCM_4BIT_SM_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "dpcm-4bit-sm",
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
            handler: "ws-compress-dpcm-4bit-sm",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: WAVE_4BIT_PACKED_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "wave-4bit-packed",
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
            handler: "ws-compress-wave-4bit-packed",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: TILEMAP_2BIT_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "tilemap-2bit",
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
            handler: "ws-compress-tilemap-2bit",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZSS_12_4_MSB_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lzss-12-4-msb",
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
            handler: "ws-compress-lzss-12-4-msb",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: STRIDED_RLE_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "strided-rle",
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
            handler: "ws-compress-strided-rle",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: DICT16_RLE_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "dict16-rle",
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
            handler: "ws-compress-dict16-rle",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: LZ44_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "lz44",
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
            handler: "ws-compress-lz44",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: RLE2_DOUBLE_BYTE_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "rle2-double-byte",
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
            handler: "ws-compress-rle2-double-byte",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: RLE_WORD_7B_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "rle-word-7b",
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
            handler: "ws-compress-rle-word-7b",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: WORD_RLE_FLAG_ID,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-compress"),
        decoder: "ws-compress",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "word-rle-flag",
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
            handler: "ws-compress-word-rle-flag",
            sample_floor: 1,
        }),
    },
];
