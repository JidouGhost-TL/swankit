//! Data-defined character maps, checked text tables and fonts.
#![forbid(unsafe_code)]

pub mod catalog;
mod charmap;
mod font;
mod table;

pub use charmap::*;
pub use font::*;
pub use table::*;

use formatkit_core::{Error, Result};

fn malformed(message: impl Into<String>) -> Error {
    Error::Malformed(message.into())
}

fn limit(resource: &'static str, requested: usize, maximum: usize) -> Result<()> {
    if requested > maximum {
        return Err(Error::ResourceLimit {
            resource,
            requested: requested as u64,
            limit: maximum as u64,
        });
    }
    Ok(())
}

/// Definition-level limits keep untrusted JSON and text operations bounded.
#[derive(Debug, Clone, Copy)]
pub struct TextLimits {
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub tokens: usize,
}

impl Default for TextLimits {
    fn default() -> Self {
        Self {
            input_bytes: 65536,
            output_bytes: 262144,
            tokens: 32768,
        }
    }
}
