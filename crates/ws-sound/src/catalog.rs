/// Explicit identities: untagged driver bytes require caller-supplied layouts.
use formatkit_catalog::FormatId;

pub const UNSIGNED_PCM: FormatId = FormatId::new("ws-sound.unsigned-pcm");
pub const WAVE: FormatId = FormatId::new("ws-sound.wave");
pub const BYTE_SEQUENCE: FormatId = FormatId::new("ws-sound.byte-sequence");
pub const CELL_SEQUENCE: FormatId = FormatId::new("ws-sound.cell-sequence");
pub const SDK_SEQUENCE: FormatId = FormatId::new("ws-sound.sdk-sequence");

use formatkit_catalog::catalog::{ContextRequirement, EmbeddedFormatSupport};
use formatkit_catalog::{Confidence, CorpusEvidence, FormatCapabilities};

/// Owner support rows for explicit selection; never global recognition probes.
pub const EMBEDDED: &[EmbeddedFormatSupport] = &[
    EmbeddedFormatSupport {
        id: WAVE,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-sound"),
        decoder: "ws-sound",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "wave",
        dialect_group: None,
        typed_context: true,
        capabilities: FormatCapabilities {
            parse: true,
            decode: true,
            edit: false,
            write: false,
            round_trip: false,
            corpus: true,
            bindings: false,
            confidence: Confidence::CorpusBacked,
        },
        corpus_evidence: Some(CorpusEvidence {
            handler: "ws-sound",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: BYTE_SEQUENCE,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-sound"),
        decoder: "ws-sound",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "byte-sequence",
        dialect_group: None,
        typed_context: true,
        capabilities: FormatCapabilities {
            parse: true,
            decode: true,
            edit: false,
            write: false,
            round_trip: false,
            corpus: true,
            bindings: false,
            confidence: Confidence::CorpusBacked,
        },
        corpus_evidence: Some(CorpusEvidence {
            handler: "ws-sound",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: CELL_SEQUENCE,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-sound"),
        decoder: "ws-sound",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "cell-sequence",
        dialect_group: None,
        typed_context: true,
        capabilities: FormatCapabilities {
            parse: true,
            decode: true,
            edit: false,
            write: false,
            round_trip: false,
            corpus: true,
            bindings: false,
            confidence: Confidence::CorpusBacked,
        },
        corpus_evidence: Some(CorpusEvidence {
            handler: "ws-sound",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: SDK_SEQUENCE,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-sound"),
        decoder: "ws-sound",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "sdk-sequence",
        dialect_group: None,
        typed_context: true,
        capabilities: FormatCapabilities {
            parse: true,
            decode: true,
            edit: false,
            write: false,
            round_trip: false,
            corpus: true,
            bindings: false,
            confidence: Confidence::CorpusBacked,
        },
        corpus_evidence: Some(CorpusEvidence {
            handler: "ws-sound",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: UNSIGNED_PCM,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-sound"),
        decoder: "ws-sound",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "unsigned-pcm",
        dialect_group: None,
        typed_context: true,
        capabilities: FormatCapabilities {
            parse: true,
            decode: true,
            edit: false,
            write: false,
            round_trip: false,
            corpus: true,
            bindings: false,
            confidence: Confidence::CorpusBacked,
        },
        corpus_evidence: Some(CorpusEvidence {
            handler: "ws-sound",
            sample_floor: 1,
        }),
    },
];
