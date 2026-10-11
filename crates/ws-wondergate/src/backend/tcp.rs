// SPDX-License-Identifier: MIT OR Apache-2.0
//! Blocking TCP with explicit DNS policy and bounded I/O waits.
use crate::{Connection, NetError, Network, Recv};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4, TcpStream, ToSocketAddrs},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct TcpOptions {
    pub resolutions: BTreeMap<Vec<u8>, [u8; 4]>,
    pub resolve_all: Option<[u8; 4]>,
    pub allow_system_dns: bool,
    pub connect_timeout: Duration,
    pub recv_timeout: Duration,
    pub send_timeout: Duration,
    /// Allocation ceiling for a single receive, including direct trait calls.
    pub max_receive: usize,
}
impl Default for TcpOptions {
    fn default() -> Self {
        Self {
            resolutions: BTreeMap::new(),
            resolve_all: None,
            allow_system_dns: false,
            connect_timeout: Duration::from_secs(5),
            recv_timeout: Duration::from_secs(5),
            send_timeout: Duration::from_secs(5),
            max_receive: 65536,
        }
    }
}
impl TcpOptions {
    pub fn validate(&self) -> Result<(), NetError> {
        if self.connect_timeout.is_zero()
            || self.recv_timeout.is_zero()
            || self.send_timeout.is_zero()
            || !(1..=65536).contains(&self.max_receive)
        {
            return Err(NetError::Contract);
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Tcp {
    options: TcpOptions,
}
impl Tcp {
    pub fn new(options: TcpOptions) -> Result<Self, NetError> {
        options.validate()?;
        Ok(Self { options })
    }
    fn resolve_name(&self, name: &[u8], explicit_redirect: bool) -> Option<[u8; 4]> {
        let text = std::str::from_utf8(name).ok()?;
        if text.is_empty() || name.len() > 4096 || name.contains(&0) {
            return None;
        }
        let ip = if let Some(ip) = self.options.resolutions.get(name) {
            Some(*ip)
        } else if let Ok(ip) = text.parse::<Ipv4Addr>() {
            // Numeric addresses require no DNS lookup (including redirect targets).
            Some(ip.octets())
        } else if let Some(ip) = self.options.resolve_all {
            Some(ip)
        } else if self.options.allow_system_dns || explicit_redirect {
            // Explicit opt-in. The OS resolver owns its wait policy; connect and
            // stream timeouts do not bound a system DNS call.
            (text, 0)
                .to_socket_addrs()
                .ok()?
                .find_map(|addr| match addr {
                    SocketAddr::V4(addr) => Some(addr.ip().octets()),
                    SocketAddr::V6(_) => None,
                })
        } else {
            None
        };
        ip.filter(|ip| *ip != [0; 4])
    }
}
impl Network for Tcp {
    fn resolve(&mut self, name: &[u8]) -> Option<[u8; 4]> {
        self.resolve_name(name, false)
    }
    fn resolve_redirect(&mut self, name: &[u8]) -> Option<[u8; 4]> {
        self.resolve_name(name, true)
    }
    fn connect(&mut self, ip: [u8; 4], port: u16) -> Result<Box<dyn Connection>, NetError> {
        if ip == [0; 4] || port == 0 {
            return Err(NetError::Denied);
        }
        let address = SocketAddr::V4(SocketAddrV4::new(ip.into(), port));
        let stream = TcpStream::connect_timeout(&address, self.options.connect_timeout)
            .map_err(|_| NetError::Connect)?;
        stream
            .set_read_timeout(Some(self.options.recv_timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.options.send_timeout)))
            .and_then(|()| stream.set_nodelay(true))
            .map_err(|_| NetError::Connect)?;
        Ok(Box::new(TcpConnection {
            stream: Some(stream),
            max_receive: self.options.max_receive,
        }))
    }
}

struct TcpConnection {
    stream: Option<TcpStream>,
    max_receive: usize,
}
impl Connection for TcpConnection {
    fn send(&mut self, data: &[u8]) -> Result<usize, NetError> {
        self.stream
            .as_mut()
            .ok_or(NetError::Send)?
            .write(data)
            .map_err(|_| NetError::Send)
    }
    fn recv(&mut self, max: usize) -> Recv {
        let Some(stream) = self.stream.as_mut() else {
            return Recv::Error;
        };
        if max == 0 {
            return Recv::Data(Vec::new());
        }
        let mut data = vec![0; max.min(self.max_receive)];
        match stream.read(&mut data) {
            Ok(0) => Recv::Eof,
            Ok(n) => {
                data.truncate(n);
                Recv::Data(data)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Recv::Timeout
            }
            Err(_) => Recv::Error,
        }
    }
    fn close(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}
impl Drop for TcpConnection {
    fn drop(&mut self) {
        self.close();
    }
}
