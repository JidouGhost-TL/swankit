//! ROM-only leads, not asset maps or evidence of runtime use.
//! Hardware layouts: docs/hardware.md and WSdev Display/Screen, Display/Palette.
mod banked_constructor;
mod reader_cores;

pub use reader_cores::{
    Captures as ReaderCaptures, DestinationSegment, Match as ReaderCore, SecondByteRegister,
    SourceStatus as ReaderSourceStatus, Variant as ReaderVariant,
};

use formatkit_core::{Error, Reader, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "swankit-format-search/1";
const MAX_ROM: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Family {
    #[serde(rename = "lz-u8-zero")]
    LzU8Zero,
    #[serde(rename = "lz-rle-7f")]
    LzRle7f,
    #[serde(rename = "lz-bits-16")]
    LzBits16,
    #[serde(rename = "lz-word16-msb")]
    LzWord16Msb,
    #[serde(rename = "lz-word-counted")]
    LzWordCounted,
    Rgb444,
    ScreenMap,
    #[serde(rename = "sgd1-tile-grid")]
    Sgd1TileGrid,
    #[serde(rename = "tile-map-record16")]
    TileMapRecord16,
    #[serde(rename = "counted-relative-directory16")]
    CountedRelativeDirectory16,
    ReaderSignature,
    /// Explicit opt-in fixed-role code pattern; no flow or asset inference.
    BankedQueueConstructor,
    /// Opt-in relational arithmetic core; no stream/header profile is inferred.
    TokenDistance12Length4Plus3,
    /// Opt-in indexed eight-word copy; source backing and entry values stay open.
    IndexedEightWordCopy,
    ReferenceBytes,
}

fn reader_family() -> Family {
    Family::ReaderSignature
}

/// External code patterns are hints about readers, not proof of resource formats.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub id: String,
    pub bytes: Vec<u8>,
    /// Per-bit mask; omitted means all bits must match.
    #[serde(default)]
    pub mask: Option<Vec<u8>>,
    #[serde(default = "reader_family")]
    pub family: Family,
}

/// Global preserves the established shared probe ceiling. Per-family repeats
/// the configured scan independently, with that ceiling for each family.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BudgetScope {
    #[default]
    Global,
    PerFamily,
}

impl BudgetScope {
    fn is_global(&self) -> bool {
        *self == Self::Global
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    #[serde(skip_serializing_if = "BudgetScope::is_global")]
    pub budget_scope: BudgetScope,
    pub families: Vec<Family>,
    pub stride: usize,
    pub max_probes: usize,
    pub max_stored: usize,
    pub max_decoded: usize,
    pub min_decoded: usize,
    /// Zero disables the savings filter and permits literal-only framing leads.
    pub min_savings: usize,
    pub max_command_words: usize,
    pub leads_per_family: usize,
    pub signatures: Vec<Signature>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            budget_scope: BudgetScope::Global,
            families: vec![
                Family::LzU8Zero,
                Family::LzRle7f,
                Family::LzBits16,
                Family::LzWord16Msb,
                Family::LzWordCounted,
                Family::Rgb444,
                Family::ScreenMap,
                Family::ReaderSignature,
                Family::ReferenceBytes,
            ],
            stride: 1,
            max_probes: 64_000_000,
            max_stored: 8192,
            max_decoded: 16384,
            min_decoded: 64,
            min_savings: 8,
            max_command_words: 256,
            leads_per_family: 64,
            signatures: Vec::new(),
        }
    }
}

