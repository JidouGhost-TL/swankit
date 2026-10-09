//! WonderSwan sound grammars and a bounded, deterministic preview renderer.
//! Hardware rules and preview limits are described in the crate README.
#![forbid(unsafe_code)]

pub mod drivers;
pub mod hardware;
pub mod instrument;
pub mod render;
pub mod wave;

pub use wave::{Wave, WaveBank, WAVE_BYTES, WAVE_SAMPLES};

pub mod catalog;
