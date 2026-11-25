//! Tick-level model of the standard SDK two-byte-cell driver.
//!
//! The model keeps the driver's own state as bytes in a 64 KiB data-segment
//! image, steps it once per driver tick and records every sound-port and
//! wave-RAM write in program order. It follows the owner's reconstruction of
//! the standard tick (C035/C036/C042), cell fetch (C016/C019), volume slide,
//! channel-mask helper, play cascade and reset. Linked RAM and table
//! locations are caller input ([`SdkLink`]). Hardware readback follows WSdev
//! Sound: port 91h bit 7 reports headphones, 90h and the divider ports read
//! back their last value. Sweep changes to the channel 3 divider, voice notes
//! (sample start) and sound DMA are not modelled; reaching them returns
//! `Unsupported` with the slot state.
use formatkit_core::{Error, Result};
use serde::{Deserialize, Serialize};

/// Slot field offsets (bytes from the slot base), as in the reconstruction.
pub mod field {
    pub const POSITION: u16 = 0x00;
    pub const CHANNEL_FLAGS: u16 = 0x02;
    pub const SEQUENCE: u16 = 0x03;
    pub const HEADER_FLAGS: u16 = 0x06;
    pub const TEMPO_ACCUMULATOR: u16 = 0x07;
    pub const VOLUME: u16 = 0x08;
    pub const DURATION: u16 = 0x09;
    pub const SWEEP: u16 = 0x0a;
    pub const MODE: u16 = 0x0b;
    pub const NOISE_OR_SAMPLE: u16 = 0x0c;
    pub const ENVELOPE_GROUP: u16 = 0x0d;
    pub const ENVELOPE_PERIOD: u16 = 0x0e;
    pub const ENVELOPE_POSITION: u16 = 0x0f;
    pub const FREQUENCY: u16 = 0x10;
    pub const VOLUME_CAP: u16 = 0x12;
    pub const KEY_ON_RELOAD: u16 = 0x13;
    pub const KEY_ON_COUNTER: u16 = 0x14;
    pub const ENVELOPE_BANK: u16 = 0x15;
    pub const LOOP_AREA: u16 = 0x16;
    pub const SLIDE_BUDGET: u16 = 0x22;
    pub const SLIDE_PERIOD: u16 = 0x23;
    pub const SLIDE_DIVIDER: u16 = 0x24;
    pub const WAVE_OFFSET: u16 = 0x25;
    pub const VOLUME_MIRROR: u16 = 0x26;
    pub const BRANCH_INDEX: u16 = 0x28;
    pub const SUB_COUNTER: u16 = 0x29;
    pub const SUB_RETURN: u16 = 0x2a;
}

/// Post-tick channel-3 mute tail: C035/C042 restore the saved slot pointer
/// before clearing its frequency; C036 clears through the live pointer;
/// earlier builds omit the low-divisor mute check altogether.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MuteTail {
    SavedPointer,
    LivePointer,
    None,
}

fn default_true() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// Global driver bytes (data-segment offsets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdkGlobals {
    pub taken_mask: u16,
    pub control_shadow: u16,
    pub channel_index: u16,
    pub channel_bits: u16,
    pub phase_counter: u16,
    pub track_index: u16,
    pub deferred_mask: u16,
    pub deferred_value: u16,
    pub expired_mask: u16,
    pub first_track: u16,
    pub route_last: u16,
    pub voice_active: u16,
    /// Track index word kept by the play cascade.
    pub play_index: u16,
}

/// Linked locations of one standard build. RAM offsets are in the driver's
/// data segment (DS); table offsets are in the code/data segment image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdkLink {
    pub slots: u16,
    pub slot_stride: u16,
    pub globals: SdkGlobals,
    pub wave_shadow: u16,
    pub wave_ram: u16,
    pub deferred_target: u16,
    pub pitch_table: u16,
    pub wave_bank: u16,
    pub envelope_groups: u16,
    pub envelope_volumes: u16,
    pub vibrato_table: u16,
    pub sweep_table: u16,
    pub track_table: u16,
    /// Four voice-state words (sample id, pointer, length, next id) followed by
    /// the banked voice-sample bank byte at `+8`.
    pub voice_state: u16,
    /// Optional 8-byte-per-entry voice descriptor table offset in the driver segment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_table: Option<u16>,
    /// Optional word table of envelope-group base pointers indexed by `slot[0x15] * 2`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope_group_table: Option<u16>,
    /// Extended jump-table SDK variant (banked envelopes `A8`, speaker volume `A6`,
    /// branch/subroutine `A9`/`AC`/`AD`, channel-3 noise toggle on note, play port-90h clear).
    #[serde(default, skip_serializing_if = "is_false")]
    pub extended_opcodes: bool,
    /// Optional global branch-selector byte used by extended opcode `A9 FF`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_selector: Option<u16>,
    /// When true, channel-3 sweep delta is computed inline as 4-bit sign-magnitude
    /// (`(n & 7)` negated if `n & 8 != 0`) rather than read from `sweep_table`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub inline_sweep: bool,
    /// Whether `merge_control` ORs `0x22` into port `0x90` when `voice_active != 0`.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub merge_voice_active: bool,
    pub mute_tail: MuteTail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Port(u8),
    /// Data-segment RAM word written by the wave copy (little-endian value).
    WaveWord(u16),
}

/// One recorded write. `tick` counts completed [`SdkDriver::tick`] calls
/// before the write; writes from `reset`/`start` carry the current count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Write {
    pub tick: u32,
    pub target: Target,
    pub value: u16,
}

/// Sound port latch with the readback behaviour the driver depends on.
///
/// Tracks hardware Channel 3 sweep stepping (WSdev Sound: when SND_CTRL bits
/// 2 and 6 are set, every `(sweep_ticks + 1) * 8192` CPU cycles the signed
/// sweep amount in port 8Ch is added modulo 2048 to the channel 3 divisor in
/// ports 84h..85h; writing port 8Dh resets the internal tick counter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortLatch {
    ports: [u8; 0x15],
    pub headphones: bool,
    sweep_cycles: u32,
    sweep_ticks: u8,
}

