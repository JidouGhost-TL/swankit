//! Headerless data needs a caller-selected definition; no content-only probe.
use formatkit_catalog::{FormatFamilyId, FormatId};

pub const TEXT_FAMILY: FormatFamilyId = FormatFamilyId::new("ws-text");
pub const CHARACTER_MAP: FormatId = FormatId::new("ws-character-map");
pub const TEXT_TABLE: FormatId = FormatId::new("ws-text-table");
pub const FONT_STRIP: FormatId = FormatId::new("ws-font-strip");

use formatkit_catalog::catalog::{ContextRequirement, EmbeddedFormatSupport};
use formatkit_catalog::{Confidence, CorpusEvidence, FormatCapabilities};

/// Owner support rows for explicit selection; never global recognition probes.
pub const EMBEDDED: &[EmbeddedFormatSupport] = &[
    EmbeddedFormatSupport {
        id: CHARACTER_MAP,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-text"),
        decoder: "ws-text",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "character-map",
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
            handler: "ws-text",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: TEXT_TABLE,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-text"),
        decoder: "ws-text",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "text-table",
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
            handler: "ws-text",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: FONT_STRIP,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-text"),
        decoder: "ws-text",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "font-strip",
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
            handler: "ws-text",
            sample_floor: 1,
        }),
    },
];
