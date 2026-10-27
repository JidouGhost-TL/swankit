//! ROM-only leads, not asset maps or evidence of runtime use.
//! Hardware layouts: docs/hardware.md and WSdev Display/Screen, Display/Palette.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
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
            retain(report, kept, Lead {
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
                reference_matches_truncated: false,
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

/// Parse one candidate directory. The entry count is the minimum-relative
/// offset divided by two; the first directory word is only an upper bound.
/// Duplicate entries stay in `entries`, while each distinct target is checked
/// once for the contiguous record run.
fn counted_relative_directory16_parse(
    src: &[u8],
    available_probes: usize,
) -> CountedDirectory16Parse {
    if src.len() < 2 {
        return CountedDirectory16Parse::Rejected { probes: 0 };
    }
    if available_probes == 0 {
        return CountedDirectory16Parse::Budget { probes: 0 };
    }

    let mut reader = Reader::new(src);
    let first = match reader.u16_le() {
        Ok(value) => usize::from(value),
        Err(_) => return CountedDirectory16Parse::Rejected { probes: 0 },
    };
    let mut probes = 1;
    if first < 2 {
        return CountedDirectory16Parse::Rejected { probes };
    }
    if probes >= available_probes {
        return CountedDirectory16Parse::Budget { probes };
    }
    probes += 1;
    let Some(first_header) = src.get(first..first.saturating_add(2)) else {
        return CountedDirectory16Parse::Rejected { probes };
    };
    let Some(first_record_length) = counted_record16_length_at(src, first, first_header) else {
        return CountedDirectory16Parse::Rejected { probes };
    };
    let max_entries = (first / 2).min(usize::from(u16::MAX) / 2);
    if max_entries == 0 {
        return CountedDirectory16Parse::Rejected { probes };
    }

    let mut minimum = first;
    let mut offsets = vec![first];
    let mut entry_count = (minimum == 2).then_some(1usize);
    for index in 1..max_entries {
        if entry_count.is_some() {
            break;
        }
        if probes >= available_probes {
            return CountedDirectory16Parse::Budget { probes };
        }
        probes += 1;
        let value = match reader.u16_le() {
            Ok(value) => usize::from(value),
            Err(_) => return CountedDirectory16Parse::Rejected { probes },
        };
        minimum = minimum.min(value);
        offsets.push(value);
        let count = index + 1;
        let table_bytes = count * 2;
        if minimum < table_bytes {
            return CountedDirectory16Parse::Rejected { probes };
        }
        if minimum == table_bytes {
            entry_count = Some(count);
            break;
        }
    }
    let Some(entry_count) = entry_count else {
        return CountedDirectory16Parse::Rejected { probes };
    };
    let directory_bytes = entry_count * 2;
    let Some(_directory) = src.get(..directory_bytes) else {
        return CountedDirectory16Parse::Rejected { probes };
    };
    offsets.truncate(entry_count);
    if offsets.iter().copied().min() != Some(directory_bytes) {
        return CountedDirectory16Parse::Rejected { probes };
    }

    let mut unique = BTreeMap::from([(first, first_record_length)]);
    for &offset in &offsets {
        if unique.contains_key(&offset) {
            continue;
        }
        if probes >= available_probes {
            return CountedDirectory16Parse::Budget { probes };
        }
        probes += 1;
        let Some(header) = src.get(offset..offset.saturating_add(2)) else {
            return CountedDirectory16Parse::Rejected { probes };
        };
        let Some(byte_length) = counted_record16_length_at(src, offset, header) else {
            return CountedDirectory16Parse::Rejected { probes };
        };
        unique.insert(offset, byte_length);
    }

    let mut boundary = directory_bytes;
    for (&offset, &byte_length) in &unique {
        if offset != boundary {
            return CountedDirectory16Parse::Rejected { probes };
        }
        let Some(end) = boundary.checked_add(byte_length) else {
            return CountedDirectory16Parse::Rejected { probes };
        };
        boundary = end;
    }
    let entries = offsets
        .into_iter()
        .map(|relative_offset| CountedRecord16 {
            relative_offset,
            byte_length: unique[&relative_offset],
        })
        .collect();
    CountedDirectory16Parse::Match { probes, entries }
}

fn counted_relative_directory16_leads(
    rom: &[u8],
    options: &Options,
    report: &mut SearchReport,
    kept: &mut BTreeMap<Family, Vec<Lead>>,
) {
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
        match counted_relative_directory16_parse(&rom[base..], available) {
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
                for entry in entries {
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
                        retain(report, kept, Lead {
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
                            reference_matches_truncated: false,
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
            retain(report,kept,Lead {
                family, offset, stored_length:length, decoded_length:None,
                stored_sha256:pattern.sha256.clone(), decoded_sha256:None, signature_id:Some(pattern.signature.id.clone()),rank,
                evidence:if family==Family::ReferenceBytes { "Exact external reference bytes occur in the ROM; this does not establish a unique origin, loader or whole resource." } else { "Exact external reader pattern matched; resource format and use remain unconfirmed." }.into(),
                requires:vec!["Establish original reference provenance, reader/caller context and complete resource boundaries; duplicates remain possible.".into()],
                reference_matches:Vec::new(), reference_matches_truncated:false,
            });
        }
    }
}

/// Search every configured offset without requiring an emulator or Ghidra.
/// Input ranges are bounded to a 64 KiB bank for codec probes. No output bytes
/// are included in the report and no candidate is promoted to an asset map.
pub fn search(rom: &[u8], options: &Options) -> Result<SearchReport> {
    options.validate()?;
    if rom.len() > MAX_ROM {
        return Err(Error::ResourceLimit {
            resource: "format search input bytes",
            requested: rom.len() as u64,
            limit: MAX_ROM as u64,
        });
    }
    let mut report = SearchReport {
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
        .filter(|s| s.mask.is_some())
        .collect();
    let mut kept = BTreeMap::new();
    exact_patterns(rom, &patterns, &mut report, &mut kept);
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
                reference_matches(&decoded.bytes, stored, &patterns, &mut report);
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
            retain(&mut report, &mut kept, Lead {
                family, offset, stored_length: decoded.consumed,
                decoded_length: Some(decoded.bytes.len()), stored_sha256: ws_assets::sha256_hex(stored),
                decoded_sha256: Some(ws_assets::sha256_hex(&decoded.bytes)), signature_id: None, rank,
                evidence: "Complete bounded parse using the existing decoder with empty initial history; no original-output comparison.".into(),
                requires: vec!["Find the actual reader/caller and establish the stream start, bank context and header ownership.".into(), "Compare the complete output and recover the selector domain before declaring an asset family.".into()],
                reference_matches: matches, reference_matches_truncated: truncated,
            });
        }
        if offset % 2 == 0 {
            for (family, length, matched, explanation) in [
                (Family::Rgb444, 32, enabled.contains(&Family::Rgb444) && palette(src), "Sixteen RGB444-compatible little-endian words with at least four distinct colours."),
                (Family::ScreenMap, 64, enabled.contains(&Family::ScreenMap) && screen_map(src), "Thirty-two screen-compatible words with varied, mostly nearby tile indices and at most three palette indices."),
            ] {
                if matched && promising(&mut report, &kept, family, offset, 100) {
                    retain(&mut report, &mut kept, Lead {
                        family, offset, stored_length: length, decoded_length: None,
                        stored_sha256: ws_assets::sha256_hex(&src[..length]), decoded_sha256: None,
                        signature_id: None, rank: 100,
                        evidence: format!("{explanation} This is a layout heuristic, not a unique signature."),
                        requires: vec!["Find the consumer, boundaries and display context; distinguish numeric tables from graphics data.".into()],
                        reference_matches: Vec::new(), reference_matches_truncated: false,
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
                        retain(&mut report, &mut kept, Lead {
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
                            reference_matches_truncated: false,
                        });
                    }
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
                    retain(&mut report, &mut kept, Lead {
                        family: signature.family, offset, stored_length: bytes.len(), decoded_length: None,
                        stored_sha256: ws_assets::sha256_hex(bytes), decoded_sha256: None,
                        signature_id: Some(signature.id.clone()), rank,
                        evidence: if signature.family == Family::ReaderSignature { "External masked reader pattern matched; code use and resource format remain unconfirmed." } else { "Exact external reference bytes occur in the ROM; this does not establish the loader, unique origin or complete asset extent." }.into(),
                        requires: vec![if signature.family == Family::ReaderSignature { "Disassemble the match, compare nonrelocated operands and follow its callers/resource tables in this ROM." } else { "Check reference provenance, all duplicate ROM locations, complete source extent and the actual loader; captures are not assets." }.into()],
                        reference_matches: Vec::new(), reference_matches_truncated: false,
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
    report.overlapping_codec_leads = overlapping(&report.leads);
    Ok(report)
}

#[cfg(test)]
mod counted_directory16_tests {
    use super::{
        counted_record16_length, counted_relative_directory16_parse, CountedDirectory16Parse,
    };

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
}
