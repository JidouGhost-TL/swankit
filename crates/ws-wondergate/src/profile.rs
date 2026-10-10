// SPDX-License-Identifier: MIT OR Apache-2.0
//! Client compatibility policies, not measurements of a physical adapter.
use crate::frame::HeaderMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReply {
    FiveByte,
    SixByte,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readline {
    Block,
    ThroughLf,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eof {
    Zero,
    Ff,
}
/// Phase-0 contract for the later BPL responder. M12 moves BPL bytes opaquely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BplReply {
    FieldPreserving24,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub header: HeaderMode,
    pub first_socket: u8,
    pub close_reply: CloseReply,
    pub readline: Readline,
    pub eof: Eof,
    pub poweroff_parameter: bool,
    pub skip_zero_padding: bool,
    pub f_stack_metadata: bool,
    pub poll_reports_allocation: bool,
    pub dial_modes: &'static [u8],
    pub status_params: &'static [u8],
    pub bpl_reply: BplReply,
}

impl Profile {
    pub const UPSTREAM: Self = Self {
        header: HeaderMode::ZeroThenLen8,
        first_socket: 0x30,
        close_reply: CloseReply::FiveByte,
        readline: Readline::Block,
        eof: Eof::Zero,
        poweroff_parameter: true,
        skip_zero_padding: false,
        f_stack_metadata: false,
        poll_reports_allocation: false,
        dial_modes: &[3],
        status_params: &[3],
        bpl_reply: BplReply::FieldPreserving24,
    };
    pub const REVIVAL: Self = Self {
        header: HeaderMode::Len16Be,
        first_socket: 0,
        close_reply: CloseReply::SixByte,
        readline: Readline::ThroughLf,
        eof: Eof::Ff,
        poweroff_parameter: false,
        skip_zero_padding: true,
        f_stack_metadata: true,
        poll_reports_allocation: true,
        dial_modes: &[1, 3],
        status_params: &[0, 3],
        bpl_reply: BplReply::FieldPreserving24,
    };
}
