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
