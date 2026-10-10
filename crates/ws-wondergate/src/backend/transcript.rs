// SPDX-License-Identifier: MIT OR Apache-2.0
//! Bounded per-connection wire replay with exact request checks.
use crate::{transcript::Streams, Connection, NetError, Network, Recv};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    fs::File,
    io::Read,
    path::Path,
    rc::Rc,
};

const MAX_PEER_BYTES: usize = 16 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 4096;

pub struct TranscriptPeer {
    pub ip: [u8; 4],
    pub port: u16,
    request: Vec<u8>,
    response: Vec<u8>,
}
impl TranscriptPeer {
    pub fn new(
        ip: [u8; 4],
        port: u16,
        request: Vec<u8>,
        response: Vec<u8>,
    ) -> Result<Self, NetError> {
        if request.len().saturating_add(response.len()) > MAX_PEER_BYTES {
            return Err(NetError::Contract);
        }
        Ok(Self {
            ip,
            port,
            request,
            response,
        })
    }
    /// The streams here are TCP request/response bytes, not adapter command bytes.
    pub fn from_streams(ip: [u8; 4], port: u16, streams: Streams) -> Result<Self, NetError> {
        Self::new(ip, port, streams.input, streams.output)
    }
    pub fn from_files(
        ip: [u8; 4],
        port: u16,
        request: &Path,
        response: &Path,
    ) -> Result<Self, NetError> {
        fn load(path: &Path, max: usize) -> Result<Vec<u8>, NetError> {
            let mut bytes = Vec::new();
            File::open(path)
                .and_then(|file| file.take(max as u64 + 1).read_to_end(&mut bytes))
                .map_err(|_| NetError::Contract)?;
            if bytes.len() > max {
                return Err(NetError::Contract);
            }
            Ok(bytes)
        }
        let request = load(request, MAX_PEER_BYTES)?;
        let response = load(response, MAX_PEER_BYTES - request.len())?;
        Self::new(ip, port, request, response)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TranscriptState {
    pub expected: usize,
    pub connected: usize,
    pub closed: usize,
    pub matched_requests: usize,
    pub mismatches: usize,
}
#[derive(Clone, Default)]
pub struct TranscriptLog(Rc<RefCell<TranscriptState>>);
impl TranscriptLog {
    pub fn snapshot(&self) -> TranscriptState {
        *self.0.borrow()
    }
    /// Call after dropping/closing the adapter. Unused peers and short requests
    /// are failures; clients may intentionally leave response tails unread.
    pub fn verify(&self) -> Result<(), NetError> {
        let s = self.snapshot();
        if s.expected != s.connected
            || s.closed != s.connected
            || s.matched_requests != s.expected
            || s.mismatches != 0
        {
            return Err(NetError::Contract);
        }
        Ok(())
    }
}
#[derive(Default)]
pub struct Transcript {
    pub resolutions: BTreeMap<Vec<u8>, [u8; 4]>,
    peers: VecDeque<TranscriptPeer>,
    retained: usize,
    log: TranscriptLog,
}
impl Transcript {
    pub fn log(&self) -> TranscriptLog {
        self.log.clone()
    }
    pub fn queue(&mut self, peer: TranscriptPeer) -> Result<(), NetError> {
        let size = peer.request.len().saturating_add(peer.response.len());
        if self.log.snapshot().expected >= MAX_CONNECTIONS
            || self.retained.saturating_add(size) > MAX_PEER_BYTES
        {
            return Err(NetError::Contract);
        }
        self.retained += size;
        self.peers.push_back(peer);
        self.log.0.borrow_mut().expected += 1;
        Ok(())
    }
}
impl Network for Transcript {
    fn resolve(&mut self, name: &[u8]) -> Option<[u8; 4]> {
        self.resolutions.get(name).copied()
    }
    fn connect(&mut self, ip: [u8; 4], port: u16) -> Result<Box<dyn Connection>, NetError> {
        let Some(peer) = self.peers.front() else {
            self.log.0.borrow_mut().mismatches += 1;
            return Err(NetError::Denied);
        };
        if (peer.ip, peer.port) != (ip, port) {
            self.log.0.borrow_mut().mismatches += 1;
            return Err(NetError::Denied);
        }
        let peer = self.peers.pop_front().ok_or(NetError::Contract)?;
        // The retention budget covers all queued and live peers for this run.
        self.log.0.borrow_mut().connected += 1;
        Ok(Box::new(TranscriptConnection {
            peer,
            sent: 0,
            received: 0,
            failed: false,
            closed: false,
            log: self.log.clone(),
        }))
    }
}
struct TranscriptConnection {
    peer: TranscriptPeer,
    sent: usize,
    received: usize,
    failed: bool,
    closed: bool,
    log: TranscriptLog,
}
impl TranscriptConnection {
    fn mismatch(&mut self) {
        if !self.failed {
            self.log.0.borrow_mut().mismatches += 1;
            self.failed = true;
        }
    }
}
impl Connection for TranscriptConnection {
    fn send(&mut self, data: &[u8]) -> Result<usize, NetError> {
        if self.failed || self.closed {
            return Err(NetError::Send);
        }
        let end = self
            .sent
            .checked_add(data.len())
            .ok_or(NetError::Contract)?;
        if self.peer.request.get(self.sent..end) != Some(data) {
            self.mismatch();
            return Err(NetError::Contract);
        }
        self.sent = end;
        Ok(data.len())
    }
    fn recv(&mut self, max: usize) -> Recv {
        if self.failed || self.closed {
            return Recv::Error;
        }
        if max == 0 {
            return Recv::Data(Vec::new());
        }
        if self.sent != self.peer.request.len() {
            self.mismatch();
            return Recv::Error;
        }
        if self.received == self.peer.response.len() {
            return Recv::Eof;
        }
        let n = max.min(self.peer.response.len() - self.received);
        let bytes = self.peer.response[self.received..self.received + n].to_vec();
        self.received += n;
        Recv::Data(bytes)
    }
    fn close(&mut self) {
        if self.closed {
            return;
        }
        if self.sent != self.peer.request.len() {
            self.mismatch();
        }
        let mut log = self.log.0.borrow_mut();
        log.closed += 1;
        if !self.failed {
            log.matched_requests += 1;
        }
        self.closed = true;
    }
}
impl Drop for TranscriptConnection {
    fn drop(&mut self) {
        self.close();
    }
}
