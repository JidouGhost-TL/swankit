//! Raw Okumura streams with an externally supplied decoded size.
//! The underlying ring decoder and tree encoder belong to Formatkit.
use crate::{BitOrder, DecodeOutcome};
use formatkit_codec::{
    okumura_lzss_compress, okumura_lzss_consumed_with_limit, OkumuraDecodeError,
};
use formatkit_core::{Error, Result};

/// Decode exactly `decoded_size` bytes; reject a token crossing that boundary.
pub fn decode(
    src: &[u8],
    decoded_size: usize,
    order: BitOrder,
    fill: u8,
    max_output: usize,
) -> Result<DecodeOutcome> {
    okumura_lzss_consumed_with_limit(src, decoded_size, order, fill, max_output).map_err(|e| match e {
        OkumuraDecodeError::OutputLimit { requested, limit } => Error::ResourceLimit {
            resource: "decoded bytes", requested: requested as u64, limit: limit as u64,
        },
        OkumuraDecodeError::Truncated { offset } => Error::Truncated { offset, needed: 1, available: 0 },
        OkumuraDecodeError::TokenOvershoot { produced, token_len, expected } => Error::Malformed(format!("Okumura token of {token_len} bytes at output {produced} crosses declared size {expected}")),
    })
}

/// Encode with the same bit order and initial ring fill as the decoder.
pub fn encode(src: &[u8], order: BitOrder, fill: u8) -> Vec<u8> {
    okumura_lzss_compress(src, order, fill)
}
