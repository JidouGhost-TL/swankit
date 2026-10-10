// SPDX-License-Identifier: MIT
// Ported from WonderFence (MIT), Copyright (C) 2014 Alex 'trap15' Marshall.
// Source: github.com/kconger/wonderfence @ edf45a9cedd2b2a4c5054f859b94bbd5477d6b2c,
// src/wonfence_impl.c, src/wongate.c
// Modifications Copyright (C) 2026 jidoughost, MIT.

use crate::{
    bpl::{self, BplAllow},
    device::Slot,
    frame::Frame,
    profile::{CloseReply, Eof, Readline},
    redirect::{self, Endpoint},
    Adapter, Network, Ppp, Recv,
};
use formatkit_core::{Error, Reader, Result};

fn malformed(what: &str) -> Error {
    Error::Malformed(format!("WonderGate {what}"))
}
fn reply(frame: &Frame, cmd: u8, params: Vec<u8>) -> Frame {
    Frame {
        kind: frame.kind,
        cmd,
        params,
    }
}

impl<N: Network> Adapter<N> {
    pub(crate) fn execute(&mut self, frame: &Frame) -> Result<Frame> {
        let mut reader = Reader::new(&frame.params);
        let params = match (frame.kind, frame.cmd) {
            (1, 2) => {
                if self.state.status == 0 {
                    self.sockets.fill_with(|| None);
                    self.ppp = Ppp::default();
                    self.state.phone.clear();
                    self.state.dns = [[0; 4]; 2];
                    self.state.pdc = 0;
                    self.state.reception = self.config.pdc_reception;
                    self.state.status = 2;
                }
                vec![b'1', b'0']
            }
            (0xf, 0xff) => {
                if reader.bytes(4)? != [0x55, 0xaa, 0x55, 0xaa] {
                    return Err(malformed("poweroff confirmation"));
                }
                self.state.status = 0;
                if self.config.profile.poweroff_parameter {
                    vec![0]
                } else {
                    vec![]
                }
            }
            (2, 1) => {
                let accepted = if frame.params.is_empty() {
                    self.config.profile.status_without_parameter
                } else {
                    self.config.profile.status_params.contains(&reader.u8()?)
                };
                if !accepted {
                    return Err(malformed("status parameter"));
                }
                return Ok(reply(frame, 2, vec![self.state.status]));
            }
            (1, 0) => vec![self.state.pdc, self.state.reception],
            (1, 1 | 3 | 4) => vec![0],
            (1, 0x10) => {
                if reader.bytes(6)? != [7, 0, 0, 0, 0, 1] {
                    return Err(malformed("PPP prefix"));
                }
                let user_len = usize::from(reader.u8()?);
                let user = reader.bytes(user_len)?;
                let pass_len = usize::from(reader.u8()?);
                let password = reader.bytes(pass_len)?;
                self.ppp = Ppp {
                    user: user.to_vec(),
                    password: password.to_vec(),
                };
                vec![1]
            }
            (1, 0x11) => {
                if reader.u8()? != 4 {
                    return Err(malformed("DNS prefix"));
                }
                reader.skip(8)?;
                let first = reader
                    .bytes(4)?
                    .try_into()
                    .map_err(|_| malformed("DNS address"))?;
                let second = reader
                    .bytes(4)?
                    .try_into()
                    .map_err(|_| malformed("DNS address"))?;
                self.state.dns = [first, second];
                vec![1]
            }
            (1, 8) => {
                if reader.bytes(2)? != [0, 1]
                    || !self.config.profile.dial_modes.contains(&reader.u8()?)
                {
                    return Err(malformed("dial prefix"));
                }
                let count = usize::from(reader.u8()?);
                let phone = reader.bytes(count)?;
                self.state.phone = phone.to_vec();
                self.state.status = 3;
                return Ok(reply(frame, 0xb, vec![0]));
            }
            (1, 9) => return Ok(reply(frame, 0xb, vec![0])),
            (1, 0xa) => {
                if reader.u8()? != 1 {
                    return Err(malformed("hangup prefix"));
                }
                self.state.phone.clear();
                self.state.status = 2;
                return Ok(reply(frame, 0xb, vec![0]));
            }
            (0x11, 1) => {
                if reader.u8()? != 0 {
                    return Err(malformed("socket-create prefix"));
                }
                let allocated = self.sockets.iter().filter(|slot| slot.is_some()).count();
                let id = if allocated >= self.config.limits.max_sockets {
                    None
                } else {
                    self.sockets
                        .iter()
                        .enumerate()
                        .skip(usize::from(self.config.profile.first_socket))
                        .find(|(_, slot)| slot.is_none())
                        .map(|(id, _)| id)
                };
                if let Some(id) = id {
                    self.sockets[id] = Some(Slot {
                        connection: None,
                        terminal: None,
                        builtin_bpl: false,
                    });
                }
                vec![id.map_or(255, |id| id as u8)]
            }
            (0x11, 3) => {
                let id = reader.u8()?;
                let family = reader.u16_be()?;
                let port = reader.u16_le()?;
                let ip = reader
                    .bytes(4)?
                    .try_into()
                    .map_err(|_| malformed("socket address"))?;
                reader.bytes(8)?;
                if ![0, 2].contains(&family) {
                    return Err(malformed("socket family"));
                }
                let slot = self
                    .sockets
                    .get_mut(usize::from(id))
                    .and_then(Option::as_mut);
                let result = if let Some(slot) = slot {
                    if slot.connection.is_some() {
                        0
                    } else {
                        // Match C's interception order: internal BPL first,
                        // then one socket redirect, then backend networking.
                        let builtin = self.config.builtin_bpl && ip == bpl::IP && port == bpl::PORT;
                        let connection: std::result::Result<
                            Box<dyn crate::Connection>,
                            crate::NetError,
                        > = if builtin {
                            BplAllow::new(
                                self.config.limits.max_bpl_message,
                                self.config.limits.max_bpl_pending,
                            )
                            .map(|bpl| Box::new(bpl) as Box<dyn crate::Connection>)
                            .map_err(|_| crate::NetError::Contract)
                        } else {
                            let address = redirect::socket(
                                Endpoint { ip, port },
                                &self.config.socket_redirects,
                            );
                            self.net.connect(address.ip, address.port)
                        };
                        match connection {
                            Ok(connection) => {
                                slot.connection = Some(connection);
                                slot.terminal = None;
                                slot.builtin_bpl = builtin;
                                1
                            }
                            Err(_) => 0,
                        }
                    }
                } else {
                    0
                };
                vec![0, result]
            }
            (0x11, 6) => {
                if self.config.profile.poll_reports_allocation {
                    let id = reader.u8()?;
                    reader.u8()?;
                    vec![
                        0,
                        u8::from(
                            self.sockets
                                .get(usize::from(id))
                                .is_some_and(Option::is_some),
                        ),
                    ]
                } else {
                    vec![1]
                }
            }
            (0x11, 7) => {
                let id = reader.u8()?;
                let result = u8::from(
                    self.sockets
                        .get_mut(usize::from(id))
                        .and_then(Option::take)
                        .is_some(),
                );
                if self.config.profile.close_reply == CloseReply::SixByte {
                    vec![0, result]
                } else {
                    vec![result]
                }
            }
            (0x11, 8) => {
                let name_len = reader
                    .rest()
                    .iter()
                    .position(|&byte| byte == 0)
                    .ok_or_else(|| malformed("unterminated hostname"))?;
                let name = Reader::new(&frame.params).bytes(name_len)?;
                let builtin = self.config.builtin_bpl && bpl::is_hostname(name);
                let redirected = !builtin
                    && self
                        .config
                        .host_redirects
                        .iter()
                        .any(|rule| rule.source == name);
                let name = if builtin {
                    name
                } else {
                    redirect::hostname(name, &self.config.host_redirects)
                };
                if name.is_empty() || name.contains(&0) {
                    return Err(malformed("empty or NUL-containing hostname"));
                }
                // Bound the actual redirected reply before resolver I/O.
                let reply_length = name.len().saturating_add(7);
                if reply_length > self.config.limits.max_frame {
                    return Err(Error::ResourceLimit {
                        resource: "WonderGate DNS reply length",
                        requested: reply_length as u64,
                        limit: self.config.limits.max_frame as u64,
                    });
                }
                let address = if builtin {
                    Some(bpl::IP)
                } else if redirected {
                    self.net.resolve_redirect(name)
                } else {
                    self.net.resolve(name)
                };
                match address.filter(|ip| *ip != [0; 4]) {
                    Some(ip) => {
                        let mut data = vec![1];
                        data.extend_from_slice(name);
                        data.push(0);
                        data.extend_from_slice(&ip);
                        data
                    }
                    None => vec![0],
                }
            }
            (0x11, 0xe) => {
                let id = reader.u8()?;
                let data = reader.rest();
                if data.len() > 254 {
                    return Err(malformed("write exceeds one-byte result"));
                }
                let count = self
                    .sockets
                    .get_mut(usize::from(id))
                    .and_then(Option::as_mut)
                    .and_then(|slot| slot.connection.as_mut())
                    .and_then(|connection| connection.send(data).ok())
                    .filter(|&n| n <= data.len())
                    .map_or(255, |n| n as u8);
                vec![0, count]
            }
            (0x11, 0xd | 0xf) => {
                let id = reader.u8()?;
                let count = usize::from(reader.u8()?);
                let max_data = self.config.limits.max_frame.saturating_sub(3).min(254);
                if count > 0 && max_data == 0 {
                    return Err(Error::ResourceLimit {
                        resource: "WonderGate read reply length",
                        requested: 4,
                        limit: self.config.limits.max_frame as u64,
                    });
                }
                let count = count.min(max_data);
                self.read_socket(
                    id,
                    count,
                    frame.cmd == 0xd && self.config.profile.readline == Readline::ThroughLf,
                )
            }
            _ => return Err(malformed("unhandled command")),
        };
        Ok(reply(frame, frame.cmd, params))
    }

