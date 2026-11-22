//! Flat cartridge offset tables and bounded padding inspection.
use crate::{check_rom_identity, malformed, map::RomIdentity, AssetMap};
use formatkit_core::{bytes_at, classify_padding, PaddingKind, Result};
use formatkit_layout::tables::{FixedStrideOffsetTable, TableSizeMode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::ops::Range;

pub const SCHEMA: &str = "swankit-fixed-stride-table/1";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub schema: String,
    pub rom: RomIdentity,
    pub table_offset: usize,
    pub entry_count: usize,
    pub entry_stride: usize,
    pub offset_field: usize,
    pub offset_unit: u64,
    pub data_offset: usize,
    pub data_length: usize,
    pub size: Size,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding_min_bytes: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Size {
    ExplicitU32 { field_offset: usize, unit: u64 },
    SequentialUntil { sentinel_end: Option<u64> },
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub id: String,
    pub offset: usize,
    pub length: usize,
}

#[derive(Debug, Serialize)]
pub struct Padding {
    pub offset: usize,
    pub length: usize,
    pub kind: &'static str,
    pub fill: u32,
}

#[derive(Debug, Serialize)]
pub struct Resolution {
    pub schema: &'static str,
    pub rom: RomIdentity,
    pub entries: Vec<Entry>,
    pub padding: Vec<Padding>,
    #[serde(skip)]
    table_span: Range<usize>,
}

pub fn resolve(rom: &[u8], definition: &Definition) -> Result<Resolution> {
    if definition.schema != SCHEMA {
        return Err(malformed("unsupported fixed-stride table schema"));
    }
    check_rom_identity(&definition.rom, rom)?;
    if definition.entry_count == 0 || definition.entry_count > crate::map::MAX_ASSETS {
        return Err(malformed("fixed-stride table entry count out of range"));
    }
    let length = definition
        .entry_count
        .checked_mul(definition.entry_stride)
        .ok_or_else(|| malformed("fixed-stride table byte length overflow"))?;
    let table = bytes_at(rom, definition.table_offset, length)?;
    let table_span = definition.table_offset..definition.table_offset + table.len();
    let data = bytes_at(rom, definition.data_offset, definition.data_length)?;
    let descriptor = FixedStrideOffsetTable {
        entry_count: definition.entry_count,
        entry_stride: definition.entry_stride,
        offset_field: definition.offset_field,
        offset_unit: definition.offset_unit,
        size_mode: match definition.size {
            Size::ExplicitU32 { field_offset, unit } => {
                TableSizeMode::ExplicitU32 { field_offset, unit }
            }
            Size::SequentialUntil { sentinel_end } => {
                TableSizeMode::SequentialUntil { sentinel_end }
            }
        },
        name_prefix: "",
    };
    let entries = descriptor
        .parse_entries(table, data.len() as u64)?
        .into_iter()
        .enumerate()
        .map(|(index, row)| Entry {
            id: format!("entry.{index:04}"),
            offset: definition.data_offset + row.offset as usize,
            length: row.size as usize,
        })
        .collect::<Vec<_>>();
    let padding = definition
        .padding_min_bytes
        .map_or_else(Vec::new, |minimum| {
            let mut end = definition.data_offset;
            let mut pads = Vec::new();
            let mut inspect_gap = |start: usize, stop: usize| {
                if start < stop {
                    if let Some(p) = classify_padding(&rom[start..stop], minimum) {
                        let (kind, fill) = match p.kind {
                            PaddingKind::AllZero => ("all-zero", 0),
                            PaddingKind::RepeatedByte(byte) => ("repeated-byte", u32::from(byte)),
                            PaddingKind::RepeatedWord32(word) => ("repeated-word32", word),
                        };
                        pads.push(Padding {
                            offset: start,
                            length: stop - start,
                            kind,
                            fill,
                        });
                    }
                }
            };
            let data_end = definition.data_offset + data.len();
            let mut occupied: Vec<_> = entries
                .iter()
                .filter(|entry| entry.length != 0)
                .map(|entry| entry.offset..entry.offset + entry.length)
                .collect();
            let covered_table =
                table_span.start.max(definition.data_offset)..table_span.end.min(data_end);
            if !covered_table.is_empty() {
                occupied.push(covered_table);
            }
            occupied.sort_unstable_by_key(|span| span.start);
            for span in occupied {
                inspect_gap(end, span.start);
                end = end.max(span.end);
            }
            inspect_gap(end, data_end);
            pads
        });
    Ok(Resolution {
        schema: SCHEMA,
        rom: definition.rom.clone(),
        entries,
        padding,
        table_span,
    })
}

fn ordered_nonempty(entries: &[Entry]) -> Vec<&Entry> {
    let mut ordered: Vec<_> = entries.iter().filter(|entry| entry.length != 0).collect();
    ordered.sort_by_key(|entry| entry.offset);
    ordered
}

impl Resolution {
    /// Project disjoint, nonempty ranges into a Raw map without allocating any gaps.
    pub fn asset_map(&self) -> Result<AssetMap> {
        let entries = ordered_nonempty(&self.entries);
        if entries.is_empty() {
            return Err(malformed("table has no nonempty ranges for a map"));
        }
        let bound = self.rom.size.usize()?;
        if entries.iter().any(|entry| {
            entry
                .offset
                .checked_add(entry.length)
                .is_none_or(|end| end > bound)
        }) {
            return Err(malformed("table range exceeds declared ROM"));
        }
        if entries
            .windows(2)
            .any(|pair| pair[0].offset + pair[0].length > pair[1].offset)
        {
            return Err(malformed(
                "overlapping table ranges require an explicit asset map",
            ));
        }
        if let Some(entry) = entries.iter().find(|entry| {
            entry.offset < self.table_span.end
                && self.table_span.start < entry.offset + entry.length
        }) {
            return Err(malformed(format!(
                "{}: mapped range overlaps source table metadata",
                entry.id
            )));
        }
        let assets: Vec<_> = entries
            .into_iter()
            .map(|entry| {
                json!({
                    "id": entry.id, "kind": "other", "codec": "raw",
                    "offset": entry.offset, "length": entry.length, "references_complete": false,
                })
            })
            .collect();
        let map = json!({"schema": crate::SCHEMA, "rom": self.rom, "assets": assets});
        AssetMap::from_json(&serde_json::to_vec(&map).map_err(|e| malformed(e.to_string()))?)
    }
}
