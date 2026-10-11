// SPDX-License-Identifier: MIT
// Ported response semantics from the private revival server's title handlers.
// Copyright (C) 2026 jidoughost. The data interpreter is newly authored.
//! Bounded templates, checked actions and scoped board rendering, with no I/O.
use crate::{
    board::Board,
    definition::*,
    form::{self, Field},
    http::{self, Request},
    malformed,
    store::{Scope, Store, Submission},
    Clock, Timestamp,
};
use formatkit_core::{Error, Result};
use std::collections::BTreeMap;

fn store_error(error: std::io::Error) -> Error {
    Error::Unsupported(format!("store: {error}"))
}
pub(crate) fn handle(
    definition: &Definition,
    request: &Request,
    store: &dyn Store,
    clock: &dyn Clock,
) -> Result<Option<Vec<u8>>> {
    if !definition.hosts().contains(&request.host) {
        return Ok(None);
    }
    let spec = &definition.spec;
    if spec.errors.method_first && spec.routes.iter().all(|r| r.method != request.method) {
        return Ok(Some(http::empty_response(
            spec.errors.method.unwrap_or(405),
        )));
    }
    let paths: Vec<_> = spec
        .routes
        .iter()
        .filter(|r| r.matcher.path == request.path)
        .collect();
    if paths.is_empty() {
        return Ok(Some(http::empty_response(
            spec.errors.host_other_path.unwrap_or(404),
        )));
    }
    if paths.iter().all(|r| r.method != request.method)
        && paths.iter().any(|r| r.errors.method_first)
    {
        return Ok(Some(http::empty_response(
            paths[0].errors.method.or(spec.errors.method).unwrap_or(405),
        )));
    }
    let mut selected = None;
    let mut parsed = None;
    for route in &paths {
        let mut candidate_fields = None;
        let matches = if let Some(exact) = &route.matcher.query_exact {
            exact == &request.query
        } else if let Some(keys) = &route.matcher.query_keys {
            let fields = match route.source {
                Source::Query => form::fields(&form::latin1(&request.query)?),
                Source::Form => form::fields(&request.body),
            };
            match fields {
                Ok(fields)
                    if fields.len() == keys.len()
                        && keys.iter().all(|k| fields.contains_key(k)) =>
                {
                    candidate_fields = Some(fields);
                    true
                }
                _ => false,
            }
        } else {
            true
        };
        if matches && (route.method == request.method || selected.is_none()) {
            selected = Some(*route);
            parsed = candidate_fields;
        }
        if matches && route.method == request.method {
            break;
        }
    }
    let Some(route) = selected else {
        let fallback = paths[0];
        let status = if paths.iter().all(|r| r.matcher.query_exact.is_none())
            && paths.iter().all(|r| r.method != request.method)
        {
            fallback.errors.method.or(spec.errors.method).unwrap_or(405)
        } else {
            fallback
                .errors
                .invalid
                .or(spec.errors.invalid)
                .unwrap_or(400)
        };
        return Ok(Some(http::empty_response(status)));
    };
    if route.method != request.method {
        return Ok(Some(http::empty_response(
            route.errors.method.or(spec.errors.method).unwrap_or(405),
        )));
    }
    let invalid = route.errors.invalid.or(spec.errors.invalid).unwrap_or(400);
    let fields = (|| {
        if matches!(route.source, Source::Form)
            && !request.headers.get("content-type").is_some_and(|v| {
                v.split(';').next().is_some_and(|v| {
                    v.trim()
                        .eq_ignore_ascii_case("application/x-www-form-urlencoded")
                })
            })
        {
            return Err(malformed("form content type required"));
        }
        if route.request.is_empty() && route.tables.is_empty() {
            return Ok((BTreeMap::new(), BTreeMap::new()));
        }
        let mut fields = if let Some(fields) = parsed {
            fields
        } else {
            match route.source {
                Source::Query => form::fields(&form::latin1(&request.query)?)?,
                Source::Form => form::fields(trim_form(&request.body, &route.form_trim_end))?,
            }
        };
        let tables = form::tables(&mut fields, &route.tables)?;
        Ok((form::validate(&fields, &route.request)?, tables))
    })();
    let (fields, tables) = match fields {
        Ok(fields) => fields,
        Err(_) => return Ok(Some(http::empty_response(invalid))),
    };
    let mut cx = Context {
        spec,
        store,
        now: clock.now_utc(),
        fields,
        submitted: BTreeMap::new(),
    };
    for action in &route.actions {
        let record = action.record();
        let identity = cx.field(&record.identity)?.bytes.clone();
        let name = cx.field(&record.name)?.bytes.clone();
        let mut score = cx
            .field(&record.score)?
            .number
            .ok_or_else(|| malformed("score field is not numeric"))?;
        if matches!(action, Action::Increment { .. }) {
            let previous = cx
                .board(&record.board)?
                .best(&identity)
                .map_err(store_error)?
                .into_iter()
                .next()
                .map_or(0, |r| r.submission.score);
            score = previous
                .checked_add(score)
                .ok_or_else(|| malformed("increment overflow"))?;
        }
        let submission = Submission {
            scope: cx.scope(&record.board)?,
            identity,
            name,
            score,
            extra: 0,
            submitted_at: cx.now,
        };
        store.submit(submission.clone()).map_err(store_error)?;
        cx.submitted.insert(record.board.clone(), submission);
    }
    for action in &route.table_actions {
        for (tag, columns) in &tables[&action.field] {
            let Some(board) = action.boards.get(tag) else {
                continue;
            };
            let score = columns[action.score]
                .number
                .ok_or_else(|| malformed("non-numeric table score"))?;
            let extra = action
                .extra
                .map(|i| {
                    columns[i]
                        .number
                        .ok_or_else(|| malformed("non-numeric table extra"))
                })
                .transpose()?
                .unwrap_or(0);
            let submission = Submission {
                scope: cx.scope(board)?,
                identity: cx.field(&action.identity)?.bytes.clone(),
                name: cx.field(&action.name)?.bytes.clone(),
                score,
                extra: i64::try_from(extra).map_err(|_| malformed("table extra overflow"))?,
                submitted_at: cx.now,
            };
            store.submit(submission.clone()).map_err(store_error)?;
            cx.submitted.insert(board.clone(), submission);
        }
    }
    // Serialization/configuration failures are server failures, never client input.
    let reply = cx
        .respond(&route.respond)
        .map_err(|e| Error::Unsupported(format!("response: {e}")))?;
    Ok(Some(reply))
}
struct Context<'a> {
    spec: &'a Spec,
    store: &'a dyn Store,
    now: Timestamp,
    fields: BTreeMap<String, Field>,
    submitted: BTreeMap<String, Submission>,
}
struct RowData {
    rank: u64,
    name: Vec<u8>,
    identity: Vec<u8>,
    score: u64,
    extra: i64,
}
impl RowData {
    fn empty() -> Self {
        Self {
            rank: 0,
            name: vec![],
            identity: vec![],
            score: 0,
            extra: 0,
        }
    }
    fn new(rank: usize, submission: &Submission) -> Self {
        Self {
            rank: rank as u64,
            name: submission.name.clone(),
            identity: submission.identity.clone(),
            score: submission.score,
            extra: submission.extra,
        }
    }
}
impl Context<'_> {
    fn field(&self, name: &str) -> Result<&Field> {
        self.fields
            .get(name)
            .ok_or_else(|| malformed("unknown field"))
    }
    fn scope(&self, id: &str) -> Result<Scope> {
        let board = self
            .spec
            .boards
            .get(id)
            .ok_or_else(|| malformed("unknown board"))?;
        Ok(Scope::new(&self.spec.id, id, &board.course))
    }
    fn board(&self, id: &str) -> Result<Board<'_>> {
        let board = self
            .spec
            .boards
            .get(id)
            .ok_or_else(|| malformed("unknown board"))?;
        Ok(
            Board::new(self.store, self.scope(id)?, board.order.into())
                .with_ties(board.ties.into()),
        )
    }
    fn respond(&self, response: &Respond) -> Result<Vec<u8>> {
        let mut body = Vec::new();
        self.render(response.body.segments(), None, &mut body)?;
        if let Body::Wrapped { envelope, .. } = &response.body {
            body = sum16_envelope(
                &body,
                &envelope.marker,
                matches!(envelope.case, HexCase::Upper),
                &envelope.terminator,
                self.spec.limits.max_body,
            )?;
        }
        let mut headers: Vec<_> = response
            .headers
            .iter()
            .map(|p| (p[0].as_str(), p[1].as_str()))
            .collect();
        if let Some(content_type) = &response.content_type {
            headers.insert(0, ("Content-Type", content_type));
        }
        let reply = http::response(response.status, &headers, &body)?;
        if reply.len() > self.spec.limits.max_response {
            return Err(malformed("response exceeds client buffer"));
        }
        Ok(reply)
    }
    fn append(&self, output: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
        if bytes.len() > self.spec.limits.max_body.saturating_sub(output.len()) {
            return Err(malformed("template exceeds client body buffer"));
        }
        output.extend_from_slice(bytes);
        Ok(())
    }
    fn render(&self, segments: &[Segment], row: Option<&RowData>, out: &mut Vec<u8>) -> Result<()> {
        for segment in segments {
            match segment {
                Segment::Date { date } => {
                    self.append(out, &self.now.utc_digits().as_bytes()[..*date])?
                }
                Segment::Board {
                    board,
                    metric,
                    identity,
                    limit,
                    pad_to,
                    fmt,
                    width,
                    max,
                    overflow_zero,
                    zero_pad,
                } => {
                    let board = self.board(board)?;
                    let mut value = match metric {
                        Metric::Count => board.count().map_err(store_error)?,
                        Metric::Position => board
                            .position(
                                &self
                                    .field(
                                        identity
                                            .as_deref()
                                            .ok_or_else(|| malformed("missing metric identity"))?,
                                    )?
                                    .bytes,
                            )
                            .map_err(store_error)?,
                        Metric::Length => board
                            .leaders(limit.unwrap_or(0))
                            .map_err(store_error)?
                            .len()
                            .max(pad_to.unwrap_or(0)),
                    } as u64;
                    if let Some(max) = max {
                        if value > *max {
                            value = if *overflow_zero { 0 } else { *max };
                        }
                    }
                    self.append(out, &numeric(value, *fmt, *width, *zero_pad)?)?;
                }
                Segment::When { when, segments } => {
                    if when
                        .equals
                        .iter()
                        .all(|(k, v)| self.fields.get(k).is_some_and(|f| &f.bytes == v))
                        && when.absent.iter().all(|k| !self.fields.contains_key(k))
                    {
                        self.render(segments, row, out)?;
                    }
                }
                Segment::Text { text } => self.append(out, text.as_bytes())?,
                Segment::Bytes { bytes } => self.append(out, bytes)?,
                Segment::Zero { zero } => {
                    if *zero > self.spec.limits.max_body.saturating_sub(out.len()) {
                        return Err(malformed("zero run exceeds client body buffer"));
                    }
                    out.resize(out.len() + zero, 0);
                }
                Segment::Value { value, format } => {
                    let (id, field) = value
                        .split_once('.')
                        .ok_or_else(|| malformed("invalid value"))?;
                    let phase = self
                        .spec
                        .values
                        .get(id)
                        .ok_or_else(|| malformed("unknown value"))?;
                    let (mode, minute, second, elapsed) = phase.parts(self.now.into())?;
                    let number = match field {
                        "mode" => u64::from(mode),
                        "minute" => u64::from(minute),
                        "second" => u64::from(second),
                        "elapsed_s" => elapsed,
                        _ => return Err(malformed("unknown phase field")),
                    };
                    self.append(out, &numeric(number, *format, None, false)?)?;
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
                    let row = row.ok_or_else(|| malformed("field outside row"))?;
                    let bytes = if matches!(field.as_str(), "name" | "identity") {
                        let bytes = if field == "name" {
                            &row.name
                        } else {
                            &row.identity
                        };
                        let bytes = if bytes.is_empty() {
                            empty_bytes.as_ref().unwrap_or(bytes)
                        } else {
                            bytes
                        };
                        let mut padded = bytes.clone();
                        if let Some(width) = width {
                            padded.resize(padded.len().max(*width), b' ');
                        }
                        match fmt {
                            Format::Raw => padded,
                            Format::HtmlShiftJis => html_shift_jis(bytes)?.into_bytes(),
                            _ => return Err(malformed("invalid byte field format")),
                        }
                    } else {
                        let mut n = match field.as_str() {
                            "rank" => row.rank,
                            "score" => row.score,
                            "extra" => u64::try_from(row.extra)
                                .map_err(|_| malformed("negative extra in unsigned format"))?,
                            _ => return Err(malformed("unknown row field")),
                        };
                        if row.rank == 0 {
                            n = empty.unwrap_or(n);
                        }
                        n /= divide.unwrap_or(1);
                        if let Some(min) = min {
                            n = n.max(*min);
                        }
                        if let Some(max) = max {
                            n = n.min(*max);
                        }
                        numeric(n, *fmt, *width, *zero_pad)?
                    };
                    self.append(out, &bytes)?;
                }
                Segment::Each { each, record } => {
                    let rows = self
                        .board(&each.leaders.board)?
                        .leaders(each.leaders.limit)
                        .map_err(store_error)?;
                    for index in 0..rows.len().max(each.leaders.pad_to) {
                        let row = rows.get(index).map_or_else(RowData::empty, |r| {
                            RowData::new(index + 1, &r.submission)
                        });
                        self.render(&self.spec.records[record], Some(&row), out)?;
                    }
                }
                Segment::Row { row, record } => {
                    let row = match row {
                        Row::Submitter { submitter } => {
                            let submitted = self
                                .submitted
                                .get(submitter)
                                .ok_or_else(|| malformed("missing submitter"))?;
                            let rank = self
                                .board(submitter)?
                                .position(&submitted.identity)
                                .map_err(store_error)?;
                            RowData::new(rank, submitted)
                        }
                        Row::Place { place } => {
                            let rows = self
                                .board(&place.board)?
                                .leaders(place.n)
                                .map_err(store_error)?;
                            rows.get(place.n - 1).map_or_else(RowData::empty, |r| {
                                RowData::new(place.n, &r.submission)
                            })
                        }
                    };
                    self.render(&self.spec.records[record], Some(&row), out)?;
                }
            }
        }
        Ok(())
    }
}
fn numeric(value: u64, format: Format, width: Option<usize>, zero_pad: bool) -> Result<Vec<u8>> {
    let width = width.unwrap_or(0);
    if width > 512 {
        return Err(malformed("numeric format width limit"));
    }
    let text = match format {
        Format::U8 if width == 0 => {
            return Ok(vec![
                u8::try_from(value).map_err(|_| malformed("u8 value overflow"))?
            ])
        }
        Format::Dec => {
            if zero_pad {
                format!("{value:0width$}")
            } else {
                format!("{value:width$}")
            }
        }
        Format::HexLower => {
            if zero_pad {
                format!("{value:0width$x}")
            } else {
                format!("{value:width$x}")
            }
        }
        Format::HexUpper => {
            if zero_pad {
                format!("{value:0width$X}")
            } else {
                format!("{value:width$X}")
            }
        }
        _ => return Err(malformed("invalid numeric format")),
    };
    Ok(text.into_bytes())
}
pub fn sum16_envelope(
    payload: &[u8],
    marker: &str,
    upper: bool,
    terminator: &[u8],
    maximum: usize,
) -> Result<Vec<u8>> {
    let length = marker
        .len()
        .checked_add(4)
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_add(payload.len()))
        .and_then(|n| n.checked_add(terminator.len()))
        .ok_or_else(|| malformed("envelope size overflow"))?;
    if maximum > 65536 || length > maximum || marker.is_empty() || !marker.is_ascii() {
        return Err(malformed("envelope exceeds bound or invalid marker"));
    }
    let sum = payload
        .iter()
        .fold(0u16, |sum, byte| sum.wrapping_add(u16::from(*byte)));
    let checksum = if upper {
        format!("{sum:04X}")
    } else {
        format!("{sum:04x}")
    };
    let mut output = Vec::with_capacity(length);
    output.extend_from_slice(marker.as_bytes());
    output.extend_from_slice(checksum.as_bytes());
    output.extend_from_slice(payload);
    output.extend_from_slice(marker.as_bytes());
    output.extend_from_slice(checksum.as_bytes());
    output.extend_from_slice(terminator);
    Ok(output)
}
fn html_shift_jis(bytes: &[u8]) -> Result<String> {
    Ok(crate::form::shift_jis(bytes, true)?
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;"))
}

fn trim_form<'a>(mut body: &'a [u8], suffix: &[u8]) -> &'a [u8] {
    while body.last().is_some_and(|b| suffix.contains(b)) {
        body = &body[..body.len() - 1];
    }
    body
}