impl PortLatch {
    pub fn new(headphones: bool) -> Self {
        Self {
            ports: [0; 0x15],
            headphones,
            sweep_cycles: 0,
            sweep_ticks: 0,
        }
    }
    fn index(port: u8) -> Result<usize> {
        match port {
            0x80..=0x94 => Ok(usize::from(port - 0x80)),
            _ => Err(Error::InvalidField {
                what: "SDK driver sound port",
                value: port.into(),
            }),
        }
    }
    /// WSdev Sound: 91h bit 7 is the read-only headphone status.
    pub fn read(&self, port: u8) -> Result<u8> {
        let value = self.ports[Self::index(port)?];
        Ok(match port {
            0x91 => (value & 0x7f) | if self.headphones { 0x80 } else { 0 },
            _ => value,
        })
    }
    pub fn write(&mut self, port: u8, value: u8) -> Result<()> {
        let idx = Self::index(port)?;
        self.ports[idx] = value;
        if port == 0x8d {
            self.sweep_ticks = 0;
        }
        Ok(())
    }
    /// Advance the hardware Channel 3 sweep counter by `cycles` CPU cycles
    /// (WSdev Sound: 375 Hz sweep clock = 1 tick per 8192 cycles at 3.072 MHz).
    pub fn advance_sweep(&mut self, cycles: u32) {
        if self.ports[0x10] & 0x44 != 0x44 {
            return;
        }
        self.sweep_cycles = self.sweep_cycles.saturating_add(cycles);
        while self.sweep_cycles >= 8192 {
            self.sweep_cycles -= 8192;
            self.sweep_ticks = self.sweep_ticks.wrapping_add(1);
            if self.sweep_ticks > (self.ports[0x0d] & 0x1f) {
                self.sweep_ticks = 0;
                let div = u16::from(self.ports[4]) | (u16::from(self.ports[5]) << 8);
                let delta = i32::from(self.ports[0x0c] as i8);
                let next = ((i32::from(div) + delta) & 0x7ff) as u16;
                self.ports[4] = next as u8;
                self.ports[5] = (next >> 8) as u8;
            }
        }
    }
}

/// Metadata recorded when the driver triggers a banked voice note (Channel 2 PCM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceTrigger {
    pub tick: u32,
    pub sample_id: u16,
    pub hblank_reload: u8,
    pub pointer: u16,
    pub bank: u8,
    pub length: u16,
    pub next: u16,
}

/// The model. `ram` is the driver's data segment; `data` its table segment.
#[derive(Debug, Clone)]
pub struct SdkDriver<'a> {
    pub link: SdkLink,
    data: &'a [u8],
    pub ram: Vec<u8>,
    pub ports: PortLatch,
    pub writes: Vec<Write>,
    pub voice_triggers: Vec<VoiceTrigger>,
    pub ticks: u32,
    si: u16,
    voice_reload: u8,
    voice_cycles: u32,
}

pub const MAX_WRITES: usize = 4_000_000;

impl<'a> SdkDriver<'a> {
    /// `data` is the 64 KiB segment holding tables and sequences (ES/CS).
    pub fn new(link: SdkLink, data: &'a [u8], headphones: bool) -> Result<Self> {
        if data.is_empty() || data.len() > 0x10000 || link.slot_stride == 0 {
            return Err(Error::Malformed("SDK driver segment/link shape".into()));
        }
        Ok(Self {
            link,
            data,
            ram: vec![0; 0x10000],
            ports: PortLatch::new(headphones),
            writes: Vec::new(),
            voice_triggers: Vec::new(),
            ticks: 0,
            si: 0,
            voice_reload: 0,
            voice_cycles: 0,
        })
    }

    /// Advance hardware state (Channel 3 sweep readback and HBlank-pumped
    /// Channel 2 voice countdown) by `cycles` CPU cycles.
    ///
    /// WSdev Timers: the HBlank timer counts down once per scanline (256 CPU
    /// cycles) and fires when its counter reaches 0, reloading `hblank_reload`.
    pub fn advance_hardware(&mut self, cycles: u32) {
        self.ports.advance_sweep(cycles);
        if self.g(|g| g.voice_active) == 0 || self.voice_reload == 0 {
            return;
        }
        let period = u32::from(self.voice_reload) * 256;
        self.voice_cycles = self.voice_cycles.saturating_add(cycles);
        while self.voice_cycles >= period && self.g(|g| g.voice_active) != 0 {
            self.voice_cycles -= period;
            let ptr = self
                .rw(self.link.voice_state.wrapping_add(2))
                .wrapping_add(1);
            self.ww(self.link.voice_state.wrapping_add(2), ptr);
            let len = self
                .rw(self.link.voice_state.wrapping_add(4))
                .wrapping_sub(1);
            self.ww(self.link.voice_state.wrapping_add(4), len);
            if len == 0 {
                if self.rw(self.link.voice_state.wrapping_add(6)) != 0xffff {
                    let _ = self.next_voice();
                } else {
                    self.set_g(|g| g.voice_active, 0);
                    for word in 0..4 {
                        self.ww(self.link.voice_state.wrapping_add(word * 2), 0);
                    }
                    self.voice_reload = 0;
                    self.voice_cycles = 0;
                    let _ = self.ports.write(0x94, 0);
                    let _ = self.ports.write(0x89, 0);
                    if let Ok(ctrl) = self.ports.read(0x90) {
                        let _ = self.ports.write(0x90, ctrl & 0xdd);
                    }
                }
            }
        }
    }

