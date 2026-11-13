//! Conditional source coordinates for eight-word table reads.
//!
//! Addressing and banking follow WSdev Memory_map and Mapper, through ws-rom.
//! Callers supply the selected source segment; destination DS/SS is not inferred.
use crate::{malformed, map::Num, sha256_hex, AssetMap};
use formatkit_core::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ws_rom::{BankState, Header, RomGeometry};

pub const SCHEMA: &str = "swankit-table-parameters/1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableDefinition {
    pub schema: String,
    pub rom: crate::map::RomIdentity,
    pub source_segment: u16,
    pub source_base: u16,
    pub route: Route,
    pub context: Context,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Route {
    Record { entry_al: u8 },
    AlignedFallthrough,
    CoreUnion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Context {
    Explicit {
        banks: Vec<Registers>,
    },
    RetainedCode {
        segment: u16,
        offset: u16,
        file_offset: Num,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registers {
    pub c0: u8,
    pub c2: u16,
    pub c3: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CompatibleContext {
    pub c0: u8,
    /// None means unbound and unused, rather than a zero bank selection.
    pub c2: Option<u16>,
    pub c3: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Slice {
    pub file_offset: Num,
    pub effective_offset: Num,
    pub length: usize,
    pub sha256: String,
    pub context_indices: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resolution {
    pub schema: &'static str,
    pub definition: TableDefinition,
    pub mapper_code: u8,
    pub effective_size: usize,
    pub padding_size: usize,
    pub first_si: u16,
    pub last_si: u16,
    pub first_cpu_address: u32,
    pub last_cpu_address: u32,
    pub compatible_contexts: Vec<CompatibleContext>,
    pub slices: Vec<Slice>,
    pub decision: &'static str,
    pub conditions: Vec<&'static str>,
}

impl TableDefinition {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 1024 * 1024 {
            return Err(malformed("table parameter definition exceeds 1 MiB"));
        }
        serde_json::from_slice(bytes).map_err(|e| malformed(e.to_string()))
    }
}

impl Resolution {
    /// An exact Raw slice using the existing schema and editing behavior.
    /// This does not add an extraction-only permission or a resource identity.
    pub fn asset_map(&self) -> Result<AssetMap> {
        if self.slices.len() != 1 {
            return Err(malformed("ambiguous source mapping; no single Raw map"));
        }
        let s = &self.slices[0];
        AssetMap::from_json(&serde_json::to_vec(&serde_json::json!({
            "schema": crate::SCHEMA,
            "rom": self.definition.rom,
            "assets": [{
                "id": "conditional-source-domain", "kind": "other", "codec": "raw",
                "offset": s.file_offset, "length": s.length,
                "confidence": "low", "references_complete": false,
                "evidence": {"notes": "Conditional source-read domain under the separately recorded route and bank context. A union is not a named resource or an editing declaration."}
            }]
        })).map_err(|e| malformed(e.to_string()))?)
    }
}

/// Resolve every declared bank possibility, refusing unbacked or noncontiguous
/// reads. A retained C0 code coordinate is a condition, not observed execution.
pub fn resolve(rom: &[u8], definition: &TableDefinition) -> Result<Resolution> {
    if definition.schema != SCHEMA {
        return Err(malformed("unsupported table parameter schema"));
    }
    if definition.rom.sha256.len() != 64
        || !definition.rom.sha256.bytes().all(|x| x.is_ascii_hexdigit())
        || definition.rom.size.usize()? != rom.len()
        || !definition.rom.sha256.eq_ignore_ascii_case(&sha256_hex(rom))
    {
        return Err(malformed("original ROM identity mismatch"));
    }
    let geometry = RomGeometry::new(rom.len())?;
    let header = Header::parse(rom)?;
    let mapper = header.mapper();
    let (add, length) = match definition.route {
        Route::Record { entry_al } => (u32::from(entry_al), 16u32),
        Route::AlignedFallthrough => (0, 256),
        Route::CoreUnion => (0, 271),
    };
    let first_si = u32::from(definition.source_base) + add;
    let last_si = first_si + length - 1;
    if last_si > 0xffff {
        return Err(malformed(
            "source SI wraps; ordered fragments are not supported",
        ));
    }
    let first = ((u32::from(definition.source_segment) << 4) + first_si) & 0xfffff;
    let last = ((u32::from(definition.source_segment) << 4) + last_si) & 0xfffff;
    if first >> 16 != last >> 16 || last < first {
        return Err(malformed("source crosses a CPU mapping window"));
    }
    if first < 0x20000 {
        return Err(malformed("source address is RAM or SRAM, not ROM"));
    }
    let mut contexts = BTreeSet::new();
    match &definition.context {
        Context::Explicit { banks } => {
            if banks.is_empty() || banks.len() > 64 {
                return Err(malformed("supply one to 64 explicit bank contexts"));
            }
            for r in banks {
                let b = BankState {
                    c0: r.c0,
                    c2: r.c2,
                    c3: r.c3,
                }
                .for_mapper(mapper)?;
                contexts.insert(CompatibleContext {
                    c0: b.c0,
                    c2: Some(b.c2),
                    c3: Some(b.c3),
                });
            }
        }
        Context::RetainedCode {
            segment,
            offset,
            file_offset,
        } => {
            let cpu = ((u32::from(*segment) << 4) + u32::from(*offset)) & 0xfffff;
            if cpu < 0x40000 || first < 0x40000 {
                return Err(malformed("retained code inference requires C0-backed code and source; C2/C3 must be explicit"));
            }
            let file = file_offset.usize()?;
            if file >= rom.len() {
                return Err(malformed("retained code file offset exceeds ROM"));
            }
            for c0 in 0..64 {
                // C2/C3 are not used for these C0-backed addresses.
                let b = BankState { c0, c2: 0, c3: 0 }.for_mapper(mapper)?;
                if geometry.address_to_file_offset(cpu, b) == Some(file) {
                    contexts.insert(CompatibleContext {
                        c0: b.c0,
                        c2: None,
                        c3: None,
                    });
                }
            }
            if contexts.is_empty() {
                return Err(malformed(
                    "no C0 context matches the retained code coordinate",
                ));
            }
        }
    }
    let contexts: Vec<_> = contexts.into_iter().collect();
    let mut slices: BTreeMap<usize, Slice> = BTreeMap::new();
    for (index, c) in contexts.iter().enumerate() {
        let b = match (c.c2, c.c3) {
            (Some(c2), Some(c3)) => BankState { c0: c.c0, c2, c3 },
            (None, None) if first >= 0x40000 => BankState {
                c0: c.c0,
                c2: 0,
                c3: 0,
            },
            _ => return Err(malformed("source requires unbound bank registers")),
        };
        // Inference only uses C0-backed addresses; C2/C3 remain explicitly
        // unbound in its receipt. Explicit contexts bind all three registers.
        let effective = geometry
            .address_to_effective_offset(first, b)
            .ok_or_else(|| malformed("source address is not ROM"))?;
        let file = geometry
            .address_to_file_offset(first, b)
            .ok_or_else(|| malformed("source lands in unbacked chip padding"))?;
        for i in 0..length {
            if geometry.address_to_file_offset(first + i, b) != Some(file + i as usize) {
                return Err(malformed(
                    "source is padding or not a contiguous file slice",
                ));
            }
        }
        let end = file
            .checked_add(length as usize)
            .ok_or_else(|| malformed("source range overflow"))?;
        let bytes = rom
            .get(file..end)
            .ok_or_else(|| malformed("source exceeds ROM"))?;
        slices
            .entry(file)
            .or_insert_with(|| Slice {
                file_offset: Num(file as u64),
                effective_offset: Num(effective as u64),
                length: length as usize,
                sha256: sha256_hex(bytes),
                context_indices: Vec::new(),
            })
            .context_indices
            .push(index);
    }
    let slices: Vec<_> = slices.into_values().collect();
    Ok(Resolution {
        schema: "swankit-table-resolution/1", definition: definition.clone(), mapper_code: header.mapper_code,
        effective_size: geometry.effective_size(), padding_size: geometry.padding_size(),
        first_si: first_si as u16, last_si: last_si as u16, first_cpu_address: first, last_cpu_address: last,
        decision: if slices.len() == 1 { "unique" } else { "ambiguous" }, compatible_contexts: contexts, slices,
        conditions: vec!["Selected source segment and route are supplied premises, not inferred DS/SS or invocation.", "Aligned fallthrough requires the source AL shift; alternate entries may bypass it.", "Core union describes possible reads, not one named resource.", "Bank context must remain stable over the reads; retained code coordinates are conditional."],
    })
}
