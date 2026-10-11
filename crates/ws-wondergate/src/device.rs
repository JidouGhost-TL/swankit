// SPDX-License-Identifier: MIT
// Ported from WonderFence (MIT), Copyright (C) 2014 Alex 'trap15' Marshall.
// Source: github.com/kconger/wonderfence @ edf45a9cedd2b2a4c5054f859b94bbd5477d6b2c,
// src/wongate.c/.h, src/wongate_ser.c/.h
// Modifications Copyright (C) 2026 jidoughost, MIT.

use crate::{
    dispatch,
    frame::{Decoder, Frame, FrameError},
    Config, Connection, Network, Ppp, Probe,
};
use formatkit_core::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketState {
    pub id: u8,
    pub connected: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterState {
    pub status: u8,
    pub pdc: u8,
    pub reception: u8,
    pub dns: [[u8; 4]; 2],
    pub ppp_configured: bool,
    pub phone: Vec<u8>,
    pub sockets: Vec<SocketState>,
}

#[derive(Debug)]
pub enum Event {
    /// Unframed detector acknowledgment; no adapter power or network action.
    Probe {
        echoed: bool,
    },
    /// Credential bytes in PPP input frames are replaced by zeros.
    FrameIn(Frame),
    FrameOut(Frame),
    Dropped(FrameError),
    Dial(Vec<u8>),
    Net(NetEvent),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetEvent {
    Resolve { success: bool },
    Connect { socket: u8, success: bool },
    Send { socket: u8, count: usize },
    Receive { socket: u8, count: usize },
    Close { socket: u8, existed: bool },
}

pub(crate) struct Slot {
    pub connection: Option<Box<dyn Connection>>,
    pub terminal: Option<bool>, // true = EOF, false = error
    pub builtin_bpl: bool,
}
impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(connection) = &mut self.connection {
            connection.close();
        }
    }
}

pub struct Adapter<N: Network> {
    pub(crate) config: Config,
    decoder: Decoder,
    pub(crate) net: N,
    pub(crate) state: AdapterState,
    pub(crate) ppp: Ppp,
    pub(crate) sockets: [Option<Slot>; 255],
}

impl<N: Network> Adapter<N> {
    pub fn new(mut config: Config, net: N) -> Self {
        config.limits.max_frame = config
            .limits
            .max_frame
            .min(config.profile.header.max_length());
        config.limits.max_sockets = config.limits.max_sockets.min(255);
        let decoder = Decoder::new(
            config.profile.header,
            config.profile.skip_zero_padding,
            config.limits,
        );
        let ppp = config.ppp.clone();
        Self {
            config,
            decoder,
            net,
            ppp,
            sockets: std::array::from_fn(|_| None),
            state: AdapterState {
                status: 0,
                pdc: 3,
                reception: 15,
                dns: [[0; 4]; 2],
                ppp_configured: false,
                phone: Vec::new(),
                sockets: Vec::new(),
            },
        }
    }

    pub fn snapshot(&self) -> AdapterState {
        let mut state = self.state.clone();
        state.ppp_configured = !self.ppp.user.is_empty() || !self.ppp.password.is_empty();
        state.sockets = self
            .sockets
            .iter()
            .enumerate()
            .filter_map(|(id, slot)| {
                slot.as_ref().map(|slot| SocketState {
                    id: id as u8,
                    connected: slot.connection.is_some(),
                })
            })
            .collect();
        state
    }

