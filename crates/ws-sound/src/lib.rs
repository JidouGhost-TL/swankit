//! WonderSwan sound grammars and a bounded, deterministic preview renderer.
//! Hardware rules and preview limits are described in the crate README.
#![forbid(unsafe_code)]

pub mod drivers;
pub mod hardware;
pub mod instrument;
pub mod render;
pub mod wave;

pub use wave::{Wave, WaveBank, WAVE_BYTES, WAVE_SAMPLES};

/// Explicit identities: untagged driver bytes require caller-supplied layouts.
pub mod catalog {
    use formatkit_catalog::FormatId;

    pub const WAVE: FormatId = FormatId::new("ws-sound.wave");
    pub const BYTE_SEQUENCE: FormatId = FormatId::new("ws-sound.byte-sequence");
    pub const CELL_SEQUENCE: FormatId = FormatId::new("ws-sound.cell-sequence");
    pub const SDK_SEQUENCE: FormatId = FormatId::new("ws-sound.sdk-sequence");
}
