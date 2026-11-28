//! Tile-level bitmask and XOR-delta stream codec.
//!
//! Framing starts with a 5-byte header:
//! - `mode: u8` in `{0x00, 0x01, 0x02, 0x10, 0x11, 0x12}`
//! - `count: u16_le` (nonzero tile count)
//! - `flags_rel: u16_le` (byte offset from stream start to the packed command flags)
//!
//! `mode & 3` selects tile geometry: `0 => 16 B` (2 B mask, 1 plane),
//! `1 => 32 B` (4 B mask, 2 planes), `2 => 64 B` (8 B mask, 4 planes).
//! For `mode & 3 == 0`, each tile uses 1 LSB-first flag bit (`0 => cmd 2`,
//! `1 => cmd 1`); otherwise each tile uses 2 LSB-first flag bits (`0..=3`):
//! - `cmd 0`: all-zero tile.
//! - `cmd 1`: bitmask-packed literal tile (LSB-first bit order across mask bytes).
//! - `cmd 2`: bitmask-packed tile followed by 2-byte row-pair XOR accumulation
//!   within each 16-byte plane.
//! - `cmd 3`: if `mode >= 0x10`, bitmask-packed tile followed by 1-byte
//!   cumulative XOR; otherwise raw literal tile.
use crate::{grow, input_size, DecodeOutcome, MAX_DECODED};
use formatkit_core::{Error, Reader, Result};

fn geometry(mode: u8) -> Result<(usize, usize, usize)> {
    match mode {
        0x00 | 0x10 => Ok((16, 2, 1)),
        0x01 | 0x11 => Ok((32, 4, 2)),
        0x02 | 0x12 => Ok((64, 8, 4)),
        _ => Err(Error::InvalidField {
            what: "tile-xor-pack mode must be 0x00..=0x02 or 0x10..=0x12",
            value: u64::from(mode),
        }),
    }
}

/// Decode a self-delimiting tile-level bitmask+XOR-delta stream.
pub fn decode(src: &[u8], max_output: usize) -> Result<DecodeOutcome> {
    let limit = max_output.min(MAX_DECODED);
    let mut header = Reader::new(src);
    let mode = header.u8()?;
    let count = usize::from(header.u16_le()?);
    let flags_rel = usize::from(header.u16_le()?);

    let (tile_sz, mask_bytes, n_planes) = geometry(mode)?;
    if count == 0 {
        return Err(Error::InvalidField {
            what: "tile-xor-pack tile count must be nonzero",
            value: 0,
        });
    }
    let total_out = count.checked_mul(tile_sz).ok_or(Error::ResourceLimit {
        resource: "decoded bytes",
        requested: u64::MAX,
        limit: limit as u64,
    })?;
    grow(&[], total_out, limit)?;

    if flags_rel < 5 || flags_rel >= src.len() {
        return Err(Error::InvalidField {
            what: "tile-xor-pack flags_rel out of range",
            value: flags_rel as u64,
        });
    }

    let mut si = 5usize;
    let mut flags_ptr = flags_rel;
    let mut bit_idx = 0u8;
    let m3 = mode & 3;
    let mut out = Vec::with_capacity(total_out);

    let read_byte = |pos: &mut usize| -> Result<u8> {
        if *pos >= src.len() {
            return Err(Error::Truncated {
                offset: *pos,
                needed: 1,
                available: 0,
            });
        }
        let b = src[*pos];
        *pos += 1;
        Ok(b)
    };

    let read_masked = |si: &mut usize| -> Result<Vec<u8>> {
        if *si + mask_bytes > src.len() {
            return Err(Error::Truncated {
                offset: *si,
                needed: mask_bytes,
                available: src.len().saturating_sub(*si),
            });
        }
        let mask = &src[*si..*si + mask_bytes];
        *si += mask_bytes;
        let mut buf = vec![0u8; tile_sz];
        for (i, slot) in buf.iter_mut().enumerate() {
            if (mask[i >> 3] >> (i & 7)) & 1 != 0 {
                *slot = read_byte(si)?;
            }
        }
        Ok(buf)
    };

    for _ in 0..count {
        let cmd = if m3 == 0 {
            if bit_idx == 8 {
                flags_ptr += 1;
                bit_idx = 0;
            }
            if flags_ptr >= src.len() {
                return Err(Error::Truncated {
                    offset: flags_ptr,
                    needed: 1,
                    available: 0,
                });
            }
            let b = (src[flags_ptr] >> bit_idx) & 1;
            bit_idx += 1;
            if b == 0 { 2 } else { 1 }
        } else {
            if bit_idx == 4 {
                flags_ptr += 1;
                bit_idx = 0;
            }
            if flags_ptr >= src.len() {
                return Err(Error::Truncated {
                    offset: flags_ptr,
                    needed: 1,
                    available: 0,
                });
            }
            let v = (src[flags_ptr] >> (bit_idx * 2)) & 3;
            bit_idx += 1;
            v
        };

        match cmd {
            0 => out.resize(out.len() + tile_sz, 0),
            1 => {
                let buf = read_masked(&mut si)?;
                out.extend_from_slice(&buf);
            }
            2 => {
                let mut buf = read_masked(&mut si)?;
                for di in (0..14).step_by(2) {
                    for p in 0..n_planes {
                        let bp = p * 16;
                        buf[bp + di + 2] ^= buf[bp + di];
                        buf[bp + di + 3] ^= buf[bp + di + 1];
                    }
                }
                out.extend_from_slice(&buf);
            }
            3 => {
                if mode >= 0x10 {
                    let mut buf = read_masked(&mut si)?;
                    for i in 0..(tile_sz - 1) {
                        buf[i + 1] ^= buf[i];
                    }
                    out.extend_from_slice(&buf);
                } else {
                    if si + tile_sz > src.len() {
                        return Err(Error::Truncated {
                            offset: si,
                            needed: tile_sz,
                            available: src.len().saturating_sub(si),
                        });
                    }
                    out.extend_from_slice(&src[si..si + tile_sz]);
                    si += tile_sz;
                }
            }
            _ => unreachable!(),
        }
    }

    if si != flags_rel {
        return Err(Error::Malformed(format!(
            "tile-xor-pack payload ended at {si}, expected flags_rel {flags_rel}"
        )));
    }

    let consumed = flags_ptr + usize::from(bit_idx > 0);
    Ok(DecodeOutcome {
        bytes: out,
        consumed,
    })
}

