// SPDX-License-Identifier: MIT
// Ported from WonderFence (MIT), Copyright (C) 2014 Alex 'trap15' Marshall.
// Source: github.com/kconger/wonderfence @ edf45a9cedd2b2a4c5054f859b94bbd5477d6b2c,
// src/wonfence.c (configuration and numeric prefixes only; inih is not ported)
// Modifications Copyright (C) 2026 jidoughost, MIT.
//! The INI syntax reader is newly authored. Errors never echo input values.
use crate::{
    backend::TcpOptions,
    frame::HeaderMode,
    redirect::{Endpoint, HostRedirect, SocketRedirect},
    Limits, Profile,
};
use std::{net::Ipv4Addr, path::PathBuf, time::Duration};

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Ppp {
    pub user: Vec<u8>,
    pub password: Vec<u8>,
}
impl std::fmt::Debug for Ppp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ppp")
            .field(
                "credentials_configured",
                &(!self.user.is_empty() || !self.password.is_empty()),
            )
            .finish()
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Tcp,
    Transcript,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Probe {
    /// Original command framing, including unknown frame kinds.
    #[default]
    Framed,
    /// Discard standalone detector bytes without buffering a reply.
    Ignore,
    /// Echo the detector byte for clients with an interrupt-driven detector.
    Echo,
}
#[derive(Clone, Debug)]
pub struct Config {
    /// Standalone detector policy; bytes inside frames are always opaque.
    pub probe: Probe,
    /// Reply with five raw zero bytes after ten boundary-padding zeros.
    /// This client compatibility policy is disabled by default.
    pub zero_sync: bool,
    /// Add a trailing zero to the dial acknowledgment for older reply readers.
    pub dial_padding: bool,
    pub pdc_reception: u8,
    pub ppp: Ppp,
    pub builtin_bpl: bool,
    pub host_redirects: Vec<HostRedirect>,
    pub socket_redirects: Vec<SocketRedirect>,
    pub profile: Profile,
    pub limits: Limits,
    pub backend: Backend,
    pub tcp: TcpOptions,
    pub transcript_file: Option<PathBuf>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            probe: Probe::Framed,
            zero_sync: false,
            dial_padding: false,
            pdc_reception: 15,
            ppp: Ppp::default(),
            builtin_bpl: true,
            host_redirects: Vec::new(),
            socket_redirects: Vec::new(),
            profile: Profile::REVIVAL,
            limits: Limits::default(),
            backend: Backend::Tcp,
            tcp: TcpOptions::default(),
            transcript_file: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    /// One-based source line; zero denotes an overall bound/validation error.
    pub line: usize,
    pub reason: &'static str,
}
impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WonderGate config line {}: {}", self.line, self.reason)
    }
}
impl std::error::Error for ConfigError {}
type Result<T> = std::result::Result<T, ConfigError>;
fn error(line: usize, reason: &'static str) -> ConfigError {
    ConfigError { line, reason }
}

/// `xtoi` prefixes with checked arithmetic and complete-token validation.
/// Unlike C, garbage suffixes and overflow are errors rather than truncations.
pub fn integer(text: &str) -> Option<u64> {
    let (base, digits) = if let Some(s) = text.strip_prefix("0x").or_else(|| text.strip_prefix('$'))
    {
        (16, s)
    } else if let Some(s) = text.strip_prefix("0b") {
        (2, s)
    } else if let Some(s) = text.strip_prefix("0o") {
        (8, s)
    } else {
        (10, text)
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, base).ok()
}
fn number(text: &str, min: u64, max: u64, line: usize) -> Result<u64> {
    integer(text)
        .filter(|&n| (min..=max).contains(&n))
        .ok_or_else(|| error(line, "numeric value outside bounds"))
}
fn boolean(text: &str, line: usize) -> Result<bool> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => integer(text)
            .map(|n| n != 0)
            .ok_or_else(|| error(line, "invalid boolean")),
    }
}
fn ip(text: &str, line: usize) -> Result<[u8; 4]> {
    text.parse::<Ipv4Addr>()
        .map(|ip| ip.octets())
        .map_err(|_| error(line, "invalid dotted IPv4 address"))
}
fn endpoint(text: &str, line: usize) -> Result<Endpoint> {
    let (host, port) = match text.split_once(':') {
        Some((host, port)) => (host, number(port, 0, 65535, line)? as u16),
        None => (text, 0),
    };
    Ok(Endpoint {
        ip: ip(host, line)?,
        port,
    })
}
fn host(text: &str, line: usize) -> Result<Vec<u8>> {
    if text.is_empty() || text.len() > 4096 || text.bytes().any(|b| b <= 32 || b == 127) {
        return Err(error(line, "invalid hostname"));
    }
    Ok(text.as_bytes().to_vec())
}