    fn rb(&self, at: u16) -> u8 {
        self.ram[usize::from(at)]
    }
    fn wb(&mut self, at: u16, value: u8) {
        self.ram[usize::from(at)] = value;
    }
    fn rw(&self, at: u16) -> u16 {
        u16::from(self.rb(at)) | u16::from(self.rb(at.wrapping_add(1))) << 8
    }
    fn ww(&mut self, at: u16, value: u16) {
        self.wb(at, value as u8);
        self.wb(at.wrapping_add(1), (value >> 8) as u8);
    }
    fn db(&self, at: u16) -> Result<u8> {
        self.data
            .get(usize::from(at))
            .copied()
            .ok_or_else(|| Error::Malformed(format!("SDK table read {at:04x} outside segment")))
    }
    fn dw(&self, at: u16) -> Result<u16> {
        Ok(u16::from(self.db(at)?) | u16::from(self.db(at.wrapping_add(1))?) << 8)
    }
    fn g(&self, at: fn(&SdkGlobals) -> u16) -> u8 {
        self.rb(at(&self.link.globals))
    }
    fn set_g(&mut self, at: fn(&SdkGlobals) -> u16, value: u8) {
        let address = at(&self.link.globals);
        self.wb(address, value);
    }
    fn record(&mut self, target: Target, value: u16) -> Result<()> {
        if self.writes.len() >= MAX_WRITES {
            return Err(Error::ResourceLimit {
                resource: "SDK driver writes",
                requested: self.writes.len() as u64 + 1,
                limit: MAX_WRITES as u64,
            });
        }
        self.writes.push(Write {
            tick: self.ticks,
            target,
            value,
        });
        Ok(())
    }
    fn out8(&mut self, port: u8, value: u8) -> Result<()> {
        self.ports.write(port, value)?;
        self.record(Target::Port(port), value.into())
    }
    /// A word OUT to an even port writes the low byte, then the high byte.
    fn out16(&mut self, port: u8, value: u16) -> Result<()> {
        self.out8(port, value as u8)?;
        self.out8(port.wrapping_add(1), (value >> 8) as u8)
    }
    fn in16(&self, port: u8) -> Result<u16> {
        Ok(u16::from(self.ports.read(port)?) | u16::from(self.ports.read(port + 1)?) << 8)
    }
    fn taken(&self) -> bool {
        self.g(|g| g.channel_bits) & self.g(|g| g.taken_mask) != 0
    }
    fn channel(&self) -> u8 {
        self.g(|g| g.channel_index)
    }

    /// Standard reset: silence, hardware defaults and eight end sentinels.
    /// `deferred` is the CH:CL pair the shipped reset passes to its first helper.
    pub fn reset(&mut self, deferred: u16) -> Result<()> {
        self.set_g(|g| g.deferred_mask, (deferred >> 8) as u8);
        self.set_g(|g| g.deferred_value, deferred as u8);
        self.set_g(|g| g.expired_mask, 0);
        self.set_g(|g| g.control_shadow, 0);
        for port in [0x88, 0x8a, 0x8b] {
            self.out8(port, 0)?;
        }
        for port in [0x80, 0x82, 0x84, 0x86] {
            self.out16(port, 0)?;
        }
        self.out8(0x8c, 0)?;
        self.out8(0x8d, 0)?;
        if self.g(|g| g.voice_active) == 0 {
            self.out8(0x89, 0)?;
            self.out8(0x90, 0x40)?;
            self.out8(0x94, 0)?;
        } else {
            self.out8(0x90, 0x62)?;
        }
        self.out8(0x91, 6)?;
        self.out8(0x8e, 0x18)?;
        self.out8(0x8f, (self.link.wave_ram >> 6) as u8)?;
        for slot in 0..8u16 {
            let base = self.link.slots.wrapping_add(slot * self.link.slot_stride);
            self.ww(base, 0xffff);
            if self.link.extended_opcodes && self.link.slot_stride >= 0x2c {
                self.ww(base.wrapping_add(field::SUB_COUNTER), 0);
            }
        }
        self.set_g(|g| g.first_track, 0);
        self.set_g(|g| g.route_last, 0);
        Ok(())
    }

    /// Voice state clear (no port writes): the helper the fetch calls before
    /// starting a voice note, also used by some play wrappers.
    pub fn clear_voice(&mut self) {
        self.set_g(|g| g.voice_active, 0);
        self.voice_reload = 0;
        self.voice_cycles = 0;
        for word in 0..4 {
            self.ww(self.link.voice_state.wrapping_add(word * 2), 0);
        }
    }

    /// Voice stop: clears the voice flag and state words, silences the voice
    /// output and clears control bits 1 and 5. Its timer/IRQ/DMA-side port
    /// writes (A2h, A4h, B2h) are outside the sound register range.
    pub fn stop_voice(&mut self) -> Result<()> {
        self.set_g(|g| g.voice_active, 0);
        self.voice_reload = 0;
        self.voice_cycles = 0;
        for word in 0..4 {
            self.ww(self.link.voice_state.wrapping_add(word * 2), 0);
        }
        self.out8(0x94, 0)?;
        self.out8(0x89, 0)?;
        let control = self.ports.read(0x90)? & 0xdd;
        self.out8(0x90, control)
    }

