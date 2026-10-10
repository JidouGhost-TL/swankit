// SPDX-License-Identifier: MIT
// Ported from WonderFence (MIT), Copyright (C) 2014 Alex 'trap15' Marshall.
// Source: github.com/kconger/wonderfence @ edf45a9cedd2b2a4c5054f859b94bbd5477d6b2c,
// src/wongate_cmd.c/.h, src/wongate_ser.c/.h
// Modifications Copyright (C) 2026 jidoughost, MIT.

use crate::Limits;
use formatkit_core::{Error, Reader};

pub type FrameError = Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderMode {
    ZeroThenLen8,
    Len16Be,
}

impl HeaderMode {
    pub const fn max_length(self) -> usize {
        match self {
            Self::ZeroThenLen8 => 255,
            Self::Len16Be => 65535,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub kind: u8,
    pub cmd: u8,
    pub params: Vec<u8>,
}

impl Frame {
    /// Encoding is fallible: a length is never silently truncated on the wire.
    pub fn encode(&self, mode: HeaderMode) -> Result<Vec<u8>, FrameError> {
        let length = self
            .params
            .len()
            .checked_add(1)
            .ok_or_else(|| limit(usize::MAX, mode.max_length()))?;
        if length > mode.max_length() {
            return Err(limit(length, mode.max_length()));
        }
        let mut bytes = Vec::with_capacity(length + 3);
        bytes.push(self.kind);
        bytes.extend_from_slice(&(length as u16).to_be_bytes());
        bytes.push(self.cmd);
        bytes.extend_from_slice(&self.params);
        Ok(bytes)
    }
}

fn limit(requested: usize, maximum: usize) -> FrameError {
    Error::ResourceLimit {
        resource: "WonderGate frame length",
        requested: requested as u64,
        limit: maximum as u64,
    }
}

/// Streaming framing with bounded storage. A rejected header drains its declared
/// body before accepting the next header, so payload bytes cannot become commands.
pub struct Decoder {
    mode: HeaderMode,
    skip_zero_padding: bool,
    max_length: usize,
    header: [u8; 3],
    header_len: usize,
    body: Vec<u8>,
    wanted: usize,
    discard: usize,
}

impl Decoder {
    pub fn new(mode: HeaderMode, skip_zero_padding: bool, limits: Limits) -> Self {
        Self {
            mode,
            skip_zero_padding,
            max_length: limits.max_frame.min(mode.max_length()),
            header: [0; 3],
            header_len: 0,
            body: Vec::new(),
            wanted: 0,
            discard: 0,
        }
    }

    pub fn push(&mut self, byte: u8) -> Option<Result<Frame, FrameError>> {
        if self.discard > 0 {
            self.discard -= 1;
            return None;
        }
        if self.header_len < 3 {
            if self.header_len == 0 && self.skip_zero_padding && byte == 0 {
                return None;
            }
            self.header[self.header_len] = byte;
            self.header_len += 1;
            if self.header_len != 3 {
                return None;
            }
            let mut reader = Reader::new(&self.header[1..]);
            let length = match self.mode {
                HeaderMode::Len16Be => reader.u16_be(),
                // Legacy C uses only byte 2; byte 1 is not an on-wire reply flag.
                HeaderMode::ZeroThenLen8 => {
                    reader.skip(1).and_then(|()| reader.u8().map(u16::from))
                }
            };
            let length = match length {
                Ok(n) => usize::from(n),
                Err(err) => {
                    self.reset();
                    return Some(Err(err));
                }
            };
            if length == 0 || length > self.max_length {
                self.reset();
                self.discard = length;
                return Some(Err(if length == 0 {
                    Error::InvalidField {
                        what: "WonderGate frame length",
                        value: 0,
                    }
                } else {
                    limit(length, self.max_length)
                }));
            }
            self.wanted = length;
            return None;
        }
        self.body.push(byte);
        if self.body.len() != self.wanted {
            return None;
        }
        let mut reader = Reader::new(&self.body);
        let frame = reader.u8().map(|cmd| Frame {
            kind: self.header[0],
            cmd,
            params: reader.rest().to_vec(),
        });
        self.reset();
        Some(frame)
    }

    /// Report a truncated stream and reset it, including an incomplete drain.
    pub fn finish(&mut self) -> Option<FrameError> {
        let error = if self.discard > 0 {
            Some(Error::Truncated {
                offset: 0,
                needed: self.discard,
                available: 0,
            })
        } else if self.header_len > 0 {
            Some(Error::Truncated {
                offset: self.header_len + self.body.len(),
                needed: if self.header_len < 3 {
                    3 - self.header_len
                } else {
                    self.wanted - self.body.len()
                },
                available: 0,
            })
        } else {
            None
        };
        self.reset();
        error
    }

    fn reset(&mut self) {
        self.header_len = 0;
        self.body.clear();
        self.wanted = 0;
        self.discard = 0;
    }
}
