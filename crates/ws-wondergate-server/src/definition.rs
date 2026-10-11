// SPDX-License-Identifier: MIT OR Apache-2.0
//! Validated, immutable data programs. Editing means validating a new JSON value.
use crate::{
    board::{Order, Ties},
    http, malformed, schema,
};
use formatkit_core::Result;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
include!(concat!(env!("OUT_DIR"), "/definitions.rs"));
pub fn definitions() -> &'static [&'static str] {
    DEFINITIONS
}

#[derive(Clone, Debug)]
pub struct Definition {
    json: Value,
    pub(crate) spec: Spec,
}
impl Definition {
    pub fn from_json(text: &str) -> Result<Self> {
        if text.len() > 262144 {
            return Err(malformed("oversized title definition"));
        }
        let json: Value = serde_json::from_str(text).map_err(|e| malformed(&e.to_string()))?;
        schema::validate(&json)?;
        let spec: Spec =
            serde_json::from_value(json.clone()).map_err(|e| malformed(&e.to_string()))?;
        spec.validate()?;
        Ok(Self { json, spec })
    }
    pub fn id(&self) -> &str {
        &self.spec.id
    }
    pub fn hosts(&self) -> &[String] {
        &self.spec.hosts
    }
    pub fn json(&self) -> &Value {
        &self.json
    }
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.json).map_err(|e| malformed(&e.to_string()))
    }
    pub fn bundled() -> Result<Vec<Self>> {
        definitions()
            .iter()
            .map(|text| Self::from_json(text))
            .collect()
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Spec {
    pub schema: String,
    pub id: String,
    pub provenance: Provenance,
    pub hosts: Vec<String>,
    #[serde(default)]
    pub boards: BTreeMap<String, BoardSpec>,
    #[serde(default)]
    pub values: BTreeMap<String, Phase>,
    #[serde(default)]
    pub records: BTreeMap<String, Vec<Segment>>,
    pub routes: Vec<Route>,
    pub limits: ResponseLimits,
    #[serde(default)]
    pub errors: Errors,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Provenance {
    pub server: String,
    pub notes: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseLimits {
    pub max_body: usize,
    pub max_response: usize,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BoardSpec {
    pub identity: String,
    pub order: SortOrder,
    pub ties: TiePolicy,
    pub course: String,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SortOrder {
    Asc,
    Desc,
}
impl From<SortOrder> for Order {
    fn from(value: SortOrder) -> Self {
        match value {
            SortOrder::Asc => Self::Ascending,
            SortOrder::Desc => Self::Descending,
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TiePolicy {
    FirstSubmission,
    FirstBestSubmission,
}
impl From<TiePolicy> for Ties {
    fn from(value: TiePolicy) -> Self {
        match value {
            TiePolicy::FirstSubmission => Self::FirstSubmission,
            TiePolicy::FirstBestSubmission => Self::FirstBestSubmission,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Phase {
    pub primitive: String,
    pub period_s: u64,
    pub phases: Vec<u8>,
    pub policy: PhasePolicy,
    pub fixed: Option<FixedPhase>,
    #[serde(default)]
    pub epoch_s: i64,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PhasePolicy {
    Fixed,
    Clock,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixedPhase {
    pub phase: u8,
    pub elapsed_s: u64,
}
impl Phase {
    pub fn parts(&self, seconds: i64) -> Result<(u8, u8, u8, u64)> {
        let (mode, elapsed) = match self.policy {
            PhasePolicy::Fixed => {
                let fixed = self
                    .fixed
                    .as_ref()
                    .ok_or_else(|| malformed("missing fixed phase"))?;
                (fixed.phase, fixed.elapsed_s)
            }
            PhasePolicy::Clock => {
                let delta = i128::from(seconds) - i128::from(self.epoch_s);
                let period = i128::from(self.period_s);
                let index = delta
                    .div_euclid(period)
                    .rem_euclid(self.phases.len() as i128) as usize;
                (self.phases[index], delta.rem_euclid(period) as u64)
            }
        };
        Ok((mode, (elapsed / 60) as u8, (elapsed % 60) as u8, elapsed))
    }
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Errors {
    pub host_other_path: Option<u16>,
    pub method: Option<u16>,
    pub invalid: Option<u16>,
    #[serde(default)]
    pub method_first: bool,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Route {
    pub id: String,
    pub method: String,
    #[serde(rename = "match")]
    pub matcher: Match,
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub request: BTreeMap<String, FieldSpec>,
    #[serde(default)]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub tables: BTreeMap<String, TableSpec>,
    #[serde(default)]
    pub table_actions: Vec<TableAction>,
    #[serde(default)]
    pub form_trim_end: Vec<u8>,
    #[serde(default)]
    pub errors: Errors,
    pub respond: Respond,
}
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Source {
    #[default]
    Query,
    Form,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Match {
    pub path: String,
    pub query_exact: Option<String>,
    pub query_keys: Option<Vec<String>>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FieldSpec {
    pub equals: Option<Vec<u8>>,
    #[serde(default)]
    pub optional: bool,
    pub default: Option<Vec<u8>>,
    pub choices: Option<Vec<Vec<u8>>>,
    pub alphabet: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub trim_end: Option<u8>,
    pub max_number: Option<u64>,
    pub last_bytes: Option<Vec<u8>>,
    #[serde(default)]
    pub decode: Decode,
    pub bytes: Option<[usize; 2]>,
    #[serde(default)]
    pub deny: Vec<u8>,
    pub min_byte: Option<u8>,
    #[serde(default)]
    pub charset: Charset,
    pub regex: Option<String>,
    #[serde(rename = "as")]
    pub number: Option<Number>,
    #[serde(default)]
    pub repeats: Repeats,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TableSpec {
    pub delimiter: u8,
    pub tag_decimal_width: Option<[usize; 2]>,
    #[serde(default)]
    pub optional: bool,
    pub rows: BTreeMap<String, Vec<FieldSpec>>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TableAction {
    pub field: String,
    pub identity: String,
    pub name: String,
    pub score: usize,
    pub extra: Option<usize>,
    pub boards: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Condition {
    #[serde(default)]
    pub equals: BTreeMap<String, Vec<u8>>,
    #[serde(default)]
    pub absent: Vec<String>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Decode {
    #[default]
    Raw,
    PercentPlus,
}
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Charset {
    #[default]
    Raw,
    Ascii,
    ShiftJis,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Number {
    U32,
    U64,
}
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Repeats {
    #[default]
    One,
    First,
    Last,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(crate) enum Action {
    Record { record: RecordAction },
    Increment { increment: RecordAction },
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordAction {
    pub board: String,
    pub identity: String,
    pub name: String,
    pub score: String,
}
impl Action {
    pub fn record(&self) -> &RecordAction {
        match self {
            Self::Record { record } => record,
            Self::Increment { increment } => increment,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Respond {
    pub status: u16,
    pub content_type: Option<String>,
    #[serde(default)]
    pub headers: Vec<[String; 2]>,
    pub expect_lines: Option<usize>,
    pub body: Body,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(crate) enum Body {
    Segments(Vec<Segment>),
    Wrapped {
        envelope: Envelope,
        segments: Vec<Segment>,
    },
}
impl Body {
    pub fn segments(&self) -> &[Segment] {
        match self {
            Self::Segments(s) | Self::Wrapped { segments: s, .. } => s,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope {
    pub primitive: String,
    pub marker: String,
    pub case: HexCase,
    pub terminator: Vec<u8>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum HexCase {
    Lower,
    Upper,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(crate) enum Segment {
    Date {
        date: usize,
    },
    Board {
        board: String,
        metric: Metric,
        identity: Option<String>,
        limit: Option<usize>,
        pad_to: Option<usize>,
        fmt: Format,
        width: Option<usize>,
        max: Option<u64>,
        #[serde(default)]
        overflow_zero: bool,
        #[serde(default)]
        zero_pad: bool,
    },
    When {
        when: Condition,
        segments: Vec<Segment>,
    },
    Text {
        text: String,
    },
    Bytes {
        bytes: Vec<u8>,
    },
    Zero {
        zero: usize,
    },
    Value {
        value: String,
        #[serde(rename = "as")]
        format: Format,
    },
    Field {
        field: String,
        fmt: Format,
        width: Option<usize>,
        min: Option<u64>,
        max: Option<u64>,
        empty: Option<u64>,
        empty_bytes: Option<Vec<u8>>,
        divide: Option<u64>,
        #[serde(default)]
        zero_pad: bool,
    },
    Each {
        each: Each,
        record: String,
    },
    Row {
        row: Row,
        record: String,
    },
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Metric {
    Count,
    Position,
    Length,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Format {
    U8,
    Dec,
    HexLower,
    HexUpper,
    Raw,
    HtmlShiftJis,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Each {
    pub leaders: Leaders,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Leaders {
    pub board: String,
    pub limit: usize,
    pub pad_to: usize,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(crate) enum Row {
    Submitter { submitter: String },
    Place { place: Place },
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Place {
    pub board: String,
    pub n: usize,
}
pub(crate) fn identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
/// The v1 regex grammar deliberately permits only anchored decimal widths.
pub(crate) fn decimal_width(regex: &str) -> Result<(usize, usize)> {
    let inner = regex
        .strip_prefix("^[0-9]{")
        .and_then(|s| s.strip_suffix("}$"))
        .ok_or_else(|| malformed("unsupported field regex"))?;
    let (lo, hi) = inner.split_once(',').unwrap_or((inner, inner));
    let lo = lo
        .parse::<usize>()
        .map_err(|_| malformed("invalid decimal width"))?;
    let hi = hi
        .parse::<usize>()
        .map_err(|_| malformed("invalid decimal width"))?;
    if lo == 0 || hi < lo || hi > 20 {
        return Err(malformed("invalid decimal width"));
    }
    Ok((lo, hi))
}
impl Spec {
    fn validate(&self) -> Result<()> {
        if self.schema != "swankit-wondergate-title/1"
            || self.provenance.server != "new"
            || self.provenance.notes.trim().is_empty()
            || !identifier(&self.id)
        {
            return Err(malformed("invalid title identity or provenance"));
        }
        let mut hosts = BTreeSet::new();
        for host in &self.hosts {
            if http::normalize_host(host)? != *host || !hosts.insert(host) {
                return Err(malformed("invalid or duplicate title host"));
            }
        }
        for (id, board) in &self.boards {
            if !identifier(id) || !field_key(&board.identity) || !identifier(&board.course) {
                return Err(malformed("invalid board declaration"));
            }
        }
        for (id, phase) in &self.values {
            if !identifier(id)
                || phase.primitive != "cyclic-phase"
                || phase.phases.is_empty()
                || phase.period_s == 0
                || phase.period_s > 15360
            {
                return Err(malformed("invalid cyclic phase"));
            }
            match phase.policy {
                PhasePolicy::Fixed => {
                    let f = phase
                        .fixed
                        .as_ref()
                        .ok_or_else(|| malformed("missing fixed phase"))?;
                    if !phase.phases.contains(&f.phase) || f.elapsed_s >= phase.period_s {
                        return Err(malformed("fixed phase outside cycle"));
                    }
                }
                PhasePolicy::Clock if phase.fixed.is_some() => {
                    return Err(malformed("clock phase cannot have fixed state"))
                }
                PhasePolicy::Clock => {}
            }
        }
        for (id, record) in &self.records {
            if !identifier(id) {
                return Err(malformed("invalid record id"));
            }
            self.segments(record, None, true, 0)?;
        }
        let mut ids = BTreeSet::new();
        self.errors.validate()?;
        for (i, route) in self.routes.iter().enumerate() {
            if !identifier(&route.id)
                || !ids.insert(&route.id)
                || !matches!(route.method.as_str(), "GET" | "POST")
                || !route.matcher.path.starts_with('/')
                || !route.matcher.path.is_ascii()
                || route.matcher.path.contains(['?', '#', '\r', '\n', ' '])
            {
                return Err(malformed("invalid route"));
            }
            if route.matcher.query_exact.is_some() && route.matcher.query_keys.is_some() {
                return Err(malformed("conflicting query selectors"));
            }
            if matches!(route.source, Source::Form) && route.method != "POST" {
                return Err(malformed("form fields require POST"));
            }
            route.errors.validate()?;
            for (name, field) in &route.request {
                if !field_key(name) || field.bytes.is_some_and(|[lo, hi]| lo > hi || hi > 512) {
                    return Err(malformed("invalid request field bounds"));
                }
                validate_field(field)?;
            }
            for (key, table) in &route.tables {
                if !field_key(key)
                    || route.request.contains_key(key)
                    || table.rows.is_empty()
                    || table.rows.len() > 128
                    || table.delimiter == 0
                    || table
                        .tag_decimal_width
                        .is_some_and(|[lo, hi]| lo == 0 || lo > hi || hi > 20)
                {
                    return Err(malformed("invalid table field"));
                }
                for (tag, columns) in &table.rows {
                    if tag.is_empty()
                        || tag.len() > 32
                        || !tag.is_ascii()
                        || tag.as_bytes().contains(&table.delimiter)
                        || columns.is_empty()
                        || columns.len() > 8
                    {
                        return Err(malformed("invalid table row grammar"));
                    }
                    if table.tag_decimal_width.is_some()
                        && (tag.parse::<u64>().is_err()
                            || tag.parse::<u64>().is_ok_and(|n| n.to_string() != *tag))
                    {
                        return Err(malformed("noncanonical decimal row tag"));
                    }
                    for column in columns {
                        validate_field(column)?;
                    }
                }
            }
            for action in &route.table_actions {
                let table = route
                    .tables
                    .get(&action.field)
                    .ok_or_else(|| malformed("unknown action table"))?;
                if action.boards.is_empty()
                    || action
                        .boards
                        .keys()
                        .any(|tag| !table.rows.contains_key(tag))
                    || !route.request.contains_key(&action.identity)
                    || !route.request.contains_key(&action.name)
                    || [&action.identity, &action.name].iter().any(|key| {
                        route
                            .request
                            .get(*key)
                            .is_some_and(|f| f.optional && f.default.is_none())
                    })
                {
                    return Err(malformed("invalid table action fields"));
                }
                for (tag, board) in &action.boards {
                    let board = self
                        .boards
                        .get(board)
                        .ok_or_else(|| malformed("unknown table action board"))?;
                    let columns = &table.rows[tag];
                    if board.identity != action.identity
                        || columns.get(action.score).is_none_or(|c| c.number.is_none())
                        || action.extra.is_some_and(|i| {
                            columns.get(i).is_none_or(|c| {
                                c.number.is_none()
                                    || c.max_number.is_none_or(|n| n > i64::MAX as u64)
                            })
                        })
                    {
                        return Err(malformed("invalid numeric table action"));
                    }
                }
            }
            if let Some(keys) = &route.matcher.query_keys {
                let declared: BTreeSet<_> = route.request.keys().collect();
                let selected: BTreeSet<_> = keys.iter().collect();
                if selected != declared {
                    return Err(malformed("query keys must match request fields"));
                }
            }
            let mut recorded = BTreeSet::new();
            for action in &route.actions {
                let record = action.record();
                let board = self
                    .boards
                    .get(&record.board)
                    .ok_or_else(|| malformed("unknown action board"))?;
                if !recorded.insert(&record.board) || board.identity != record.identity {
                    return Err(malformed("ambiguous board action identity"));
                }
                for field in [&record.identity, &record.name, &record.score] {
                    if !route.request.contains_key(field) {
                        return Err(malformed("unknown action field"));
                    }
                    if route.request[field].optional && route.request[field].default.is_none() {
                        return Err(malformed("action field may be absent"));
                    }
                }
                if route.request[&record.score].number.is_none() {
                    return Err(malformed("score field must be numeric"));
                }
            }
            route.respond.validate()?;
            if let Body::Wrapped { envelope, .. } = &route.respond.body {
                if envelope.primitive != "sum16-hex-envelope"
                    || envelope.marker.is_empty()
                    || !envelope.marker.is_ascii()
                {
                    return Err(malformed("unknown or invalid envelope"));
                }
            }
            self.segments(route.respond.body.segments(), Some(route), false, 0)?;
            let mut minimum = self.minimum_size(route.respond.body.segments())?;
            if let Body::Wrapped { envelope, .. } = &route.respond.body {
                minimum = minimum
                    .checked_add(2 * (envelope.marker.len() + 4) + envelope.terminator.len())
                    .ok_or_else(|| malformed("template size overflow"))?;
            }
            let headers: Vec<_> = route
                .respond
                .headers
                .iter()
                .map(|p| (p[0].as_str(), p[1].as_str()))
                .chain(
                    route
                        .respond
                        .content_type
                        .as_deref()
                        .map(|v| ("Content-Type", v)),
                )
                .collect();
            let header_size = http::response(route.respond.status, &headers, &[])?.len();
            if minimum > self.limits.max_body
                || header_size + minimum + minimum.to_string().len() - 1 > self.limits.max_response
            {
                return Err(malformed("template minimum exceeds client response limits"));
            }
            for other in &self.routes[..i] {
                if overlap(route, other)? {
                    return Err(malformed("overlapping title routes"));
                }
            }
        }
        Ok(())
    }
    fn minimum_size(&self, segments: &[Segment]) -> Result<usize> {
        let mut total = 0usize;
        for segment in segments {
            let size = match segment {
                Segment::Date { date } => *date,
                Segment::Board { width, .. } => width.unwrap_or(1).max(1),
                Segment::When { .. } => 0,
                Segment::Text { text } => text.len(),
                Segment::Bytes { bytes } => bytes.len(),
                Segment::Zero { zero } => *zero,
                Segment::Value { .. } => 1,
                Segment::Field {
                    fmt: Format::Raw | Format::HtmlShiftJis,
                    ..
                } => 0,
                Segment::Field { width, .. } => width.unwrap_or(1).max(1),
                Segment::Each { each, record } => self
                    .minimum_size(&self.records[record])?
                    .checked_mul(each.leaders.pad_to)
                    .ok_or_else(|| malformed("template size overflow"))?,
                Segment::Row { record, .. } => self.minimum_size(&self.records[record])?,
            };
            total = total
                .checked_add(size)
                .ok_or_else(|| malformed("template size overflow"))?;
        }
        Ok(total)
    }
    fn segments(
        &self,
        segments: &[Segment],
        route: Option<&Route>,
        in_record: bool,
        depth: usize,
    ) -> Result<()> {
        if depth > 4 || segments.len() > 1024 {
            return Err(malformed("template nesting or segment limit"));
        }
        for segment in segments {
            match segment {
                Segment::Date { date } if !matches!(date, 8 | 14) => {
                    return Err(malformed("invalid date width"))
                }
                Segment::Board {
                    board,
                    metric,
                    identity,
                    limit,
                    pad_to,
                    fmt,
                    width,
                    zero_pad,
                    ..
                } => {
                    if in_record
                        || !self.boards.contains_key(board)
                        || matches!(fmt, Format::Raw | Format::HtmlShiftJis)
                        || width.is_some_and(|n| n > 512)
                        || (matches!(fmt, Format::U8) && (width.is_some() || *zero_pad))
                        || limit.is_some_and(|n| n > 4096)
                        || pad_to.is_some_and(|n| n > 4096)
                    {
                        return Err(malformed("invalid board metric"));
                    }
                    match metric {
                        Metric::Position
                            if identity.as_ref().is_none_or(|key| {
                                !route.is_some_and(|r| r.request.contains_key(key))
                            }) =>
                        {
                            return Err(malformed("position requires identity field"))
                        }
                        Metric::Length if limit.is_none() || pad_to.is_none() => {
                            return Err(malformed("length requires selection bounds"))
                        }
                        _ => {}
                    }
                }
                Segment::When { when, segments } => {
                    if in_record
                        || when.equals.len() + when.absent.len() == 0
                        || !route.is_some_and(|r| {
                            when.equals
                                .keys()
                                .chain(when.absent.iter())
                                .all(|k| r.request.contains_key(k))
                        })
                    {
                        return Err(malformed("invalid template condition"));
                    }
                    self.segments(segments, route, false, depth + 1)?;
                }
                Segment::Text { text } if text.len() > self.limits.max_body => {
                    return Err(malformed("literal exceeds client body limit"))
                }
                Segment::Bytes { bytes } if bytes.len() > self.limits.max_body => {
                    return Err(malformed("literal exceeds client body limit"))
                }
                Segment::Zero { zero } if *zero > self.limits.max_body => {
                    return Err(malformed("zero run exceeds client body limit"))
                }
                Segment::Field {
                    field,
                    fmt,
                    width,
                    min,
                    max,
                    empty,
                    empty_bytes,
                    divide,
                    zero_pad,
                } => {
                    if !in_record
                        || !matches!(
                            field.as_str(),
                            "rank" | "name" | "identity" | "score" | "extra"
                        )
                        || width.is_some_and(|v| v > 512)
                        || matches!((min, max), (Some(a), Some(b)) if a > b)
                    {
                        return Err(malformed("invalid row field format"));
                    }
                    let bytes = matches!(field.as_str(), "name" | "identity");
                    if bytes != matches!(fmt, Format::Raw | Format::HtmlShiftJis) {
                        return Err(malformed("row field format type mismatch"));
                    }
                    if (bytes
                        && (min.is_some()
                            || max.is_some()
                            || empty.is_some()
                            || divide.is_some()
                            || *zero_pad))
                        || (!bytes && empty_bytes.is_some())
                        || empty_bytes.as_ref().is_some_and(|b| b.len() > 512)
                        || *divide == Some(0)
                        || (matches!(fmt, Format::U8)
                            && (width.is_some_and(|v| v != 0) || *zero_pad))
                    {
                        return Err(malformed("inapplicable row formatting options"));
                    }
                }
                Segment::Value { value, format } => {
                    if matches!(format, Format::Raw | Format::HtmlShiftJis) {
                        return Err(malformed("numeric value requires numeric format"));
                    }
                    let (id, field) = value
                        .split_once('.')
                        .ok_or_else(|| malformed("invalid value reference"))?;
                    if !self.values.contains_key(id)
                        || !matches!(field, "mode" | "minute" | "second" | "elapsed_s")
                    {
                        return Err(malformed("unknown value reference"));
                    }
                }
                Segment::Each { each, record } => {
                    if in_record
                        || !self.boards.contains_key(&each.leaders.board)
                        || !self.records.contains_key(record)
                        || each.leaders.limit > 4096
                        || each.leaders.pad_to > 4096
                    {
                        return Err(malformed("invalid bounded leaders template"));
                    }
                }
                Segment::Row { row, record } => {
                    if in_record || !self.records.contains_key(record) {
                        return Err(malformed("invalid row template"));
                    }
                    let board = match row {
                        Row::Submitter { submitter } => {
                            if !route.is_some_and(|r| {
                                r.actions.iter().any(|a| &a.record().board == submitter)
                            }) {
                                return Err(malformed("submitter row requires action"));
                            }
                            submitter
                        }
                        Row::Place { place } => {
                            if !(1..=4096).contains(&place.n) {
                                return Err(malformed("invalid board place"));
                            }
                            &place.board
                        }
                    };
                    if !self.boards.contains_key(board) {
                        return Err(malformed("unknown row board"));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}
impl Errors {
    fn validate(&self) -> Result<()> {
        for status in [self.host_other_path, self.method, self.invalid]
            .into_iter()
            .flatten()
        {
            if !matches!(status, 400 | 404 | 405) {
                return Err(malformed("unsupported route error status"));
            }
        }
        Ok(())
    }
}
impl Respond {
    fn validate(&self) -> Result<()> {
        let mut headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|p| (p[0].as_str(), p[1].as_str()))
            .collect();
        if let Some(content_type) = &self.content_type {
            headers.insert(0, ("Content-Type", content_type));
        }
        let mut seen = BTreeSet::new();
        for (key, _) in &headers {
            if key.is_empty()
                || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                || !seen.insert(key.to_ascii_lowercase())
                || matches!(
                    key.to_ascii_lowercase().as_str(),
                    "content-length" | "connection"
                )
            {
                return Err(malformed("invalid or reserved response header"));
            }
        }
        let response = http::response(self.status, &headers, &[])?;
        let lines = response.windows(2).filter(|p| *p == b"\r\n").count();
        if self.expect_lines.is_some_and(|expected| expected != lines) {
            return Err(malformed(
                "response expect_lines does not match header shape",
            ));
        }
        Ok(())
    }
}
pub(crate) fn overlap(a: &Route, b: &Route) -> Result<bool> {
    if a.matcher.path != b.matcher.path || a.method != b.method {
        return Ok(false);
    }
    // Selectors over different sources can both match the same request.
    if std::mem::discriminant(&a.source) != std::mem::discriminant(&b.source) {
        return Ok(true);
    }
    match (
        &a.matcher.query_exact,
        &b.matcher.query_exact,
        &a.matcher.query_keys,
        &b.matcher.query_keys,
    ) {
        (Some(a), Some(b), _, _) => Ok(a == b),
        (_, _, Some(a), Some(b)) => Ok(a.iter().collect::<BTreeSet<_>>() == b.iter().collect()),
        (Some(exact), _, _, Some(keys)) | (_, Some(exact), Some(keys), _) => {
            let fields = crate::form::fields(exact.as_bytes())?;
            Ok(fields.keys().collect::<BTreeSet<_>>() == keys.iter().collect())
        }
        _ => Ok(true),
    }
}

pub(crate) fn field_key(key: &str) -> bool {
    key == "*" || identifier(key)
}
fn validate_field(field: &FieldSpec) -> Result<()> {
    if field.bytes.is_some_and(|[lo, hi]| lo > hi || hi > 512)
        || field.default.as_ref().is_some_and(|b| b.len() > 512)
        || field.choices.as_ref().is_some_and(|choices| {
            choices.is_empty() || choices.len() > 128 || choices.iter().any(|b| b.len() > 512)
        })
        || field
            .alphabet
            .as_ref()
            .is_some_and(|a| a.is_empty() || !a.is_ascii())
    {
        return Err(malformed("invalid field grammar"));
    }
    if let Some(regex) = &field.regex {
        decimal_width(regex)?;
    }
    if field.max_number.is_some() && field.number.is_none() {
        return Err(malformed("numeric maximum needs numeric field"));
    }
    Ok(())
}