fn pack_masked(tile: &[u8], mask_bytes: usize) -> Vec<u8> {
    let mut mask = vec![0u8; mask_bytes];
    let mut vals = Vec::new();
    for (i, &b) in tile.iter().enumerate() {
        if b != 0 {
            mask[i >> 3] |= 1 << (i & 7);
            vals.push(b);
        }
    }
    mask.extend_from_slice(&vals);
    mask
}

/// Deterministic encoder selecting mode `0x01` (32 B tiles) or `0x00` (16 B tiles).
pub fn encode(src: &[u8]) -> Result<Vec<u8>> {
    if src.is_empty() {
        return Err(Error::InvalidField {
            what: "tile-xor-pack input must be non-empty",
            value: 0,
        });
    }
    let mode = if src.len() % 32 == 0 { 0x01 } else { 0x00 };
    encode_with_mode(src, mode)
}

/// Deterministic encoder with explicit mode (`0x00..=0x02` or `0x10..=0x12`).
pub fn encode_with_mode(src: &[u8], mode: u8) -> Result<Vec<u8>> {
    input_size(src)?;
    let (tile_sz, mask_bytes, _) = geometry(mode)?;
    if src.is_empty() || src.len() % tile_sz != 0 {
        return Err(Error::InvalidField {
            what: "tile-xor-pack input length must be a nonzero multiple of tile size",
            value: src.len() as u64,
        });
    }
    let count = src.len() / tile_sz;
    let count_u16 = u16::try_from(count).map_err(|_| Error::InvalidField {
        what: "tile-xor-pack tile count exceeds u16",
        value: count as u64,
    })?;

    let m3 = mode & 3;
    let mut payload = Vec::new();
    let mut cmds = Vec::with_capacity(count);
    for tile in src.chunks_exact(tile_sz) {
        if m3 == 0 {
            // 1-bit flag mode: bit 1 => cmd 1 (masked literal)
            cmds.push(1u8);
            payload.extend_from_slice(&pack_masked(tile, mask_bytes));
        } else if tile.iter().all(|&b| b == 0) {
            cmds.push(0u8);
        } else {
            let masked = pack_masked(tile, mask_bytes);
            if mode < 0x10 && masked.len() >= tile_sz {
                cmds.push(3u8);
                payload.extend_from_slice(tile);
            } else {
                cmds.push(1u8);
                payload.extend_from_slice(&masked);
            }
        }
    }

    let flags_rel = 5usize
        .checked_add(payload.len())
        .and_then(|v| u16::try_from(v).ok())
        .ok_or(Error::ResourceLimit {
            resource: "flags_rel offset",
            requested: (5 + payload.len()) as u64,
            limit: u64::from(u16::MAX),
        })?;

    let mut flags = Vec::new();
    if m3 == 0 {
        for chunk in cmds.chunks(8) {
            let mut byte = 0u8;
            for (i, &c) in chunk.iter().enumerate() {
                if c == 1 {
                    byte |= 1 << i;
                }
            }
            flags.push(byte);
        }
    } else {
        for chunk in cmds.chunks(4) {
            let mut byte = 0u8;
            for (i, &c) in chunk.iter().enumerate() {
                byte |= (c & 3) << (i * 2);
            }
            flags.push(byte);
        }
    }

    let mut out = Vec::with_capacity(5 + payload.len() + flags.len());
    out.push(mode);
    out.extend_from_slice(&count_u16.to_le_bytes());
    out.extend_from_slice(&flags_rel.to_le_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(&flags);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_all_modes() {
        for (mode, tile_sz) in [
            (0x00, 16),
            (0x01, 32),
            (0x02, 64),
            (0x10, 16),
            (0x11, 32),
            (0x12, 64),
        ] {
            let mut data = vec![0u8; tile_sz]; // zero tile
            data.extend((0..tile_sz).map(|i| if i % 3 == 0 { (i + 1) as u8 } else { 0 }));
            data.extend((0..tile_sz).map(|i| (i * 7 + 3) as u8));
            let enc = encode_with_mode(&data, mode).unwrap();
            let dec = decode(&enc, MAX_DECODED).unwrap();
            assert_eq!(dec.bytes, data, "mode 0x{mode:02x}");
            assert_eq!(dec.consumed, enc.len(), "mode 0x{mode:02x}");
        }
    }

    #[test]
    fn decodes_xor_delta_commands_2_and_3() {
        // Mode 0x11 (32 B tiles, 2 planes):
        // Tile 0: cmd 2 (2B row-pair XOR per 16B plane) with mask bit 0 set -> byte 0 = 0xAA,
        //         which propagates to bytes 0, 2, 4, 6, 8, 10, 12, 14 of plane 0.
        // Tile 1: cmd 3 (1B cumulative XOR) with mask bit 0 set -> byte 0 = 0x55,
        //         which propagates to all 32 bytes.
        let mut stream = vec![0x11, 0x02, 0x00, 0x0f, 0x00]; // flags_rel = 5 + 5 + 5 = 15
        stream.extend_from_slice(&[0x01, 0x00, 0x00, 0x00, 0xaa]); // tile 0
        stream.extend_from_slice(&[0x01, 0x00, 0x00, 0x00, 0x55]); // tile 1
        stream.push(0x02 | (0x03 << 2)); // cmd 2, cmd 3
        let dec = decode(&stream, 64).unwrap();
        assert_eq!(dec.consumed, stream.len());
        let mut expected = vec![0u8; 64];
        for i in (0..16).step_by(2) {
            expected[i] = 0xaa;
        }
        for slot in &mut expected[32..64] {
            *slot = 0x55;
        }
        assert_eq!(dec.bytes, expected);
    }

    #[test]
    fn rejects_invalid_header_and_flags_mismatch() {
        assert!(decode(&[0x03, 0x01, 0x00, 0x05, 0x00, 0x00], 64).is_err());
        assert!(decode(&[0x01, 0x00, 0x00, 0x05, 0x00, 0x00], 64).is_err());
        // flags_rel mismatch: zero tile consumes 0 payload bytes, so si=5 != 6
        assert!(decode(&[0x01, 0x01, 0x00, 0x06, 0x00, 0x00, 0x00], 64).is_err());
    }
}
