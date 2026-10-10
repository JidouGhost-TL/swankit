// SPDX-License-Identifier: MIT
// Ported from the private WonderGate revival server, wgserver/shared.py (MIT),
// corrected against PHASE0 and WonderFence src/bpl_proxy.c at
// edf45a9cedd2b2a4c5054f859b94bbd5477d6b2c (Keith Conger, MIT umbrella).
// Copyright (C) 2026 jidoughost. Protocol: WSdev WonderGate BPL (CC0).
use crate::{malformed, Limits};
use formatkit_core::{Reader, Result};
pub const HEADER_SIZE: usize = 24;
pub fn length(prefix: &[u8], limits: &Limits) -> Result<usize> {
    limits.validate()?;
    let mut reader = Reader::new(prefix);
    reader.skip(4)?;
    let total = reader.u32_le()? as usize;
    if !(HEADER_SIZE..=limits.max_bpl).contains(&total) {
        return Err(malformed("invalid BPL length"));
    }
    Ok(total)
}
pub fn reply(packet: &[u8], limits: &Limits) -> Result<Vec<u8>> {
    if length(packet, limits)? != packet.len() {
        return Err(malformed("BPL length mismatch"));
    }
    let mut reader = Reader::new(packet);
    let mut reply = reader.bytes(HEADER_SIZE)?.to_vec();
    reply[..8].copy_from_slice(&[1, 24, 16, 0, 24, 0, 0, 0]);
    Ok(reply)
}
/// Fragmented and consecutive messages; no allocation from unchecked lengths.
pub struct Assembler {
    raw: Vec<u8>,
    limits: Limits,
    failed: bool,
}
impl Assembler {
    pub fn new(limits: Limits) -> Self {
        Self {
            raw: Vec::new(),
            limits,
            failed: false,
        }
    }
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }
    pub fn feed(&mut self, mut bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
        if self.failed {
            return Err(malformed("BPL assembler is failed"));
        }
        self.limits.validate()?;
        // Bound one batch as well as each packet, including the returned list.
        if bytes.len() > self.limits.max_bpl {
            self.failed = true;
            return Err(malformed("oversized BPL batch"));
        }
        let mut packets = Vec::new();
        while !bytes.is_empty() {
            let needed = if self.raw.len() < 8 {
                8
            } else {
                match length(&self.raw, &self.limits) {
                    Ok(size) => size,
                    Err(e) => {
                        self.failed = true;
                        return Err(e);
                    }
                }
            };
            let count = (needed - self.raw.len()).min(bytes.len());
            self.raw.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.raw.len() >= 8 {
                let total = match length(&self.raw, &self.limits) {
                    Ok(size) => size,
                    Err(e) => {
                        self.failed = true;
                        return Err(e);
                    }
                };
                if self.raw.len() == total {
                    packets.push(std::mem::take(&mut self.raw));
                }
            }
        }
        Ok(packets)
    }
    pub fn finish(&self) -> Result<()> {
        if self.failed || !self.raw.is_empty() {
            return Err(malformed("early BPL EOF"));
        }
        Ok(())
    }
}
