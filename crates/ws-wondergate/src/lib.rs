// SPDX-License-Identifier: MIT OR Apache-2.0
//! WonderGate request/reply device core with injected network I/O.
#![forbid(unsafe_code)]

pub mod backend;
mod commands;
mod device;
mod dispatch;
pub mod frame;
pub mod profile;
pub use device::{Adapter, AdapterState, Config, Event, NetEvent, Ppp, SocketState};
pub use profile::Profile;

/// Bounds on declared command-plus-parameter length and socket allocations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_frame: usize,
    pub max_sockets: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_frame: 4096,
            max_sockets: 255,
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
    Error,
}

pub trait Network {
    fn resolve(&mut self, name: &[u8]) -> Option<[u8; 4]>;
    fn connect(&mut self, ip: [u8; 4], port: u16) -> Result<Box<dyn Connection>, NetError>;
}
/// A backend owns blocking/wait policy. `send` may write partially; `recv`
/// returns at most `max` bytes. Errors and EOF never carry data.
pub trait Connection {
    fn send(&mut self, data: &[u8]) -> Result<usize, NetError>;
    fn recv(&mut self, max: usize) -> Recv;
    fn close(&mut self);
}
