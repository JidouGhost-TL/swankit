//! Bounded WonderSwan compression streams.
//! Select the grammar explicitly: streams have no reliable global magic.
#![forbid(unsafe_code)]

pub mod catalog;
pub mod flag16_distance11;
pub mod lz_bits_16;
pub mod lz_command16_lsb;
pub mod lz_rle_7f;
pub mod lz_u8_zero;
pub mod lz_word16_msb;
pub mod lz_word_counted;
pub mod lzs2;
pub mod lzss_12;
pub mod lzss_12_ext;
pub mod lzss_be_units;
pub mod lzss_msb_distance12;
pub mod lzss_u16_zero;
pub mod mask16_fill;
pub mod okumura;

use formatkit_codec::sliding_window::{append_match, MatchError};
pub use formatkit_codec::{BitOrder, DecodeOutcome};
use formatkit_core::{Error, Result};

/// Maximum decoded size supported by the stream grammars.
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
    append_match(out, history, distance, count, limit).map_err(|error| match error {
        MatchError::OutputLimit => Error::ResourceLimit {
            resource: "decoded bytes",
            requested: out
                .len()
                .checked_add(count)
                .map_or(u64::MAX, |end| end as u64),
            limit: limit as u64,
        },
        MatchError::Distance => Error::InvalidField {
            what: "back-reference distance beyond available history",
            value: distance as u64,
        },
    })
}

/// Preserve `Error::InvalidField` for out-of-range back-reference distances
/// when delegating to `formatkit_codec::flag_lzss::decode_flag_lzss`.
fn map_flag_lzss_err(err: Error) -> Error {
    if let Error::Malformed(ref msg) = err {
        if let Some(rest) = msg.strip_prefix("flag_lzss match distance ") {
            let value = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            return Error::InvalidField {
                what: "back-reference distance beyond available history",
                value,
            };
        }
    }
    err
}
