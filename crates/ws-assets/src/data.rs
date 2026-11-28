//! Stored ranges and codecs. Codec grammars belong to `ws-compress`.
use crate::map::{Asset, Codec, Kind, Num};
use crate::text::Charmaps;
use crate::{malformed, AssetMap};
use formatkit_core::{bytes_at, Result};
use ws_compress::{
    byte_lzss_ring8, counted_word_lz, dpcm_4bit_nibble, dpcm_6bit, flag16_distance11, lz_bits_16,
    lz_command16_lsb, lz_rle_7f, lz_rle_pat, lz_tile_7b, lz_u8_zero, lz_word16_msb,
    lz_word_counted, lzs2, lzss_11_5, lzss_12, lzss_12_ext, lzss_bandai_g, lzss_be_units,
    lzss_msb_distance12, lzss_token_be, lzss_u16_zero, mask16_fill, nibble_lzss_ring8, okumura,
    planar_rle, rle16_bit15, rle_77ee, rle_pcm_00, tile_xor_pack, zero_rle16, BitOrder,
    MAX_DECODED,
};

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

fn lzss_u16_params(asset: &Asset) -> Result<(u16, lzss_u16_zero::Prehistory)> {
    let p = &asset.codec_params;
    let destination = p
        .destination
        .and_then(|n| u16::try_from(n.0).ok())
        .ok_or_else(|| malformed(format!("{}: LZSS needs destination u16", asset.id)))?;
    let policy = match p.prehistory.as_deref() {
        Some("low5-prefix") => lzss_u16_zero::Prehistory::Low5Prefix,
        Some("low8-prefix") => lzss_u16_zero::Prehistory::Low8Prefix,
        Some("zero-until-start") => lzss_u16_zero::Prehistory::ZeroUntilStart,
        _ => {
            return Err(malformed(format!(
                "{}: LZSS needs a known prehistory policy",
                asset.id
            )))
        }
    };
    Ok((destination, policy))
}

fn lzss12_params(asset: &Asset) -> Result<(usize, &[u8])> {
    let p = &asset.codec_params;
    let size = p
        .decoded_length
        .ok_or_else(|| malformed(format!("{}: LZSS needs decoded_length", asset.id)))?
        .usize()?;
    if size > MAX_ASSET_BYTES {
        return Err(malformed(format!("{}: decoded asset too large", asset.id)));
    }
    let history = p.history.as_deref().ok_or_else(|| {
        malformed(format!(
            "{}: LZSS needs explicit history (possibly empty)",
            asset.id
        ))
    })?;
    if history.len() > 4096 {
        return Err(malformed(format!(
            "{}: LZSS history exceeds 4096 bytes",
            asset.id
        )));
    }
    Ok((size, history))
}

fn lzss12_ext_params(asset: &Asset) -> Result<(usize, &[u8])> {
    let p = &asset.codec_params;
    let size = p
        .decoded_length
        .ok_or_else(|| malformed(format!("{}: ws-lzss-12-ext needs decoded_length", asset.id)))?
        .usize()?;
    if size > MAX_ASSET_BYTES {
        return Err(malformed(format!("{}: decoded asset too large", asset.id)));
    }
    let history = p.history.as_deref().unwrap_or(&[]);
    if history.len() > 4096 {
        return Err(malformed(format!(
            "{}: LZSS history exceeds 4096 bytes",
            asset.id
        )));
    }
    Ok((size, history))
}

fn lzss_be_units_params(asset: &Asset) -> Result<(u8, u8)> {
    let p = &asset.codec_params;
    let unit_size = match p.unit_size {
        Some(n) => u8::try_from(n.0)
            .ok()
            .filter(|&v| matches!(v, 16 | 32))
            .ok_or_else(|| malformed(format!("{}: unit_size must be 16 or 32", asset.id)))?,
        None => 16,
    };
    let source_width = match p.source_width {
        Some(n) => u8::try_from(n.0)
            .ok()
            .filter(|&v| matches!(v, 16 | 32))
            .ok_or_else(|| malformed(format!("{}: source_width must be 16 or 32", asset.id)))?,
        None => 16,
    };
    Ok((unit_size, source_width))
}