impl Options {
    pub fn validate(&self) -> Result<()> {
        let invalid = |s: &str| Error::Malformed(format!("format search: {s}"));
        if self.families.is_empty()
            || self.families.iter().collect::<BTreeSet<_>>().len() != self.families.len()
        {
            return Err(invalid("families must be nonempty and unique"));
        }
        if self.stride == 0 || self.stride > 65536 {
            return Err(invalid("stride must be 1..65536"));
        }
        if self.max_probes == 0 || self.max_probes > 1_000_000_000 {
            return Err(invalid("max_probes must be 1..1000000000"));
        }
        if !(4..=65536).contains(&self.max_stored)
            || !(32..=65536).contains(&self.max_decoded)
            || self.min_decoded < 32
            || self.min_decoded > self.max_decoded
            || self.min_savings > 65536
            || !(1..=32767).contains(&self.max_command_words)
            || !(1..=1024).contains(&self.leads_per_family)
            || self.signatures.len() > 128
        {
            return Err(invalid(
                "invalid input/output, command, signature or lead bounds",
            ));
        }
        let mut ids = BTreeSet::new();
        for signature in &self.signatures {
            if signature.id.is_empty()
                || signature.id.len() > 128
                || !ids.insert(&signature.id)
                || !(4..=512).contains(&signature.bytes.len())
                || !matches!(
                    signature.family,
                    Family::ReaderSignature | Family::ReferenceBytes
                )
                || !self.families.contains(&signature.family)
                || (signature.family == Family::ReferenceBytes
                    && (signature.mask.is_some()
                        || signature.bytes.len() < 16
                        || !diverse(&signature.bytes, 3)))
            {
                return Err(invalid(
                    "signature ids must be unique, lengths 4..512, and families enabled; reference bytes must be exact, nonuniform and at least16bytes",
                ));
            }
            if let Some(mask) = &signature.mask {
                if mask.len() != signature.bytes.len()
                    || mask.iter().map(|b| b.count_ones()).sum::<u32>() < 32
                {
                    return Err(invalid(
                        "signature mask must match length and constrain at least 32 bits",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Lead {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory_provenance: Option<DirectoryProvenance>,
    pub family: Family,
    pub offset: usize,
    pub stored_length: usize,
    pub decoded_length: Option<usize>,
    pub stored_sha256: String,
    pub decoded_sha256: Option<String>,
    pub signature_id: Option<String>,
    /// Ranking weight only. It is not a probability or an asset confidence grade.
    pub rank: u32,
    pub evidence: String,
    pub requires: Vec<String>,
    pub reference_matches: Vec<ReferenceMatch>,
    pub reference_matches_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_provenance: Option<crate::retained::ProvenanceReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reader_core: Option<ReaderCore>,
}

/// Structural occurrence identity; aliases and alternative inferred bases remain
/// separate occurrences. Group span includes the directory and all records.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectoryProvenance {
    pub base: usize,
    pub group_length: usize,
    pub entry_count: usize,
    pub entry_index: usize,
    pub distinct_records: usize,
    /// First directory entry with the same target within this group.
    pub target_entry_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReferenceMatch {
    pub id: String,
    pub decoded_offset: usize,
    pub length: usize,
    pub reference_sha256: String,
    /// Exact contiguous occurrence in this stored span; not a provenance claim.
    pub already_present_in_stored_input: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureSummary {
    pub id: String,
    pub family: Family,
    pub length: usize,
    pub masked: bool,
    pub bytes_sha256: String,
    pub mask_sha256: Option<String>,
}

/// Report metadata deliberately omits the external pattern bytes and masks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportOptions {
    #[serde(default)]
    pub budget_scope: BudgetScope,
    pub families: Vec<Family>,
    pub stride: usize,
    pub max_probes: usize,
    pub max_stored: usize,
    pub max_decoded: usize,
    pub min_decoded: usize,
    pub min_savings: usize,
    pub max_command_words: usize,
    pub leads_per_family: usize,
    pub signatures: Vec<SignatureSummary>,
}
impl From<&Options> for ReportOptions {
    fn from(options: &Options) -> Self {
        Self {
            budget_scope: options.budget_scope,
            families: options.families.clone(),
            stride: options.stride,
            max_probes: options.max_probes,
            max_stored: options.max_stored,
            max_decoded: options.max_decoded,
            min_decoded: options.min_decoded,
            min_savings: options.min_savings,
            max_command_words: options.max_command_words,
            leads_per_family: options.leads_per_family,
            signatures: options
                .signatures
                .iter()
                .map(|s| SignatureSummary {
                    id: s.id.clone(),
                    family: s.family,
                    length: s.bytes.len(),
                    masked: s.mask.is_some(),
                    bytes_sha256: ws_assets::sha256_hex(&s.bytes),
                    mask_sha256: s.mask.as_ref().map(|m| ws_assets::sha256_hex(m)),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Counts {
    pub candidates: usize,
    pub retained: usize,
    pub probes: usize,
    pub rejected_probes: usize,
    pub probes_skipped_by_budget: usize,
    pub decoded_parses: usize,
    pub filtered_parses: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchReport {
    #[serde(default)]
    pub complete_by_family: BTreeMap<Family, bool>,
    pub schema: String,
    pub rom_sha256: String,
    pub rom_bytes: usize,
    pub options: ReportOptions,
    pub configuration_sha256: String,
    pub positions_scanned: usize,
    pub complete_for_configured_scan: bool,
    pub decoder_probes: usize,
    pub signature_probes: usize,
    pub decoded_reference_probes: usize,
    pub decoded_reference_searches_skipped: usize,
    pub counts: BTreeMap<Family, Counts>,
    pub leads: Vec<Lead>,
    pub overlapping_codec_leads: Vec<OverlapGroup>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlapGroup {
    pub start: usize,
    pub end: usize,
    pub lead_indices: Vec<usize>,
    /// Ranking choice only; not a source-boundary or independence verdict.
    pub preferred_lead: usize,
}

fn overlapping(leads: &[Lead]) -> Vec<OverlapGroup> {
    let mut indices: Vec<_> = leads
        .iter()
        .enumerate()
        .filter(|(_, l)| l.decoded_length.is_some())
        .map(|(i, _)| i)
        .collect();
    indices.sort_by_key(|&i| (leads[i].offset, leads[i].stored_length, leads[i].family));
    let mut groups: Vec<OverlapGroup> = Vec::new();
    for i in indices {
        let lead = &leads[i];
        if let Some(group) = groups.last_mut().filter(|g| lead.offset < g.end) {
            group.end = group.end.max(lead.offset + lead.stored_length);
            group.lead_indices.push(i);
            let preferred = &leads[group.preferred_lead];
            if (lead.rank, std::cmp::Reverse(lead.offset))
                > (preferred.rank, std::cmp::Reverse(preferred.offset))
            {
                group.preferred_lead = i;
            }
        } else {
            groups.push(OverlapGroup {
                start: lead.offset,
                end: lead.offset + lead.stored_length,
                lead_indices: vec![i],
                preferred_lead: i,
            });
        }
    }
    groups.retain(|g| g.lead_indices.len() > 1);
    groups
}

fn word(src: &[u8]) -> Option<u16> {
    Reader::new(src).u16_le().ok()
}

fn diverse(bytes: &[u8], minimum: usize) -> bool {
    let mut seen = [false; 256];
    let mut count = 0;
    for &byte in bytes {
        if !seen[usize::from(byte)] {
            seen[usize::from(byte)] = true;
            count += 1;
            if count >= minimum {
                return true;
            }
        }
    }
    false
}

fn palette(src: &[u8]) -> bool {
    let Some(bytes) = src.get(..32) else {
        return false;
    };
    let mut reader = Reader::new(bytes);
    let mut colors = [0u16; 16];
    for slot in &mut colors {
        let Ok(color) = reader.u16_le() else {
            return false;
        };
        if color & 0xf000 != 0 {
            return false;
        }
        *slot = color;
    }
    colors.sort_unstable();
    1 + colors.windows(2).filter(|p| p[0] != p[1]).count() >= 4
}

fn screen_map(src: &[u8]) -> bool {
    let Some(bytes) = src.get(..64) else {
        return false;
    };
    let mut reader = Reader::new(bytes);
    let mut tiles = [false; 512];
    let mut tile_count = 0;
    let mut palettes = 0u16;
    let mut previous = None;
    let mut nearby = 0;
    for _ in 0..32 {
        let Ok(cell) = reader.u16_le() else {
            return false;
        };
        let tile = cell & 0x1ff;
        if let Some(last) = previous {
            if tile.abs_diff(last) <= 32 {
                nearby += 1;
            }
        }
        previous = Some(tile);
        if !tiles[usize::from(tile)] {
            tiles[usize::from(tile)] = true;
            tile_count += 1;
        }
        palettes |= 1 << ((cell >> 9) & 15);
        if palettes.count_ones() > 3 {
            return false;
        }
    }
    tile_count >= 8 && nearby >= 24
}

fn prefilter(family: Family, src: &[u8], options: &Options) -> bool {
    if src.len() < 4 || !diverse(&src[..src.len().min(16)], 2) {
        return false;
    }
    match family {
        Family::LzU8Zero => (1..=127).contains(&src[0]),
        Family::LzRle7f => src[0] >= 0x80,
        Family::LzBits16 => word(src).is_some_and(|flags| flags & 1 != 0),
        Family::LzWord16Msb => {
            if src[0] == 0 {
                word(&src[1..]).is_some_and(|count| {
                    usize::from(count) >= options.min_decoded
                        && usize::from(count) <= options.max_decoded
                        && usize::from(count) + 3 <= src.len()
                })
            } else {
                word(&src[1..]).is_some_and(|f| f & 0x8000 != 0)
            }
        }
        Family::LzWordCounted => {
            let Some(count) = word(src).map(usize::from) else {
                return false;
            };
            let Some(first) = word(&src[2..]) else {
                return false;
            };
            count > 0
                && count <= options.max_command_words
                && 2 + count * 2 <= src.len()
                && first & 0x8000 == 0
                && if first & 0x4000 != 0 {
                    (first >> 10) & 15 != 0
                } else {
                    first != 0 && usize::from(first) * 2 <= options.max_decoded
                }
        }
        _ => false,
    }
}

fn decode(
    family: Family,
    src: &[u8],
    limit: usize,
    offset: usize,
) -> Result<ws_compress::DecodeOutcome> {
    match family {
        Family::LzU8Zero => ws_compress::lz_u8_zero::decode(src, limit),
        Family::LzRle7f => ws_compress::lz_rle_7f::decode(src, limit),
        Family::LzBits16 => ws_compress::lz_bits_16::decode(src, limit),
        Family::LzWord16Msb => ws_compress::lz_word16_msb::decode(src, limit),
        Family::LzWordCounted => ws_compress::lz_word_counted::decode_at(src, limit, offset),
        _ => Err(Error::Malformed("not a compressed search family".into())),
    }
}

fn promising(
    report: &mut SearchReport,
    kept: &BTreeMap<Family, Vec<Lead>>,
    family: Family,
    offset: usize,
    rank: u32,
) -> bool {
    report.counts.entry(family).or_default().candidates += 1;
    kept.get(&family).is_none_or(|leads| {
        leads.len() < report.options.leads_per_family
            || leads.last().is_some_and(|last| {
                rank > last.rank || (rank == last.rank && offset <= last.offset)
            })
    })
}

fn retain(report: &mut SearchReport, kept: &mut BTreeMap<Family, Vec<Lead>>, lead: Lead) {
    let entries = kept.entry(lead.family).or_default();
    entries.push(lead);
    entries.sort_by(|a, b| {
        b.rank
            .cmp(&a.rank)
            .then(a.offset.cmp(&b.offset))
            .then(a.signature_id.cmp(&b.signature_id))
    });
    entries.truncate(report.options.leads_per_family);
}

struct PreparedPattern<'a> {
    signature: &'a Signature,
    finder: memchr::memmem::Finder<'a>,
    sha256: String,
}

pub(crate) fn reader_pattern(
    family: Family,
    src: &[u8],
    preceding: &[u8],
) -> Option<(usize, ReaderCore)> {
    match family {
        Family::IndexedEightWordCopy => reader_cores::table(src, preceding),
        Family::TokenDistance12Length4Plus3 => reader_cores::token(src),
        _ => None,
    }
}

pub(crate) fn reader_instruction_lengths(core: &ReaderCore) -> Vec<usize> {
    reader_cores::instruction_lengths(core)
}

fn reader_core_leads(
    rom: &[u8],
    options: &Options,
    report: &mut SearchReport,
    kept: &mut BTreeMap<Family, Vec<Lead>>,
) {
    for (family, prefix) in [
        (Family::IndexedEightWordCopy, &[0xbf][..]),
        (
            Family::TokenDistance12Length4Plus3,
            &[0x4b, 0x26, 0x8a, 0x0f][..],
        ),
    ] {
        if !options.families.contains(&family) {
            continue;
        }
        let finder = memchr::memmem::Finder::new(prefix);
        let mut start = 0;
        while start < rom.len() {
            let Some(found) = finder.find(&rom[start..]) else {
                break;
            };
            let offset = start + found;
            start = offset + 1;
            if !offset.is_multiple_of(options.stride) {
                continue;
            }
            if budget_used(report) >= options.max_probes {
                report.complete_for_configured_scan = false;
                report
                    .counts
                    .entry(family)
                    .or_default()
                    .probes_skipped_by_budget += 1;
                break;
            }
            report.signature_probes += 1;
            report.counts.entry(family).or_default().probes += 1;
            let Some((length, core)) = reader_pattern(family, &rom[offset..], &rom[..offset])
            else {
                report.counts.entry(family).or_default().rejected_probes += 1;
                continue;
            };
            if !promising(report, kept, family, offset, 250) {
                continue;
            }
            retain(report, kept, Lead {
                family, offset, stored_length: length, decoded_length: None,
                stored_sha256: ws_assets::sha256_hex(&rom[offset..offset + length]),
                decoded_sha256: None, signature_id: Some(format!("{}-v{}", core.pattern, core.version)),
                rank: 250,
                evidence: "Relational instruction bytes matched with bounded literal captures. This is not proof of instruction boundaries, a callable routine, a source profile or resource ownership.".into(),
                requires: vec![
                    "Join every original instruction boundary before treating the range as retained code; entry paths and callee effects remain separate.".into(),
                    "Establish input values and segment/bank backing; an arithmetic core does not establish header units, flag order, helper behavior, stream start, media or SDK ancestry.".into(),
                ],
                reference_matches: Vec::new(), reference_matches_truncated: false,
                source_provenance: None, reader_core: Some(core), directory_provenance: None,
            });
        }
    }
}
fn budget_used(report: &SearchReport) -> usize {
    report.decoder_probes
        + report.signature_probes
        + report.decoded_reference_probes
        + report
            .counts
            .get(&Family::TileMapRecord16)
            .map_or(0, |counts| counts.probes)
        + report
            .counts
            .get(&Family::Sgd1TileGrid)
            .map_or(0, |counts| counts.probes)
        + report
            .counts
            .get(&Family::CountedRelativeDirectory16)
            .map_or(0, |counts| counts.probes)
}

/// Return the accepted full record size when an SGD1 header satisfies the
/// measured tile-area relation. Bytes 6..14 remain unknown and unconstrained.
fn sgd1_tile_grid_length(src: &[u8]) -> Option<usize> {
    if src.get(..4)? != b"SGD1" || src.len() < 16 {
        return None;
    }
    let mut reader = Reader::new(src);
    reader.skip(4).ok()?;
    let width = usize::from(reader.u8().ok()?);
    let height = usize::from(reader.u8().ok()?);
    reader.skip(8).ok()?;
    let declared = usize::from(reader.u16_le().ok()?);
    if width == 0 || height == 0 {
        return None;
    }
    let payload = width.checked_mul(height)?.checked_mul(16)?;
    let expected = 16usize.checked_add(payload)?;
    if declared != expected || src.get(..declared).is_none() {
        return None;
    }
    Some(declared)
}

fn sgd1_tile_grid_leads(
    rom: &[u8],
    options: &Options,
    report: &mut SearchReport,
    kept: &mut BTreeMap<Family, Vec<Lead>>,
) {
    let finder = memchr::memmem::Finder::new(b"SGD1");
    let mut start = 0;
    while rom.len().saturating_sub(start) >= 4 {
        let Some(found) = finder.find(&rom[start..]) else {
            break;
        };
        let offset = start + found;
        start = offset + 1;
        if offset % options.stride != 0 {
            continue;
        }
        if budget_used(report) >= options.max_probes {
            report.complete_for_configured_scan = false;
            report
                .counts
                .entry(Family::Sgd1TileGrid)
                .or_default()
                .probes_skipped_by_budget += 1;
            break;
        }
        report
            .counts
            .entry(Family::Sgd1TileGrid)
            .or_default()
            .probes += 1;
        let src = &rom[offset..];
        let Some(record_length) = sgd1_tile_grid_length(src) else {
            report
                .counts
                .entry(Family::Sgd1TileGrid)
                .or_default()
                .rejected_probes += 1;
            continue;
        };
        if promising(report, kept, Family::Sgd1TileGrid, offset, 100) {
            retain(report, kept, Lead { directory_provenance: None,
                family: Family::Sgd1TileGrid,
                offset,
                stored_length: record_length,
                decoded_length: None,
                stored_sha256: ws_assets::sha256_hex(&src[..record_length]),
                decoded_sha256: None,
                signature_id: None,
                rank: 100,
                evidence: "The SGD1 tag, positive byte grid, declared full record size and 16-byte-per-cell area agree; this structural match does not establish field meanings, a reader, runtime use or asset identity.".into(),
                requires: vec!["Establish the resource reader, field semantics, palette and runtime context before assigning meaning to this bounded record.".into()],
                reference_matches: Vec::new(),
                reference_matches_truncated: false, source_provenance: None, reader_core: None,
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CountedRecord16 {
    relative_offset: usize,
    byte_length: usize,
}

#[derive(Debug)]
enum CountedDirectory16Parse {
    Rejected {
        probes: usize,
    },
    Budget {
        probes: usize,
    },
    Match {
        probes: usize,
        entries: Vec<CountedRecord16>,
    },
}

fn counted_record16_length(units: usize) -> Option<usize> {
    2usize.checked_add(units.checked_mul(16)?)
}

fn counted_record16_length_at(src: &[u8], offset: usize, header: &[u8]) -> Option<usize> {
    let units = usize::from(Reader::new(header).u16_le().ok()?);
    if units == 0 {
        return None;
    }
    let byte_length = counted_record16_length(units)?;
    src.get(offset..offset.checked_add(byte_length)?)?;
    Some(byte_length)
}

// Word equality is independent for the two byte parities. An endpoint query
// skips only equal u16 words; it never skips a new value or a count boundary.
// Two source-word reads are charged per indexed comparison.
fn word_run_ends(src: &[u8]) -> Vec<u32> {
    let mut ends = vec![0u32; src.len()];
    for offset in (0..src.len().saturating_sub(1)).rev() {
        ends[offset] =
            if offset + 4 <= src.len() && src[offset..offset + 2] == src[offset + 2..offset + 4] {
                ends[offset + 2]
            } else {
                (offset + 2) as u32
            };
    }
    ends
}

/// Preserve alias ordering while finding the minimum-relative count boundary.
/// Equal-word runs are held compactly until the entire record run is validated.
fn counted_relative_directory16_with_runs(
    src: &[u8],
    available: usize,
    runs: Option<&[u32]>,
    base: usize,
) -> CountedDirectory16Parse {
    if src.len() < 2 {
        return CountedDirectory16Parse::Rejected { probes: 0 };
    }
    if available == 0 {
        return CountedDirectory16Parse::Budget { probes: 0 };
    }
    let first = match Reader::new(src).u16_le() {
        Ok(value) => usize::from(value),
        Err(_) => return CountedDirectory16Parse::Rejected { probes: 0 },
    };
    let mut probes = 1;
    if first < 2 || first % 2 != 0 {
        return CountedDirectory16Parse::Rejected { probes };
    }
    if probes >= available {
        return CountedDirectory16Parse::Budget { probes };
    }
    probes += 1;
    let Some(header) = src.get(first..first.saturating_add(2)) else {
        return CountedDirectory16Parse::Rejected { probes };
    };
    let Some(first_length) = counted_record16_length_at(src, first, header) else {
        return CountedDirectory16Parse::Rejected { probes };
    };
    let max_entries = first / 2;
    let mut minimum = first;
    let mut offset_runs = vec![(first, 1usize)];
    let mut count = 1;
    while count < minimum / 2 {
        let cost = if runs.is_some() { 2 } else { 1 };
        if available.saturating_sub(probes) < cost {
            return CountedDirectory16Parse::Budget { probes };
        }
        probes += cost;
        let Some(bytes) = src.get(count * 2..) else {
            return CountedDirectory16Parse::Rejected { probes };
        };
        let value = match Reader::new(bytes).u16_le() {
            Ok(v) => usize::from(v),
            Err(_) => return CountedDirectory16Parse::Rejected { probes },
        };
        // Another distinct record cannot start inside the already bounded
        // first record. Aliases of its start and earlier records remain legal.
        if value % 2 != 0 || (value > first && value < first + first_length) {
            return CountedDirectory16Parse::Rejected { probes };
        }
        minimum = minimum.min(value);
        if minimum < (count + 1) * 2 {
            return CountedDirectory16Parse::Rejected { probes };
        }
        let equal_count = runs
            .map(|r| (r[base + count * 2] as usize - (base + count * 2)) / 2)
            .unwrap_or(1);
        let take = equal_count
            .min(minimum / 2 - count)
            .min(max_entries - count);
        assert!(take > 0);
        offset_runs.push((value, take));
        count += take;
    }
    if minimum != count * 2 {
        return CountedDirectory16Parse::Rejected { probes };
    }
    let directory_bytes = count * 2;
    let mut unique = BTreeMap::from([(first, first_length)]);
    for &(offset, _) in &offset_runs {
        if unique.contains_key(&offset) {
            continue;
        }
        if probes >= available {
            return CountedDirectory16Parse::Budget { probes };
        }
        probes += 1;
        let Some(header) = src.get(offset..offset.saturating_add(2)) else {
            return CountedDirectory16Parse::Rejected { probes };
        };
        let Some(length) = counted_record16_length_at(src, offset, header) else {
            return CountedDirectory16Parse::Rejected { probes };
        };
        unique.insert(offset, length);
    }
    let mut boundary = directory_bytes;
    for (&offset, &length) in &unique {
        if offset != boundary {
            return CountedDirectory16Parse::Rejected { probes };
        }
        let Some(end) = boundary.checked_add(length) else {
            return CountedDirectory16Parse::Rejected { probes };
        };
        boundary = end;
    }
    let mut entries = Vec::with_capacity(count);
    for (offset, n) in offset_runs {
        entries.extend(std::iter::repeat_n(
            CountedRecord16 {
                relative_offset: offset,
                byte_length: unique[&offset],
            },
            n,
        ));
    }
    CountedDirectory16Parse::Match { probes, entries }
}

#[cfg(test)]
fn counted_relative_directory16_parse(src: &[u8], available: usize) -> CountedDirectory16Parse {
    counted_relative_directory16_with_runs(src, available, None, 0)
}

fn counted_relative_directory16_leads(
    rom: &[u8],
    options: &Options,
    report: &mut SearchReport,
    kept: &mut BTreeMap<Family, Vec<Lead>>,
) {
    // The linear index is optional for small inputs/tight budgets. Account
    // for its source comparisons before any candidate; endpoint lookups are
    // charged by the parser. It does not change the directory grammar.
    let available = options.max_probes.saturating_sub(budget_used(report));
    let runs = if rom.len() >= 1024 && available >= rom.len().saturating_mul(6) {
        let runs = word_run_ends(rom);
        report
            .counts
            .entry(Family::CountedRelativeDirectory16)
            .or_default()
            .probes += rom.len().saturating_sub(3).saturating_mul(2);
        Some(runs)
    } else {
        None
    };
    for base in (0..rom.len()).step_by(options.stride) {
        if rom.len().saturating_sub(base) < 2 {
            break;
        }
        let available = options.max_probes.saturating_sub(budget_used(report));
        if available == 0 {
            report.complete_for_configured_scan = false;
            report
                .counts
                .entry(Family::CountedRelativeDirectory16)
                .or_default()
                .probes_skipped_by_budget += 1;
            break;
        }
        match counted_relative_directory16_with_runs(&rom[base..], available, runs.as_deref(), base)
        {
            CountedDirectory16Parse::Rejected { probes } => {
                let counts = report
                    .counts
                    .entry(Family::CountedRelativeDirectory16)
                    .or_default();
                counts.probes += probes;
                counts.rejected_probes += 1;
            }
            CountedDirectory16Parse::Budget { probes } => {
                let counts = report
                    .counts
                    .entry(Family::CountedRelativeDirectory16)
                    .or_default();
                counts.probes += probes;
                counts.probes_skipped_by_budget += 1;
                report.complete_for_configured_scan = false;
                break;
            }
            CountedDirectory16Parse::Match { probes, entries } => {
                report
                    .counts
                    .entry(Family::CountedRelativeDirectory16)
                    .or_default()
                    .probes += probes;
                let entry_count = entries.len();
                let mut targets = BTreeMap::new();
                for (index, entry) in entries.iter().enumerate() {
                    targets.entry(entry.relative_offset).or_insert(index);
                }
                let distinct_records = targets.len();
                let group_length = entries
                    .iter()
                    .map(|entry| entry.relative_offset + entry.byte_length)
                    .max()
                    .unwrap_or(0);
                for (entry_index, entry) in entries.into_iter().enumerate() {
                    if entry.byte_length > options.max_stored {
                        continue;
                    }
                    let offset = base + entry.relative_offset;
                    if promising(
                        report,
                        kept,
                        Family::CountedRelativeDirectory16,
                        offset,
                        100,
                    ) {
                        retain(report, kept, Lead { directory_provenance: Some(DirectoryProvenance {
                            base, group_length, entry_count, entry_index, distinct_records,
                            target_entry_index: targets[&entry.relative_offset],
                        }),
                            family: Family::CountedRelativeDirectory16,
                            offset,
                            stored_length: entry.byte_length,
                            decoded_length: None,
                            stored_sha256: ws_assets::sha256_hex(
                                &rom[offset..offset + entry.byte_length],
                            ),
                            decoded_sha256: None,
                            signature_id: None,
                            rank: 100,
                            evidence: "A relative-u16 directory, positive 16-byte-unit record headers and a contiguous raw record run matched; directory aliases are retained. This structural match does not establish payload meaning, a reader, runtime use or asset identity.".into(),
                            requires: vec!["Establish the table base, reader and record meanings before assigning a format or using the record as an asset.".into()],
                            reference_matches: Vec::new(),
                            reference_matches_truncated: false, source_provenance: None, reader_core: None,
                        });
                    }
                }
            }
        }
    }
}

/// Check the measured 16-byte descriptor relations using the shared checked reader.
/// Returns the complete raw record span and map entry count when the descriptor is
/// structurally compatible. The unknown first two words are deliberately ignored.
fn tile_map_record16_layout(src: &[u8]) -> Option<(usize, usize, usize)> {
    if src.len() < 16 {
        return None;
    }
    let mut reader = Reader::new(src);
    reader.skip(4).ok()?;
    let width = usize::from(reader.u16_le().ok()?);
    let height = usize::from(reader.u16_le().ok()?);
    let tile_count = usize::from(reader.u16_le().ok()?);
    let map_entries = usize::from(reader.u16_le().ok()?);
    let tile_offset = usize::from(reader.u16_le().ok()?);
    let map_offset = usize::from(reader.u16_le().ok()?);
    if width == 0
        || height == 0
        || width % 8 != 0
        || height % 8 != 0
        || !(1..=512).contains(&tile_count)
        || tile_offset != 16
    {
        return None;
    }
    let expected_entries = (width / 8).checked_mul(height / 8)?;
    let expected_map_offset = tile_count.checked_mul(16)?.checked_add(16)?;
    if map_entries != expected_entries || map_offset != expected_map_offset {
        return None;
    }
    let map_bytes = map_entries.checked_mul(2)?;
    let record_end = map_offset.checked_add(map_bytes)?;
    if record_end > src.len() {
        return None;
    }
    Some((record_end, map_entries, tile_count))
}

/// Verify map indices and the unsupported attribute bits. Flip bits 14 and 15
/// are retained as permitted attributes. Returns false as soon as a word fails.
fn tile_map_record16_entries(
    src: &[u8],
    entries: usize,
    tile_count: usize,
    available_probes: usize,
) -> (bool, usize, usize) {
    let map_offset = match tile_count.checked_mul(16).and_then(|n| n.checked_add(16)) {
        Some(offset) => offset,
        None => return (false, 0, 0),
    };
    let map_bytes = match entries.checked_mul(2) {
        Some(bytes) => bytes,
        None => return (false, 0, 0),
    };
    let Some(map) = src
        .get(map_offset..)
        .and_then(|bytes| bytes.get(..map_bytes))
    else {
        return (false, 0, 0);
    };
    let mut reader = Reader::new(map);
    let to_check = entries.min(available_probes);
    for _ in 0..to_check {
        let Ok(word) = reader.u16_le() else {
            return (false, 0, 0);
        };
        if usize::from(word & 0x01ff) >= tile_count || word & 0x3e00 != 0 {
            return (false, reader.pos() / 2, 0);
        }
    }
    if to_check < entries {
        (false, to_check, entries - to_check)
    } else {
        (true, to_check, 0)
    }
}
fn reference_matches(
    bytes: &[u8],
    stored: &[u8],
    patterns: &[PreparedPattern<'_>],
    report: &mut SearchReport,
    family: Family,
) -> (Vec<ReferenceMatch>, bool) {
    let mut matches = Vec::new();
    let mut truncated = false;
    for pattern in patterns
        .iter()
        .filter(|p| p.signature.family == Family::ReferenceBytes)
    {
        let mut start = 0;
        let mut occurrence = 0;
        let mut in_stored = None;
        while bytes.len().saturating_sub(start) >= pattern.signature.bytes.len() {
            if budget_used(report) >= report.options.max_probes {
                report.complete_for_configured_scan = false;
                report.decoded_reference_searches_skipped += 1;
                report.complete_by_family.insert(family, false);
                break;
            }
            report.decoded_reference_probes += 1;
            let Some(found) = pattern.finder.find(&bytes[start..]) else {
                break;
            };
            let offset = start + found;
            if occurrence >= 2 || matches.len() >= 64 {
                truncated = true;
                break;
            }
            let stored_match =
                *in_stored.get_or_insert_with(|| pattern.finder.find(stored).is_some());
            matches.push(ReferenceMatch {
                id: pattern.signature.id.clone(),
                decoded_offset: offset,
                length: pattern.signature.bytes.len(),
                reference_sha256: pattern.sha256.clone(),
                already_present_in_stored_input: stored_match,
            });
            occurrence += 1;
            start = offset + 1;
        }
    }
    (matches, truncated)
}
fn exact_patterns(
    rom: &[u8],
    patterns: &[PreparedPattern<'_>],
    report: &mut SearchReport,
    kept: &mut BTreeMap<Family, Vec<Lead>>,
) {
    for pattern in patterns {
        if !report.options.families.contains(&pattern.signature.family) {
            continue;
        }
        let mut start = 0;
        while rom.len().saturating_sub(start) >= pattern.signature.bytes.len() {
            if budget_used(report) >= report.options.max_probes {
                report.complete_for_configured_scan = false;
                report
                    .counts
                    .entry(pattern.signature.family)
                    .or_default()
                    .probes_skipped_by_budget += 1;
                break;
            }
            report.signature_probes += 1;
            report
                .counts
                .entry(pattern.signature.family)
                .or_default()
                .probes += 1;
            let Some(found) = pattern.finder.find(&rom[start..]) else {
                break;
            };
            let offset = start + found;
            start = offset + 1;
            if offset % report.options.stride != 0 {
                continue;
            }
            let family = pattern.signature.family;
            let length = pattern.signature.bytes.len();
            let rank = 500 + length as u32;
            if !promising(report, kept, family, offset, rank) {
                continue;
            }
            retain(report,kept,Lead { directory_provenance: None,
                family, offset, stored_length:length, decoded_length:None,
                stored_sha256:pattern.sha256.clone(), decoded_sha256:None, signature_id:Some(pattern.signature.id.clone()),rank,
                evidence:if family==Family::ReferenceBytes { "Exact external reference bytes occur in the ROM; this does not establish a unique origin, loader or whole resource." } else { "Exact external reader pattern matched; resource format and use remain unconfirmed." }.into(),
                requires:vec!["Establish original reference provenance, reader/caller context and complete resource boundaries; duplicates remain possible.".into()],
                reference_matches:Vec::new(), reference_matches_truncated:false, source_provenance:None, reader_core:None,
            });
        }
    }
}

/// Search every configured offset without requiring an emulator or Ghidra.
/// Input ranges are bounded to a 64 KiB bank for codec probes. No output bytes
/// are included in the report and no candidate is promoted to an asset map.
pub fn search(rom: &[u8], options: &Options) -> Result<SearchReport> {
    options.validate()?;
    if options.budget_scope == BudgetScope::PerFamily {
        let mut combined: Option<SearchReport> = None;
        for family in &options.families {
            let mut family_options = options.clone();
            family_options.families = vec![*family];
            // Keep all reference patterns for decoded comparisons. Only enabled
            // families perform ROM pattern searches in the worker.
            let current = search_shared(rom, &family_options)?;
            if let Some(report) = &mut combined {
                report.complete_for_configured_scan &= current.complete_for_configured_scan;
                report.complete_by_family.extend(current.complete_by_family);
                report.decoder_probes += current.decoder_probes;
                report.signature_probes += current.signature_probes;
                report.decoded_reference_probes += current.decoded_reference_probes;
                report.decoded_reference_searches_skipped +=
                    current.decoded_reference_searches_skipped;
                report.counts.extend(current.counts);
                report.leads.extend(current.leads);
            } else {
                combined = Some(current);
            }
        }
        let mut report = combined.expect("validated nonempty family list");
        report.options = ReportOptions::from(options);
        report.configuration_sha256 = ws_assets::sha256_hex(
            &serde_json::to_vec(options).map_err(|e| Error::Malformed(e.to_string()))?,
        );
        report.leads.sort_by_key(|lead| lead.family);
        report.overlapping_codec_leads = overlapping(&report.leads);
        report.limitations.push("Per-family probe ceilings apply independently; positions_scanned counts configured offsets, not the sum of repeated family traversals. Completion remains bounded by all configured limits.".into());
        return Ok(report);
    }
    search_shared(rom, options)
}

fn search_shared(rom: &[u8], options: &Options) -> Result<SearchReport> {
    if rom.len() > MAX_ROM {
        return Err(Error::ResourceLimit {
            resource: "format search input bytes",
            requested: rom.len() as u64,
            limit: MAX_ROM as u64,
        });
    }
    let mut report = SearchReport {
        complete_by_family: options.families.iter().map(|family| (*family, true)).collect(),
        schema: SCHEMA.into(), rom_sha256: ws_assets::sha256_hex(rom), rom_bytes: rom.len(),
        options: ReportOptions::from(options), configuration_sha256: ws_assets::sha256_hex(&serde_json::to_vec(options).map_err(|e| Error::Malformed(e.to_string()))?), positions_scanned: 0, complete_for_configured_scan: true,
        decoder_probes: 0, signature_probes: 0, decoded_reference_probes: 0, decoded_reference_searches_skipped: 0, counts: options.families.iter().map(|f| (*f, Counts::default())).collect(),
        leads: Vec::new(), overlapping_codec_leads:Vec::new(),
        limitations: vec![
            "Leads only: no reader, source ownership, runtime use, media meaning or editing compatibility is established.".into(),
            "Byte stride, codec bank windows, codec command count, checked raw-layout spans, input/output limits, prefilters and per-family retention bound this search. Completion is only for that configuration.".into(),
            "Raw tiles, unframed fonts/text/PCM and externally sized or history-dependent codecs have no reliable context-free identification here.".into(),
            "Palette and screen-map leads are fixed inspection windows, not proven resource extents. Uniform output and short streams are suppressed.".into(),
            "Headerless parses can be false positives or starts inside another stream; overlapping leads are retained, not asserted to be independent resources.".into(),
        ],
    };
    let enabled: BTreeSet<_> = options.families.iter().copied().collect();
    let patterns: Vec<_> = options
        .signatures
        .iter()
        .filter(|s| s.mask.is_none())
        .map(|signature| PreparedPattern {
            signature,
            finder: memchr::memmem::Finder::new(&signature.bytes),
            sha256: ws_assets::sha256_hex(&signature.bytes),
        })
        .collect();
    let masked: Vec<_> = options
        .signatures
        .iter()
        .filter(|s| s.mask.is_some() && enabled.contains(&s.family))
        .collect();
    let mut kept = BTreeMap::new();
    exact_patterns(rom, &patterns, &mut report, &mut kept);
    reader_core_leads(rom, options, &mut report, &mut kept);
    if enabled.contains(&Family::Sgd1TileGrid) {
        sgd1_tile_grid_leads(rom, options, &mut report, &mut kept);
    }
    if enabled.contains(&Family::CountedRelativeDirectory16) {
        counted_relative_directory16_leads(rom, options, &mut report, &mut kept);
    }
    for offset in (0..rom.len()).step_by(options.stride) {
        report.positions_scanned += 1;
        let src = &rom[offset..];
        let bank_available = 65536 - (offset & 65535);
        let window = &src[..src.len().min(bank_available).min(options.max_stored)];
        for family in [
            Family::LzU8Zero,
            Family::LzRle7f,
            Family::LzBits16,
            Family::LzWord16Msb,
            Family::LzWordCounted,
        ] {
            if !enabled.contains(&family) || !prefilter(family, window, options) {
                continue;
            }
            if budget_used(&report) >= options.max_probes {
                report.complete_for_configured_scan = false;
                report
                    .counts
                    .entry(family)
                    .or_default()
                    .probes_skipped_by_budget += 1;
                continue;
            }
            report.decoder_probes += 1;
            report.counts.entry(family).or_default().probes += 1;
            let Ok(decoded) = decode(family, window, options.max_decoded, offset) else {
                report.counts.entry(family).or_default().rejected_probes += 1;
                continue;
            };
            report.counts.entry(family).or_default().decoded_parses += 1;
            if decoded.bytes.len() < options.min_decoded
                || decoded.consumed < 4
                || !diverse(&decoded.bytes, 3)
                || (options.min_savings != 0
                    && decoded.bytes.len() < decoded.consumed + options.min_savings)
            {
                report.counts.entry(family).or_default().rejected_probes += 1;
                report.counts.entry(family).or_default().filtered_parses += 1;
                continue;
            }
            let stored = &window[..decoded.consumed];
            let ratio = (decoded.bytes.len() * 100 / decoded.consumed).min(1000) as u32;
            let (matches, truncated) =
                reference_matches(&decoded.bytes, stored, &patterns, &mut report, family);
            let distinct: BTreeSet<_> = matches.iter().map(|m| &m.reference_sha256).collect();
            let decoded_only: BTreeSet<_> = matches
                .iter()
                .filter(|m| !m.already_present_in_stored_input)
                .map(|m| &m.reference_sha256)
                .collect();
            let rank = ratio * 10
                + (decoded.bytes.len().min(4096) / 4) as u32
                + distinct.len() as u32 * 100
                + decoded_only.len() as u32 * 2000;
            if !promising(&mut report, &kept, family, offset, rank) {
                continue;
            }
            retain(&mut report, &mut kept, Lead { directory_provenance: None,
                family, offset, stored_length: decoded.consumed,
                decoded_length: Some(decoded.bytes.len()), stored_sha256: ws_assets::sha256_hex(stored),
                decoded_sha256: Some(ws_assets::sha256_hex(&decoded.bytes)), signature_id: None, rank,
                evidence: "Complete bounded parse using the existing decoder with empty initial history; no original-output comparison.".into(),
                requires: vec!["Find the actual reader/caller and establish the stream start, bank context and header ownership.".into(), "Compare the complete output and recover the selector domain before declaring an asset family.".into()],
                reference_matches: matches, reference_matches_truncated: truncated, source_provenance: None, reader_core: None,
            });
        }
        if offset % 2 == 0 {
            for (family, length, matched, explanation) in [
                (Family::Rgb444, 32, enabled.contains(&Family::Rgb444) && palette(src), "Sixteen RGB444-compatible little-endian words with at least four distinct colours."),
                (Family::ScreenMap, 64, enabled.contains(&Family::ScreenMap) && screen_map(src), "Thirty-two screen-compatible words with varied, mostly nearby tile indices and at most three palette indices."),
            ] {
                if matched && promising(&mut report, &kept, family, offset, 100) {
                    retain(&mut report, &mut kept, Lead { directory_provenance: None,
                        family, offset, stored_length: length, decoded_length: None,
                        stored_sha256: ws_assets::sha256_hex(&src[..length]), decoded_sha256: None,
                        signature_id: None, rank: 100,
                        evidence: format!("{explanation} This is a layout heuristic, not a unique signature."),
                        requires: vec!["Find the consumer, boundaries and display context; distinguish numeric tables from graphics data.".into()],
                        reference_matches: Vec::new(), reference_matches_truncated: false, source_provenance: None, reader_core: None,
                    });
                }
            }
        }
        if enabled.contains(&Family::TileMapRecord16) && src.len() >= 16 {
            if budget_used(&report) >= options.max_probes {
                report.complete_for_configured_scan = false;
                report
                    .counts
                    .get_mut(&Family::TileMapRecord16)
                    .unwrap()
                    .probes_skipped_by_budget += 1;
            } else {
                report
                    .counts
                    .get_mut(&Family::TileMapRecord16)
                    .unwrap()
                    .probes += 1;
                if let Some((record_length, map_entries, tile_count)) =
                    tile_map_record16_layout(src)
                {
                    let available = options.max_probes.saturating_sub(budget_used(&report));
                    let (valid_map, used, skipped) =
                        tile_map_record16_entries(src, map_entries, tile_count, available);
                    let counts = report.counts.get_mut(&Family::TileMapRecord16).unwrap();
                    counts.probes += used;
                    counts.probes_skipped_by_budget += skipped;
                    if skipped != 0 {
                        report.complete_for_configured_scan = false;
                    } else if !valid_map {
                        counts.rejected_probes += 1;
                    } else if promising(&mut report, &kept, Family::TileMapRecord16, offset, 100) {
                        retain(&mut report, &mut kept, Lead { directory_provenance: None,
                            family: Family::TileMapRecord16,
                            offset,
                            stored_length: record_length,
                            decoded_length: None,
                            stored_sha256: ws_assets::sha256_hex(&src[..record_length]),
                            decoded_sha256: None,
                            signature_id: None,
                            rank: 100,
                            evidence: "Bounded 16-byte descriptor relations and map-word constraints matched; this does not establish a format, reader context, runtime use or asset identity.".into(),
                            requires: vec!["Establish the actual reader, header ownership, bank context and complete resource boundaries before treating this as a resource.".into()],
                            reference_matches: Vec::new(),
                            reference_matches_truncated: false, source_provenance: None, reader_core: None,
                        });
                    }
                }
            }
        }
        if enabled.contains(&Family::BankedQueueConstructor)
            && src.len() >= banked_constructor::LENGTH
        {
            let family = Family::BankedQueueConstructor;
            if budget_used(&report) >= options.max_probes {
                report.complete_for_configured_scan = false;
                report
                    .counts
                    .entry(family)
                    .or_default()
                    .probes_skipped_by_budget += 1;
            } else {
                report.signature_probes += 1;
                report.counts.entry(family).or_default().probes += 1;
                if let Some(evidence) = banked_constructor::evidence(src) {
                    let rank = 200;
                    if promising(&mut report, &kept, family, offset, rank) {
                        retain(&mut report, &mut kept, Lead { directory_provenance: None,
                            family, offset, stored_length: banked_constructor::LENGTH,
                            decoded_length: None,
                            stored_sha256: ws_assets::sha256_hex(&src[..banked_constructor::LENGTH]),
                            decoded_sha256: None,
                            signature_id: Some("fixed-register-banked-queue-v1".into()), rank,
                            evidence,
                            requires: vec![
                                "Independently establish original instruction boundaries, containing function and incoming branch targets; a match inside data or another instruction is not code proof.".into(),
                                "Bind retained caller bytes/flow and register-lane clobber facts before resolving any selector: reset on calls, branches, gaps and every recorded incoming target; unknown indirect entries remain HOLD.".into(),
                                "Establish original bank/table identities, complete resource boundaries, consumer and runtime/output ownership; no SDK, codec, asset or support claim follows.".into(),
                            ],
                            reference_matches: Vec::new(), reference_matches_truncated: false, source_provenance: None, reader_core: None,
                        });
                    }
                } else {
                    report.counts.entry(family).or_default().rejected_probes += 1;
                }
            }
        }
        if enabled.contains(&Family::ReaderSignature) || enabled.contains(&Family::ReferenceBytes) {
            for signature in &masked {
                if !enabled.contains(&signature.family) {
                    continue;
                }
                if budget_used(&report) >= options.max_probes {
                    report.complete_for_configured_scan = false;
                    report
                        .counts
                        .entry(signature.family)
                        .or_default()
                        .probes_skipped_by_budget += 1;
                    continue;
                }
                let Some(bytes) = src.get(..signature.bytes.len()) else {
                    continue;
                };
                report.signature_probes += 1;
                report.counts.entry(signature.family).or_default().probes += 1;
                let matched = match &signature.mask {
                    Some(mask) => bytes
                        .iter()
                        .zip(&signature.bytes)
                        .zip(mask)
                        .all(|((&a, &b), &mask)| a & mask == b & mask),
                    None => bytes == signature.bytes.as_slice(),
                };
                if matched {
                    let rank = 500 + bytes.len() as u32;
                    if !promising(&mut report, &kept, signature.family, offset, rank) {
                        continue;
                    }
                    retain(&mut report, &mut kept, Lead { directory_provenance: None,
                        family: signature.family, offset, stored_length: bytes.len(), decoded_length: None,
                        stored_sha256: ws_assets::sha256_hex(bytes), decoded_sha256: None,
                        signature_id: Some(signature.id.clone()), rank,
                        evidence: if signature.family == Family::ReaderSignature { "External masked reader pattern matched; code use and resource format remain unconfirmed." } else { "Exact external reference bytes occur in the ROM; this does not establish the loader, unique origin or complete asset extent." }.into(),
                        requires: vec![if signature.family == Family::ReaderSignature { "Disassemble the match, compare nonrelocated operands and follow its callers/resource tables in this ROM." } else { "Check reference provenance, all duplicate ROM locations, complete source extent and the actual loader; captures are not assets." }.into()],
                        reference_matches: Vec::new(), reference_matches_truncated: false, source_provenance: None, reader_core: None,
                    });
                } else {
                    report
                        .counts
                        .entry(signature.family)
                        .or_default()
                        .rejected_probes += 1;
                }
            }
        }
    }
    for (family, entries) in kept {
        report.counts.entry(family).or_default().retained = entries.len();
        report.leads.extend(entries);
    }
    for (family, complete) in &mut report.complete_by_family {
        *complete &= report.counts[family].probes_skipped_by_budget == 0;
    }
    report.overlapping_codec_leads = overlapping(&report.leads);
    Ok(report)
}

pub(crate) fn constructor_pattern_matches(bytes: &[u8]) -> bool {
    banked_constructor::evidence(bytes).is_some()
}

#[cfg(test)]
mod counted_directory16_tests {
    use super::{
        counted_record16_length, counted_relative_directory16_parse,
        counted_relative_directory16_with_runs, word_run_ends, CountedDirectory16Parse,
    };

    #[test]
    fn repeated_directory_run_stops_at_count_boundary_before_equal_payload() {
        let mut bytes = [6u8, 0].repeat(52);
        let mut prefixed = vec![0x81; 5];
        prefixed.append(&mut bytes);
        let runs = word_run_ends(&prefixed);
        match counted_relative_directory16_with_runs(&prefixed[5..], 4, Some(&runs), 5) {
            CountedDirectory16Parse::Match { probes, entries } => {
                assert_eq!(probes, 4);
                assert_eq!(entries.len(), 3);
                assert!(entries
                    .iter()
                    .all(|entry| entry.relative_offset == 6 && entry.byte_length == 98));
            }
            other => panic!("equal payload words changed the directory boundary: {other:?}"),
        }
    }

    #[test]
    fn repeated_directory_index_does_not_skip_a_changed_target() {
        let mut bytes = [6u8, 0].repeat(52);
        bytes[4] = 8;
        let runs = word_run_ends(&bytes);
        assert!(matches!(
            counted_relative_directory16_with_runs(&bytes, 100, Some(&runs), 0),
            CountedDirectory16Parse::Rejected { .. }
        ));
    }

    #[test]
    fn indexed_read_respects_a_budget_smaller_than_one_query() {
        let bytes = [6u8, 0].repeat(52);
        let runs = word_run_ends(&bytes);
        assert!(matches!(
            counted_relative_directory16_with_runs(&bytes, 3, Some(&runs), 0),
            CountedDirectory16Parse::Budget { probes: 2 }
        ));
    }

    #[test]
    fn counted_record_length_checks_arithmetic_overflow() {
        assert_eq!(counted_record16_length(0), Some(2));
        assert_eq!(counted_record16_length(1), Some(18));
        assert_eq!(counted_record16_length(usize::MAX), None);
    }

    #[test]
    fn directory_parser_stops_before_payload_words() {
        let mut bytes = vec![0u8; 60];
        for (index, offset) in [42u16, 6, 24].into_iter().enumerate() {
            bytes[index * 2..index * 2 + 2].copy_from_slice(&offset.to_le_bytes());
        }
        for offset in [6usize, 24, 42] {
            bytes[offset..offset + 2].copy_from_slice(&1u16.to_le_bytes());
        }
        // These look like additional relative offsets but are part of payload.
        bytes[8..14].copy_from_slice(&[8, 0, 10, 0, 12, 0]);
        match counted_relative_directory16_parse(&bytes, 100) {
            CountedDirectory16Parse::Match { entries, .. } => assert_eq!(entries.len(), 3),
            other => panic!("expected three directory entries, got {other:?}"),
        }
    }

    #[test]
    fn odd_first_target_rejects_before_following_its_header() {
        let bytes = vec![0xff; 1_200_000];
        assert!(matches!(
            counted_relative_directory16_parse(&bytes, 1),
            CountedDirectory16Parse::Rejected { probes: 1 }
        ));
    }

    #[test]
    fn odd_later_target_rejects_before_another_table_word() {
        let mut bytes = vec![0u8; 24];
        bytes[..8].copy_from_slice(&[6, 0, 7, 0, 6, 0, 1, 0]);
        assert!(matches!(
            counted_relative_directory16_parse(&bytes, 3),
            CountedDirectory16Parse::Rejected { probes: 3 }
        ));
    }

    #[test]
    fn even_aliases_and_reordered_records_still_match() {
        for offsets in [
            vec![2u16],
            vec![6, 6, 6],
            vec![42, 6, 24],
            vec![26, 8, 26, 8],
        ] {
            let directory_bytes = offsets.len() * 2;
            let last = usize::from(*offsets.iter().max().unwrap());
            let mut bytes = vec![0u8; last + 18];
            for (index, &offset) in offsets.iter().enumerate() {
                bytes[index * 2..index * 2 + 2].copy_from_slice(&offset.to_le_bytes());
                bytes[usize::from(offset)..usize::from(offset) + 2]
                    .copy_from_slice(&1u16.to_le_bytes());
            }
            assert_eq!(usize::from(*offsets.iter().min().unwrap()), directory_bytes);
            match counted_relative_directory16_parse(&bytes, 100) {
                CountedDirectory16Parse::Match { entries, .. } => {
                    assert_eq!(
                        entries
                            .iter()
                            .map(|e| e.relative_offset)
                            .collect::<Vec<_>>(),
                        offsets.iter().map(|&v| usize::from(v)).collect::<Vec<_>>()
                    );
                }
                other => panic!("valid even directory rejected: {other:?}"),
            }
        }
    }
}