    /// Standard banked voice start (C003): loads the 4-word voice descriptor
    /// from `voice_table + sample_id * 8` into `voice_state`, enables Channel 2
    /// voice mode (`0x90 |= 0x22`, `0x89 = 0`, `0x94 = 0x0f`), and sets
    /// `voice_active = 1`. Timer/IRQ/bank ports (`A2h`, `A4h`, `B2h`, `C2h`)
    /// are outside the sound register range; the trigger is recorded in
    /// [`Self::voice_triggers`].
    pub fn start_voice(&mut self, sample_id: u16, hblank_reload: u8) -> Result<()> {
        let table = self.link.voice_table.ok_or_else(|| {
            Error::Unsupported(format!(
                "SDK voice note (sample {sample_id:04x}) requires voice_table in SdkLink"
            ))
        })?;
        self.ww(self.link.voice_state, sample_id);
        self.out8(0x89, 0)?;
        let control = self.ports.read(0x90)? | 0x22;
        self.out8(0x90, control)?;
        let entry = table.wrapping_add(sample_id.wrapping_mul(8));
        let pointer = self.dw(entry)?;
        let bank_word = self.dw(entry.wrapping_add(2))?;
        let length = self.dw(entry.wrapping_add(4))?;
        let next = self.dw(entry.wrapping_add(6))?;
        self.ww(self.link.voice_state.wrapping_add(2), pointer);
        self.wb(self.link.voice_state.wrapping_add(8), bank_word as u8);
        self.ww(self.link.voice_state.wrapping_add(4), length);
        self.ww(self.link.voice_state.wrapping_add(6), next);
        self.set_g(|g| g.voice_active, 1);
        self.voice_reload = hblank_reload;
        self.voice_cycles = 0;
        self.out8(0x94, 0x0f)?;
        self.voice_triggers.push(VoiceTrigger {
            tick: self.ticks,
            sample_id,
            hblank_reload,
            pointer,
            bank: bank_word as u8,
            length,
            next,
        });
        Ok(())
    }

    /// Standard banked voice chain step: advances to the descriptor indexed by
    /// `voice_state + 6` (`next`) without modifying sound ports.
    pub fn next_voice(&mut self) -> Result<()> {
        let table = self.link.voice_table.ok_or_else(|| {
            Error::Unsupported("SDK voice chain requires voice_table in SdkLink".into())
        })?;
        let next_id = self.rw(self.link.voice_state.wrapping_add(6));
        self.ww(self.link.voice_state, next_id);
        let entry = table.wrapping_add(next_id.wrapping_mul(8));
        let pointer = self.dw(entry)?;
        let bank_word = self.dw(entry.wrapping_add(2))?;
        let length = self.dw(entry.wrapping_add(4))?;
        let next = self.dw(entry.wrapping_add(6))?;
        self.ww(self.link.voice_state.wrapping_add(2), pointer);
        self.wb(self.link.voice_state.wrapping_add(8), bank_word as u8);
        self.ww(self.link.voice_state.wrapping_add(4), length);
        self.ww(self.link.voice_state.wrapping_add(6), next);
        Ok(())
    }

    /// Deferred parameter helper: CH is the track mask, CL the value applied
    /// to header flags when a masked track's note expires.
    pub fn set_deferred(&mut self, mask: u8, value: u8) {
        self.set_g(|g| g.deferred_mask, mask);
        self.set_g(|g| g.deferred_value, value);
        self.set_g(|g| g.expired_mask, 0);
    }

    /// First-track helpers: the scan starts its track counter at `first`
    /// (the slot pointer still starts at slot 0); some builds also replace
    /// the control shadow.
    pub fn set_first_track(&mut self, first: u8, shadow: Option<u8>) {
        self.set_g(|g| g.first_track, first);
        if let Some(shadow) = shadow {
            self.set_g(|g| g.control_shadow, shadow);
        }
    }

    /// The standard play cascade: start `count` consecutive track-table
    /// entries beginning at `first` (music uses four).
    pub fn start(&mut self, first: u16, count: u16) -> Result<()> {
        if !(1..=8).contains(&count) {
            return Err(Error::InvalidField {
                what: "SDK play track count",
                value: count.into(),
            });
        }
        let mut index = first;
        for _ in 0..count {
            let at = self.link.globals.play_index;
            self.ww(at, index);
            let entry = self.link.track_table.wrapping_add(index.wrapping_mul(6));
            let slot = self.link.slots.wrapping_add(self.dw(entry)?);
            if self.rw(slot) != 0xffff {
                let mask = [0xee, 0xdd, 0xbb, 0x77][usize::from(self.rb(slot + 2) & 3)];
                let shadow = self.g(|g| g.control_shadow) & mask;
                self.set_g(|g| g.control_shadow, shadow);
                if self.link.extended_opcodes {
                    let control = self.ports.read(0x90)? & (mask | 0xf0);
                    self.out8(0x90, control)?;
                }
            }
            self.ww(slot, 0);
            self.wb(slot + 2, self.dw(entry + 2)? as u8);
            self.ww(slot + 3, self.dw(entry + 4)?);
            self.wb(slot + 0x0a, 0);
            self.ww(slot + 0x15, 0);
            for offset in [0x18, 0x1b, 0x1e] {
                self.wb(slot + offset, 0);
            }
            for offset in [0x21, 0x23, 0x25] {
                self.ww(slot + offset, 0);
            }
            if self.link.extended_opcodes && self.link.slot_stride >= 0x2c {
                self.ww(slot + 0x27, 0);
            }
            index = index.wrapping_add(1);
        }
        Ok(())
    }

    /// One driver tick (the VBlank-called tick routine).
    pub fn tick(&mut self) -> Result<()> {
        use field::*;
        let route = self.ports.read(0x91)?;
        if route != self.g(|g| g.route_last) {
            self.set_g(|g| g.route_last, route);
            let value = if route & 0x80 != 0 {
                (route | 8) & 0xfe
            } else {
                (route | 1) & 0xf7
            };
            self.out8(0x91, value)?;
        }
        let first = self.g(|g| g.first_track);
        self.set_g(|g| g.track_index, first);
        self.set_g(|g| g.taken_mask, 0);
        let phase = self.g(|g| g.phase_counter).wrapping_add(1);
        self.set_g(|g| g.phase_counter, phase);
        let mut slot = self.link.slots;
        loop {
            self.si = slot;
            if self.rw(slot + POSITION) != 0xffff {
                let channel = self.rb(slot + CHANNEL_FLAGS) & 3;
                self.set_g(|g| g.channel_index, channel);
                self.set_g(|g| g.channel_bits, 0x11u8 << channel);
                let continue_with = if self.rw(slot + POSITION) == 0 {
                    self.initialize(slot)?;
                    self.fetch_next_note(slot)?
                } else {
                    self.step_slot(slot)?
                };
                if continue_with && self.reserve_channel(slot)? {
                    // Mute tail already advanced past the slot.
                    if self.advance(&mut slot) {
                        break;
                    }
                    continue;
                }
            }
            if self.advance(&mut slot) {
                break;
            }
        }
        self.merge_control()?;
        self.ticks = self.ticks.wrapping_add(1);
        Ok(())
    }