fn lz_command16_lsb_params(asset: &Asset) -> Result<&[u8]> {
    let history = asset.codec_params.history.as_deref().unwrap_or(&[]);
    if history.len() > 4096 {
        return Err(malformed(format!(
            "{}: LZ history exceeds 4096 bytes",
            asset.id
        )));
    }
    Ok(history)
}

fn lz_rle_pat_params(asset: &Asset) -> Result<usize> {
    let size = asset
        .codec_params
        .decoded_length
        .ok_or_else(|| malformed(format!("{}: ws-lz-rle-pat needs decoded_length", asset.id)))?
        .usize()?;
    if size > MAX_DECODED {
        return Err(malformed(format!("{}: decoded asset too large", asset.id)));
    }
    Ok(size)
}

fn lzss_bandai_g_params(asset: &Asset) -> Result<(u8, usize)> {
    let p = &asset.codec_params;
    let planes = match p.planes {
        Some(n) => u8::try_from(n.0)
            .ok()
            .filter(|&v| matches!(v, 1 | 2))
            .ok_or_else(|| malformed(format!("{}: planes must be 1 or 2", asset.id)))?,
        None => 1,
    };
    let header_bytes = match p.header_bytes {
        Some(n) => n.usize()?,
        None => 0,
    };
    if planes == 1 && header_bytes != 0 {
        return Err(malformed(format!(
            "{}: header_bytes only applies to 2-plane ws-lzss-bandai-g",
            asset.id
        )));
    }
    if header_bytes > 16 {
        return Err(malformed(format!(
            "{}: header_bytes exceeds 16",
            asset.id
        )));
    }
    Ok((planes, header_bytes))
}

