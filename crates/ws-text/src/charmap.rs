use crate::{limit, malformed, TextLimits};
use formatkit_core::{bytes_at, Reader, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextEncoding {
    #[default]
    Table,
    /// WHATWG Shift_JIS (Windows-31J), strict, without replacement.
    ShiftJis,
    /// Eight-bit JIS X 0201: Roman yen/overline and half-width katakana.
    JisX0201,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodeUnit {
    #[default]
    Byte,
    WordLe,
    WordBe,
}
impl CodeUnit {
    fn width(self) -> usize {
        if self == Self::Byte {
            1
        } else {
            2
        }
    }
    fn read(self, reader: &mut Reader<'_>) -> Result<u16> {
        match self {
            Self::Byte => Ok(u16::from(reader.u8()?)),
            Self::WordLe => reader.u16_le(),
            Self::WordBe => reader.u16_be(),
        }
    }
    fn bytes(self, code: u16) -> Vec<u8> {
        match self {
            Self::Byte => vec![code as u8],
            Self::WordLe => code.to_le_bytes().to_vec(),
            Self::WordBe => code.to_be_bytes().to_vec(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterEntry {
    /// Hexadecimal bytes in source order, e.g. "fc10" or "0100".
    pub bytes: String,
    pub text: String,
    #[serde(default)]
    pub page: Option<u8>,
    /// Alternative readable spellings for context-dependent glyphs.
    #[serde(default)]
    pub aliases: Vec<String>,
}

fn full_mask() -> u16 {
    u16::MAX
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArgumentLength {
    pub low_min: u8,
    pub low_max: u8,
    pub bytes: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlCode {
    pub code: u16,
    #[serde(default = "full_mask")]
    pub mask: u16,
    #[serde(default)]
    pub argument_bytes: usize,
    #[serde(default)]
    pub argument_lengths: Vec<ArgumentLength>,
    /// Literal text, optionally {lo:02x}, {lo:d}, {args}. Operands use
    /// argument_unit, argument_separator and zero-padded hexadecimal.
    #[serde(default)]
    pub display: String,
    #[serde(default)]
    pub display_overrides: Vec<ControlDisplay>,
    #[serde(default)]
    pub argument_unit: CodeUnit,
    #[serde(default)]
    pub argument_separator: String,
    #[serde(default)]
    pub ends: bool,
    #[serde(default)]
    pub set_page: Option<u8>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlDisplay {
    pub low_min: u8,
    pub low_max: u8,
    pub display: String,
}
impl ControlCode {
    fn arguments(&self, code: u16) -> usize {
        self.argument_lengths
            .iter()
            .find(|r| (r.low_min..=r.low_max).contains(&(code as u8)))
            .map_or(self.argument_bytes, |r| r.bytes)
    }
    fn template(&self, code: u16) -> String {
        self.display_overrides
            .iter()
            .find(|r| (r.low_min..=r.low_max).contains(&(code as u8)))
            .map_or(&self.display, |r| &r.display)
            .replace("{lo:02x}", &format!("{:02x}", code as u8))
            .replace("{lo:d}", &(code as u8).to_string())
    }
    fn render(&self, code: u16, arguments: &[u8]) -> Result<String> {
        let mut r = Reader::new(arguments);
        let mut values = String::new();
        while r.remaining() > 0 {
            values.push_str(&self.argument_separator);
            let v = self.argument_unit.read(&mut r)?;
            values.push_str(&format!(
                "{v:0width$x}",
                width = self.argument_unit.width() * 2
            ));
        }
        Ok(self.template(code).replace("{args}", &values))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GlyphMask {
    pub below: u16,
    pub mask: u16,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Normalization {
    #[serde(default)]
    pub compose_kana: bool,
    #[serde(default)]
    pub long_dash_after_kana: bool,
    #[serde(default)]
    pub contextual_dash: Option<ContextualDash>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextualDash {
    pub placeholder: char,
    pub counters: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterMapDefinition {
    pub schema: String,
    #[serde(default)]
    pub encoding: TextEncoding,
    #[serde(default)]
    pub unit: CodeUnit,
    #[serde(default)]
    pub initial_page: u8,
    #[serde(default)]
    pub entries: Vec<CharacterEntry>,
    #[serde(default)]
    pub controls: Vec<ControlCode>,
    #[serde(default)]
    pub glyph_mask: Option<GlyphMask>,
    #[serde(default)]
    pub normalization: Normalization,
}

#[derive(Debug, Clone)]
struct Entry {
    bytes: Vec<u8>,
    text: String,
    page: Option<u8>,
    aliases: Vec<String>,
}

#[derive(Debug, Clone)]
struct ControlSpelling {
    index: usize,
    code: u16,
    template: String,
}

/// Validated, compiled definition. Generic Rust code has no per-title branch.
#[derive(Debug, Clone)]
pub struct CharacterMap {
    definition: CharacterMapDefinition,
    entries: Vec<Entry>,
    by_first: BTreeMap<u8, Vec<usize>>,
    by_text_first: BTreeMap<char, Vec<usize>>,
    controls_by_first: BTreeMap<char, Vec<ControlSpelling>>,
    controls_without_prefix: Vec<ControlSpelling>,
}

/// Token source bytes preserve aliases, invisible page switches and attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextToken {
    pub bytes: Vec<u8>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedText {
    pub text: String,
    pub normalized: String,
    pub consumed: usize,
    pub terminated: bool,
    pub final_page: u8,
    pub tokens: Vec<TextToken>,
}

pub fn hex_bytes(text: &str) -> Result<Vec<u8>> {
    if text.is_empty() || text.len() > 32 || !text.len().is_multiple_of(2) || !text.is_ascii() {
        return Err(malformed("code must contain 1..16 hexadecimal bytes"));
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|p| {
            let digit = |c: u8| {
                char::from(c)
                    .to_digit(16)
                    .ok_or_else(|| malformed("invalid hex code"))
            };
            Ok((digit(p[0])? * 16 + digit(p[1])?) as u8)
        })
        .collect()
}

impl CharacterMap {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        limit("character map JSON bytes", bytes.len(), 4 * 1024 * 1024)?;
        Self::new(serde_json::from_slice(bytes).map_err(|e| malformed(e.to_string()))?)
    }
    /// TSV columns: bytes, text (JSON string), optional page. Comments start #.
    pub fn from_tsv(tsv: &str, unit: CodeUnit) -> Result<Self> {
        limit("character map TSV bytes", tsv.len(), 4 * 1024 * 1024)?;
        let mut entries = Vec::new();
        for line in tsv.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if line.starts_with("bytes\t") {
                continue;
            }
            let fields: Vec<_> = line.split('\t').collect();
            if !(2..=3).contains(&fields.len()) {
                return Err(malformed("invalid character map TSV row"));
            }
            entries.push(CharacterEntry {
                bytes: fields[0].into(),
                text: serde_json::from_str(fields[1]).map_err(|e| malformed(e.to_string()))?,
                page: fields
                    .get(2)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|_| malformed("invalid TSV page"))?,
                aliases: vec![],
            });
        }
        Self::new(CharacterMapDefinition {
            schema: "swankit-charmap/1".into(),
            encoding: TextEncoding::Table,
            unit,
            initial_page: 0,
            entries,
            controls: vec![],
            glyph_mask: None,
            normalization: Normalization::default(),
        })
    }
    pub fn new(definition: CharacterMapDefinition) -> Result<Self> {
        if definition.schema != "swankit-charmap/1" {
            return Err(malformed("unknown character map schema"));
        }
        limit("character map entries", definition.entries.len(), 65536)?;
        limit("character map controls", definition.controls.len(), 256)?;
        if definition.encoding != TextEncoding::Table
            && (definition.unit != CodeUnit::Byte || definition.glyph_mask.is_some())
        {
            return Err(malformed(
                "standard encoding requires byte units and no glyph mask",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut code_pages = BTreeMap::<Vec<u8>, bool>::new();
        let mut entries = Vec::new();
        let mut by_first: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
        let mut by_text_first: BTreeMap<char, Vec<usize>> = BTreeMap::new();
        let mut retained_text = 0usize;
        for e in &definition.entries {
            let bytes = hex_bytes(&e.bytes)?;
            if !bytes.len().is_multiple_of(definition.unit.width()) || e.text.is_empty() {
                return Err(malformed("empty glyph text or incomplete code unit"));
            }
            limit("glyph text bytes", e.text.len(), 1024)?;
            if !seen.insert((e.page, bytes.clone())) {
                return Err(malformed("duplicate character code/page"));
            }
            if code_pages
                .get(&bytes)
                .is_some_and(|global| *global || e.page.is_none())
            {
                return Err(malformed(
                    "global and page-specific character codes overlap",
                ));
            }
            code_pages.insert(bytes.clone(), e.page.is_none());
            limit("glyph aliases", e.aliases.len(), 32)?;
            for alias in &e.aliases {
                limit("glyph alias bytes", alias.len(), 1024)?;
            }
            retained_text = retained_text
                .checked_add(e.text.len())
                .and_then(|n| {
                    e.aliases
                        .iter()
                        .try_fold(n, |sum, alias| sum.checked_add(alias.len()))
                })
                .ok_or_else(|| malformed("character map text size overflow"))?;
            limit("character map text bytes", retained_text, 4 * 1024 * 1024)?;
            if e.aliases.iter().any(String::is_empty) {
                return Err(malformed("empty glyph alias"));
            }
            by_first.entry(bytes[0]).or_default().push(entries.len());
            for spelling in std::iter::once(&e.text).chain(e.aliases.iter()) {
                if let Some(first) = spelling.chars().next() {
                    by_text_first.entry(first).or_default().push(entries.len());
                }
            }
            entries.push(Entry {
                bytes,
                text: e.text.clone(),
                page: e.page,
                aliases: e.aliases.clone(),
            });
        }
        for indexes in by_first.values_mut() {
            indexes.sort_by_key(|&i| std::cmp::Reverse(entries[i].bytes.len()));
        }
        let maximum_code = if definition.unit == CodeUnit::Byte {
            255
        } else {
            65535
        };
        for (i, c) in definition.controls.iter().enumerate() {
            let mask = c.mask & maximum_code;
            if c.code > maximum_code || mask == 0 || c.code & mask != c.code {
                return Err(malformed("control code/mask outside unit"));
            }
            limit("control display bytes", c.display.len(), 1024)?;
            limit(
                "control operand separator bytes",
                c.argument_separator.len(),
                32,
            )?;
            limit("control operand bytes", c.argument_bytes, 256)?;
            limit("control operand ranges", c.argument_lengths.len(), 256)?;
            limit("control display ranges", c.display_overrides.len(), 256)?;
            if !c.argument_bytes.is_multiple_of(c.argument_unit.width()) {
                return Err(malformed("partial control operand unit"));
            }
            for (n, r) in c.argument_lengths.iter().enumerate() {
                if r.low_min > r.low_max
                    || !r.bytes.is_multiple_of(c.argument_unit.width())
                    || r.bytes > 256
                    || c.argument_lengths[..n]
                        .iter()
                        .any(|q| r.low_min <= q.low_max && q.low_min <= r.low_max)
                {
                    return Err(malformed("invalid or overlapping control operand lengths"));
                }
            }
            if definition.controls[..i]
                .iter()
                .any(|q| ((q.code ^ c.code) & q.mask & mask) == 0)
            {
                return Err(malformed("overlapping control codes"));
            }
            if c.display.matches("{args}").count() > 1 {
                return Err(malformed("repeated operand placeholder"));
            }
            for (n, r) in c.display_overrides.iter().enumerate() {
                limit("control display bytes", r.display.len(), 1024)?;
                if r.low_min > r.low_max
                    || r.display.matches("{args}").count() > 1
                    || c.display_overrides[..n]
                        .iter()
                        .any(|q| r.low_min <= q.low_max && q.low_min <= r.low_max)
                {
                    return Err(malformed("invalid or overlapping control displays"));
                }
            }
            for e in &entries {
                let mut reader = Reader::new(&e.bytes);
                if definition.unit.read(&mut reader)? & mask == c.code {
                    return Err(malformed("character code overlaps a control"));
                }
            }
        }
        // Compile each low-byte spelling once. A masked high byte uses its
        // lowest matching value for canonical encoding; source tokens retain
        // every original bit. Controls are disjoint, so at most 65,536 forms.
        let mut controls_by_first: BTreeMap<char, Vec<ControlSpelling>> = BTreeMap::new();
        let mut controls_without_prefix = Vec::new();
        let mut spelling_bytes = 0usize;
        for (index, c) in definition.controls.iter().enumerate() {
            for lo in 0..=255 {
                let code = (c.code & 0xff00) | lo;
                if code & c.mask != c.code {
                    continue;
                }
                let template = c.template(code);
                if let Some(first) = template.chars().next() {
                    spelling_bytes = spelling_bytes.saturating_add(template.len());
                    limit(
                        "compiled control spelling bytes",
                        spelling_bytes,
                        8 * 1024 * 1024,
                    )?;
                    let form = ControlSpelling {
                        index,
                        code,
                        template,
                    };
                    if form.template.starts_with("{args}") {
                        controls_without_prefix.push(form);
                    } else {
                        controls_by_first.entry(first).or_default().push(form);
                    }
                }
            }
        }
        Ok(Self {
            definition,
            entries,
            by_first,
            by_text_first,
            controls_by_first,
            controls_without_prefix,
        })
    }
    pub fn definition(&self) -> &CharacterMapDefinition {
        &self.definition
    }

    pub fn decode(&self, bytes: &[u8], limits: TextLimits) -> Result<DecodedText> {
        limit("text input bytes", bytes.len(), limits.input_bytes)?;
        let d = &self.definition;
        let mut reader = Reader::new(bytes);
        let mut page = d.initial_page;
        let mut tokens = Vec::new();
        let mut text = String::new();
        let mut terminated = false;
        while reader.remaining() > 0 {
            limit("text tokens", tokens.len().saturating_add(1), limits.tokens)?;
            let start = reader.pos();
            let code = d.unit.read(&mut reader)?;
            let mut ends = false;
            let rendered = if let Some(c) = d.controls.iter().find(|c| code & c.mask == c.code) {
                let args = reader.bytes(c.arguments(code))?;
                if let Some(p) = c.set_page {
                    page = p;
                }
                ends = c.ends;
                c.render(code, args)?
            } else {
                let masked = d
                    .glyph_mask
                    .as_ref()
                    .filter(|m| code < m.below)
                    .map_or(code, |m| code & m.mask);
                let first = d.unit.bytes(masked)[0];
                let entry = self
                    .by_first
                    .get(&first)
                    .into_iter()
                    .flatten()
                    .map(|&i| &self.entries[i])
                    .find(|e| {
                        e.page.is_none_or(|p| p == page)
                            && if masked != code {
                                e.bytes == d.unit.bytes(masked)
                            } else {
                                bytes.get(start..start.saturating_add(e.bytes.len()))
                                    == Some(e.bytes.as_slice())
                            }
                    });
                if let Some(e) = entry {
                    if e.bytes.len() > d.unit.width() {
                        reader.skip(e.bytes.len() - d.unit.width())?;
                    }
                    e.text.clone()
                } else if d.encoding != TextEncoding::Table {
                    let width = if d.encoding == TextEncoding::ShiftJis
                        && ((0x81..=0x9f).contains(&code) || (0xe0..=0xfc).contains(&code))
                    {
                        2
                    } else {
                        1
                    };
                    if width == 2 {
                        reader.skip(1)?;
                    }
                    decode_standard(d.encoding, bytes_at(bytes, start, width)?)?
                } else {
                    return Err(malformed(format!(
                        "unknown text code {code:04x} at {start:#x}, page {page}"
                    )));
                }
            };
            limit(
                "decoded text bytes",
                text.len().saturating_add(rendered.len()),
                limits.output_bytes,
            )?;
            text.push_str(&rendered);
            tokens.push(TextToken {
                bytes: bytes_at(bytes, start, reader.pos() - start)?.to_vec(),
                text: rendered,
            });
            if ends {
                terminated = true;
                break;
            }
        }
        let text = resolve_dash(&text, &d.normalization);
        let normalized = normalize(&text, &d.normalization);
        limit(
            "normalized text bytes",
            normalized.len(),
            limits.output_bytes,
        )?;
        Ok(DecodedText {
            text,
            normalized,
            consumed: reader.pos(),
            terminated,
            final_page: page,
            tokens,
        })
    }

    /// Exact source encoding for a decoded token stream, with validation.
    pub fn encode_tokens(&self, tokens: &[TextToken], limits: TextLimits) -> Result<Vec<u8>> {
        limit("text tokens", tokens.len(), limits.tokens)?;
        let mut bytes = Vec::new();
        for token in tokens {
            limit(
                "encoded text bytes",
                bytes.len().saturating_add(token.bytes.len()),
                limits.input_bytes,
            )?;
            bytes.extend_from_slice(&token.bytes);
        }
        let got = self.decode(&bytes, limits)?;
        if got.consumed != bytes.len() || got.tokens != tokens {
            return Err(malformed("invalid or trailing text tokens"));
        }
        Ok(bytes)
    }

    /// Hidden attribute bits of a single-unit glyph token: the bits a
    /// `glyph_mask` ignores when choosing the glyph (`None` for controls,
    /// multi-unit entries or maps without a mask).
    fn attribute_bits(&self, token: &TextToken) -> Option<u16> {
        let d = &self.definition;
        let mask = d.glyph_mask.as_ref()?;
        if token.bytes.len() != d.unit.width() {
            return None;
        }
        let code = d.unit.read(&mut Reader::new(&token.bytes)).ok()?;
        if code >= mask.below || d.controls.iter().any(|c| code & c.mask == c.code) {
            return None;
        }
        Some(code & !mask.mask)
    }

    /// Bytes for an edited text that keep the source exactly where it is
    /// unchanged: the longest unchanged token prefix and suffix are copied
    /// (hidden control operands, aliases, attribute bits). Inside the edited
    /// span a token equal to the source token at the same span position keeps
    /// its bytes, and every other glyph takes the hidden attribute bits of the
    /// source glyph at that position (the last source glyph past its end, the
    /// neighbouring glyph for a pure insertion). The result must re-decode to
    /// `text` and end where the source ended.
    pub fn encode_edit(&self, original: &[u8], text: &str, limits: TextLimits) -> Result<Vec<u8>> {
        let old = self.decode(original, limits)?;
        let tokens = &old.tokens;
        // The terminating control never joins the copied prefix.
        let limit = tokens.len() - usize::from(old.terminated && !tokens.is_empty());
        let mut head = 0;
        let mut used = 0;
        while head < limit && text[used..].starts_with(tokens[head].text.as_str()) {
            used += tokens[head].text.len();
            head += 1;
        }
        let mut tail = tokens.len();
        let mut rest = text.len();
        while tail > head
            && rest >= used + tokens[tail - 1].text.len()
            && text[..rest].ends_with(tokens[tail - 1].text.as_str())
        {
            rest -= tokens[tail - 1].text.len();
            tail -= 1;
        }
        let middle = &text[used..rest];
        let mut bytes: Vec<u8> = tokens[..head]
            .iter()
            .flat_map(|t| t.bytes.iter().copied())
            .collect();
        if !middle.is_empty() {
            let encoded = self.encode(middle, limits.input_bytes)?;
            let mut fresh = self.decode(&encoded, limits)?.tokens;
            if tail < tokens.len() && fresh.last().is_some_and(|t| t.text.is_empty()) {
                // Drop the canonical end appended to the fragment; the source suffix ends it.
                fresh.pop();
            }
            let span = &tokens[head..tail];
            let glyph_attributes: Vec<Option<u16>> =
                span.iter().map(|t| self.attribute_bits(t)).collect();
            let neighbour = head
                .checked_sub(1)
                .and_then(|i| self.attribute_bits(&tokens[i]))
                .or_else(|| tokens.get(tail).and_then(|t| self.attribute_bits(t)));
            let d = &self.definition;
            for (k, token) in fresh.iter().enumerate() {
                if let Some(source) = span.get(k).filter(|s| s.text == token.text) {
                    bytes.extend_from_slice(&source.bytes);
                    continue;
                }
                let inherited = if span.is_empty() {
                    neighbour
                } else {
                    // The nearest source glyph at or before this position, else after it.
                    let at = k.min(span.len() - 1);
                    glyph_attributes[..=at]
                        .iter()
                        .rev()
                        .chain(glyph_attributes[at + 1..].iter())
                        .find_map(|a| *a)
                };
                let attributed = match (inherited, self.attribute_bits(token)) {
                    (Some(bits), Some(0)) if bits != 0 => {
                        let code = d.unit.read(&mut Reader::new(&token.bytes))? | bits;
                        let candidate = d.unit.bytes(code);
                        let check = self.decode(&candidate, limits)?;
                        (check.text == token.text && check.consumed == candidate.len())
                            .then_some(candidate)
                    }
                    _ => None,
                };
                bytes.extend_from_slice(attributed.as_deref().unwrap_or(&token.bytes));
            }
        }
        bytes.extend(tokens[tail..].iter().flat_map(|t| t.bytes.iter().copied()));
        let check = self.decode(&bytes, limits)?;
        if check.text != text
            || check.consumed != bytes.len()
            || (old.terminated && !check.terminated)
        {
            return Err(malformed(
                "edited text does not re-decode to the requested text",
            ));
        }
        Ok(bytes)
    }

    /// Encode readable cell text canonically. Ambiguous glyphs use definition
    /// order; token encoding retains the original bytes. Append a canonical end
    /// when the definition has one. Validation catches context/alias changes.
    pub fn encode(&self, text: &str, maximum_bytes: usize) -> Result<Vec<u8>> {
        limit("text to encode", text.len(), 262144)?;
        let d = &self.definition;
        let mut bytes = Vec::new();
        let mut page = d.initial_page;
        let mut remaining = text;
        while !remaining.is_empty() {
            let mut best: Option<(usize, Vec<u8>, Option<u8>)> = None;
            let first = remaining
                .chars()
                .next()
                .ok_or_else(|| malformed("empty encoding input"))?;
            for &index in self.by_text_first.get(&first).into_iter().flatten() {
                let e = &self.entries[index];
                for spelling in std::iter::once(&e.text).chain(e.aliases.iter()) {
                    if remaining.starts_with(spelling)
                        && best.as_ref().is_none_or(|b| spelling.len() > b.0)
                    {
                        let mut encoded = Vec::new();
                        if let Some(p) = e.page.filter(|&p| p != page) {
                            let c = d
                                .controls
                                .iter()
                                .find(|c| {
                                    c.set_page == Some(p)
                                        && c.display.is_empty()
                                        && c.arguments(c.code) == 0
                                })
                                .ok_or_else(|| malformed("character page has no switch control"))?;
                            encoded.extend_from_slice(&d.unit.bytes(c.code));
                        }
                        encoded.extend_from_slice(&e.bytes);
                        best = Some((spelling.len(), encoded, e.page));
                    }
                }
            }
            for form in self
                .controls_by_first
                .get(&first)
                .into_iter()
                .flatten()
                .chain(self.controls_without_prefix.iter())
            {
                let c = &d.controls[form.index];
                if let Some((used, args)) = parse_control(c, form.code, &form.template, remaining)?
                {
                    if used > 0 && best.as_ref().is_none_or(|b| used > b.0) {
                        let mut encoded = d.unit.bytes(form.code);
                        encoded.extend_from_slice(&args);
                        best = Some((used, encoded, c.set_page));
                    }
                }
            }
            if let Some((used, encoded, new_page)) = best {
                limit(
                    "encoded text bytes",
                    bytes.len().saturating_add(encoded.len()),
                    maximum_bytes,
                )?;
                bytes.extend_from_slice(&encoded);
                if let Some(p) = new_page {
                    page = p;
                }
                remaining = &remaining[used..];
            } else if d.encoding != TextEncoding::Table {
                let ch = remaining
                    .chars()
                    .next()
                    .ok_or_else(|| malformed("empty encoding input"))?;
                let encoded = encode_standard(d.encoding, &ch.to_string())?;
                limit(
                    "encoded text bytes",
                    bytes.len().saturating_add(encoded.len()),
                    maximum_bytes,
                )?;
                bytes.extend_from_slice(&encoded);
                remaining = &remaining[ch.len_utf8()..];
            } else {
                return Err(malformed(format!(
                    "unencodable text at UTF-8 offset {}",
                    text.len() - remaining.len()
                )));
            }
        }
        let already_ended = self
            .decode(
                &bytes,
                TextLimits {
                    input_bytes: maximum_bytes,
                    ..TextLimits::default()
                },
            )?
            .terminated;
        if !already_ended {
            if let Some(c) = d
                .controls
                .iter()
                .find(|c| c.ends && c.display.is_empty() && c.arguments(c.code) == 0)
            {
                limit(
                    "encoded text bytes",
                    bytes.len().saturating_add(d.unit.width()),
                    maximum_bytes,
                )?;
                bytes.extend_from_slice(&d.unit.bytes(c.code));
            }
        }
        let got = self.decode(
            &bytes,
            TextLimits {
                input_bytes: maximum_bytes,
                ..TextLimits::default()
            },
        )?;
        if got.text != text || got.consumed != bytes.len() {
            return Err(malformed(
                "canonical encoding changes text or contains trailing data",
            ));
        }
        Ok(bytes)
    }
}

fn parse_control(
    c: &ControlCode,
    code: u16,
    template: &str,
    text: &str,
) -> Result<Option<(usize, Vec<u8>)>> {
    let Some((prefix, suffix)) = template.split_once("{args}") else {
        return Ok(text
            .starts_with(template)
            .then(|| (template.len(), vec![0; c.arguments(code)])));
    };
    let Some(mut rest) = text.strip_prefix(prefix) else {
        return Ok(None);
    };
    let mut encoded = Vec::new();
    for _ in 0..c.arguments(code) / c.argument_unit.width() {
        let Some(next) = rest.strip_prefix(&c.argument_separator) else {
            return Ok(None);
        };
        let n = c.argument_unit.width() * 2;
        let Some(hex) = next.get(..n) else {
            return Ok(None);
        };
        let Ok(value) = u16::from_str_radix(hex, 16) else {
            return Ok(None);
        };
        encoded.extend_from_slice(&c.argument_unit.bytes(value));
        rest = &next[n..];
    }
    if !rest.starts_with(suffix) {
        return Ok(None);
    }
    Ok(Some((text.len() - rest.len() + suffix.len(), encoded)))
}

pub fn decode_standard(encoding: TextEncoding, bytes: &[u8]) -> Result<String> {
    limit("standard text input", bytes.len(), 65536)?;
    match encoding {
        TextEncoding::ShiftJis => encoding_rs::SHIFT_JIS
            .decode_without_bom_handling_and_without_replacement(bytes)
            .map(|s| s.into_owned())
            .ok_or_else(|| malformed("invalid Shift_JIS sequence")),
        TextEncoding::JisX0201 => bytes
            .iter()
            .map(|&b| match b {
                0x5c => Ok('¥'),
                0x7e => Ok('‾'),
                0..=0x7f => Ok(char::from(b)),
                0xa1..=0xdf => char::from_u32(0xff61 + u32::from(b - 0xa1))
                    .ok_or_else(|| malformed("invalid kana")),
                _ => Err(malformed("undefined JIS X 0201 byte")),
            })
            .collect(),
        TextEncoding::Table => Err(malformed("table encoding requires a map")),
    }
}
pub fn encode_standard(encoding: TextEncoding, text: &str) -> Result<Vec<u8>> {
    limit("standard text output", text.len(), 262144)?;
    match encoding {
        TextEncoding::ShiftJis => {
            let (bytes, _, errors) = encoding_rs::SHIFT_JIS.encode(text);
            if errors {
                Err(malformed("unencodable Shift_JIS character"))
            } else {
                Ok(bytes.into_owned())
            }
        }
        TextEncoding::JisX0201 => text
            .chars()
            .map(|c| match c {
                '¥' => Ok(0x5c),
                '‾' => Ok(0x7e),
                '\u{0}'..='\u{7f}' if c != '\\' && c != '~' => Ok(c as u8),
                '\u{ff61}'..='\u{ff9f}' => Ok(0xa1 + (c as u32 - 0xff61) as u8),
                _ => Err(malformed("unencodable JIS X 0201 character")),
            })
            .collect(),
        TextEncoding::Table => Err(malformed("table encoding requires a map")),
    }
}
fn kana(c: char) -> bool {
    ('\u{3041}'..='\u{30fa}').contains(&c)
}
fn kanji(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c) || c == '々'
}
fn resolve_dash(text: &str, rules: &Normalization) -> String {
    let Some(dash) = &rules.contextual_dash else {
        return text.into();
    };
    let mut chars: Vec<_> = text.chars().collect();
    for i in 0..chars.len() {
        if chars[i] != dash.placeholder {
            continue;
        }
        let prev = i.checked_sub(1).map_or('\n', |n| chars[n]);
        let next = chars.get(i + 1).copied().unwrap_or('\n');
        chars[i] = if dash.counters.contains(next) {
            '一'
        } else if kana(prev) || prev == 'ー' {
            'ー'
        } else if kanji(next) && (kanji(prev) || ['\n', '「', '　', '（'].contains(&prev)) {
            '一'
        } else if crate::java_categories::java_letter_or_digit_below_3000(prev)
            || ('０'..='９').contains(&prev)
            || ('Ａ'..='ｚ').contains(&prev)
        {
            '－'
        } else {
            'ー'
        };
    }
    chars.into_iter().collect()
}
pub fn normalize(text: &str, rules: &Normalization) -> String {
    let mut text = if rules.compose_kana {
        text.replace('゛', "\u{3099}")
            .replace('゜', "\u{309a}")
            .nfc()
            .collect::<String>()
    } else {
        text.into()
    };
    if rules.long_dash_after_kana {
        let mut chars: Vec<_> = text.chars().collect();
        for i in 1..chars.len() {
            if chars[i] == '－'
                && (('ぁ'..='ゖ').contains(&chars[i - 1])
                    || ('ァ'..='ヺ').contains(&chars[i - 1])
                    || chars[i - 1] == 'ー')
            {
                chars[i] = 'ー';
            }
        }
        text = chars.into_iter().collect();
    }
    text
}