    fn read_socket(&mut self, id: u8, count: usize, line: bool) -> Vec<u8> {
        let Some(slot) = self
            .sockets
            .get_mut(usize::from(id))
            .and_then(Option::as_mut)
        else {
            return vec![0, 255];
        };
        let Some(connection) = slot.connection.as_mut() else {
            return vec![0, 255];
        };
        if count == 0 {
            return vec![0, 0];
        }
        let mut bytes = Vec::new();
        if slot.terminal.is_none() {
            loop {
                let request = if line { 1 } else { count };
                match connection.recv(request) {
                    Recv::Data(data) if !data.is_empty() && data.len() <= request => {
                        let at_lf = data.last() == Some(&b'\n');
                        bytes.extend_from_slice(&data);
                        if !line || at_lf || bytes.len() == count {
                            break;
                        }
                    }
                    Recv::Eof => {
                        if !slot.builtin_bpl {
                            slot.terminal = Some(true);
                        }
                        break;
                    }
                    _ => {
                        slot.terminal = Some(false);
                        break;
                    }
                }
            }
        }
        let result = if bytes.is_empty() {
            if slot.terminal == Some(false) || self.config.profile.eof == Eof::Ff {
                255
            } else {
                0
            }
        } else {
            bytes.len() as u8
        };
        let mut params = vec![0, result];
        params.extend_from_slice(&bytes);
        params
    }

    pub(crate) fn failure(&self, frame: &Frame) -> Option<Frame> {
        let (cmd, params) = match (frame.kind, frame.cmd) {
            (1, 8..=0xa) => (0xb, vec![255]),
            (1, 0x10 | 0x11) | (0x11, 8) => (frame.cmd, vec![0]),
            (0x11, 1) => (1, vec![255]),
            (0x11, 3) => (3, vec![0, 0]),
            (0x11, 6) if self.config.profile.poll_reports_allocation => (6, vec![0, 0]),
            (0x11, 7) => (
                7,
                if self.config.profile.close_reply == CloseReply::SixByte {
                    vec![0, 0]
                } else {
                    vec![0]
                },
            ),
            (0x11, 0xd..=0xf) => (frame.cmd, vec![0, 255]),
            _ => return None,
        };
        if params.len() + 1 > self.config.limits.max_frame {
            return None;
        }
        Some(reply(frame, cmd, params))
    }
}