    /// Returns true when the scan is finished.
    fn advance(&mut self, slot: &mut u16) -> bool {
        *slot = slot.wrapping_add(self.link.slot_stride);
        let index = self.g(|g| g.track_index).wrapping_add(1);
        self.set_g(|g| g.track_index, index);
        index >= 8
    }

    fn initialize(&mut self, slot: u16) -> Result<()> {
        use field::*;
        let header = self.rw(slot + SEQUENCE);
        self.wb(slot + TEMPO_ACCUMULATOR, 0);
        let flags = self.db(header)?;
        self.wb(slot + HEADER_FLAGS, flags);
        let channel = self.channel();
        let wave = self.db(header.wrapping_add(1))?;
        let offset;
        if channel == 1 && self.db(header.wrapping_add(3))? == 1 {
            self.wb(slot + NOISE_OR_SAMPLE, wave);
            offset = wave;
        } else {
            offset = wave << 4;
            self.wb(slot + WAVE_OFFSET, offset);
            let shadow = self.link.wave_shadow.wrapping_add(channel.into());
            self.wb(shadow, offset);
        }
        // The copy uses AX as left by the branch taken above.
        self.copy_wave(channel, offset)?;
        self.wb(slot + VOLUME, self.db(header.wrapping_add(2))?);
        let mode = self.db(header.wrapping_add(3))?;
        if channel & 1 == 0 {
            self.wb(slot + SWEEP, mode);
        } else {
            self.wb(slot + MODE, mode);
            if !self.link.extended_opcodes && mode != 0 && channel == 3 {
                let control = self.ports.read(0x90)? | 0x80;
                self.out8(0x90, control)?;
            }
        }
        self.wb(slot + ENVELOPE_GROUP, 0);
        self.wb(slot + VOLUME_CAP, 0xff);
        self.ww(slot + POSITION, 2);
        Ok(())
    }

    fn copy_wave(&mut self, channel: u8, offset: u8) -> Result<()> {
        let source = self.link.wave_bank.wrapping_add(offset.into());
        let target = self.link.wave_ram.wrapping_add(u16::from(channel << 4));
        for word in 0..8u16 {
            let value = self.dw(source.wrapping_add(word * 2))?;
            let at = target.wrapping_add(word * 2);
            self.ww(at, value);
            self.record(Target::WaveWord(at), value)?;
        }
        Ok(())
    }

    /// Envelope lookup shared by tick and fetch: a 4-bit fraction of
    /// position/period selects a step in the group table, combined with the
    /// track volume through the volume table.
    fn envelope_volume(&self, slot: u16) -> Result<u8> {
        use field::*;
        let period = self.rb(slot + ENVELOPE_PERIOD);
        let mut position = self.rb(slot + ENVELOPE_POSITION);
        let mut fraction = 0u8;
        for _ in 0..4 {
            position = position.wrapping_add(position);
            let bit = if position >= period {
                position = position.wrapping_sub(period);
                1
            } else {
                0
            };
            fraction = (fraction << 1) | bit;
        }
        let index = u16::from(self.rb(slot + ENVELOPE_GROUP) | fraction);
        let group_base = if let Some(table) = self.link.envelope_group_table {
            let bank_off = u16::from(self.rb(slot + ENVELOPE_BANK).wrapping_shl(1));
            self.dw(table.wrapping_add(bank_off))?
        } else {
            self.link.envelope_groups
        };
        let step = self.db(group_base.wrapping_sub(0x10).wrapping_add(index))?;
        let combined = (step & 0xf0) | self.rb(slot + VOLUME);
        self.db(self.link.envelope_volumes.wrapping_add(combined.into()))
    }

    /// Steps a running slot; returns whether the reserve step follows.
    fn step_slot(&mut self, slot: u16) -> Result<bool> {
        use field::*;
        let channel = self.channel();
        let flags = self.rb(slot + CHANNEL_FLAGS);
        if !self.taken() && self.rb(slot + KEY_ON_COUNTER) == 0 && flags & 0xf0 != 0 {
            let index = (flags & 0xf0).wrapping_sub(0x10) | (self.g(|g| g.phase_counter) & 0xf);
            let delta = self.db(self.link.vibrato_table.wrapping_add(index.into()))? as i8;
            let value = (i16::from(delta) as u16).wrapping_add(self.rw(slot + FREQUENCY));
            self.out16(0x80 + channel * 2, value)?;
        }
        if !self.taken() {
            if self.rw(slot + FREQUENCY) == 0 {
                self.wb(slot + VOLUME_MIRROR, 0);
            }
            if self.rb(slot + ENVELOPE_GROUP) != 0 && self.rb(slot + ENVELOPE_PERIOD) != 0 {
                let value = self.envelope_volume(slot)?;
                self.wb(slot + VOLUME_MIRROR, value);
            }
        }
        let voice_holds_channel = channel == 1 && self.g(|g| g.voice_active) != 0;
        if !self.taken() && !voice_holds_channel {
            let mirror = self.rb(slot + VOLUME_MIRROR);
            let cap = self.rb(slot + VOLUME_CAP);
            let mut high = mirror;
            let mut low = mirror >> 4;
            if high >= cap & 0xf0 {
                high = cap & 0xf0;
            }
            if low >= cap & 0x0f {
                low = cap & 0x0f;
            }
            self.out8(0x88 + channel, low | high)?;
        }
        let period = self.rb(slot + ENVELOPE_PERIOD);
        let mut position = self.rb(slot + ENVELOPE_POSITION).wrapping_add(1);
        if position >= period {
            position = period;
        }
        self.wb(slot + ENVELOPE_POSITION, position);
        let tempo =
            (self.rb(slot + HEADER_FLAGS) & 0xf).wrapping_add(self.rb(slot + TEMPO_ACCUMULATOR));
        if tempo >= 0x10 {
            self.wb(slot + TEMPO_ACCUMULATOR, tempo - 0x10);
            return Ok(true);
        }
        self.wb(slot + TEMPO_ACCUMULATOR, tempo);
        self.volume_slide(slot);
        let counter = self.rb(slot + KEY_ON_COUNTER);
        if counter != 0 {
            self.wb(slot + KEY_ON_COUNTER, counter - 1);
        }
        let duration = self.rb(slot + DURATION).wrapping_sub(1);
        self.wb(slot + DURATION, duration);
        if duration != 0 {
            return Ok(true);
        }
        let track = self.g(|g| g.track_index);
        let bit = if track < 8 { 1u8 << track } else { 0 };
        let expired = self.g(|g| g.expired_mask) | bit;
        self.set_g(|g| g.expired_mask, expired);
        self.fetch_next_note(slot)
    }

