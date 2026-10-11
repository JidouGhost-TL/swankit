//! Map verification against its ROM: decoding, ranges, overlaps, free space,
//! references, plus coverage statistics.
use crate::check_identity;
use crate::data::decode_asset;
use crate::extract::extract_one;
use crate::map::{AssetMap, Kind};
use crate::refs::{reference_matches, references_for};
use crate::text::Charmaps;
use formatkit_core::Result;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub id: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct VerifyReport {
    pub assets: usize,
    pub references: usize,
    pub errors: Vec<Finding>,
    /// Union of stored asset bytes.
    pub mapped_bytes: usize,
    pub free_bytes: usize,
    pub rom_bytes: usize,
    /// Stored bytes and counts by kind.
    pub by_kind: BTreeMap<String, (usize, usize)>,
    /// Stored bytes by confidence.
    pub by_confidence: BTreeMap<String, usize>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
    pub fn coverage(&self) -> f64 {
        self.mapped_bytes as f64 / self.rom_bytes.max(1) as f64
    }
}

fn union(mut ranges: Vec<(usize, usize)>) -> usize {
    ranges.sort();
    let (mut total, mut end) = (0usize, 0usize);
    for (start, stop) in ranges {
        if stop > end {
            total += stop - start.max(end);
            end = stop;
        }
    }
    total
}

/// Check every declared fact. Errors are collected; the ROM identity must match.
pub fn verify(map: &AssetMap, charmaps: &Charmaps, rom: &[u8]) -> Result<VerifyReport> {
    check_identity(map, rom)?;
    let mut report = VerifyReport {
        assets: map.assets.len(),
        rom_bytes: rom.len(),
        ..VerifyReport::default()
    };
    let mut err = |id: &str, message: String| {
        report.errors.push(Finding {
            id: id.into(),
            message,
        })
    };
    let mut ranges: Vec<(usize, usize, &str)> = Vec::new();
    for asset in &map.assets {
        match decode_asset(map, charmaps, rom, &asset.id) {
            Ok(s) => {
                ranges.push((s.offset, s.offset + s.length, asset.id.as_str()));
                let e = report.by_kind.entry(asset.kind.name().into()).or_default();
                e.0 += s.length;
                e.1 += 1;
                *report
                    .by_confidence
                    .entry(format!("{:?}", asset.confidence).to_lowercase())
                    .or_default() += s.length;
            }
            Err(e) => {
                err(&asset.id, format!("does not decode: {e}"));
                continue;
            }
        }
        if let Err(e) = extract_one(map, charmaps, rom, asset, &mut Vec::new()) {
            err(
                &asset.id,
                format!("{} does not render or parse: {e}", asset.kind.name()),
            );
        }
        match references_for(map, &asset.id) {
            Ok(refs) => {
                for r in refs {
                    report.references += 1;
                    let target = asset.offset.0 as usize;
                    match reference_matches(rom, r.at, &r.encoding, target) {
                        Ok((false, stored, expected)) => err(
                            &asset.id,
                            format!(
                                "reference at {:#x} ({}) holds {stored:#x}, expected {expected:#x}",
                                r.at, r.source
                            ),
                        ),
                        Ok(_) => {}
                        Err(e) => err(&asset.id, format!("reference at {:#x}: {e}", r.at)),
                    }
                }
            }
            Err(e) => err(&asset.id, e.to_string()),
        }
        if asset.kind == Kind::PointerTable && asset.format.pointer.is_none() {
            err(&asset.id, "pointer table needs format.pointer".into());
        }
    }
    let mut sorted = ranges.clone();
    sorted.sort();
    for (i, a) in sorted.iter().enumerate() {
        for b in sorted[i + 1..].iter().take_while(|b| b.0 < a.1) {
            let declared = |x: &str, y: &str| {
                map.asset(x)
                    .map(|asset| asset.overlaps.iter().any(|o| o == y))
                    .unwrap_or(false)
            };
            if !declared(a.2, b.2) && !declared(b.2, a.2) {
                err(
                    a.2,
                    format!(
                        "overlaps {} ({:#x}..{:#x} vs {:#x}..{:#x}) without a declaration",
                        b.2, a.0, a.1, b.0, b.1
                    ),
                );
            }
        }
    }
    let mut free = Vec::new();
    for f in &map.free_space {
        let (Ok(start), Ok(len)) = (f.offset.usize(), f.length.usize()) else {
            err("free_space", "range exceeds address width".into());
            continue;
        };
        let end = start.saturating_add(len);
        if len == 0 || end > rom.len().saturating_sub(ws_rom::FOOTER_SIZE) {
            err(
                "free_space",
                format!("{start:#x}+{len:#x} is empty, outside the ROM or in the footer"),
            );
            continue;
        }
        if let Some(a) = ranges.iter().find(|a| a.0 < end && start < a.1) {
            err(
                "free_space",
                format!("{start:#x}..{end:#x} overlaps asset {}", a.2),
            );
        }
        if let Some(fill) = f.fill {
            if let Some(p) = rom[start..end].iter().position(|&b| u64::from(b) != fill.0) {
                err(
                    "free_space",
                    format!("{:#x} is not the declared fill {:#x}", start + p, fill.0),
                );
            }
        }
        free.push((start, end));
    }
    report.free_bytes = union(free);
    report.mapped_bytes = union(ranges.iter().map(|r| (r.0, r.1)).collect());
    Ok(report)
}
