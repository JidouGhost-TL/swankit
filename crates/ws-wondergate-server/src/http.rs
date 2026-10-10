// SPDX-License-Identifier: MIT
// Ported from the private WonderGate revival server, wgserver/protocol.py (MIT).
// Copyright (C) 2026 jidoughost.
//! Raw body and request bytes remain unchanged; text fields map Latin-1 bytes.
use crate::{malformed, Limits};
use formatkit_core::{Reader, Result};
use std::collections::BTreeMap;
pub type Headers = BTreeMap<String, String>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub raw: Vec<u8>,
    pub method: String,
    pub host: String,
    pub path: String,
    pub query: String,
    pub headers: Headers,
    pub body: Vec<u8>,
}
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}
fn header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|part| part == b"\r\n\r\n")
}
fn headers(head: &[u8]) -> Result<(String, String, Headers)> {
    let head = latin1(head);
    let mut lines = head.split("\r\n");
    let parts: Vec<_> = lines.next().unwrap_or("").split(' ').collect();
    if parts.len() != 3 || !matches!(parts[2], "HTTP/1.0" | "HTTP/1.1") {
        return Err(malformed("invalid request line"));
    }
    let mut fields = Headers::new();
    for line in lines {
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| malformed("invalid header"))?;
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(malformed("invalid header"));
        }
        if fields
            .insert(
                key.to_ascii_lowercase(),
                value
                    .trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
                    .to_owned(),
            )
            .is_some()
        {
            return Err(malformed("duplicate header"));
        }
    }
    Ok((parts[0].into(), parts[1].into(), fields))
}
pub fn normalize_host(authority: &str) -> Result<String> {
    let (host, port) = authority
        .split_once(':')
        .map_or((authority, None), |(h, p)| (h, Some(p)));
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        || port.is_some_and(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(malformed("invalid host"));
    }
    Ok(host.to_ascii_lowercase().trim_end_matches('.').into())
}
pub fn body_length(headers: &Headers, limits: &Limits) -> Result<usize> {
    limits.validate()?;
    if headers.contains_key("transfer-encoding") {
        return Err(malformed("transfer encoding unsupported"));
    }
    let value = headers
        .get("content-length")
        .map(String::as_str)
        .unwrap_or("0");
    if value.is_empty() || value.len() > 6 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed("invalid content length"));
    }
    let length: usize = value
        .parse()
        .map_err(|_| malformed("invalid content length"))?;
    if length > limits.max_body {
        return Err(malformed("invalid content length"));
    }
    Ok(length)
}
pub fn parse(raw: &[u8], limits: &Limits) -> Result<Request> {
    let services = crate::services::Services::bundled()?;
    parse_with_aliases(raw, limits, &services.http_aliases)
}
pub fn parse_with_aliases(raw: &[u8], limits: &Limits, aliases: &[[String; 2]]) -> Result<Request> {
    limits.validate()?;
    if raw.len() > limits.max_header + 4 + limits.max_body + 1 {
        return Err(malformed("oversized HTTP input"));
    }
    let end = header_end(raw).ok_or_else(|| malformed("missing HTTP headers"))?;
    if end > limits.max_header {
        return Err(malformed("oversized HTTP headers"));
    }
    let mut reader = Reader::new(raw);
    let (method, target, headers) = headers(reader.bytes(end)?)?;
    reader.skip(4)?;
    let length = body_length(&headers, limits)?;
    let body = reader.rest();
    if body.len() != length && !(length == 0 && body == [0]) {
        return Err(malformed("body length mismatch"));
    }
    // urlsplit discards these controls and leading C0/space, then fragments.
    let target = target
        .trim_start_matches(|c: char| c <= ' ')
        .replace(['\r', '\n', '\t'], "");
    let target = target.split('#').next().unwrap_or("");
    let after_scheme = if let Some((scheme, rest)) = target.split_once(':').filter(|(s, _)| {
        s.starts_with(|c: char| c.is_ascii_alphabetic())
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
    }) {
        if !scheme.eq_ignore_ascii_case("http") {
            return Err(malformed("only plain HTTP is supported"));
        }
        rest
    } else {
        if !target.starts_with('/') {
            return Err(malformed("invalid request target"));
        }
        target
    };
    let (authority, resource) = if let Some(rest) = after_scheme.strip_prefix("//") {
        let stop = rest.find(['/', '?']).unwrap_or(rest.len());
        (&rest[..stop], &rest[stop..])
    } else {
        ("", after_scheme)
    };
    let host = normalize_host(
        headers
            .get("host")
            .filter(|v| !v.is_empty())
            .map(String::as_str)
            .unwrap_or(authority),
    )?;
    if host.is_empty() {
        return Err(malformed("missing host"));
    }
    if !authority.is_empty() {
        let uri_host = normalize_host(authority)?;
        if host != uri_host
            && !aliases
                .iter()
                .any(|pair| pair.contains(&host) && pair.contains(&uri_host))
        {
            return Err(malformed("Host and absolute URI disagree"));
        }
    }
    let (path, query) = resource.split_once('?').unwrap_or((resource, ""));
    Ok(Request {
        raw: raw.to_vec(),
        method,
        host,
        path: if path.is_empty() {
            "/".into()
        } else {
            path.into()
        },
        query: query.into(),
        headers,
        body: body[..length].to_vec(),
    })
}
/// Content-Length, Connection, Content-Type (if supplied), then extra fields.
pub fn response(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Result<Vec<u8>> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        501 => "Not Implemented",
        500 => "Internal Server Error",
        _ => return Err(malformed("unsupported HTTP status")),
    };
    if body.len() > 65536 {
        return Err(malformed("oversized HTTP response body"));
    }
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (key, value) in headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("content-type"))
        .chain(
            headers
                .iter()
                .filter(|(key, _)| !key.eq_ignore_ascii_case("content-type")),
        )
    {
        if !key.is_ascii()
            || !value.is_ascii()
            || key.contains(['\r', '\n'])
            || value.contains(['\r', '\n'])
        {
            return Err(malformed("response header injection or non-ASCII header"));
        }
        if head
            .len()
            .saturating_add(key.len())
            .saturating_add(value.len())
            .saturating_add(4)
            > 16384
        {
            return Err(malformed("oversized HTTP response headers"));
        }
        head.push_str(&format!("{key}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let mut reply = head.into_bytes();
    reply.extend_from_slice(body);
    Ok(reply)
}
pub(crate) fn empty_response(status: u16) -> Vec<u8> {
    // All callers use supported constant statuses, with no user headers/body.
    response(status, &[], &[]).unwrap_or_default()
}
/// One bounded transaction; callers decide when the optional NUL window ends.
pub struct Assembler {
    raw: Vec<u8>,
    limits: Limits,
    expected: Option<(usize, usize)>,
    failed: bool,
}
impl Assembler {
    pub fn new(limits: Limits) -> Self {
        Self {
            raw: Vec::new(),
            limits,
            expected: None,
            failed: false,
        }
    }
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }
    pub fn body_size(&self) -> Option<usize> {
        self.expected.map(|(_, body)| body)
    }
    pub fn feed(&mut self, bytes: &[u8]) -> Result<bool> {
        if self.failed {
            return Err(malformed("HTTP assembler is failed"));
        }
        let result = self.push(bytes);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn push(&mut self, bytes: &[u8]) -> Result<bool> {
        self.limits.validate()?;
        let bound = self.limits.max_header + 4 + self.limits.max_body + 1;
        if bytes.len() > bound.saturating_sub(self.raw.len()) {
            return Err(malformed("oversized HTTP input"));
        }
        self.raw.extend_from_slice(bytes);
        if self.expected.is_none() {
            if let Some(end) = header_end(&self.raw) {
                if end > self.limits.max_header {
                    return Err(malformed("oversized HTTP headers"));
                }
                let (_, _, headers) = headers(&self.raw[..end])?;
                self.expected = Some((end + 4, body_length(&headers, &self.limits)?));
            } else if self.raw.len() > self.limits.max_header + 3 {
                return Err(malformed("oversized HTTP headers"));
            }
        }
        if let Some((end, length)) = self.expected {
            let expected = end + length;
            if self.raw.len() > expected && !(length == 0 && self.raw[expected..] == [0]) {
                return Err(malformed("body length mismatch"));
            }
            return Ok(self.raw.len() >= expected);
        }
        Ok(false)
    }
    pub fn finish(&self) -> Result<()> {
        if self.failed
            || self
                .expected
                .is_none_or(|(end, length)| self.raw.len() < end + length)
        {
            return Err(malformed("early EOF"));
        }
        Ok(())
    }
}