    fn fetch_next_note(&mut self, slot: u16) -> Result<bool> {
        let reload = self.rb(slot + field::KEY_ON_RELOAD);
        self.wb(slot + field::KEY_ON_COUNTER, reload);
        self.si = slot;
        self.fetch()?;
        Ok(true)
    }

    /// Reserve the channel; returns true when the mute tail ran (the caller
    /// then advances without the common pointer restore).
    fn reserve_channel(&mut self, slot: u16) -> Result<bool> {
        let bits = self.g(|g| g.channel_bits);
        let taken = self.g(|g| g.taken_mask) | bits;
        self.set_g(|g| g.taken_mask, taken);
        if self.link.mute_tail != MuteTail::None && self.channel() == 2 && self.in16(0x84)? < 0x96 {
            let target = match self.link.mute_tail {
                MuteTail::SavedPointer => slot,
                MuteTail::LivePointer => self.si,
                MuteTail::None => unreachable!(),
            };
            self.ww(target.wrapping_add(field::FREQUENCY), 0);
            self.out16(0x84, 0)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn merge_control(&mut self) -> Result<()> {
        let mut control = self.ports.read(0x90)? | (self.g(|g| g.control_shadow) & 0xf);
        if self.link.merge_voice_active && self.g(|g| g.voice_active) != 0 {
            control |= 0x22;
        }
        self.out8(0x90, control)?;
        if self.g(|g| g.expired_mask) & self.g(|g| g.deferred_mask) != 0 {
            let value = self.g(|g| g.deferred_value) & 0xf;
            let mut mask = self.g(|g| g.deferred_mask);
            let mut target = self.link.deferred_target;
            loop {
                let carry = mask & 1 != 0;
                mask >>= 1;
                self.set_g(|g| g.expired_mask, mask);
                if carry {
                    let byte = (self.rb(target) & 0xf0) | value;
                    self.wb(target, byte);
                }
                target = target.wrapping_add(self.link.slot_stride);
                if mask == 0 {
                    break;
                }
            }
            self.set_g(|g| g.deferred_mask, 0);
        }
        self.set_g(|g| g.expired_mask, 0);
        Ok(())
    }

    /// Volume slide (signed budget, period and divider at +22h..+24h).
    fn volume_slide(&mut self, slot: u16) {
        use field::*;
        let budget = self.rb(slot + SLIDE_BUDGET);
        if budget == 0 {
            return;
        }
        let divider = self.rb(slot + SLIDE_DIVIDER).wrapping_sub(1);
        self.wb(slot + SLIDE_DIVIDER, divider);
        if divider != 0 {
            return;
        }
        let swapped = self.rb(slot + VOLUME).rotate_right(4);
        if swapped >= 0x10 {
            return;
        }
        let level = swapped & 0xf;
        self.wb(slot + SLIDE_DIVIDER, self.rb(slot + SLIDE_PERIOD));
        if budget & 0x80 == 0 {
            self.wb(slot + SLIDE_BUDGET, budget.wrapping_sub(1));
            if level != 0xf {
                let volume = self.rb(slot + VOLUME).wrapping_add(0x10);
                self.wb(slot + VOLUME, volume);
                self.wb(slot + VOLUME_MIRROR, volume);
            }
        } else {
            self.wb(slot + SLIDE_BUDGET, budget.wrapping_add(1));
            if level != 0 {
                let volume = self.rb(slot + VOLUME).wrapping_sub(0x10);
                self.wb(slot + VOLUME, volume);
                self.wb(slot + VOLUME_MIRROR, volume);
            }
        }
    }

    /// Channel-off helper: skipped when the channel is taken this tick.
    fn channel_off(&mut self) -> Result<()> {
        if self.taken() {
            return Ok(());
        }
        let bits = self.g(|g| g.channel_bits);
        let shadow = self.g(|g| g.control_shadow) & !bits;
        self.set_g(|g| g.control_shadow, shadow);
        let mut control = self.ports.read(0x90)? & (!bits | 0xf0);
        if self.g(|g| g.voice_active) != 0 {
            control |= 0x22;
        }
        self.out8(0x90, control)
    }

    fn enable_shadow(&mut self) {
        let bits = self.g(|g| g.channel_bits);
        let shadow = (self.g(|g| g.control_shadow) & !bits) | bits;
        self.set_g(|g| g.control_shadow, shadow);
    }

    /// Cell fetch for the slot in `self.si`, until a note, rest, wait or end.
    fn fetch(&mut self) -> Result<()> {
        use field::*;
        let slot = self.si;
        let mut cells = 0usize;
        'restart: loop {
            let mut bx = self
                .rw(slot + POSITION)
                .wrapping_mul(2)
                .wrapping_add(self.rw(slot + SEQUENCE));
            loop {
                cells += 1;
                if cells > super::MAX_RECORDS {
                    return Err(Error::ResourceLimit {
                        resource: "SDK cells in one fetch",
                        requested: cells as u64,
                        limit: super::MAX_RECORDS as u64,
                    });
                }
                let position = self.rw(slot + POSITION).wrapping_add(1);
                self.ww(slot + POSITION, position);
                let opcode = self.db(bx)?;
                bx = bx.wrapping_add(1);
                let operand = self.db(bx)?;
                match opcode {
                    0x00..=0x9f => return self.note(slot, opcode, operand),
                    0xa0 => {
                        self.wb(slot + VOLUME, operand);
                        if !self.taken() {
                            self.wb(slot + VOLUME_MIRROR, 0);
                        }
                    }
                    0xa1 => {
                        self.wb(slot + SWEEP, operand);
                        if operand == 0 {
                            self.out8(0x8c, 0)?;
                        }
                    }
                    0xa2 => {
                        if self.rb(slot + MODE) != 0 && self.channel() & 1 != 0 {
                            self.wb(slot + NOISE_OR_SAMPLE, operand);
                        } else {
                            self.wb(slot + WAVE_OFFSET, operand << 4);
                        }
                    }
                    0xa3 => {
                        let flags = self.rb(slot + CHANNEL_FLAGS) & 0x0f;
                        if operand == 0xfe {
                            self.wb(slot + CHANNEL_FLAGS, flags);
                        } else {
                            let reload = (operand & 0xf) << 1;
                            self.wb(slot + KEY_ON_RELOAD, reload);
                            self.wb(slot + KEY_ON_COUNTER, reload);
                            let high = operand.wrapping_add(0x10) & 0xf0;
                            self.wb(slot + CHANNEL_FLAGS, flags | high);
                        }
                    }
                    0xa4 => {
                        self.wb(slot + MODE, operand);
                        if !self.link.extended_opcodes && self.channel() == 3 {
                            let control = self.ports.read(0x90)?;
                            let control = if operand != 0 {
                                control | 0x80
                            } else {
                                control & 0x7f
                            };
                            self.out8(0x90, control)?;
                        }
                    }
                    0xa5 => self.wb(slot + VOLUME_CAP, operand),
                    0xa6 if self.link.extended_opcodes => {
                        let control = (self.ports.read(0x91)? & 0xf9) | operand.wrapping_shl(1);
                        self.out8(0x91, control)?;
                    }
                    0xa7 => {
                        self.wb(slot + DURATION, operand);
                        self.enable_shadow();
                        if self.link.extended_opcodes && self.channel() == 3 {
                            self.apply_extended_ch3_noise(slot)?;
                        }
                        return Ok(());
                    }
                    0xa8 if self.link.extended_opcodes => {
                        self.wb(slot + ENVELOPE_BANK, operand);
                    }
                    0xa9 if self.link.extended_opcodes => match operand {
                        0xf0 => {
                            let v = self.rb(slot + BRANCH_INDEX).wrapping_add(1);
                            self.wb(slot + BRANCH_INDEX, v);
                        }
                        0xf1 => {
                            let v = self.rb(slot + BRANCH_INDEX).wrapping_sub(1);
                            self.wb(slot + BRANCH_INDEX, v);
                        }
                        0xfe | 0xff => {
                            let sel = if operand == 0xfe {
                                self.rb(slot + BRANCH_INDEX)
                            } else {
                                self.link.branch_selector.map_or(0, |a| self.rb(a))
                            };
                            let table_off = bx.wrapping_add(1).wrapping_add(u16::from(sel) << 1);
                            let target = self.dw(table_off)?;
                            self.ww(slot + POSITION, target);
                            continue 'restart;
                        }
                        _ => {
                            self.wb(slot + BRANCH_INDEX, operand);
                        }
                    },
                    0xac if self.link.extended_opcodes => {
                        let count = self.rb(slot + SUB_COUNTER);
                        let next = count.wrapping_sub(1);
                        if next == 0 {
                            self.wb(slot + SUB_COUNTER, 0);
                            let pos = self.rw(slot + POSITION).wrapping_add(1);
                            self.ww(slot + POSITION, pos);
                            continue 'restart;
                        }
                        if count == 0 {
                            self.wb(slot + SUB_COUNTER, operand);
                        } else {
                            self.wb(slot + SUB_COUNTER, next);
                        }
                        let ret_pos = self.rw(slot + POSITION).wrapping_sub(1);
                        self.ww(slot + SUB_RETURN, ret_pos);
                        let target = self.dw(bx.wrapping_add(1))? >> 1;
                        self.ww(slot + POSITION, target);
                        continue 'restart;
                    }
                    0xad if self.link.extended_opcodes => {
                        let ret_pos = self.rw(slot + SUB_RETURN);
                        self.ww(slot + POSITION, ret_pos);
                        continue 'restart;
                    }
                    0xae => {
                        let flags = (self.rb(slot + HEADER_FLAGS) & 0xef) | (operand & 0x10);
                        self.wb(slot + HEADER_FLAGS, flags);
                    }
                    0xaf => {
                        let low = operand & 0xf;
                        self.wb(slot + TEMPO_ACCUMULATOR, low);
                        let flags = (self.rb(slot + HEADER_FLAGS) & 0xf0) | low;
                        self.wb(slot + HEADER_FLAGS, flags);
                    }
                    0xb0 => {
                        if operand & 0xf0 == 0xf0 {
                            let area = slot + LOOP_AREA + u16::from((operand & 0xf) * 3);
                            let target = self.rw(area + 1);
                            self.ww(slot + POSITION, target);
                            continue 'restart;
                        }
                        let index = (operand >> 4)
                            .wrapping_add(operand >> 4)
                            .wrapping_add(operand);
                        self.wb(slot + LOOP_AREA + u16::from(index), operand & 0xf);
                    }
                    0xb1..=0xbf => {
                        let area = slot + LOOP_AREA + u16::from((operand & 0xf).wrapping_mul(3));
                        let mut count = self.rb(area);
                        if count == 0 {
                            count = (opcode & 0xf) + 1;
                        }
                        count = count.wrapping_sub(1);
                        self.wb(area, count);
                        if count != 0 {
                            let target = self.rw(area + 1);
                            self.ww(slot + POSITION, target);
                        }
                        continue 'restart;
                    }
                    0xc0..=0xcf => {
                        self.wb(slot + ENVELOPE_PERIOD, operand);
                        self.wb(slot + ENVELOPE_GROUP, (opcode & 0xf) << 4);
                    }
                    0xd0..=0xef => {
                        let amount = if opcode >= 0xe0 {
                            (opcode & 0xf).wrapping_neg()
                        } else {
                            opcode & 0xf
                        };
                        self.wb(slot + SLIDE_BUDGET, amount);
                        self.wb(slot + SLIDE_PERIOD, operand);
                        self.wb(slot + SLIDE_DIVIDER, operand);
                    }
                    0xfd => {
                        let area = slot + LOOP_AREA + u16::from((operand & 0xf) * 3) + 1;
                        let position = self.rw(slot + POSITION);
                        self.ww(area, position);
                    }
                    0xff => {
                        self.ww(slot + POSITION, 0xffff);
                        return self.channel_off();
                    }
                    _ => {}
                }
                bx = bx.wrapping_add(1);
            }
        }
    }

    fn note(&mut self, slot: u16, opcode: u8, duration: u8) -> Result<()> {
        use field::*;
        let channel = self.channel();
        if self.rb(slot + MODE) != 0 && channel == 1 {
            let sample_id = u16::from(self.rb(slot + NOISE_OR_SAMPLE));
            self.clear_voice();
            return self.start_voice(sample_id, opcode);
        }
        self.wb(slot + DURATION, duration);
        let class = opcode & 0xf;
        if class >= 0xc {
            self.ww(slot + FREQUENCY, 0);
            if self.link.extended_opcodes || !self.taken() {
                self.wb(slot + VOLUME_MIRROR, 0);
            }
            return Ok(());
        }
        let mut index = class << 1;
        if self.rb(slot + HEADER_FLAGS) & 0x10 != 0 {
            index = index.wrapping_add(0x18);
        }
        let mut period = self.dw(self.link.pitch_table.wrapping_add(index.into()))?;
        let octave = opcode >> 4;
        for _ in 0..octave {
            period >>= 1;
        }
        let divisor = 0x800u16.wrapping_sub(period);
        self.wb(slot + ENVELOPE_POSITION, 0);
        if self.taken() {
            return Ok(());
        }
        let shadow = self.link.wave_shadow.wrapping_add(channel.into());
        let offset = self.rb(slot + WAVE_OFFSET);
        if offset != self.rb(shadow) {
            self.wb(shadow, offset);
            self.copy_wave(channel, offset)?;
        }
        if self.rb(slot + ENVELOPE_GROUP) != 0 && self.rb(slot + ENVELOPE_PERIOD) != 0 {
            let value = self.envelope_volume(slot)?;
            self.wb(slot + VOLUME_MIRROR, value);
        }
        self.wb(slot + VOLUME_MIRROR, self.rb(slot + VOLUME).rotate_right(4));
        if !self.link.extended_opcodes && channel == 3 && self.rb(slot + MODE) != 0 {
            self.out8(0x8e, self.rb(slot + NOISE_OR_SAMPLE) | 0x18)?;
        }
        self.out16(0x80 + channel * 2, divisor)?;
        self.ww(slot + FREQUENCY, divisor);
        if channel == 2 {
            let sweep = self.rb(slot + SWEEP);
            let mut amount = 0;
            if sweep != 0 {
                self.out8(0x8d, sweep >> 4)?;
                amount = if self.link.inline_sweep {
                    let mag = sweep & 7;
                    if sweep & 8 != 0 {
                        mag.wrapping_neg()
                    } else {
                        mag
                    }
                } else {
                    let offset = u16::from(sweep & 0xf).wrapping_sub(1);
                    self.si = self.link.sweep_table.wrapping_add(offset);
                    self.db(self.si)?
                };
            }
            self.out8(0x8c, amount)?;
        }
        self.enable_shadow();
        if self.link.extended_opcodes && channel == 3 {
            self.apply_extended_ch3_noise(slot)?;
        }
        Ok(())
    }

    fn apply_extended_ch3_noise(&mut self, slot: u16) -> Result<()> {
        if self.rb(slot + field::MODE) != 0 {
            self.out8(0x8e, self.rb(slot + field::NOISE_OR_SAMPLE) | 0x18)?;
            let control = self.ports.read(0x90)? | 0x80;
            self.out8(0x90, control)
        } else {
            let control = self.ports.read(0x90)? & 0x7f;
            self.out8(0x90, control)
        }
    }
}

/// Outcome of replaying a captured schedule through the model. Stream
/// equality is meaningful only for a run that completed every captured tick
/// without a model error; [`OracleVerdict::passed`] requires all of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OracleVerdict {
    pub oracle_ticks: u32,
    pub model_ticks: u32,
    pub model_error: Option<String>,
    pub ports_equal: bool,
    pub waves_equal: bool,
    pub ram_equal: bool,
}

impl OracleVerdict {
    /// The model ran to the end of the capture without an error.
    pub fn complete(&self) -> bool {
        self.model_error.is_none() && self.model_ticks == self.oracle_ticks
    }
    pub fn port_exact(&self) -> bool {
        self.complete() && self.ports_equal
    }
    pub fn wave_exact(&self) -> bool {
        self.complete() && self.waves_equal
    }
    pub fn ram_exact(&self) -> bool {
        self.complete() && self.ram_equal
    }
    pub fn passed(&self) -> bool {
        self.port_exact() && self.wave_exact() && self.ram_exact()
    }
}
