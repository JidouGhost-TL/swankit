//! Optional, identity-bound retained listing analysis. This is neither a
//! disassembler nor proof of complete control flow or runtime behavior.
//! Static ROM1 policy: docs/hardware.md (WSdev memory map and mapper).
use crate::search::{Family, SearchReport};
use formatkit_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const ATTACHMENT_SCHEMA: &str = "swankit-retained-source/1";
pub const SIDECAR_SCHEMA: &str = "swankit-retained-callers/1";
pub const MAX_LISTING_BYTES: usize = 64 * 1024 * 1024;
const MAX_INSTRUCTIONS: usize = 500_000;
const MAX_CALLERS: usize = 16_384;
const MAX_BLOCK: usize = 8192;
fn bad(s: impl Into<String>) -> Error {
    Error::Malformed(format!("retained source: {}", s.into()))
}
fn hash_ok(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListingFile {
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub logical_start: u32,
    pub physical_start: usize,
    pub length: usize,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BankPolicy {
    Rom1LowBytePowerOfTwo,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub schema: String,
    pub rom_sha256: String,
    pub rom_bytes: usize,
    pub listing: ListingFile,
    pub mapping: Vec<Mapping>,
    #[serde(default)]
    pub bank_policy: Option<BankPolicy>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceReference {
    pub sidecar: String,
    pub sha256: String,
    pub constructor_indices: Vec<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reader_indices: Vec<usize>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Producer {
    pub value: u8,
    pub logical: u32,
    pub physical: usize,
    pub bytes: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RegisterValue {
    pub value: Option<u16>,
    pub low: Option<Producer>,
    pub high: Option<Producer>,
    pub hold: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_at: Option<SourceRow>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SourceRow {
    pub logical: u32,
    pub physical: usize,
    pub bytes: String,
}

/// Original instruction boundaries only; no entry or source-profile inference.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderJoin {
    pub family: Family,
    pub pattern: String,
    pub version: u16,
    pub lead_offset: usize,
    pub lead_sha256: String,
    pub core_logical: Option<u32>,
    pub function_entry: Option<u32>,
    pub status: String,
    pub hold: Option<String>,
    pub rows: Vec<SourceRow>,
    pub recorded_alternate_entries: Vec<u32>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Constructor {
    pub lead_offset: usize,
    pub lead_sha256: String,
    pub core_logical: Option<u32>,
    pub function_entry: Option<u32>,
    pub function_address: Option<String>,
    pub function_instruction_count: usize,
    pub function_instruction_sha256: Option<String>,
    pub status: String,
    pub hold: Option<String>,
    pub entry_to_core_hold: Option<String>,
    pub table_segment: u16,
    pub table_offset: u16,
    pub queue_pointer: u16,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Pointer {
    pub status: String,
    pub hold: Option<String>,
    pub selector: u16,
    pub group_logical: Option<u32>,
    pub group_physical: Option<usize>,
    pub group_sha256: Option<String>,
    pub bank_word: Option<u16>,
    pub out_al: Option<u8>,
    pub physical_bank: Option<usize>,
    pub pointer_field_physical: Option<usize>,
    pub pointer_word: Option<u16>,
    pub source_physical: Option<usize>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Caller {
    pub call_logical: u32,
    pub call_physical: Option<usize>,
    pub caller_function: Option<u32>,
    pub caller_function_address: Option<String>,
    pub target_logical: u32,
    pub target_segment: u16,
    pub target_offset: u16,
    pub constructor_index: usize,
    pub call_bytes: String,
    pub source_status: String,
    pub hold: Option<String>,
    pub arguments: BTreeMap<String, RegisterValue>,
    pub block: Vec<SourceRow>,
    pub pointer: Option<Pointer>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DirectEntry {
    pub source_logical: u32,
    pub source_physical: Option<usize>,
    pub source_function: Option<u32>,
    pub source_bytes: String,
    pub flow: String,
    pub target_logical: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UnresolvedFlow {
    pub logical: u32,
    pub physical: Option<usize>,
    pub function: Option<u32>,
    pub bytes: String,
    pub flow: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Sidecar {
    pub schema: String,
    pub policy: String,
    pub assurance: String,
    pub rom_sha256: String,
    pub rom_bytes: usize,
    pub attachment_sha256: String,
    pub listing_sha256: String,
    pub mapping_sha256: String,
    pub listed_instructions: usize,
    pub unmapped_instructions: usize,
    pub recorded_direct_targets: usize,
    pub direct_entries: Vec<DirectEntry>,
    pub unresolved_flow_sites: Vec<UnresolvedFlow>,
    pub constructors: Vec<Constructor>,
    pub callers: Vec<Caller>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readers: Vec<ReaderJoin>,
    pub limitations: Vec<String>,
}
#[derive(Debug)]
struct Instruction {
    at: u32,
    function: Option<u32>,
    flow: String,
    bytes: Vec<u8>,
    writes: BTreeSet<String>,
    targets: Vec<u32>,
    physical: Option<usize>,
}
#[derive(Debug)]
struct Function {
    address: String,
    count: usize,
    min: u32,
    max: u32,
}
fn hex(s: &str) -> Result<u32> {
    u32::from_str_radix(s, 16).map_err(|_| bad("invalid hexadecimal address"))
}
fn bytes(s: &str) -> Result<Vec<u8>> {
    if s.is_empty()
        || s.len() > 64
        || !s.len().is_multiple_of(2)
        || !s.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(bad("invalid instruction bytes"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| bad("instruction bytes")))
        .collect()
}
fn byte_hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}
fn mapped(mapping: &[Mapping], at: u32, len: usize) -> Option<usize> {
    mapping.iter().find_map(|m| {
        let delta = at.checked_sub(m.logical_start)? as usize;
        (delta.checked_add(len)? <= m.length).then(|| m.physical_start + delta)
    })
}
fn listing(
    rom: &[u8],
    attachment: &Attachment,
    input: &[u8],
    require_functions: bool,
) -> Result<(BTreeMap<u32, Function>, Vec<Instruction>)> {
    if input.len() > MAX_LISTING_BYTES
        || input.len() != attachment.listing.bytes
        || ws_assets::sha256_hex(input) != attachment.listing.sha256.to_ascii_lowercase()
    {
        return Err(bad("listing identity/size differs"));
    }
    let text = std::str::from_utf8(input).map_err(|_| bad("listing must be UTF-8"))?;
    let mut functions = BTreeMap::new();
    let mut ins = BTreeMap::new();
    for line in text.lines().filter(|s| !s.is_empty()) {
        if line.len() > 4096 {
            return Err(bad("listing line exceeds bound"));
        }
        let c: Vec<_> = line.split('\t').collect();
        match c.first().copied() {
            Some("F") if c.len() == 8 => {
                let at = hex(c[1])?;
                let parts: Vec<_> = c[2].rsplit(':').collect();
                let address_value = if parts.len() >= 2 {
                    let segment = hex(parts[1])?;
                    let offset = hex(parts[0])?;
                    if segment > 65535 || offset > 65535 {
                        return Err(bad("function segmented address width"));
                    }
                    segment * 16 + offset
                } else {
                    hex(c[2])?
                };
                if address_value != at {
                    return Err(bad("function address identity differs"));
                }
                let count = c[4].parse::<usize>().map_err(|_| bad("function count"))?;
                let min = hex(c[6])?;
                let max = hex(c[7])?;
                if count == 0
                    || count > MAX_INSTRUCTIONS
                    || min > at
                    || at > max
                    || functions.len() >= 100_000
                    || functions
                        .insert(
                            at,
                            Function {
                                address: c[2].into(),
                                count,
                                min,
                                max,
                            },
                        )
                        .is_some()
                {
                    return Err(bad("invalid/duplicate function"));
                }
            }
            Some("I") if c.len() == 10 => {
                let at = hex(c[1])?;
                let function = if c[2] == "-" { None } else { Some(hex(c[2])?) };
                if c[3].is_empty() || c[7].is_empty() {
                    return Err(bad("missing flow/write facts"));
                }
                let code = bytes(c[4])?;
                if at.checked_add(code.len() as u32).is_none() {
                    return Err(bad("instruction address wraps"));
                }
                if bytes(c[5])?.len() != code.len() {
                    return Err(bad("instruction mask extent differs"));
                }
                let mut writes = BTreeSet::new();
                for w in c[7].split(',').filter(|s| *s != ".") {
                    let parent = match w {
                        "AL" | "AH" => "AX",
                        "BL" | "BH" => "BX",
                        "CL" | "CH" => "CX",
                        "DL" | "DH" => "DX",
                        x => x,
                    };
                    if ![
                        "AX", "BX", "CX", "DX", "SI", "DI", "BP", "SP", "DS", "ES", "SS", "CS",
                    ]
                    .contains(&parent)
                    {
                        return Err(bad("unknown register write"));
                    }
                    writes.insert(parent.to_owned());
                }
                let targets = if c[9] == "-" {
                    Vec::new()
                } else {
                    c[9].split(',').map(hex).collect::<Result<Vec<_>>>()?
                };
                let physical = mapped(&attachment.mapping, at, code.len());
                if let Some(p) = physical {
                    if rom.get(p..p + code.len()) != Some(code.as_slice()) {
                        return Err(bad(format!("original instruction bytes differ at {at:x}")));
                    }
                }
                if ins.len() >= MAX_INSTRUCTIONS
                    || ins
                        .insert(
                            at,
                            Instruction {
                                at,
                                function,
                                flow: c[3].into(),
                                bytes: code,
                                writes,
                                targets,
                                physical,
                            },
                        )
                        .is_some()
                {
                    return Err(bad("duplicate instruction/resource bound"));
                }
            }
            _ => return Err(bad("unsupported or partial listing row")),
        }
    }
    if ins.is_empty() || (require_functions && functions.is_empty()) {
        return Err(bad("empty/partial listing"));
    }
    let ins: Vec<_> = ins.into_values().collect();
    for pair in ins.windows(2) {
        if pair[0]
            .at
            .checked_add(pair[0].bytes.len() as u32)
            .is_none_or(|end| end > pair[1].at)
        {
            return Err(bad("overlapping instruction rows"));
        }
    }
    let mut members_by_function: BTreeMap<u32, Vec<&Instruction>> = BTreeMap::new();
    for i in &ins {
        if let Some(f) = i.function {
            members_by_function.entry(f).or_default().push(i);
        }
    }
    for (entry, f) in &functions {
        let members = members_by_function.get(entry).cloned().unwrap_or_default();
        if members.len() != f.count
            || !members.iter().any(|i| i.at == *entry)
            || members
                .iter()
                .any(|i| i.at < f.min || i.at + i.bytes.len() as u32 - 1 > f.max)
        {
            return Err(bad("partial/inconsistent function membership"));
        }
    }
    if ins
        .iter()
        .any(|i| i.function.is_some_and(|f| !functions.contains_key(&f)))
    {
        return Err(bad("missing containing function"));
    }
    Ok((functions, ins))
}
#[derive(Clone)]
struct State {
    lanes: [Option<Producer>; 2],
    reason: String,
    hold_at: Option<SourceRow>,
}
impl State {
    fn unknown(reason: &str) -> Self {
        Self {
            lanes: [None, None],
            reason: reason.into(),
            hold_at: None,
        }
    }
    fn invalidated(reason: &str, i: &Instruction) -> Self {
        let mut state = Self::unknown(reason);
        state.hold_at = i.physical.map(|physical| SourceRow {
            logical: i.at,
            physical,
            bytes: byte_hex(&i.bytes),
        });
        state
    }
    fn snapshot(&self) -> RegisterValue {
        let value = self.lanes[0]
            .as_ref()
            .zip(self.lanes[1].as_ref())
            .map(|(lo, hi)| u16::from(lo.value) + (u16::from(hi.value) << 8));
        RegisterValue {
            value,
            low: self.lanes[0].clone(),
            high: self.lanes[1].clone(),
            hold: value.is_none().then(|| self.reason.clone()),
            hold_at: if value.is_none() {
                self.hold_at.clone()
            } else {
                None
            },
        }
    }
}
fn clear(state: &mut BTreeMap<String, State>, why: &str) {
    for v in state.values_mut() {
        *v = State::unknown(why);
    }
}
fn parent(index: u8, byte: bool) -> &'static str {
    if byte {
        ["AX", "CX", "DX", "BX", "AX", "CX", "DX", "BX"][usize::from(index)]
    } else {
        ["AX", "CX", "DX", "BX", "SP", "BP", "SI", "DI"][usize::from(index)]
    }
}
// Supported effect rules use the exact default-16-bit ModR/M extent. A
// longer retained row must not conceal another instruction's register write.
fn modrm16_length(code: &[u8]) -> Option<usize> {
    let byte = *code.get(1)?;
    let displacement = match (byte >> 6, byte & 7) {
        (0, 6) => 2,
        (1, _) => 1,
        (2, _) => 2,
        _ => 0,
    };
    Some(2 + displacement)
}
fn transfer(i: &Instruction, state: &mut BTreeMap<String, State>) -> Result<()> {
    let b = &i.bytes;
    let physical = i.physical.ok_or_else(|| bad("unmapped value producer"))?;
    let producer = |value| Producer {
        value,
        logical: i.at,
        physical,
        bytes: byte_hex(b),
    };
    let mut known = false;
    if b.len() == 3 && (0xb8..=0xbf).contains(&b[0]) {
        let reg = parent(b[0] - 0xb8, false);
        if !i.writes.contains(reg) {
            return Err(bad("immediate definition missing write fact"));
        }
        if let Some(s) = state.get_mut(reg) {
            s.lanes = [Some(producer(b[1])), Some(producer(b[2]))];
        }
        known = true;
    } else if b.len() == 2 && (0xb0..=0xb7).contains(&b[0]) {
        let index = b[0] - 0xb0;
        let reg = parent(index, true);
        if !i.writes.contains(reg) {
            return Err(bad("byte definition missing write fact"));
        }
        if let Some(s) = state.get_mut(reg) {
            s.lanes[usize::from(index >= 4)] = Some(producer(b[1]));
        }
        known = true;
    } else if b.len() == 2 && [0x31, 0x33].contains(&b[0]) && [0xc9, 0xdb, 0xd2].contains(&b[1]) {
        let reg = parent(b[1] & 7, false);
        if !i.writes.contains(reg) {
            return Err(bad("zero definition missing write fact"));
        }
        if let Some(s) = state.get_mut(reg) {
            s.lanes = [Some(producer(0)), Some(producer(0))];
        }
        known = true;
    }
    if known {
        return Ok(());
    }
    // Explicit safe effects only. Unsupported operations kill all tracked state,
    // even when an export omitted an implicit register write.
    let mut code = b.as_slice();
    while code
        .first()
        .is_some_and(|p| [0x26, 0x2e, 0x36, 0x3e].contains(p))
    {
        code = &code[1..];
    }
    let modrm_valid = modrm16_length(code) == Some(code.len());
    let kill = if modrm_valid && code[0] == 0xf7 && (code[1] >> 3) & 7 >= 4 {
        Some("DX")
    } else if modrm_valid && [0x8a, 0x8b].contains(&code[0]) {
        Some(parent((code[1] >> 3) & 7, code[0] == 0x8a))
    } else if code.len() == 1 && (0x58..=0x5f).contains(&code[0]) {
        Some(parent(code[0] - 0x58, false))
    } else if code.len() == 1 && (0x40..=0x4f).contains(&code[0]) {
        Some(parent(code[0] & 7, false))
    } else if code.len() == 3
        && code[0] == 0x80
        && code[1] & 0xc0 == 0xc0
        && (code[1] >> 3) & 7 != 7
    {
        Some(parent(code[1] & 7, true))
    } else {
        None
    };
    if let Some(reg) = kill {
        if let Some(s) = state.get_mut(reg) {
            *s = State::invalidated("register clobber/unsupported value producer", i);
        }
        return Ok(());
    }
    let safe = code == [0x90]
        || (code.len() == 1 && (0x50..=0x57).contains(&code[0]))
        || (modrm_valid && ((0x38..=0x3b).contains(&code[0]) || [0x84, 0x85].contains(&code[0])))
        || (code.len() == 2 && [0x3c, 0xa8].contains(&code[0]))
        || (code.len() == 3 && [0x3d, 0xa9].contains(&code[0]))
        || (code.len() == 3 && code[0] == 0x80 && code[1] & 0xf8 == 0xf8);
    if safe {
        for (reg, s) in state {
            if i.writes.contains(reg) {
                *s = State::invalidated("retained register write", i);
            }
        }
    } else {
        for s in state.values_mut() {
            *s = State::invalidated("unsupported instruction effect", i);
        }
    }
    Ok(())
}
fn pointer(rom: &[u8], a: &Attachment, c: &Constructor, dx: u16) -> Pointer {
    let mut p = Pointer {
        status: "HOLD".into(),
        hold: Some("mapping/selector not resolved".into()),
        selector: dx,
        group_logical: None,
        group_physical: None,
        group_sha256: None,
        bank_word: None,
        out_al: None,
        physical_bank: None,
        pointer_field_physical: None,
        pointer_word: None,
        source_physical: None,
    };
    if let Some(reason) = &c.entry_to_core_hold {
        p.hold = Some(format!(
            "constructor entry-to-core transparency unproven: {reason}"
        ));
        return p;
    }
    let off = usize::from(c.table_offset) + usize::from(dx >> 8) * 4;
    if off > 65532 {
        p.hold = Some("group offset wraps".into());
        return p;
    }
    let logical = (u32::from(c.table_segment) << 4) + off as u32;
    p.group_logical = Some(logical);
    let Some(field) = mapped(&a.mapping, logical, 4) else {
        p.hold = Some("table outside reviewed mapping".into());
        return p;
    };
    p.group_physical = Some(field);
    p.group_sha256 = Some(ws_assets::sha256_hex(&rom[field..field + 4]));
    let table = u16::from_le_bytes([rom[field], rom[field + 1]]);
    let word = u16::from_le_bytes([rom[field + 2], rom[field + 3]]);
    p.bank_word = Some(word);
    p.out_al = Some(word as u8);
    if a.bank_policy.is_none()
        || rom.len() < 65536
        || rom.len() > 16 * 1024 * 1024
        || !rom.len().is_power_of_two()
        || !rom.len().is_multiple_of(65536)
    {
        p.hold = Some("explicit supported ROM1 geometry/policy required".into());
        return p;
    }
    let bank = usize::from(word as u8) & (rom.len() / 65536 - 1);
    p.physical_bank = Some(bank);
    let offset = usize::from(table) + usize::from(dx & 255) * 2;
    if offset > 65534 {
        p.hold = Some("pointer field wraps".into());
        return p;
    }
    let field = bank * 65536 + offset;
    p.pointer_field_physical = Some(field);
    let v = u16::from_le_bytes([rom[field], rom[field + 1]]);
    p.pointer_word = Some(v);
    p.source_physical = Some(bank * 65536 + usize::from(v));
    p.status = "SOURCE_POINTER_LEAD_ONLY".into();
    p.hold = None;
    p
}

fn entry_to_core_hold(
    members: &[&Instruction],
    entry: u32,
    core: u32,
    incoming: &BTreeSet<u32>,
) -> Option<String> {
    let mut next = entry;
    for i in members.iter().filter(|i| i.at < core) {
        if i.at != next || i.physical.is_none() {
            return Some("prefix has a gap or unmapped original instruction".into());
        }
        if i.flow != "FALL_THROUGH" || !i.targets.is_empty() {
            return Some("prefix contains control flow".into());
        }
        // Exact pushes, NOP and immediate-port input to AL preserve DX/BX/CX.
        // The matched core overwrites AX before using it. No port behavior or
        // bank state is inferred from this register-effect check.
        // Exported write metadata alone cannot prove a neutral prefix.
        let neutral = (i.bytes.len() == 1
            && matches!(i.bytes[0], 0x06 | 0x0e | 0x16 | 0x1e | 0x50..=0x57 | 0x90))
            || (i.bytes.len() == 2 && i.bytes[0] == 0xe4);
        if !neutral {
            return Some("prefix is outside the exact neutral opcode set".into());
        }
        let Some(end) = i.at.checked_add(i.bytes.len() as u32) else {
            return Some("prefix instruction address wraps".into());
        };
        next = end;
    }
    if next != core {
        return Some("prefix does not reach the matched core contiguously".into());
    }
    if entry < core && incoming.range((entry + 1)..=core).next().is_some() {
        return Some("recorded alternate entry reaches the prefix or core".into());
    }
    None
}

pub fn analyze(
    rom: &[u8],
    report: &SearchReport,
    a: &Attachment,
    attachment_sha256: &str,
    input: &[u8],
) -> Result<Sidecar> {
    if a.schema != ATTACHMENT_SCHEMA
        || report.schema != crate::search::SCHEMA
        || report.rom_bytes != rom.len()
        || rom.len() > 64 * 1024 * 1024
        || !hash_ok(attachment_sha256)
        || !hash_ok(&a.listing.sha256)
        || a.listing.path.is_empty()
        || a.rom_bytes != rom.len()
        || a.rom_sha256.to_ascii_lowercase() != ws_assets::sha256_hex(rom)
        || a.rom_sha256.to_ascii_lowercase() != report.rom_sha256
        || !report.options.families.iter().any(|f| {
            matches!(
                f,
                Family::BankedQueueConstructor
                    | Family::IndexedEightWordCopy
                    | Family::TokenDistance12Length4Plus3
            )
        })
    {
        return Err(bad("attachment ROM/configuration identity differs"));
    }
    if a.mapping.is_empty() || a.mapping.len() > 64 {
        return Err(bad("mapping count"));
    }
    let mut ranges = a.mapping.clone();
    ranges.sort_by_key(|m| m.logical_start);
    for m in &ranges {
        if m.length == 0
            || m.physical_start
                .checked_add(m.length)
                .is_none_or(|end| end > rom.len())
            || u32::try_from(m.length)
                .ok()
                .and_then(|n| m.logical_start.checked_add(n))
                .is_none()
        {
            return Err(bad("mapping extent"));
        }
    }
    if ranges
        .windows(2)
        .any(|m| m[0].logical_start as u64 + m[0].length as u64 > u64::from(m[1].logical_start))
    {
        return Err(bad("overlapping logical mappings"));
    }
    let (functions, ins) = listing(
        rom,
        a,
        input,
        report
            .options
            .families
            .contains(&Family::BankedQueueConstructor),
    )?;
    let incoming: BTreeSet<_> = ins.iter().flat_map(|i| i.targets.iter().copied()).collect();
    // A recorded entry inside a listed instruction has no compatible retained
    // decode. Do not apply that instruction's transfer to an alternate entry.
    let interior_functions: BTreeSet<_> = ins
        .iter()
        .filter(|i| {
            incoming
                .range((i.at + 1)..(i.at + i.bytes.len() as u32))
                .next()
                .is_some()
        })
        .filter_map(|i| i.function)
        .collect();
    if ins.iter().map(|i| i.targets.len()).sum::<usize>() > 1_000_000 {
        return Err(bad("direct target count exceeds bound"));
    }
    let direct_entries = ins
        .iter()
        .flat_map(|i| {
            i.targets.iter().map(|target| DirectEntry {
                source_logical: i.at,
                source_physical: i.physical,
                source_function: i.function,
                source_bytes: byte_hex(&i.bytes),
                flow: i.flow.clone(),
                target_logical: *target,
            })
        })
        .collect();
    let unresolved_flow_sites = ins
        .iter()
        .filter(|i| {
            i.flow.contains("COMPUTED")
                || ![
                    "FALL_THROUGH",
                    "UNCONDITIONAL_CALL",
                    "CONDITIONAL_CALL",
                    "UNCONDITIONAL_JUMP",
                    "CONDITIONAL_JUMP",
                    "TERMINATOR",
                ]
                .contains(&i.flow.as_str())
        })
        .map(|i| UnresolvedFlow {
            logical: i.at,
            physical: i.physical,
            function: i.function,
            bytes: byte_hex(&i.bytes),
            flow: i.flow.clone(),
        })
        .collect();
    let mut readers = Vec::new();
    for lead in report.leads.iter().filter(|l| {
        matches!(
            l.family,
            Family::IndexedEightWordCopy | Family::TokenDistance12Length4Plus3
        )
    }) {
        let span = rom
            .get(lead.offset..lead.offset.saturating_add(lead.stored_length))
            .ok_or_else(|| bad("reader lead bounds"))?;
        let (length, expected) =
            crate::search::reader_pattern(lead.family, span, &rom[..lead.offset])
                .ok_or_else(|| bad("reader core pattern differs"))?;
        let declared = lead
            .reader_core
            .as_ref()
            .ok_or_else(|| bad("reader captures absent"))?;
        if length != lead.stored_length
            || ws_assets::sha256_hex(span) != lead.stored_sha256
            || expected.pattern != declared.pattern
            || expected.version != declared.version
            || expected.variant != declared.variant
            || expected.captures != declared.captures
        {
            return Err(bad("reader lead identity/captures differ"));
        }
        let mut joined = ReaderJoin {
            family: lead.family,
            pattern: expected.pattern.clone(),
            version: expected.version,
            lead_offset: lead.offset,
            lead_sha256: lead.stored_sha256.clone(),
            core_logical: None,
            function_entry: None,
            status: "HOLD".into(),
            hold: Some("unique retained instruction boundary absent".into()),
            rows: Vec::new(),
            recorded_alternate_entries: Vec::new(),
        };
        let candidates: Vec<_> = ins
            .iter()
            .filter(|i| i.physical == Some(lead.offset))
            .collect();
        if candidates.len() == 1 {
            let first = candidates[0];
            joined.core_logical = Some(first.at);
            let lengths = crate::search::reader_instruction_lengths(&expected);
            let end = first
                .at
                .checked_add(length as u32)
                .ok_or_else(|| bad("reader logical span wraps"))?;
            let mut cursor = first.at;
            let mut physical = lead.offset;
            let mut compatible = true;
            let mut core_function = first.function;
            for (index, n) in lengths.iter().copied().enumerate() {
                let Some(row) = ins
                    .binary_search_by_key(&cursor, |i| i.at)
                    .ok()
                    .map(|n| &ins[n])
                else {
                    compatible = false;
                    break;
                };
                let loop_row =
                    lead.family == Family::IndexedEightWordCopy && index + 2 == lengths.len();
                let flow_ok = if loop_row {
                    row.flow == "CONDITIONAL_JUMP" && row.targets == [first.at + 40]
                } else {
                    row.flow == "FALL_THROUGH" && row.targets.is_empty()
                };
                if row.bytes.len() != n
                    || row.physical != Some(physical)
                    || !flow_ok
                    || row.function != core_function
                    || incoming
                        .range((cursor + 1)..(cursor + n as u32))
                        .next()
                        .is_some()
                {
                    compatible = false;
                    break;
                }
                joined.rows.push(SourceRow {
                    logical: row.at,
                    physical,
                    bytes: byte_hex(&row.bytes),
                });
                cursor += n as u32;
                physical += n;
            }
            if compatible && cursor == end {
                joined.status = "RETAINED_INSTRUCTIONS_COMPATIBLE".into();
                joined.hold = None;
                joined.function_entry = core_function.take();
                joined.recorded_alternate_entries = ins
                    .iter()
                    .filter(|i| i.at < first.at || i.at >= end)
                    .flat_map(|i| i.targets.iter().copied())
                    .filter(|t| *t > first.at && *t < end)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
            } else {
                joined.hold = Some(
                    "reader boundaries, original mapping, function membership or flow roles differ"
                        .into(),
                );
            }
        }
        readers.push(joined);
    }
    let mut constructors = Vec::new();
    for lead in report
        .leads
        .iter()
        .filter(|l| l.family == Family::BankedQueueConstructor)
    {
        let span = rom
            .get(lead.offset..lead.offset.saturating_add(lead.stored_length))
            .ok_or_else(|| bad("lead bounds"))?;
        if !crate::search::constructor_pattern_matches(span)
            || lead.stored_sha256 != ws_assets::sha256_hex(span)
            || lead.stored_length != 73
        {
            return Err(bad("code lead identity differs"));
        }
        let candidates: Vec<_> = ins
            .iter()
            .filter(|i| i.physical == Some(lead.offset))
            .collect();
        let mut c = Constructor {
            lead_offset: lead.offset,
            lead_sha256: lead.stored_sha256.clone(),
            core_logical: None,
            function_entry: None,
            function_address: None,
            function_instruction_count: 0,
            function_instruction_sha256: None,
            status: "HOLD".into(),
            hold: Some("unique retained instruction/function boundary absent".into()),
            entry_to_core_hold: Some("compatible function entry/core binding absent".into()),
            table_segment: u16::from_le_bytes([span[4], span[5]]),
            table_offset: u16::from_le_bytes([span[1], span[2]]),
            queue_pointer: u16::from_le_bytes([span[46], span[47]]),
        };
        if candidates.len() == 1 {
            let first = candidates[0];
            c.core_logical = Some(first.at);
            c.function_entry = first.function;
            c.function_address = first
                .function
                .and_then(|f| functions.get(&f).map(|f| f.address.clone()));
            if let Some(entry) = first.function {
                let members: Vec<_> = ins.iter().filter(|i| i.function == Some(entry)).collect();
                c.function_instruction_count = members.len();
                let end = first
                    .at
                    .checked_add(73)
                    .ok_or_else(|| bad("core logical address wraps"))?;
                let core: Vec<_> = members
                    .iter()
                    .filter(|i| i.at >= first.at && i.at < end)
                    .copied()
                    .collect();
                let mut next = first.at;
                let lengths = [
                    3usize, 3, 2, 2, 2, 2, 2, 2, 2, 4, 2, 3, 1, 3, 2, 2, 2, 2, 3, 4, 1, 2, 2, 3, 3,
                    3, 3, 4, 4,
                ];
                let roles = core
                    .iter()
                    .all(|r| r.flow == "FALL_THROUGH" && r.targets.is_empty())
                    && core.len() == lengths.len()
                    && core.iter().zip(lengths).all(|(r, n)| r.bytes.len() == n);
                let contiguous = roles
                    && core.iter().all(|i| {
                        let ok = i.at == next;
                        next = i.at.saturating_add(i.bytes.len() as u32);
                        ok
                    })
                    && next == end;
                if contiguous
                    && !interior_functions.contains(&entry)
                    && members.iter().all(|i| i.physical.is_some())
                {
                    c.function_instruction_sha256 = Some(ws_assets::sha256_hex(
                        &members
                            .iter()
                            .flat_map(|i| i.bytes.iter().copied())
                            .collect::<Vec<_>>(),
                    ));
                    c.status = "RETAINED_LISTING_COMPATIBLE".into();
                    c.hold = None;
                    c.entry_to_core_hold = entry_to_core_hold(&members, entry, first.at, &incoming);
                } else {
                    c.hold = Some(
                        "constructor core/function has a gap, interior entry or unmapped original bytes".into(),
                    );
                }
            }
        }
        constructors.push(c);
    }
    let mut entry_counts = BTreeMap::new();
    for c in &constructors {
        if c.status == "RETAINED_LISTING_COMPATIBLE" {
            if let Some(f) = c.function_entry {
                *entry_counts.entry(f).or_insert(0usize) += 1;
            }
        }
    }
    for c in &mut constructors {
        if c.function_entry
            .is_some_and(|f| entry_counts.get(&f).is_some_and(|n| *n > 1))
        {
            c.status = "HOLD".into();
            c.hold = Some("multiple constructor cores share one entry".into());
        }
    }
    let target_indices: BTreeMap<_, _> = constructors
        .iter()
        .enumerate()
        .filter(|(_, c)| c.status == "RETAINED_LISTING_COMPATIBLE")
        .filter_map(|(n, c)| c.function_entry.map(|at| (at, n)))
        .collect();
    let mapped_entries: BTreeSet<_> = ins
        .iter()
        .filter(|i| i.physical.is_some())
        .map(|i| i.at)
        .collect();
    let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for i in &ins {
        groups.entry(i.function).or_insert_with(Vec::new).push(i);
    }
    let relevant_functions: BTreeSet<_> = ins
        .iter()
        .filter(|i| i.bytes.len() == 5 && i.bytes[0] == 0x9a)
        .filter(|i| {
            let target = u32::from(u16::from_le_bytes([i.bytes[3], i.bytes[4]])) * 16
                + u32::from(u16::from_le_bytes([i.bytes[1], i.bytes[2]]));
            target_indices.contains_key(&target)
        })
        .map(|i| i.function)
        .collect();
    let mut callers = Vec::new();
    for (fn_entry, rows) in groups {
        if !relevant_functions.contains(&fn_entry) {
            continue;
        }
        let mut state: BTreeMap<_, _> = ["DX", "BX", "CX"]
            .into_iter()
            .map(|r| (r.into(), State::unknown("no definition in retained block")))
            .collect();
        let mut block: Vec<&Instruction> = Vec::new();
        let mut next = None;
        for i in rows {
            if next != Some(i.at) || incoming.contains(&i.at) {
                clear(&mut state, "function/gap/recorded incoming target reset");
                block.clear();
            }
            next = i.at.checked_add(i.bytes.len() as u32);
            let call = (i.bytes.len() == 5 && i.bytes[0] == 0x9a).then(|| {
                u32::from(u16::from_le_bytes([i.bytes[3], i.bytes[4]])) * 16
                    + u32::from(u16::from_le_bytes([i.bytes[1], i.bytes[2]]))
            });
            if let Some((target, index)) =
                call.and_then(|t| target_indices.get(&t).map(|n| (t, *n)))
            {
                if callers.len() >= MAX_CALLERS {
                    return Err(bad("caller count exceeds bound"));
                }
                let mut source = block.clone();
                source.push(i);
                let mut source_gaps = Vec::new();
                if fn_entry.is_none() {
                    source_gaps.push("no retained containing function");
                }
                if fn_entry.is_some_and(|f| !mapped_entries.contains(&f)) {
                    source_gaps.push("function entry is outside reviewed original mapping");
                }
                if fn_entry.is_some_and(|f| interior_functions.contains(&f)) {
                    source_gaps.push("recorded entry inside a function instruction");
                }
                if i.flow != "UNCONDITIONAL_CALL" {
                    source_gaps.push("direct CALLF flow fact is incompatible");
                }
                if !i.targets.contains(&target) {
                    source_gaps.push("retained direct target does not bind CALLF operand");
                }
                if source.iter().any(|r| r.physical.is_none()) {
                    source_gaps.push("caller block contains unmapped original instructions");
                }
                let source_hold = source_gaps.join("; ");
                let bound = source_gaps.is_empty();
                let arguments = state
                    .iter()
                    .map(|(r, s)| {
                        (
                            r.clone(),
                            if bound {
                                s.snapshot()
                            } else {
                                State::unknown(&source_hold).snapshot()
                            },
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                let pointer = arguments
                    .get("DX")
                    .and_then(|v| v.value)
                    .map(|dx| pointer(rom, a, &constructors[index], dx));
                callers.push(Caller {
                    call_logical: i.at,
                    call_physical: i.physical,
                    caller_function: fn_entry,
                    caller_function_address: fn_entry
                        .and_then(|f| functions.get(&f).map(|f| f.address.clone())),
                    target_logical: target,
                    target_segment: u16::from_le_bytes([i.bytes[3], i.bytes[4]]),
                    target_offset: u16::from_le_bytes([i.bytes[1], i.bytes[2]]),
                    constructor_index: index,
                    call_bytes: byte_hex(&i.bytes),
                    source_status: if bound {
                        "SOURCE_BOUND_DIRECT_CALL"
                    } else {
                        "HOLD"
                    }
                    .into(),
                    hold: (!bound).then(|| source_hold.clone()),
                    arguments,
                    block: source
                        .iter()
                        .filter_map(|r| {
                            r.physical.map(|physical| SourceRow {
                                logical: r.at,
                                physical,
                                bytes: byte_hex(&r.bytes),
                            })
                        })
                        .collect(),
                    pointer,
                });
            }
            if fn_entry.is_none() {
                clear(&mut state, "no retained containing function");
                block.clear();
            } else if i.physical.is_none() {
                clear(&mut state, "unmapped original instruction");
                block.clear();
            } else if call.is_some()
                || i.flow.contains("CALL")
                || i.flow.contains("JUMP")
                || i.flow.contains("TERMINATOR")
                || i.flow != "FALL_THROUGH"
            {
                clear(&mut state, "call/branch/terminator/unknown flow reset");
                block.clear();
            } else {
                transfer(i, &mut state)?;
                block.push(i);
                if block.len() > MAX_BLOCK {
                    return Err(bad("straightline block exceeds bound"));
                }
            }
        }
    }
    let has_readers = !readers.is_empty();
    Ok(Sidecar {
        schema: SIDECAR_SCHEMA.into(),
        policy: if has_readers {
            "relational-reader-instruction-join-v1"
        } else {
            "immediate-lanes-neutral-constructor-prefix-v2"
        }
        .into(),
        assurance: if has_readers {
            "retained-reader-join-candidates; entry-input-profile-open"
        } else {
            "retained-direct-flow-candidate; incomplete-listing"
        }
        .into(),
        rom_sha256: report.rom_sha256.clone(),
        rom_bytes: rom.len(),
        attachment_sha256: attachment_sha256.to_ascii_lowercase(),
        listing_sha256: a.listing.sha256.to_ascii_lowercase(),
        mapping_sha256: ws_assets::sha256_hex(
            &serde_json::to_vec(&a.mapping).map_err(|e| bad(e.to_string()))?,
        ),
        listed_instructions: ins.len(),
        unmapped_instructions: ins.iter().filter(|i| i.physical.is_none()).count(),
        recorded_direct_targets: incoming.len(),
        direct_entries,
        unresolved_flow_sites,
        constructors,
        callers,
        readers,
        limitations: {
            let mut limits = vec![
            "Retained direct-flow candidates only; indirect/unexported entries and listing correctness are not proven complete. No project/code adoption or execution claim.".into(),
            "Unknown register values remain HOLD; no arithmetic folding, memory-value propagation, branch joins or callee-preservation inference.".into(),
            "Caller arguments describe function-entry values. Downstream pointer resolution also requires the retained exact neutral entry-to-core prefix; no register rewrite, callee preservation or alternate entry is assumed.".into(),
            "Pointer leads use only explicitly reviewed original mapping/bank policy. No table extent, complete object, codec, output, media, SDK or runtime ownership follows.".into(),
            "Unmapped/unvisited code is preserved; absence of a retained match is not absence of code or resources.".into(),
            "Only retained constructor code leads are analyzed; stride, budget and retention limits are not whole-listing constructor coverage.".into(),
            ];
            if has_readers {
                limits.pop();
                limits.push("Reader joins verify exact original bytes, each listed instruction boundary and family-specific flow roles only. A missing function record is not fabricated into a containing function.".into());
                limits.push("Recorded alternate entries and unrecorded/indirect control flow do not establish whole-core invocation. Header, helper, input, segment/bank, SDK and resource semantics remain OPEN.".into());
            }
            limits
        },
    })
}
