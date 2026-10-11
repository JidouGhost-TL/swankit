//! Stored ranges and codecs. Codec grammars belong to `ws-compress`.
use crate::map::{Asset, Codec, Kind};
use crate::text::Charmaps;
use crate::{malformed, AssetMap};
use formatkit_core::{bytes_at, Result};
use ws_compress::{lz_bits_16, lz_rle_7f, okumura, BitOrder, MAX_DECODED};

/// Upper bound on the decoded bytes of one asset.
pub const MAX_ASSET_BYTES: usize = 4 * 1024 * 1024;

/// An asset's stored range and decoded payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub offset: usize,
    /// Stored bytes, including a codec terminator.
    pub length: usize,
    /// Decoded (uncompressed) payload.
    pub decoded: Vec<u8>,
}

fn okumura_params(asset: &Asset) -> Result<(usize, BitOrder, u8)> {
    let p = &asset.codec_params;
    let length = p
        .decoded_length
        .ok_or_else(|| malformed(format!("{}: okumura needs decoded_length", asset.id)))?
        .usize()?;
    let order = match p.bit_order.as_deref().unwrap_or("lsb") {
        "lsb" => BitOrder::LsbFirst,
        "msb" => BitOrder::MsbFirst,
        other => return Err(malformed(format!("{}: bit order {other:?}", asset.id))),
    };
    let fill = p.ring_fill.map_or(Ok(0x20), |n| {
        u8::try_from(n.0).map_err(|_| malformed("ring fill must be a byte"))
    })?;
    Ok((length, order, fill))
}

/// Decode the stored bytes at `offset`. With no declared length the codec's
/// terminator (or the text map's end control) defines the extent.
pub fn decode_asset(map: &AssetMap, charmaps: &Charmaps, rom: &[u8], id: &str) -> Result<Stored> {
    let asset = map.asset(id)?;
    let offset = asset.offset.usize()?;
    if offset >= rom.len() {
        return Err(malformed(format!("{id}: offset beyond ROM")));
    }
    let declared = asset.length.map(|n| n.usize()).transpose()?;
    let window = match declared {
        Some(n) => bytes_at(rom, offset, n)?,
        None => &rom[offset..],
    };
    let (length, decoded) = match asset.codec {
        Codec::Raw => match declared {
            Some(n) => (n, window.to_vec()),
            None if asset.kind == Kind::Text => {
                let consumed = charmaps.terminated_length(asset, window)?;
                (consumed, window[..consumed].to_vec())
            }
            None => return Err(malformed(format!("{id}: raw asset needs a length"))),
        },
        Codec::LzRle7f | Codec::LzBits16 => {
            let outcome = if asset.codec == Codec::LzRle7f {
                lz_rle_7f::decode(window, MAX_DECODED)?
            } else {
                lz_bits_16::decode(window, MAX_DECODED)?
            };
            if let Some(n) = declared {
                if outcome.consumed != n {
                    return Err(malformed(format!(
                        "{id}: codec consumed {} of {n} declared stored bytes",
                        outcome.consumed
                    )));
                }
            }
            (outcome.consumed, outcome.bytes)
        }
        Codec::Okumura => {
            let (size, order, fill) = okumura_params(asset)?;
            let outcome = okumura::decode(window, size, order, fill, MAX_ASSET_BYTES)?;
            (declared.unwrap_or(outcome.consumed), outcome.bytes)
        }
    };
    if decoded.len() > MAX_ASSET_BYTES {
        return Err(malformed(format!("{id}: decoded asset too large")));
    }
    Ok(Stored {
        offset,
        length,
        decoded,
    })
}

/// Stored range without keeping the payload.
pub fn stored_range(
    map: &AssetMap,
    charmaps: &Charmaps,
    rom: &[u8],
    id: &str,
) -> Result<(usize, usize)> {
    let s = decode_asset(map, charmaps, rom, id)?;
    Ok((s.offset, s.length))
}

/// Encode a decoded payload with the asset's codec.
pub fn encode_payload(asset: &Asset, decoded: &[u8]) -> Result<Vec<u8>> {
    match asset.codec {
        Codec::Raw => Ok(decoded.to_vec()),
        Codec::LzRle7f => lz_rle_7f::encode(decoded),
        Codec::LzBits16 => lz_bits_16::encode(decoded),
        Codec::Okumura => {
            let (size, order, fill) = okumura_params(asset)?;
            if decoded.len() != size {
                return Err(malformed(format!(
                    "{}: okumura payload must stay {size} bytes (the length lives in code)",
                    asset.id
                )));
            }
            Ok(okumura::encode(decoded, order, fill))
        }
    }
}
