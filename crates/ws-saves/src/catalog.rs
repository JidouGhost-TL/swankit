//! Explicit definitions own semantic validity. Many share media and signatures.
use formatkit_catalog::{
    catalog::{ContextRequirement, EmbeddedFormatSupport},
    Confidence, CorpusEvidence, FormatCapabilities, FormatFamilyId, FormatId,
};
pub const DEFINED_SAVE: FormatId = FormatId::new("ws-saves-defined");
pub const SAVE_SUPPORT: EmbeddedFormatSupport = EmbeddedFormatSupport {
    id: DEFINED_SAVE,
    detectable_carrier: None,
    family: FormatFamilyId::new("ws-saves"),
    decoder: "ws-saves",
    context: ContextRequirement::ExplicitSelection,
    local_discriminator: "save-definition",
    dialect_group: None,
    typed_context: true,
    capabilities: FormatCapabilities {
        parse: true,
        decode: true,
        edit: true,
        write: false,
        round_trip: false,
        corpus: true,
        bindings: false,
        confidence: Confidence::CorpusBacked,
    },
    corpus_evidence: Some(CorpusEvidence {
        handler: "ws-saves",
        sample_floor: 1,
    }),
};
