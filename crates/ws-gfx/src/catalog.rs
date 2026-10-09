//! Owner identities for explicit carrier-selected graphics operations.
//! These headerless records have no safe content-only recognition probe;
//! product composition must supply the tile format and display context.
use formatkit_catalog::{FormatFamilyId, FormatId};

pub const GRAPHICS_FAMILY: FormatFamilyId = FormatFamilyId::new("ws-gfx");
pub const TILE_2BPP: FormatId = FormatId::new("ws-tile-2bpp");
pub const TILE_4BPP_PLANAR: FormatId = FormatId::new("ws-tile-4bpp-planar");
pub const TILE_4BPP_PACKED: FormatId = FormatId::new("ws-tile-4bpp-packed");
pub const MONO_PALETTES: FormatId = FormatId::new("ws-mono-palettes");
pub const COLOR_PALETTES: FormatId = FormatId::new("ws-color-palettes");
pub const SCREEN_MAP: FormatId = FormatId::new("ws-screen-map");
pub const SPRITE_TABLE: FormatId = FormatId::new("ws-sprite-table");

use formatkit_catalog::catalog::{ContextRequirement, EmbeddedFormatSupport};
use formatkit_catalog::{Confidence, CorpusEvidence, FormatCapabilities};

/// Owner support rows for explicit selection; never global recognition probes.
pub const EMBEDDED: &[EmbeddedFormatSupport] = &[
    EmbeddedFormatSupport {
        id: TILE_2BPP,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-gfx"),
        decoder: "ws-gfx",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "tile-2bpp",
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
            handler: "ws-gfx",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: TILE_4BPP_PLANAR,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-gfx"),
        decoder: "ws-gfx",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "tile-4bpp-planar",
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
            handler: "ws-gfx",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: TILE_4BPP_PACKED,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-gfx"),
        decoder: "ws-gfx",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "tile-4bpp-packed",
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
            handler: "ws-gfx",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: MONO_PALETTES,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-gfx"),
        decoder: "ws-gfx",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "mono-palettes",
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
            handler: "ws-gfx",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: COLOR_PALETTES,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-gfx"),
        decoder: "ws-gfx",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "color-palettes",
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
            handler: "ws-gfx",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: SCREEN_MAP,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-gfx"),
        decoder: "ws-gfx",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "screen-map",
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
            handler: "ws-gfx",
            sample_floor: 1,
        }),
    },
    EmbeddedFormatSupport {
        id: SPRITE_TABLE,
        detectable_carrier: None,
        family: formatkit_catalog::FormatFamilyId::new("ws-gfx"),
        decoder: "ws-gfx",
        context: ContextRequirement::ExplicitSelection,
        local_discriminator: "sprite-table",
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
            handler: "ws-gfx",
            sample_floor: 1,
        }),
    },
];
