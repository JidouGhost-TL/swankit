//! Bounded, headerless WonderSwan compression streams.
//! Select the grammar explicitly: these streams have no reliable global magic.
#![forbid(unsafe_code)]

pub mod catalog;
pub mod lz_bits_16;
pub mod lz_rle_7f;
pub mod okumura;

pub use formatkit_codec::{BitOrder, DecodeOutcome};
use formatkit_core::{Error, Result};

/// Maximum decoded size supported by the two terminated stream grammars.
pub const MAX_DECODED: usize = 0x10000;

/// Check growth before allocating or copying an entire token.
fn grow(out: &[u8], count: usize, limit: usize) -> Result<()> {
    let requested = out.len().checked_add(count).ok_or(Error::ResourceLimit {
        resource: "decoded bytes",
        requested: u64::MAX,
        limit: limit as u64,
    })?;
    if requested > limit {
        return Err(Error::ResourceLimit {
            resource: "decoded bytes",
            requested: requested as u64,
            limit: limit as u64,
        });
    }
    Ok(())
}

fn input_size(src: &[u8]) -> Result<()> {
    grow(&[], src.len(), MAX_DECODED)
}

/// Bytewise copying permits a match to read the bytes it has just produced.
/// History is the destination contents immediately preceding the first output.
fn copy_match(
    out: &mut Vec<u8>,
    history: &[u8],
    distance: usize,
    count: usize,
    limit: usize,
) -> Result<()> {
    grow(out, count, limit)?;
    if distance == 0 || distance > out.len().saturating_add(history.len()) {
        return Err(Error::InvalidField {
            what: "back-reference distance beyond available history",
            value: distance as u64,
        });
    }
    for _ in 0..count {
        let value = if distance > out.len() {
            history[history.len() - (distance - out.len())]
        } else {
            out[out.len() - distance]
        };
        out.push(value);
    }
    Ok(())
}