impl Config {
    pub fn from_ini(text: &str) -> Result<Self> {
        if text.len() > 1024 * 1024 || text.contains('\0') {
            return Err(error(0, "invalid text or configuration exceeds 1 MiB"));
        }
        let mut config = Self::default();
        let mut section = "";
        let mut header = None;
        for (index, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = index + 1;
            if raw.len() > 8192 {
                return Err(error(line, "line exceeds 8192 bytes"));
            }
            // inih's inline semicolon is recognized only after whitespace.
            // Also support inline '#'; embedded punctuation is preserved.
            let end = raw
                .char_indices()
                .find(|&(i, c)| {
                    (c == ';' || c == '#') && (i == 0 || raw[..i].ends_with(char::is_whitespace))
                })
                .map_or(raw.len(), |(i, _)| i);
            let input = raw[..end].trim();
            if input.is_empty() {
                continue;
            }
            if input.starts_with('[') {
                section = input
                    .strip_prefix('[')
                    .and_then(|s| s.strip_suffix(']'))
                    .ok_or_else(|| error(line, "invalid section"))?
                    .trim();
                if ![
                    "pdc",
                    "ppp",
                    "bpl_proxy",
                    "hostname_hack",
                    "socket_hack",
                    "swankit",
                ]
                .contains(&section)
                {
                    return Err(error(line, "unknown section"));
                }
                continue;
            }
            let (key, value) = input
                .split_once('=')
                .ok_or_else(|| error(line, "expected key = value"))?;
            let (key, value) = (key.trim(), value.trim());
            if key.is_empty() {
                return Err(error(line, "empty key"));
            }
            match (section, key) {
                ("swankit", "zero_sync") => config.zero_sync = boolean(value, line)?,
                ("swankit", "dial_padding") => config.dial_padding = boolean(value, line)?,
                ("swankit", "probe") => {
                    config.probe = match value {
                        "framed" => Probe::Framed,
                        "ignore" => Probe::Ignore,
                        "echo" => Probe::Echo,
                        _ => return Err(error(line, "unknown detector policy")),
                    }
                }
                ("pdc", "reception") => config.pdc_reception = number(value, 0, 15, line)? as u8,
                ("ppp", "user" | "pass") => {
                    if value.len() > 255 {
                        return Err(error(line, "credential exceeds 255 bytes"));
                    }
                    if key == "user" {
                        config.ppp.user = value.as_bytes().to_vec();
                    } else {
                        config.ppp.password = value.as_bytes().to_vec();
                    }
                }
                ("bpl_proxy", "enabled") => config.builtin_bpl = boolean(value, line)?,
                ("hostname_hack", _) => config.host_redirects.push(HostRedirect {
                    source: host(key, line)?,
                    target: host(value, line)?,
                }),
                ("socket_hack", _) => config.socket_redirects.push(SocketRedirect {
                    source: endpoint(key, line)?,
                    target: endpoint(value, line)?,
                }),
                ("swankit", "profile") => {
                    config.profile = match value {
                        "upstream" => Profile::UPSTREAM,
                        "revival" => Profile::REVIVAL,
                        _ => return Err(error(line, "unknown profile")),
                    }
                }
                ("swankit", "header") => {
                    header = Some(match value {
                        "len16be" => HeaderMode::Len16Be,
                        "zero-then-len8" => HeaderMode::ZeroThenLen8,
                        _ => return Err(error(line, "unknown header mode")),
                    })
                }
                ("swankit", "backend") => {
                    config.backend = match value {
                        "tcp" => Backend::Tcp,
                        "transcript" => Backend::Transcript,
                        _ => return Err(error(line, "unknown backend")),
                    }
                }
                ("swankit", "transcript_file") => {
                    if value.is_empty() {
                        return Err(error(line, "empty transcript path"));
                    }
                    config.transcript_file = Some(PathBuf::from(value))
                }
                ("swankit", "max_frame") => {
                    config.limits.max_frame = number(value, 1, 65535, line)? as usize
                }
                ("swankit", "max_sockets") => {
                    config.limits.max_sockets = number(value, 0, 255, line)? as usize
                }
                ("swankit", "max_bpl_message") => {
                    config.limits.max_bpl_message = number(value, 24, 65536, line)? as usize
                }
                ("swankit", "max_bpl_pending") => {
                    config.limits.max_bpl_pending = number(value, 24, 65536, line)? as usize
                }
                ("swankit", "max_receive") => {
                    config.tcp.max_receive = number(value, 1, 65536, line)? as usize
                }
                ("swankit", "allow_system_dns") => {
                    config.tcp.allow_system_dns = boolean(value, line)?
                }
                ("swankit", "resolve_all") => config.tcp.resolve_all = Some(ip(value, line)?),
                ("swankit", "connect_timeout_ms" | "recv_timeout_ms" | "send_timeout_ms") => {
                    let duration = Duration::from_millis(number(value, 1, 600_000, line)?);
                    match key {
                        "connect_timeout_ms" => config.tcp.connect_timeout = duration,
                        "recv_timeout_ms" => config.tcp.recv_timeout = duration,
                        _ => config.tcp.send_timeout = duration,
                    }
                }
                ("swankit", _) if key.starts_with("resolve.") => {
                    config
                        .tcp
                        .resolutions
                        .insert(host(&key[8..], line)?, ip(value, line)?);
                }
                _ => return Err(error(line, "unknown key or key outside section")),
            }
        }
        if let Some(header) = header {
            config.profile.header = header;
        }
        Ok(config)
    }
}
