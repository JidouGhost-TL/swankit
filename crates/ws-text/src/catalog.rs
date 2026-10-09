//! Headerless data needs a caller-selected definition; no content-only probe.
use formatkit_catalog::{FormatFamilyId, FormatId};

pub const TEXT_FAMILY: FormatFamilyId = FormatFamilyId::new("ws-text");
pub const CHARACTER_MAP: FormatId = FormatId::new("ws-character-map");
pub const TEXT_TABLE: FormatId = FormatId::new("ws-text-table");
pub const FONT_STRIP: FormatId = FormatId::new("ws-font-strip");
