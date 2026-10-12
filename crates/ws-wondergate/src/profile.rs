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
/// Client-code-supported BPL response policy.
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
    /// Alternate mode-selection initialization accepted by revival clients.
    pub mode_select: bool,
    pub poll_reports_allocation: bool,
    pub dial_modes: &'static [u8],
    pub status_params: &'static [u8],
    /// A client lineage sends the status command with no parameter bytes.
    pub status_without_parameter: bool,
    /// Gather Beat dial validator (ROM 38C887) requires a 24+ byte success
    /// frame (BX>=0x18) with result 00; short 5/6-byte replies return 99.
    /// When true, dial success returns 21 placeholder zero params (25-byte
    /// frame). Extra bytes are length-checked only; content is placeholder.
    pub dial_long_success: bool,
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
        mode_select: false,
        poll_reports_allocation: false,
        dial_modes: &[3],
        status_params: &[3],
        status_without_parameter: false,
        dial_long_success: false,
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
        mode_select: true,
        poll_reports_allocation: true,
        dial_modes: &[1, 3],
        status_params: &[0, 3],
        status_without_parameter: true,
        dial_long_success: false,
        bpl_reply: BplReply::FieldPreserving24,
    };
}
