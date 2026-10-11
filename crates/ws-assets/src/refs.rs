//! Reference arithmetic: `stored = ((anchor(target) + add) >> shift) mod 2^(8*width)`.
//! The bits above the stored width (a bank or segment the code supplies
//! elsewhere) are implied; a relocated target must reproduce them exactly or
//! the reference cannot express it. `align` adds an alignment requirement.
use crate::malformed;
use crate::map::{Anchor, AssetMap, Codec, Encoding, Endian, Form, Kind};
use formatkit_core::{bytes_at, Result};
use ws_rom::RomGeometry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedReference {
    pub at: usize,
    pub encoding: Encoding,
    pub kind: String,
    /// `explicit` or `<pointer table id>[index]`.
    pub source: String,
}

fn anchor(encoding: &Encoding, target: usize, rom_len: usize) -> Result<i128> {
    let base = match encoding.anchor {
        Anchor::File => target,
        Anchor::Chip => target + RomGeometry::new(rom_len)?.padding_size(),
    };
    Ok(base as i128 + i128::from(encoding.add.0))
}

/// Full (unmasked) reference value for a target, rejecting misalignment.
fn full_value(encoding: &Encoding, target: usize, rom_len: usize) -> Result<i128> {
    let v = anchor(encoding, target, rom_len)?;
    if v < 0 {
        return Err(malformed(format!(
            "reference to {target:#x} is negative after add"
        )));
    }
    let align = i128::from(encoding.align.map_or(1, |a| a.0.max(1)));
    if v % align != 0 {
        return Err(malformed(format!(
            "target {target:#x} is not aligned to {align} bytes for this reference"
        )));
    }
    Ok(v >> encoding.shift)
}

fn mask(width: u8) -> i128 {
    (1i128 << (8 * u32::from(width))) - 1
}

/// Value stored at a reference.
pub fn read_reference(rom: &[u8], at: usize, encoding: &Encoding) -> Result<u64> {
    let raw = bytes_at(rom, at, usize::from(encoding.width))?;
    let fold = |acc: u64, b: &u8| (acc << 8) | u64::from(*b);
    Ok(match encoding.endian {
        Endian::Big => raw.iter().fold(0, fold),
        Endian::Little => raw.iter().rev().fold(0, fold),
    })
}

/// Far pointer (offset, segment) for a target inside the encoding's window.
fn far_parts(encoding: &Encoding, target: usize) -> Result<(u16, u16)> {
    let target = usize::try_from(target as i64 + encoding.add.0)
        .map_err(|_| malformed("far reference target negative after add"))?;
    let window = encoding.window.unwrap_or_default().usize()?;
    let base = encoding.segment_base.unwrap_or_default().0 as usize;
    if !(window..window + 0x10000).contains(&target) {
        return Err(malformed(format!(
            "target {target:#x} is outside the far pointer's 64 KiB window at {window:#x}"
        )));
    }
    let linear = base * 16 + (target - window);
    Ok(match encoding.normalize.as_deref() {
        Some("paragraph") => ((linear & 15) as u16, (linear >> 4) as u16),
        _ => ((target - window) as u16, base as u16),
    })
}

/// Whether the stored reference designates `target`.
pub fn reference_matches(
    rom: &[u8],
    at: usize,
    encoding: &Encoding,
    target: usize,
) -> Result<(bool, u64, u64)> {
    let stored = read_reference(rom, at, encoding)?;
    if encoding.form == Form::Far {
        let (off, seg) = (stored & 0xffff, stored >> 16);
        let window = encoding.window.unwrap_or_default().usize()?;
        let base = encoding.segment_base.unwrap_or_default().0;
        let (eo, es) = far_parts(encoding, target)?;
        let expected = u64::from(es) << 16 | u64::from(eo);
        let linear = seg * 16 + off;
        let ok = linear >= base * 16
            && ((linear - base * 16) as usize + window) as i64 == target as i64 + encoding.add.0;
        return Ok((ok, stored, expected));
    }
    let expected = expected_value(encoding, target, rom.len())?;
    Ok((stored == expected, stored, expected))
}

/// Bytes a reference must hold for `target`; `previous` (the current target)
/// fixes the implied high bits, which must not change.
pub fn encode_reference(
    encoding: &Encoding,
    target: usize,
    previous: usize,
    rom_len: usize,
) -> Result<Vec<u8>> {
    if encoding.form == Form::Far {
        let (off, seg) = far_parts(encoding, target)?;
        let mut out = off.to_le_bytes().to_vec();
        out.extend_from_slice(&seg.to_le_bytes());
        return Ok(out);
    }
    let new = full_value(encoding, target, rom_len)?;
    let old = full_value(encoding, previous, rom_len)?;
    let bits = 8 * u32::from(encoding.width);
    if !encoding.companion && new >> bits != old >> bits {
        return Err(malformed(format!(
            "target {target:#x} leaves the {}-byte reference's implied range of {previous:#x} (bank or segment limit)",
            encoding.width
        )));
    }
    let value = (new & mask(encoding.width)) as u64;
    if let Some(below) = encoding.below.filter(|b| value >= b.0) {
        return Err(malformed(format!(
            "target {target:#x} needs stored value {value:#x}; the referencing code accepts values below {:#x}",
            below.0
        )));
    }
    let le: Vec<u8> = (0..encoding.width)
        .map(|i| (value >> (8 * i)) as u8)
        .collect();
    Ok(match encoding.endian {
        Endian::Little => le,
        Endian::Big => le.into_iter().rev().collect(),
    })
}

/// Expected stored value for a target.
pub fn expected_value(encoding: &Encoding, target: usize, rom_len: usize) -> Result<u64> {
    Ok((full_value(encoding, target, rom_len)? & mask(encoding.width)) as u64)
}

/// Every known reference to an asset: explicit ones plus pointer-table entries.
pub fn references_for(map: &AssetMap, id: &str) -> Result<Vec<ResolvedReference>> {
    let asset = map.asset(id)?;
    let mut out: Vec<ResolvedReference> = asset
        .references
        .iter()
        .map(|r| {
            Ok(ResolvedReference {
                at: r.at.usize()?,
                encoding: r.encoding(),
                kind: r.kind.clone(),
                source: "explicit".into(),
            })
        })
        .collect::<Result<_>>()?;
    for table in map.assets.iter().filter(|a| a.kind == Kind::PointerTable) {
        for (index, target) in table.relations.targets.iter().enumerate() {
            if target.as_deref() != Some(id) {
                continue;
            }
            if table.codec != Codec::Raw {
                return Err(malformed(format!(
                    "{}: pointer tables must be raw",
                    table.id
                )));
            }
            let encoding = table.format.pointer.clone().ok_or_else(|| {
                malformed(format!("{}: pointer table needs format.pointer", table.id))
            })?;
            let stride = table
                .format
                .stride
                .map_or(Ok(usize::from(encoding.width)), |s| s.usize())?;
            let inner = table.format.pointer_offset.unwrap_or_default().usize()?;
            let at = index
                .checked_mul(stride)
                .and_then(|n| n.checked_add(table.offset.usize().ok()?))
                .and_then(|n| n.checked_add(inner))
                .ok_or_else(|| malformed("pointer entry offset overflow"))?;
            out.push(ResolvedReference {
                at,
                encoding,
                kind: "pointer-table-entry".into(),
                source: format!("{}[{index}]", table.id),
            });
        }
    }
    Ok(out)
}
