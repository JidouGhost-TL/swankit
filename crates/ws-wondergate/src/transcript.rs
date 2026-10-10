// SPDX-License-Identifier: MIT OR Apache-2.0
//! Serial tap format: `T <milliseconds> > <hex>` and `T <milliseconds> < <hex>`.
use crate::{
    frame::{Decoder, Frame, HeaderMode},
    Limits,
};
use formatkit_core::Error;
use std::{
    io::{self, Read, Write},
    ops::Range,
    time::Instant,
};

const MAX_TEXT: usize = 64 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    ToAdapter,
    FromAdapter,
}
impl Direction {
    fn marker(self) -> char {
        match self {
            Self::ToAdapter => '>',
            Self::FromAdapter => '<',
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct TranscriptLimits {
    pub max_text: usize,
    pub max_stream: usize,
    pub max_records: usize,
}
impl Default for TranscriptLimits {
    fn default() -> Self {
        Self {
            max_text: MAX_TEXT,
            max_stream: MAX_TEXT / 2,
            max_records: 1_000_000,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub ms: u64,
    pub direction: Direction,
    /// Range in the corresponding joined stream, avoiding duplicate retention.
    pub range: Range<usize>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Streams {
    pub input: Vec<u8>,
    pub output: Vec<u8>,
    pub records: Vec<Record>,
}
impl Streams {
    pub fn bytes(&self, record: &Record) -> Option<&[u8]> {
        match record.direction {
            Direction::ToAdapter => &self.input,
            Direction::FromAdapter => &self.output,
        }
        .get(record.range.clone())
    }
}
#[derive(Debug)]
pub enum TranscriptError {
    Line {
        line: usize,
        reason: &'static str,
    },
    Limit {
        resource: &'static str,
        requested: usize,
        limit: usize,
    },
    Frame {
        offset: usize,
        error: Error,
    },
    Io(io::Error),
}
impl std::fmt::Display for TranscriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Line { line, reason } => write!(f, "WonderGate transcript line {line}: {reason}"),
            Self::Limit {
                resource,
                requested,
                limit,
            } => write!(
                f,
                "WonderGate transcript {resource}: {requested} exceeds {limit}"
            ),
            Self::Frame { offset, error } => {
                write!(f, "WonderGate transcript frame at {offset}: {error}")
            }
            Self::Io(error) => write!(f, "WonderGate transcript I/O: {error}"),
        }
    }
}
impl std::error::Error for TranscriptError {}
type Result<T> = std::result::Result<T, TranscriptError>;
fn bound(resource: &'static str, requested: usize, limit: usize) -> Result<()> {
    if requested > limit {
        return Err(TranscriptError::Limit {
            resource,
            requested,
            limit,
        });
    }
    Ok(())
}
fn line_error(line: usize, reason: &'static str) -> TranscriptError {
    TranscriptError::Line { line, reason }
}

pub fn parse(text: &str) -> Result<Streams> {
    parse_with_limits(text, TranscriptLimits::default())
}
pub fn parse_with_limits(text: &str, limits: TranscriptLimits) -> Result<Streams> {
    bound("text bytes", text.len(), limits.max_text.min(MAX_TEXT))?;
    let mut streams = Streams::default();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let raw = raw.trim();
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let mut fields = raw.split_whitespace();
        if fields.next() != Some("T") {
            return Err(line_error(line, "expected T record"));
        }
        let stamp = fields
            .next()
            .ok_or_else(|| line_error(line, "missing timestamp"))?;
        if !stamp.bytes().all(|b| b.is_ascii_digit()) {
            return Err(line_error(line, "invalid timestamp"));
        }
        let ms = stamp
            .parse::<u64>()
            .map_err(|_| line_error(line, "timestamp overflow"))?;
        let direction = match fields.next() {
            Some("#") => continue, // wrapper start/end metadata
            Some(">") => Direction::ToAdapter,
            Some("<") => Direction::FromAdapter,
            _ => return Err(line_error(line, "invalid direction")),
        };
        let hex = fields.next().unwrap_or("");
        if fields.next().is_some()
            || hex.len() % 2 != 0
            || !hex.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(line_error(line, "invalid hexadecimal bytes"));
        }
        bound(
            "records",
            streams.records.len() + 1,
            limits.max_records.min(1_000_000),
        )?;
        let stream = match direction {
            Direction::ToAdapter => &mut streams.input,
            Direction::FromAdapter => &mut streams.output,
        };
        let start = stream.len();
        let end = start
            .checked_add(hex.len() / 2)
            .ok_or(TranscriptError::Limit {
                resource: "stream bytes",
                requested: usize::MAX,
                limit: limits.max_stream,
            })?;
        bound("stream bytes", end, limits.max_stream.min(MAX_TEXT))?;
        for pair in hex.as_bytes().chunks_exact(2) {
            fn digit(b: u8) -> u8 {
                if b.is_ascii_digit() {
                    b - b'0'
                } else {
                    b.to_ascii_lowercase() - b'a' + 10
                }
            }
            stream.push((digit(pair[0]) << 4) | digit(pair[1]));
        }
        streams.records.push(Record {
            ms,
            direction,
            range: start..end,
        });
    }
    Ok(streams)
}
pub fn read(reader: impl Read, limits: TranscriptLimits) -> Result<Streams> {
    let max = limits.max_text.min(MAX_TEXT);
    let mut text = String::new();
    reader
        .take(max as u64 + 1)
        .read_to_string(&mut text)
        .map_err(TranscriptError::Io)?;
    parse_with_limits(&text, limits)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatedFrame {
    pub offset: usize,
    pub frame: Frame,
}
/// Decode joined directional bytes using the core codec, retaining wire offsets.
pub fn frames(
    bytes: &[u8],
    header: HeaderMode,
    skip_padding: bool,
    limits: Limits,
) -> Result<Vec<LocatedFrame>> {
    bound("frame stream bytes", bytes.len(), MAX_TEXT)?;
    let mut decoder = Decoder::new(header, skip_padding, limits);
    let mut output = Vec::new();
    let mut boundary = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        if index == boundary && skip_padding && byte == 0 {
            boundary += 1;
        }
        match decoder.push(byte) {
            Some(Ok(frame)) => {
                bound("frames", output.len() + 1, 1_000_000)?;
                let offset = index + 1 - (frame.params.len() + 4);
                output.push(LocatedFrame { offset, frame });
                boundary = index + 1;
            }
            Some(Err(error)) => {
                return Err(TranscriptError::Frame {
                    offset: boundary,
                    error,
                })
            }
            None => {}
        }
    }
    if let Some(error) = decoder.finish() {
        return Err(TranscriptError::Frame {
            offset: boundary,
            error,
        });
    }
    Ok(output)
}

/// Caller-owned sink. Every record flushes; explicit timestamps support replay
/// and deterministic tests, while `record` uses elapsed wall-clock milliseconds.
pub struct Tap<W: Write> {
    writer: W,
    start: Instant,
}
impl<W: Write> Tap<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            start: Instant::now(),
        }
    }
    pub fn write_at(&mut self, ms: u64, direction: Direction, bytes: &[u8]) -> io::Result<()> {
        write!(self.writer, "T {ms:08} {} ", direction.marker())?;
        for byte in bytes {
            write!(self.writer, "{byte:02x}")?;
        }
        writeln!(self.writer)?;
        self.writer.flush()
    }
    pub fn record(&mut self, direction: Direction, bytes: &[u8]) -> io::Result<()> {
        self.write_at(
            self.start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            direction,
            bytes,
        )
    }
    pub fn into_inner(self) -> W {
        self.writer
    }
}
