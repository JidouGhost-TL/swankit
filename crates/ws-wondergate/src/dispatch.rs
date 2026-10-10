// SPDX-License-Identifier: MIT
// Ported from WonderFence (MIT), Copyright (C) 2014 Alex 'trap15' Marshall.
// Source: github.com/kconger/wonderfence @ edf45a9cedd2b2a4c5054f859b94bbd5477d6b2c,
// src/wonfence.c, src/wonfence_cmdlist.h
// Modifications Copyright (C) 2026 jidoughost, MIT.

use crate::{frame::Frame, Profile};

pub(crate) fn known(frame: &Frame, profile: Profile) -> bool {
    match (frame.kind, frame.cmd) {
        (1, 1 | 3 | 4 | 9) => profile.f_stack_metadata,
        (1, 0 | 2 | 8 | 0xa | 0x10 | 0x11)
        | (2, 1)
        | (0xf, 0xff)
        | (0x11, 1 | 3 | 6 | 7 | 8 | 0xd | 0xe | 0xf) => true,
        _ => false,
    }
}