pub(crate) fn validate_codec_params(asset: &Asset) -> Result<()> {
    match asset.codec {
        Codec::LzssU16Zero => {
            lzss_u16_params(asset)?;
        }
        Codec::Lzss12 => {
            lzss12_params(asset)?;
        }
        Codec::Lzss12Ext | Codec::LzssMsbDistance12 => {
            lzss12_ext_params(asset)?;
        }
        Codec::LzCommand16Lsb => {
            lz_command16_lsb_params(asset)?;
        }
        Codec::LzssBeUnits => {
            lzss_be_units_params(asset)?;
        }
        Codec::LzRlePat => {
            lz_rle_pat_params(asset)?;
        }
        Codec::LzssBandaiG => {
            lzss_bandai_g_params(asset)?;
        }
        Codec::Lzs2 if asset.codec_params != Default::default() => {
            return Err(malformed(format!(
                "{}: LZS2 takes sizes from its header",
                asset.id
            )));
        }
        _ => {}
    }
    Ok(())
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
        Codec::LzRle7f
        | Codec::LzBits16
        | Codec::LzU8Zero
        | Codec::LzWord16Msb
        | Codec::LzWordCounted
        | Codec::LzssU16Zero
        | Codec::Lzss12
        | Codec::Lzs2
        | Codec::Rle77ee
        | Codec::RlePcm00
        | Codec::CountedWordLz
        | Codec::Dpcm6bit
        | Codec::Lzss11_5
        | Codec::Flag16Distance11
        | Codec::PlanarRle
        | Codec::LzssTokenBe
        | Codec::LzssBeUnits
        | Codec::Lzss12Ext
        | Codec::NibbleLzssRing8
        | Codec::ByteLzssRing8
        | Codec::ZeroRle16
        | Codec::LzTile7b
        | Codec::LzCommand16Lsb
        | Codec::LzssMsbDistance12
        | Codec::Mask16Fill
        | Codec::Dpcm4bitNibble
        | Codec::LzRlePat
        | Codec::LzssBandaiG
        | Codec::Rle16Bit15
        | Codec::TileXorPack => {
            let outcome = match asset.codec {
                Codec::Lzs2 => lzs2::decode(window, MAX_DECODED)?,
                Codec::LzRle7f => lz_rle_7f::decode(window, MAX_DECODED)?,
                Codec::LzBits16 => lz_bits_16::decode(window, MAX_DECODED)?,
                Codec::LzU8Zero => lz_u8_zero::decode(window, MAX_DECODED)?,
                Codec::LzWord16Msb => lz_word16_msb::decode(window, MAX_DECODED)?,
                Codec::LzWordCounted => lz_word_counted::decode_at(window, MAX_DECODED, offset)?,
                Codec::LzssU16Zero => {
                    let (destination, policy) = lzss_u16_params(asset)?;
                    lzss_u16_zero::decode_with_policy(window, MAX_ASSET_BYTES, destination, policy)?
                }
                Codec::Lzss12 => {
                    let (size, history) = lzss12_params(asset)?;
                    lzss_12::decode_with_history(window, history, size, MAX_ASSET_BYTES)?
                }
                Codec::Lzss12Ext => {
                    let (size, history) = lzss12_ext_params(asset)?;
                    lzss_12_ext::decode_with_history(window, history, size, MAX_ASSET_BYTES)?
                }
                Codec::LzssBeUnits => {
                    let (unit_size, source_width) = lzss_be_units_params(asset)?;
                    lzss_be_units::decode(window, unit_size, source_width, MAX_DECODED)?
                }
                Codec::Rle77ee => rle_77ee::decode(window, MAX_ASSET_BYTES)?,
                Codec::RlePcm00 => {
                    if declared.is_none() {
                        return Err(malformed(format!(
                            "{id}: ws-rle-pcm-00 asset needs a length"
                        )));
                    }
                    rle_pcm_00::decode(window, MAX_ASSET_BYTES)?
                }
                Codec::CountedWordLz => counted_word_lz::decode(window, MAX_DECODED)?,
                Codec::Dpcm6bit => {
                    if declared.is_none() {
                        return Err(malformed(format!(
                            "{id}: ws-dpcm-6bit asset needs a length"
                        )));
                    }
                    dpcm_6bit::decode(window, MAX_ASSET_BYTES)?
                }
                Codec::Lzss11_5 => lzss_11_5::decode(window, MAX_DECODED)?,
                Codec::Flag16Distance11 => flag16_distance11::decode(window, MAX_DECODED)?,
                Codec::PlanarRle => planar_rle::decode(window, MAX_DECODED)?,
                Codec::LzssTokenBe => lzss_token_be::decode(window, MAX_DECODED)?,
                Codec::NibbleLzssRing8 => nibble_lzss_ring8::decode(window, MAX_DECODED)?,
                Codec::ByteLzssRing8 => byte_lzss_ring8::decode(window, MAX_DECODED)?,
                Codec::ZeroRle16 => {
                    if declared.is_none() {
                        return Err(malformed(format!(
                            "{id}: ws-zero-rle16 asset needs a length"
                        )));
                    }
                    zero_rle16::decode(window, MAX_ASSET_BYTES)?
                }
                Codec::LzTile7b => lz_tile_7b::decode(window, MAX_DECODED)?,
                Codec::LzCommand16Lsb => {
                    let history = lz_command16_lsb_params(asset)?;
                    lz_command16_lsb::decode_with_history(window, history, MAX_DECODED)?
                }
                Codec::LzssMsbDistance12 => {
                    let (size, history) = lzss12_ext_params(asset)?;
                    lzss_msb_distance12::decode_with_history(window, history, size, MAX_ASSET_BYTES)?
                }
                Codec::Mask16Fill => mask16_fill::decode(window, MAX_DECODED)?,
                Codec::Dpcm4bitNibble => {
                    if declared.is_none() {
                        return Err(malformed(format!(
                            "{id}: ws-dpcm-4bit-nibble asset needs a length"
                        )));
                    }
                    dpcm_4bit_nibble::decode(window, MAX_ASSET_BYTES)?
                }
                Codec::LzRlePat => {
                    let size = lz_rle_pat_params(asset)?;
                    lz_rle_pat::decode(window, size, MAX_DECODED)?
                }
                Codec::LzssBandaiG => {
                    let (planes, header_bytes) = lzss_bandai_g_params(asset)?;
                    if planes == 2 {
                        lzss_bandai_g::decode_2plane(window, header_bytes, MAX_DECODED)?
                    } else {
                        lzss_bandai_g::decode(window, MAX_DECODED)?
                    }
                }
                Codec::Rle16Bit15 => {
                    if declared.is_none() {
                        return Err(malformed(format!(
                            "{id}: ws-rle16-bit15 asset needs a length"
                        )));
                    }
                    rle16_bit15::decode(window, MAX_ASSET_BYTES)?
                }
                Codec::TileXorPack => tile_xor_pack::decode(window, MAX_DECODED)?,
                _ => unreachable!(),
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
    let decoded = strip_header(asset, decoded)?;
    allocation_capacity(asset, offset, length, rom.len())?;
    Ok(Stored {
        offset,
        length,
        decoded,
    })
}

/// Validate explicit placement ownership without changing codec consumption.
pub(crate) fn allocation_capacity(
    asset: &Asset,
    offset: usize,
    length: usize,
    rom_len: usize,
) -> Result<usize> {
    let Some(capacity) = asset.reserved_capacity else {
        return Ok(length);
    };
    let capacity = capacity.usize()?;
    let end = offset
        .checked_add(capacity)
        .ok_or_else(|| malformed("reserved capacity overflow"))?;
    if capacity < length || capacity == 0 || end > rom_len.saturating_sub(ws_rom::FOOTER_SIZE) {
        return Err(malformed(format!(
            "{}: reserved capacity is smaller than stored length, empty, outside ROM or in footer",
            asset.id
        )));
    }
    Ok(capacity)
}

/// Additional reserved bytes cannot share ownership, even when stored aliases
/// are explicitly allowed to overlap. Old maps retain their existing policy.
pub(crate) fn validate_reservations(map: &AssetMap, charmaps: &Charmaps, rom: &[u8]) -> Result<()> {
    if !map.assets.iter().any(|a| a.reserved_capacity.is_some()) {
        return Ok(());
    }
    let mut ranges = Vec::new();
    for asset in &map.assets {
        let stored = decode_asset(map, charmaps, rom, &asset.id)?;
        let capacity = allocation_capacity(asset, stored.offset, stored.length, rom.len())?;
        ranges.push((
            asset,
            stored.offset,
            stored.offset + stored.length,
            stored.offset + capacity,
        ));
    }
    for (asset, start, stored_end, end) in &ranges {
        if asset.reserved_capacity.is_none() {
            continue;
        }
        for (other, other_start, _, other_end) in &ranges {
            if stored_end < end
                && asset.id != other.id
                && stored_end < other_end
                && other_start < end
            {
                return Err(malformed(format!(
                    "{}: reserved tail overlaps asset {}",
                    asset.id, other.id
                )));
            }
        }
        for free in &map.free_space {
            let fs = free.offset.usize()?;
            let fe = fs
                .checked_add(free.length.usize()?)
                .ok_or_else(|| malformed("free range overflow"))?;
            if fs < *end && *start < fe {
                return Err(malformed(format!(
                    "{}: reserved capacity overlaps free space",
                    asset.id
                )));
            }
        }
    }
    Ok(())
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

fn u16_at(bytes: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

/// Bytes per payload unit counted by a header: one tile, or one map cell.
fn header_unit(asset: &Asset) -> Result<usize> {
    match asset.format.header.as_deref() {
        Some("u16-count") if asset.kind == Kind::Tiles => {
            Ok(crate::gfx::tile_format(asset)?.bytes_per_tile())
        }
        Some("u16-width-height") if asset.kind == Kind::Tilemap => {
            Ok(if asset.format.entry.as_deref() == Some("byte") {
                1
            } else {
                2
            })
        }
        Some(other) => Err(malformed(format!(
            "{}: header {other:?} does not apply to a {} asset",
            asset.id,
            asset.kind.name()
        ))),
        None => Ok(0),
    }
}

/// Check and remove the size header: the payload must be exactly what it counts.
fn strip_header(asset: &Asset, mut bytes: Vec<u8>) -> Result<Vec<u8>> {
    let unit = header_unit(asset)?;
    let Some(kind) = asset.format.header.as_deref() else {
        return Ok(bytes);
    };
    let (size, counted) = match kind {
        "u16-count" if bytes.len() >= 2 => (2, u16_at(&bytes, 0)),
        "u16-width-height" if bytes.len() >= 4 => {
            let (w, h) = (u16_at(&bytes, 0), u16_at(&bytes, 2));
            let f = &asset.format;
            let declared = |n: Option<Num>| n.map(|n| n.usize()).transpose();
            if declared(f.width)?.is_some_and(|d| d != w)
                || declared(f.height)?.is_some_and(|d| d != h)
            {
                return Err(malformed(format!(
                    "{}: header says {w}x{h}, the map declares {:?}x{:?}",
                    asset.id, f.width, f.height
                )));
            }
            (4, w * h)
        }
        _ => {
            return Err(malformed(format!(
                "{}: payload shorter than its header",
                asset.id
            )))
        }
    };
    if bytes.len() - size != counted * unit {
        return Err(malformed(format!(
            "{}: header counts {counted} units of {unit} bytes but {} payload bytes follow",
            asset.id,
            bytes.len() - size
        )));
    }
    bytes.drain(..size);
    Ok(bytes)
}

/// Prefix a payload with its size header.
fn add_header(asset: &Asset, decoded: &[u8]) -> Result<Vec<u8>> {
    let unit = header_unit(asset)?;
    let header: Vec<u8> = match asset.format.header.as_deref() {
        None => return Ok(decoded.to_vec()),
        Some("u16-count") => {
            if !decoded.len().is_multiple_of(unit) {
                return Err(malformed(format!(
                    "{}: not a whole number of tiles",
                    asset.id
                )));
            }
            let count = u16::try_from(decoded.len() / unit)
                .map_err(|_| malformed(format!("{}: tile count exceeds 16 bits", asset.id)))?;
            count.to_le_bytes().to_vec()
        }
        Some(_) => {
            let f = &asset.format;
            let w = f
                .width
                .ok_or_else(|| malformed(format!("{}: tilemap width", asset.id)))?
                .usize()?;
            let h = f
                .height
                .ok_or_else(|| malformed(format!("{}: tilemap height", asset.id)))?
                .usize()?;
            if decoded.len() != w * h * unit {
                return Err(malformed(format!(
                    "{}: a {w}x{h} map needs {} payload bytes",
                    asset.id,
                    w * h * unit
                )));
            }
            let dim =
                |n: usize| u16::try_from(n).map_err(|_| malformed("map dimension exceeds 16 bits"));
            [dim(w)?.to_le_bytes(), dim(h)?.to_le_bytes()].concat()
        }
    };
    Ok([header, decoded.to_vec()].concat())
}

/// Encode a decoded payload (with its size header, when declared) with the
/// asset's codec.
pub fn encode_payload(asset: &Asset, decoded: &[u8]) -> Result<Vec<u8>> {
    let with_header = add_header(asset, decoded)?;
    let decoded = with_header.as_slice();
    match asset.codec {
        Codec::Raw => Ok(decoded.to_vec()),
        Codec::LzRle7f => lz_rle_7f::encode(decoded),
        Codec::Lzs2 => Err(malformed(format!(
            "{}: LZS2 assets are decode-only",
            asset.id
        ))),
        Codec::LzBits16 => lz_bits_16::encode(decoded),
        Codec::LzU8Zero => lz_u8_zero::encode(decoded),
        Codec::LzWord16Msb => lz_word16_msb::encode(decoded),
        Codec::LzWordCounted => lz_word_counted::encode_at(decoded, asset.offset.usize()?),
        Codec::LzssU16Zero => {
            lzss_u16_params(asset)?;
            lzss_u16_zero::encode(decoded)
        }
        Codec::Lzss12 => {
            let (size, _) = lzss12_params(asset)?;
            if decoded.len() != size {
                return Err(malformed(format!(
                    "{}: LZSS payload must stay {size} bytes (the length lives in the carrier)",
                    asset.id
                )));
            }
            lzss_12::encode(decoded)
        }
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
        Codec::Rle77ee => rle_77ee::encode(decoded),
        Codec::RlePcm00 => rle_pcm_00::encode(decoded),
        Codec::CountedWordLz => counted_word_lz::encode(decoded),
        Codec::Dpcm6bit => dpcm_6bit::encode(decoded),
        Codec::Lzss11_5 => lzss_11_5::encode(decoded),
        Codec::Flag16Distance11 => Err(malformed(format!(
            "{}: ws-flag16-distance11 assets are decode-only",
            asset.id
        ))),
        Codec::PlanarRle => Err(malformed(format!(
            "{}: ws-planar-rle assets require explicit header geometry for encoding",
            asset.id
        ))),
        Codec::LzssTokenBe => lzss_token_be::encode(decoded),
        Codec::LzssBeUnits => Err(malformed(format!(
            "{}: ws-lzss-be-units assets are decode-only",
            asset.id
        ))),
        Codec::Lzss12Ext => Err(malformed(format!(
            "{}: ws-lzss-12-ext assets are decode-only",
            asset.id
        ))),
        Codec::NibbleLzssRing8 => nibble_lzss_ring8::encode(decoded),
        Codec::ByteLzssRing8 => byte_lzss_ring8::encode(decoded),
        Codec::ZeroRle16 => zero_rle16::encode(decoded),
        Codec::LzTile7b => lz_tile_7b::encode(decoded),
        Codec::LzCommand16Lsb => Err(malformed(format!(
            "{}: ws-lz-command16-lsb assets are decode-only",
            asset.id
        ))),
        Codec::LzssMsbDistance12 => Err(malformed(format!(
            "{}: ws-lzss-msb-distance12 assets are decode-only",
            asset.id
        ))),
        Codec::Mask16Fill => Err(malformed(format!(
            "{}: ws-mask16-fill assets are decode-only",
            asset.id
        ))),
        Codec::Dpcm4bitNibble => dpcm_4bit_nibble::encode(decoded),
        Codec::LzRlePat => {
            let size = lz_rle_pat_params(asset)?;
            if decoded.len() != size {
                return Err(malformed(format!(
                    "{}: ws-lz-rle-pat payload must stay {size} bytes",
                    asset.id
                )));
            }
            lz_rle_pat::encode(decoded)
        }
        Codec::LzssBandaiG => {
            let (planes, header_bytes) = lzss_bandai_g_params(asset)?;
            if planes == 2 {
                if header_bytes != 0 {
                    return Err(malformed(format!(
                        "{}: 2-plane ws-lzss-bandai-g with prefix header_bytes is decode-only",
                        asset.id
                    )));
                }
                lzss_bandai_g::encode_2plane(decoded)
            } else {
                lzss_bandai_g::encode(decoded)
            }
        }
        Codec::Rle16Bit15 => rle16_bit15::encode(decoded),
        Codec::TileXorPack => tile_xor_pack::encode(decoded),
    }
}
