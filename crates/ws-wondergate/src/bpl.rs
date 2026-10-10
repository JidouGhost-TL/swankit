// SPDX-License-Identifier: MIT
// Ported from WonderFence (MIT), Copyright (C) 2014 Alex 'trap15' Marshall.
// Source: github.com/kconger/wonderfence @ edf45a9cedd2b2a4c5054f859b94bbd5477d6b2c,
// src/bpl_proxy.c/.h
// Modifications Copyright (C) 2026 jidoughost, MIT.
// BPL responder follows the 2026 proxy by Keith Conger in the same repository
// (MIT umbrella) and the CC0 WSdev page WonderGate/bplXX.mopera.ne.jp.
use crate::{Connection, NetError, Recv};
use formatkit_core::{Error, Reader, Result};
use std::collections::VecDeque;

pub const HEADER_SIZE: usize = 24;
pub const PORT: u16 = 5555;
pub const IP: [u8; 4] = [127, 0, 0, 1];

pub fn is_hostname(name: &[u8]) -> bool {
    include_bytes!("../data/bpl-hosts.txt")
        .split(|&b| b == b'\n')
        .any(|host| !host.is_empty() && host == name)
}

/// Sans-IO, field-preserving allow responder. Bodies are counted and discarded;
/// only a partial header and a bounded FIFO of replies are retained.
pub struct BplAllow {
    header: [u8; HEADER_SIZE],
    header_len: usize,
    remaining: usize,
    replies: VecDeque<[u8; HEADER_SIZE]>,
    reply_pos: usize,
    max_message: usize,
    max_pending: usize,
    failed: bool,
    closed: bool,
}
impl Default for BplAllow {
    fn default() -> Self {
        Self {
            header: [0; HEADER_SIZE],
            header_len: 0,
            remaining: 0,
            replies: VecDeque::new(),
            reply_pos: 0,
            max_message: 65536,
            max_pending: 65536,
            failed: false,
            closed: false,
        }
    }
}
impl BplAllow {
    pub fn new(max_message: usize, max_pending: usize) -> Result<Self> {
        if !(HEADER_SIZE..=65536).contains(&max_message)
            || !(HEADER_SIZE..=65536).contains(&max_pending)
        {
            return Err(Error::Malformed("WonderGate BPL limits".into()));
        }
        Ok(Self {
            max_message,
            max_pending,
            ..Self::default()
        })
    }
    pub fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        if self.closed || self.failed {
            return Err(Error::Malformed(
                "WonderGate BPL connection unavailable".into(),
            ));
        }
        let result = self.feed_checked(bytes);
        if result.is_err() {
            self.failed = true;
            self.replies.clear();
        }
        result
    }
    fn feed_checked(&mut self, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            if self.header_len < HEADER_SIZE {
                let n = bytes.len().min(HEADER_SIZE - self.header_len);
                self.header[self.header_len..self.header_len + n].copy_from_slice(&bytes[..n]);
                self.header_len += n;
                bytes = &bytes[n..];
                if self.header_len < HEADER_SIZE {
                    continue;
                }
                let mut reader = Reader::new(&self.header);
                if reader.u8()? != 1 || usize::from(reader.u8()?) != HEADER_SIZE {
                    return Err(Error::Malformed(
                        "WonderGate BPL version/header size".into(),
                    ));
                }
                reader.skip(2)?;
                let size = u64::from(reader.u32_le()?);
                if size < HEADER_SIZE as u64 {
                    return Err(Error::Malformed(
                        "WonderGate BPL total shorter than header".into(),
                    ));
                }
                if size > self.max_message as u64 {
                    return Err(Error::ResourceLimit {
                        resource: "WonderGate BPL message",
                        requested: size,
                        limit: self.max_message as u64,
                    });
                }
                self.remaining = size as usize - HEADER_SIZE;
            } else {
                let n = bytes.len().min(self.remaining);
                self.remaining -= n;
                bytes = &bytes[n..];
            }
            if self.header_len == HEADER_SIZE && self.remaining == 0 {
                let pending = (self.replies.len() + 1) * HEADER_SIZE;
                if pending > self.max_pending {
                    return Err(Error::ResourceLimit {
                        resource: "WonderGate BPL queued replies",
                        requested: pending as u64,
                        limit: self.max_pending as u64,
                    });
                }
                let mut response = self.header;
                response[..8].copy_from_slice(&[1, 24, 16, 0, 24, 0, 0, 0]);
                self.replies.push_back(response);
                self.header_len = 0;
            }
        }
        Ok(())
    }
    pub fn finish(&self) -> Result<()> {
        if self.header_len != 0 || self.remaining != 0 {
            return Err(Error::Malformed("WonderGate BPL truncated message".into()));
        }
        if self.failed {
            return Err(Error::Malformed("WonderGate BPL failed message".into()));
        }
        Ok(())
    }
}
impl Connection for BplAllow {
    fn send(&mut self, data: &[u8]) -> std::result::Result<usize, NetError> {
        self.feed(data).map_err(|_| NetError::Contract)?;
        Ok(data.len())
    }
    fn recv(&mut self, max: usize) -> Recv {
        if self.closed || self.failed {
            return Recv::Error;
        }
        if max == 0 {
            return Recv::Data(Vec::new());
        }
        let Some(reply) = self.replies.front() else {
            // Same zero-byte result as C when no complete request is ready.
            // The device recognizes this internal socket as nonterminal.
            return Recv::Eof;
        };
        let n = max.min(HEADER_SIZE - self.reply_pos);
        let data = reply[self.reply_pos..self.reply_pos + n].to_vec();
        self.reply_pos += n;
        if self.reply_pos == HEADER_SIZE {
            self.replies.pop_front();
            self.reply_pos = 0;
        }
        Recv::Data(data)
    }
    fn close(&mut self) {
        self.closed = true;
        self.replies.clear();
        self.header.fill(0);
        self.header_len = 0;
        self.remaining = 0;
        self.reply_pos = 0;
    }
}
