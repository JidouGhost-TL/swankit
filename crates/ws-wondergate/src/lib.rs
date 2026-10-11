// SPDX-License-Identifier: MIT OR Apache-2.0
//! WonderGate request/reply device core with injected network I/O.
#![forbid(unsafe_code)]

pub mod backend;
pub mod bpl;
mod commands;
pub mod config;
mod device;
mod dispatch;
pub mod frame;
pub mod profile;
pub mod raw;
pub mod redirect;
pub mod transcript;
pub use config::{Config, ConfigError, Ppp, Probe};
pub use device::{Adapter, AdapterState, Event, NetEvent, SocketState};
pub use profile::Profile;

/// Bounds on command-plus-parameter length, socket allocation and BPL retention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_frame: usize,
    pub max_sockets: usize,
    pub max_bpl_message: usize,
    pub max_bpl_pending: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_frame: 4096,
            max_sockets: 255,
            max_bpl_message: 65536,
            max_bpl_pending: 65536,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetError {
    Denied,
    Connect,
    Send,
    Contract,
}
pub enum Recv {
    Data(Vec<u8>),
    Eof,
    /// A bounded wait elapsed; retrying the same connection is permitted.
    Timeout,
    Error,
}

pub trait Network {
    fn resolve(&mut self, name: &[u8]) -> Option<[u8; 4]>;
    /// Resolve the target of an explicit, exact hostname redirect. Live
    /// backends may treat that target as a narrowly scoped DNS authorization.
    /// In-memory backends retain their ordinary exact-map policy by default.
    fn resolve_redirect(&mut self, name: &[u8]) -> Option<[u8; 4]> {
        self.resolve(name)
    }
    fn connect(&mut self, ip: [u8; 4], port: u16) -> Result<Box<dyn Connection>, NetError>;
}
/// A backend owns blocking/wait policy. `send` may write partially; `recv`
/// returns at most `max` bytes. Errors and EOF never carry data.
pub trait Connection {
    fn send(&mut self, data: &[u8]) -> Result<usize, NetError>;
    fn recv(&mut self, max: usize) -> Recv;
    fn close(&mut self);
}