    /// Checked single-command entry. Unknown commands return `Ok(None)`.
    /// Malformed commands return an error before changing device state.
    pub fn try_handle(&mut self, frame: &Frame) -> Result<Option<Frame>, FrameError> {
        let length = frame.params.len().saturating_add(1);
        if length > self.config.limits.max_frame {
            return Err(Error::ResourceLimit {
                resource: "WonderGate frame length",
                requested: length as u64,
                limit: self.config.limits.max_frame as u64,
            });
        }
        if !dispatch::known(frame, self.config.profile) {
            return Ok(None);
        }
        let minimum_reply = match (frame.kind, frame.cmd) {
            (0xf, 0xff) => {
                if self.config.profile.poweroff_parameter {
                    2
                } else {
                    1
                }
            }
            (1, 0 | 2) | (0x11, 3 | 0xd | 0xe | 0xf) => 3,
            (0x11, 7) if self.config.profile.close_reply == crate::profile::CloseReply::SixByte => {
                3
            }
            (0x11, 6) if self.config.profile.poll_reports_allocation => 3,
            _ => 2,
        };
        if minimum_reply > self.config.limits.max_frame {
            return Err(Error::ResourceLimit {
                resource: "WonderGate reply length",
                requested: minimum_reply as u64,
                limit: self.config.limits.max_frame as u64,
            });
        }
        self.execute(frame).map(Some)
    }

    /// Wire-compatible command entry, using documented failure replies for
    /// malformed known requests where their reply shape permits a failure.
    pub fn handle(&mut self, frame: &Frame) -> Option<Frame> {
        match self.try_handle(frame) {
            Ok(reply) => reply,
            Err(_) => self.failure(frame),
        }
    }

    pub fn push(&mut self, input: &[u8], out: &mut Vec<u8>, events: &mut dyn FnMut(Event)) {
        for &byte in input {
            if self.config.probe != Probe::Framed && byte == 0x8f && self.decoder.at_boundary() {
                let echoed = self.config.probe == Probe::Echo;
                if echoed {
                    out.push(byte);
                }
                events(Event::Probe { echoed });
                continue;
            }
            match self.decoder.push(byte) {
                Some(Ok(frame)) => {
                    events(Event::FrameIn(redacted(&frame)));
                    let reply = match self.try_handle(&frame) {
                        Ok(reply) => reply,
                        Err(error) => {
                            events(Event::Dropped(error));
                            self.failure(&frame)
                        }
                    };
                    if let Some(reply) = reply {
                        self.command_event(&frame, &reply, events);
                        if reply.params.len().saturating_add(1) > self.config.limits.max_frame {
                            events(Event::Dropped(Error::ResourceLimit {
                                resource: "WonderGate reply length",
                                requested: reply.params.len().saturating_add(1) as u64,
                                limit: self.config.limits.max_frame as u64,
                            }));
                            continue;
                        }
                        match reply.encode(self.config.profile.header) {
                            Ok(bytes) => {
                                out.extend_from_slice(&bytes);
                                events(Event::FrameOut(reply));
                            }
                            Err(error) => events(Event::Dropped(error)),
                        }
                    }
                }
                Some(Err(error)) => events(Event::Dropped(error)),
                None => {}
            }
        }
    }

    pub fn finish(&mut self) -> Option<FrameError> {
        self.decoder.finish()
    }

    fn command_event(&self, request: &Frame, reply: &Frame, events: &mut dyn FnMut(Event)) {
        let result = reply.params.last().copied().unwrap_or(0);
        match (request.kind, request.cmd) {
            (1, 8) if result == 0 => events(Event::Dial(self.state.phone.clone())),
            (0x11, 8) => events(Event::Net(NetEvent::Resolve {
                success: reply.params.first() == Some(&1),
            })),
            (0x11, cmd) => {
                let Some(&socket) = request.params.first() else {
                    return;
                };
                let event = match cmd {
                    3 => NetEvent::Connect {
                        socket,
                        success: result == 1,
                    },
                    7 => NetEvent::Close {
                        socket,
                        existed: result == 1,
                    },
                    0xe => NetEvent::Send {
                        socket,
                        count: if result == 255 {
                            0
                        } else {
                            usize::from(result)
                        },
                    },
                    0xd | 0xf => NetEvent::Receive {
                        socket,
                        count: reply.params.len().saturating_sub(2),
                    },
                    _ => return,
                };
                events(Event::Net(event));
            }
            _ => {}
        }
    }
}

fn redacted(frame: &Frame) -> Frame {
    let mut safe = frame.clone();
    if (safe.kind, safe.cmd) == (1, 0x10) {
        safe.params.fill(0);
    }
    safe
}
