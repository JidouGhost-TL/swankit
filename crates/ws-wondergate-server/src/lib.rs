// SPDX-License-Identifier: MIT OR Apache-2.0
#![forbid(unsafe_code)]
//! Byte-preserving, self-hosted WonderGate shared services. No outbound network.

pub mod board;
pub mod http;
#[cfg(feature = "net")]
pub mod net;
pub mod services;
pub mod store;
mod time;

use formatkit_core::{Error, Result};
use std::sync::{Arc, Mutex};
use store::Store;
pub use time::{Clock, FixedClock, OsRng, Rng, SeededRng, SystemClock, Timestamp};

/// Resource policy cannot exceed the protocol's hard ceilings.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_header: usize,
    pub max_body: usize,
    pub max_bpl: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_header: 16384,
            max_body: 65536,
            max_bpl: 65536,
        }
    }
}
impl Limits {
    pub fn validate(self) -> Result<Self> {
        if self.max_header == 0
            || self.max_header > 16384
            || self.max_body > 65536
            || !(24..=65536).contains(&self.max_bpl)
        {
            return Err(malformed("invalid server limits"));
        }
        Ok(self)
    }
}
pub(crate) fn malformed(message: &str) -> Error {
    Error::Malformed(message.into())
}

struct Inner {
    services: services::Services,
    store: Box<dyn Store>,
    clock: Box<dyn Clock>,
    rng: Mutex<Box<dyn Rng>>,
    limits: Limits,
}
/// Clone shares the journal, clock and serialized RNG with connection threads.
#[derive(Clone)]
pub struct Server {
    inner: Arc<Inner>,
}
impl Server {
    pub fn bundled(
        store: Box<dyn Store>,
        clock: Box<dyn Clock>,
        rng: Box<dyn Rng>,
    ) -> Result<Self> {
        Self::new(
            services::Services::bundled()?,
            store,
            clock,
            rng,
            Limits::default(),
        )
    }
    pub fn new(
        services: services::Services,
        store: Box<dyn Store>,
        clock: Box<dyn Clock>,
        rng: Box<dyn Rng>,
        limits: Limits,
    ) -> Result<Self> {
        services.validate()?;
        Ok(Self {
            inner: Arc::new(Inner {
                services,
                store,
                clock,
                rng: Mutex::new(rng),
                limits: limits.validate()?,
            }),
        })
    }
    pub fn limits(&self) -> Limits {
        self.inner.limits
    }
    pub fn services(&self) -> &services::Services {
        &self.inner.services
    }
    pub fn clock(&self) -> &dyn Clock {
        self.inner.clock.as_ref()
    }
    pub fn resolves(&self, name: &str) -> bool {
        http::normalize_host(name)
            .is_ok_and(|host| self.services().hosts().any(|known| known == host))
    }
    pub fn try_dispatch_http(&self, raw: &[u8]) -> Result<Vec<u8>> {
        let request = http::parse_with_aliases(raw, &self.limits(), &self.services().http_aliases)?;
        let mut rng = self
            .inner
            .rng
            .lock()
            .map_err(|_| malformed("RNG lock poisoned"))?;
        Ok(services::charge::handle(
            &request,
            self.services(),
            self.inner.store.as_ref(),
            self.clock(),
            rng.as_mut(),
        )?
        .unwrap_or_else(|| http::empty_response(404)))
    }
    pub fn dispatch_http(&self, raw: &[u8]) -> Vec<u8> {
        match self.try_dispatch_http(raw) {
            Ok(reply) => reply,
            Err(
                Error::Malformed(_)
                | Error::Truncated { .. }
                | Error::ResourceLimit { .. }
                | Error::Trailing(_),
            ) => http::empty_response(400),
            Err(_) => http::empty_response(500),
        }
    }
    /// Logical service ports, independent of the TCP listener's physical port.
    pub fn session(&self, port: u16) -> Session {
        Session {
            server: self.clone(),
            input: if port == 5555 {
                Input::Bpl(services::bpl::Assembler::new(self.limits()))
            } else {
                Input::Http(http::Assembler::new(self.limits()))
            },
            closed: false,
        }
    }
}
enum Input {
    Http(http::Assembler),
    Bpl(services::bpl::Assembler),
}
pub struct Session {
    server: Server,
    input: Input,
    closed: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Progress {
    NeedMore,
    Reply { bytes: Vec<u8>, close: bool },
}
impl Session {
    /// BPL can return several responses in one feed. HTTP is one transaction.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Progress> {
        if self.closed {
            return Err(malformed("session is closed"));
        }
        match &mut self.input {
            Input::Http(input) => {
                if !input.feed(bytes)? {
                    return Ok(Progress::NeedMore);
                }
                self.closed = true;
                Ok(Progress::Reply {
                    bytes: self.server.dispatch_http(input.raw()),
                    close: true,
                })
            }
            Input::Bpl(input) => {
                let packets = input.feed(bytes)?;
                if packets.is_empty() {
                    return Ok(Progress::NeedMore);
                }
                let mut reply = Vec::with_capacity(packets.len() * 24);
                for packet in packets {
                    reply.extend(services::bpl::reply(&packet, &self.server.limits())?);
                }
                Ok(Progress::Reply {
                    bytes: reply,
                    close: false,
                })
            }
        }
    }
    pub fn finish(&self) -> Result<()> {
        match &self.input {
            Input::Http(input) => input.finish(),
            Input::Bpl(input) => input.finish(),
        }
    }
}
