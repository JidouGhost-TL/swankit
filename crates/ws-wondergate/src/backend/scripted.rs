// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::{Connection, NetError, Network, Recv};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    rc::Rc,
};

#[derive(Clone, Default)]
pub struct PeerLog(Rc<RefCell<Log>>);
#[derive(Default)]
struct Log {
    sent: Vec<u8>,
    recv_calls: usize,
    close_calls: usize,
}
impl PeerLog {
    pub fn sent(&self) -> Vec<u8> {
        self.0.borrow().sent.clone()
    }
    pub fn recv_calls(&self) -> usize {
        self.0.borrow().recv_calls
    }
    pub fn close_calls(&self) -> usize {
        self.0.borrow().close_calls
    }
}

pub struct ScriptedPeer {
    pub ip: [u8; 4],
    pub port: u16,
    pub receives: VecDeque<Recv>,
    pub send_limit: usize,
    pub send_error: bool,
    log: PeerLog,
    closed: bool,
}
impl ScriptedPeer {
    pub fn new(ip: [u8; 4], port: u16, receives: impl IntoIterator<Item = Recv>) -> Self {
        Self {
            ip,
            port,
            receives: receives.into_iter().collect(),
            send_limit: usize::MAX,
            send_error: false,
            log: PeerLog::default(),
            closed: false,
        }
    }
    pub fn log(&self) -> PeerLog {
        self.log.clone()
    }
}
impl Connection for ScriptedPeer {
    fn send(&mut self, data: &[u8]) -> Result<usize, NetError> {
        if self.closed || self.send_error {
            return Err(NetError::Send);
        }
        let n = data.len().min(self.send_limit);
        self.log.0.borrow_mut().sent.extend_from_slice(&data[..n]);
        Ok(n)
    }
    fn recv(&mut self, max: usize) -> Recv {
        self.log.0.borrow_mut().recv_calls += 1;
        if self.closed {
            return Recv::Error;
        }
        if max == 0 {
            return Recv::Data(Vec::new());
        }
        loop {
            match self.receives.pop_front() {
                Some(Recv::Data(mut data)) => {
                    if data.is_empty() {
                        continue;
                    }
                    if data.len() > max {
                        let tail = data.split_off(max);
                        self.receives.push_front(Recv::Data(tail));
                    }
                    return Recv::Data(data);
                }
                Some(other) => return other,
                None => return Recv::Eof,
            }
        }
    }
    fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            self.log.0.borrow_mut().close_calls += 1;
        }
    }
}

/// Exact byte-name resolution and queued connection outcomes. Unlisted names
/// and endpoints are denied; fragments, partial writes, EOF and errors are explicit.
#[derive(Default)]
pub struct Scripted {
    pub resolutions: BTreeMap<Vec<u8>, [u8; 4]>,
    pub peers: VecDeque<Result<ScriptedPeer, NetError>>,
}
impl Scripted {
    pub fn queue(&mut self, peer: ScriptedPeer) -> PeerLog {
        let log = peer.log();
        self.peers.push_back(Ok(peer));
        log
    }
}
impl Network for Scripted {
    fn resolve(&mut self, name: &[u8]) -> Option<[u8; 4]> {
        self.resolutions.get(name).copied()
    }
    fn connect(&mut self, ip: [u8; 4], port: u16) -> Result<Box<dyn Connection>, NetError> {
        let peer = self.peers.pop_front().ok_or(NetError::Denied)??;
        if (peer.ip, peer.port) != (ip, port) {
            return Err(NetError::Denied);
        }
        Ok(Box::new(peer))
    }
}
