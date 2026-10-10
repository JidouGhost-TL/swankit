// SPDX-License-Identifier: MIT OR Apache-2.0
//! Deterministic in-memory network backend. No host networking is performed.
mod scripted;
pub use scripted::{PeerLog, Scripted, ScriptedPeer};
