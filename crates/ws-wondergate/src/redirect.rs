// SPDX-License-Identifier: MIT
// Ported from WonderFence (MIT), Copyright (C) 2014 Alex 'trap15' Marshall.
// Source: github.com/kconger/wonderfence @ edf45a9cedd2b2a4c5054f859b94bbd5477d6b2c,
// src/wongate_hack.c/.h
// Modifications Copyright (C) 2026 jidoughost, MIT.
//! Ordered, exact matches; redirects are applied once, never recursively.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRedirect {
    pub source: Vec<u8>,
    pub target: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub ip: [u8; 4],
    pub port: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SocketRedirect {
    /// Port zero matches any port on this IP.
    pub source: Endpoint,
    pub target: Endpoint,
}

pub fn hostname<'a>(name: &'a [u8], rules: &'a [HostRedirect]) -> &'a [u8] {
    rules
        .iter()
        .find(|rule| rule.source == name)
        .map_or(name, |rule| rule.target.as_slice())
}
pub fn socket(mut endpoint: Endpoint, rules: &[SocketRedirect]) -> Endpoint {
    if let Some(rule) = rules.iter().find(|rule| {
        rule.source.ip == endpoint.ip
            && (rule.source.port == 0 || rule.source.port == endpoint.port)
    }) {
        // C changes the port only on equality. An IP-only rule preserves the
        // requested nonzero port even if its target specifies another port.
        if rule.source.port == endpoint.port {
            endpoint.port = rule.target.port;
        }
        endpoint.ip = rule.target.ip;
    }
    endpoint
}
