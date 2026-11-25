//! Tick-level model of the four-byte-cell ADSR sound driver.
//!
//! Models the eight-slot (four music + four SFX overlay) four-byte-cell driver
//! with queue commands, four-stage ADSR envelopes, 16x16 volume-curve lookup,
//! waveform RAM reloads, channel-4 noise control, and SFX masking/restoration.
//! Linked table locations are caller input ([`CellLink`]). Hardware register
//! rules follow WSdev Sound.
use crate::drivers::{
    cell::Song,
    sdk_driver::{PortLatch, Target, Write, MAX_WRITES},
    RomView,
};
use formatkit_core::{Error, Result};
use serde::{Deserialize, Serialize};
use ws_rom::BankState;

/// Linked locations of one four-byte-cell driver build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellLink {
    /// Segment containing the driver tables (e.g. `0xD000`).
    pub driver_segment: u16,
    /// Offset of the song-group directory in `driver_segment`.
    pub directory: u16,
    /// Number of 4-byte `{segment, first_id, pad}` directory groups.
    pub group_count: u8,
    /// Offset of the 16-byte waveform bank in `driver_segment`.
    pub wave_table: u16,
    /// Offset of the 256-byte 16x16 volume curve in `driver_segment`.
    pub volume_curve: u16,
    /// Data-segment RAM base of the 64-byte waveform block (must be 64-byte aligned).
    pub wave_ram: u16,
}

/// Queue command dispatched by the driver's frame-start queue processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CellCommand {
    Nop,
    Play { song_id: u8 },
    Stop,
    Mute,
    Unmute,
    SetVolume { volume: u8 },
}

#[derive(Debug, Clone, Copy, Default)]
struct CellSlot {
    pos: u16,
    duration: u16,
    track_base: u16,
    vol_port: u8,
    freq_port: u8,
    tempo: u16,
    env_ptr: u16,
    env_vol: u8,
    env_phase: u8,
    env_step: u8,
    env_timer: u8,
    direct_pitch: u8,
    masked: u8,
    key_vol: u8,
    wave_id: u8,
    segment: u16,
    vol_adj_l: i8,
    vol_adj_r: i8,
    last_divisor: u16,
}

/// Tick-level model of the four-byte-cell sound driver.
#[derive(Debug, Clone)]
pub struct CellDriver<'a> {
    pub link: CellLink,
    view: RomView<'a>,
    slots: [CellSlot; 8],
    pub ports: PortLatch,
    pub writes: Vec<Write>,
    pub ticks: u32,
    ctrl_shadow: u8,
    saved_ctrl: u8,
    saved_noise: u8,
    master_vol: u8,
    target_vol: u8,
    mute_state: u8,
    balance_l: i8,
    balance_r: i8,
}

impl<'a> CellDriver<'a> {
    pub fn new(link: CellLink, rom: &'a [u8], banks: BankState) -> Result<Self> {
        if link.group_count == 0 {
            return Err(Error::Malformed("cell driver group_count is zero".into()));
        }
        let view = RomView::new(rom, banks)?;
        let mut slots = [CellSlot::default(); 8];
        for (i, slot) in slots.iter_mut().enumerate() {
            let ch = (i & 3) as u8;
            slot.vol_port = 0x88 + ch;
            slot.freq_port = 0x80 + ch * 2;
            slot.track_base = 0xffff;
            slot.key_vol = 0xff;
            slot.wave_id = 0xff;
        }
        Ok(Self {
            link,
            view,
            slots,
            ports: PortLatch::new(true),
            writes: Vec::new(),
            ticks: 0,
            ctrl_shadow: 0x0f,
            saved_ctrl: 0x0f,
            saved_noise: 0,
            master_vol: 0x0f,
            target_vol: 0x0f,
            mute_state: 0,
            balance_l: 0,
            balance_r: 0,
        })
    }

    fn record(&mut self, target: Target, value: u16) -> Result<()> {
        if self.writes.len() >= MAX_WRITES {
            return Err(Error::ResourceLimit {
                resource: "cell driver writes",
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
        self.record(Target::Port(port), u16::from(value))
    }

    fn out16(&mut self, port: u8, value: u16) -> Result<()> {
        self.out8(port, value as u8)?;
        self.out8(port.wrapping_add(1), (value >> 8) as u8)
    }

    fn seg_u8(&self, segment: u16, offset: u16) -> Result<u8> {
        Ok(self.view.span(segment, offset, 1)?[0])
    }

    fn seg_u16(&self, segment: u16, offset: u16) -> Result<u16> {
        let b = self.view.span(segment, offset, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn copy_wave(&mut self, channel: usize, wave_id: u8) -> Result<()> {
        let src_off = self
            .link
            .wave_table
            .wrapping_add(u16::from(wave_id).wrapping_mul(16));
        let bytes = self.view.span(self.link.driver_segment, src_off, 16)?;
        let dst_base = self.link.wave_ram.wrapping_add((channel as u16) * 16);
        for w in 0..8u16 {
            let val =
                u16::from_le_bytes([bytes[usize::from(w * 2)], bytes[usize::from(w * 2 + 1)]]);
            self.record(Target::WaveWord(dst_base.wrapping_add(w * 2)), val)?;
        }
        Ok(())
    }

    /// Driver initialization (`snd_init`): loads wave 0 to all 4 channels,
    /// sets wave base / output / zero volumes / control = `0x0f`, and resets slots.
    pub fn init(&mut self) -> Result<()> {
        for ch in 0..4 {
            self.slots[ch].wave_id = 0;
            self.copy_wave(ch, 0)?;
        }
        self.out8(0x8f, (self.link.wave_ram >> 6) as u8)?;
        self.out8(0x91, 0x0f)?;
        for p in 0x88..=0x8b {
            self.out8(p, 0)?;
        }
        self.out8(0x90, 0x0f)?;
        self.ctrl_shadow = 0x0f;
        self.master_vol = 0x0f;
        self.target_vol = 0x0f;
        self.mute_state = 0;
        self.balance_l = 0;
        self.balance_r = 0;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let ch = (i & 3) as u8;
            *slot = CellSlot {
                vol_port: 0x88 + ch,
                freq_port: 0x80 + ch * 2,
                track_base: 0xffff,
                key_vol: 0xff,
                wave_id: 0xff,
                ..CellSlot::default()
            };
        }
        Ok(())
    }

    /// Stop all music and SFX slots (`snd_cmd_stop`).
    pub fn stop(&mut self) -> Result<()> {
        self.out8(0x94, 0)?;
        for p in 0x88..=0x8b {
            self.out8(p, 0)?;
        }
        for slot in &mut self.slots {
            slot.pos = 0;
            slot.duration = 0;
            slot.masked = 0;
            slot.direct_pitch = 0;
            slot.key_vol = 0xff;
            slot.track_base = 0xffff;
        }
        self.ctrl_shadow &= 0x5f;
        self.out8(0x90, self.ctrl_shadow)
    }

    /// Resolve a song ID through the 4-byte group directory and start it (`snd_cmd_play`).
    pub fn play(&mut self, song_id: u8) -> Result<()> {
        let dir_bytes = self.view.span(
            self.link.driver_segment,
            self.link.directory,
            usize::from(self.link.group_count) * 4,
        )?;
        let mut chosen = None;
        for g in 0..usize::from(self.link.group_count) {
            let seg = u16::from_le_bytes([dir_bytes[g * 4], dir_bytes[g * 4 + 1]]);
            let base = dir_bytes[g * 4 + 2];
            let bound = if g + 1 < usize::from(self.link.group_count) {
                u16::from(dir_bytes[(g + 1) * 4 + 2])
            } else {
                0x100
            };
            if u16::from(song_id) < bound {
                chosen = Some((seg, base));
                break;
            }
        }
        let (seg, base) = chosen.ok_or_else(|| Error::InvalidField {
            what: "cell song id",
            value: song_id.into(),
        })?;
        let idx = u16::from(song_id.wrapping_sub(base));
        let entry_off = idx.wrapping_mul(13);
        let song = Song::parse(self.view.span(seg, entry_off, 13)?)?;

        // Check for PCM entry (word at tracks[0] == 0x8003).
        if self.seg_u16(seg, song.tracks[0])? == 0x8003 {
            if song.flag != 0 {
                self.slots[1].masked = 1;
            }
            self.ctrl_shadow |= 0x20;
            self.out8(0x90, self.ctrl_shadow)?;
            let vctrl = if self.master_vol != 0 { 0x0f } else { 0 };
            self.out8(0x94, vctrl)?;
            return Ok(());
        }

        let base_slot = if song.flag == 0 {
            self.stop()?;
            0usize
        } else {
            for k in 0..4 {
                self.slots[4 + k].pos = 0;
                self.slots[4 + k].duration = 0;
                self.slots[k].masked = 0;
            }
            for p in 0x88..=0x8b {
                self.out8(p, 0)?;
            }
            self.saved_ctrl = self.ports.read(0x90)?;
            4usize
        };

        let waves = song.waves(self.view, seg)?;
        for (k, &wave) in waves.iter().enumerate() {
            let s = &mut self.slots[base_slot + k];
            s.track_base = song.tracks[k];
            s.tempo = song.tempo;
            s.segment = seg;
            s.wave_id = wave;
        }
        for (k, &wave) in waves.iter().enumerate() {
            self.copy_wave(k, wave)?;
        }
        for k in 0..4 {
            let tb = {
                let s = &mut self.slots[base_slot + k];
                s.env_ptr = 0;
                s.env_phase = 0;
                s.env_step = 0;
                s.env_timer = 0;
                s.track_base
            };
            if self.seg_u16(seg, tb.wrapping_add(4))? == 0xffff {
                self.slots[base_slot + k].track_base = 0xffff;
            }
        }
        for k in 0..4 {
            if self.slots[4 + k].track_base != 0xffff {
                self.slots[k].masked = 1;
            }
        }
        Ok(())
    }

    /// Execute one queued command.
    pub fn dispatch(&mut self, cmd: CellCommand) -> Result<()> {
        match cmd {
            CellCommand::Nop => Ok(()),
            CellCommand::Play { song_id } => self.play(song_id),
            CellCommand::Stop => self.stop(),
            CellCommand::Mute => {
                self.mute_state = 2;
                Ok(())
            }
            CellCommand::Unmute => {
                self.mute_state = 0;
                Ok(())
            }
            CellCommand::SetVolume { volume } => {
                self.master_vol = volume & 0x0f;
                self.target_vol = volume & 0x0f;
                Ok(())
            }
        }
    }

    /// Advance the driver by one frame tick (`snd_tick`).
    pub fn tick(&mut self) -> Result<()> {
        if self.mute_state == 1 {
            self.ticks = self.ticks.wrapping_add(1);
            return Ok(());
        }
        for idx in 0..8 {
            if self.slots[idx].track_base == 0xffff {
                continue;
            }
            if self.slots[idx].duration != 0xffff {
                if self.slots[idx].duration == 0 {
                    self.fetch_cell(idx)?;
                } else {
                    self.slots[idx].duration = self.slots[idx].duration.wrapping_sub(1);
                }
            }
            self.channel_tick(idx)?;
        }
        self.ticks = self.ticks.wrapping_add(1);
        Ok(())
    }

    fn fetch_cell(&mut self, idx: usize) -> Result<()> {
        let mut steps = 0usize;
        loop {
            steps += 1;
            if steps > super::MAX_RECORDS {
                return Err(Error::ResourceLimit {
                    resource: "cell driver commands in one tick",
                    requested: steps as u64,
                    limit: super::MAX_RECORDS as u64,
                });
            }
            let seg = self.slots[idx].segment;
            let base = self.slots[idx].track_base;
            let pos = self.slots[idx].pos.wrapping_add(4);
            self.slots[idx].pos = pos;
            let at = base.wrapping_add(pos);
            let b0 = self.seg_u8(seg, at)?;
            let b1 = self.seg_u8(seg, at.wrapping_add(1))?;
            let w0 = u16::from_le_bytes([b0, b1]);
            let w2 = self.seg_u16(seg, at.wrapping_add(2))?;

            if b1 == 0x80 && w0 != 0xffff {
                let cmd = w0 & 0x7fff;
                match cmd {
                    2 => {
                        self.slots[idx].env_ptr =
                            w2.wrapping_shl(4).wrapping_add(self.slots[idx].tempo);
                        continue;
                    }
                    5 => {
                        if self.slots[idx].masked == 0 {
                            self.ctrl_shadow |= 0x80;
                            self.out8(0x90, self.ctrl_shadow)?;
                            let c = self.ports.read(0x90)? | 0x80;
                            self.out8(0x90, c)?;
                        } else {
                            self.saved_ctrl |= 0x80;
                        }
                        continue;
                    }
                    6 => {
                        if self.slots[idx].masked == 0 {
                            self.ctrl_shadow &= 0x7f;
                            self.out8(0x90, self.ctrl_shadow)?;
                        } else {
                            self.saved_ctrl &= 0x7f;
                        }
                        continue;
                    }
                    7 => {
                        if self.slots[idx].masked == 0 {
                            self.out8(0x8e, (w2 as u8) | 0xf8)?;
                        } else {
                            self.saved_noise = (w2 as u8) & 7;
                        }
                        continue;
                    }
                    8 => {
                        self.slots[idx].direct_pitch = 1;
                        continue;
                    }
                    9 => {
                        self.slots[idx].direct_pitch = 0;
                        continue;
                    }
                    10 => {
                        let wave_id = w2 as u8;
                        self.slots[idx].wave_id = wave_id;
                        self.copy_wave(idx & 3, wave_id)?;
                        continue;
                    }
                    11 => {
                        self.slots[idx].vol_adj_l = (w2 & 0xff) as i8;
                        self.slots[idx].vol_adj_r = (w2 >> 8) as i8;
                        continue;
                    }
                    _ => {
                        // Commands 0, 1, 3, 4 alias to the note path.
                    }
                }
            }

            // Note / release / jump / end path.
            self.note_trigger(idx, b1, w2)?;
            self.slots[idx].last_divisor = w2;
            let dur = if b0 != 0 { u16::from(b0 - 1) } else { 0 };
            self.slots[idx].duration = dur;
            if (dur as u8) == 0xfe {
                if w2 == 0xffff {
                    self.slots[idx].duration = 0xffff;
                    return Ok(());
                }
                self.slots[idx].pos = w2;
                self.slots[idx].duration = 0;
                continue;
            }
            return Ok(());
        }
    }

    fn note_trigger(&mut self, idx: usize, key: u8, divisor: u16) -> Result<()> {
        if key == 0xff || key == 0x00 {
            if self.slots[idx].env_phase == 3 {
                return Ok(());
            }
            self.slots[idx].env_phase = 3;
        } else {
            if self.slots[idx].masked == 1 {
                return Ok(());
            }
            if self.slots[idx].direct_pitch == 1 {
                let port = self.slots[idx].freq_port;
                self.out16(port, divisor)?;
                return Ok(());
            }
            self.slots[idx].key_vol = key;
            self.slots[idx].env_vol = 0;
            self.slots[idx].env_phase = 0;
            let port = self.slots[idx].freq_port;
            self.out16(port, divisor)?;
        }
        self.slots[idx].env_step = 0;
        self.slots[idx].env_timer = 0;
        Ok(())
    }

    fn channel_tick(&mut self, idx: usize) -> Result<()> {
        let env_ptr = self.slots[idx].env_ptr;
        if env_ptr == 0 {
            return Ok(());
        }
        let seg = self.slots[idx].segment;
        let phase = self.slots[idx].env_phase;
        let desc = env_ptr.wrapping_add(u16::from(phase.wrapping_shl(2)));
        let updated_vol: Option<u8> = match phase {
            0 => {
                // Attack (`snd_env_attack`)
                let period = self.seg_u8(seg, desc)?;
                let step_mask = self.seg_u8(seg, desc.wrapping_add(1))?;
                let count = self.seg_u8(seg, desc.wrapping_add(2))?;
                let s = &mut self.slots[idx];
                if s.env_step == count {
                    s.env_phase = s.env_phase.wrapping_add(1);
                    s.env_timer = 0;
                    s.env_step = 0;
                    Some(s.key_vol)
                } else if s.env_timer == 0 {
                    let next = s.env_vol.wrapping_add(step_mask & s.key_vol);
                    s.env_step = s.env_step.wrapping_add(1);
                    s.env_timer = s.env_timer.wrapping_add(1);
                    Some(next)
                } else {
                    if s.env_timer == period {
                        s.env_timer = 0;
                    } else {
                        s.env_timer = s.env_timer.wrapping_add(1);
                    }
                    None
                }
            }
            1 => {
                // Decay (`snd_env_decay`)
                let period = self.seg_u8(seg, desc)?;
                let step = self.seg_u8(seg, desc.wrapping_add(1))?;
                let count = self.seg_u8(seg, desc.wrapping_add(2))?;
                let s = &mut self.slots[idx];
                if s.env_step == count {
                    s.env_phase = s.env_phase.wrapping_add(1);
                    s.env_timer = 0;
                    s.env_step = 0;
                    None
                } else if s.env_timer == period {
                    let mut next = s.env_vol.wrapping_sub(step);
                    if next & 0x88 != 0 {
                        next = 0;
                    }
                    s.env_step = s.env_step.wrapping_add(1);
                    s.env_timer = 0;
                    Some(next)
                } else {
                    s.env_timer = s.env_timer.wrapping_add(1);
                    None
                }
            }
            2 => {
                // Sustain (`snd_env_sustain`)
                let period = self.seg_u8(seg, desc)?;
                let step = self.seg_u8(seg, desc.wrapping_add(1))?;
                let count = self.seg_u8(seg, desc.wrapping_add(2))?;
                let s = &mut self.slots[idx];
                if count == 0 {
                    s.env_phase = 4;
                    None
                } else if s.env_timer == period {
                    if s.env_vol == 0 {
                        None
                    } else {
                        let mut next = s.env_vol.wrapping_sub(step);
                        if next & 0x88 != 0 {
                            next = 0;
                        }
                        s.env_step = s.env_step.wrapping_add(1);
                        s.env_timer = 0;
                        Some(next)
                    }
                } else {
                    s.env_timer = s.env_timer.wrapping_add(1);
                    None
                }
            }
            3 => {
                // Release (`snd_env_release`)
                let period = self.seg_u8(seg, desc)?;
                let step = self.seg_u8(seg, desc.wrapping_add(1))?;
                let count = self.seg_u8(seg, desc.wrapping_add(2))?;
                let s = &mut self.slots[idx];
                if count == 0 {
                    s.env_phase = 4;
                    None
                } else if s.env_timer == period {
                    let mut next = s.env_vol;
                    if next != 0 {
                        next = next.wrapping_sub(step);
                        if next & 0x88 != 0 {
                            next = 0;
                        }
                    }
                    s.env_step = s.env_step.wrapping_add(1);
                    s.env_timer = 0;
                    Some(next)
                } else {
                    s.env_timer = s.env_timer.wrapping_add(1);
                    None
                }
            }
            4 => {
                // Note end (`snd_note_end`): finish track and restore masked music channel if SFX.
                if self.slots[idx].duration == 0xffff {
                    self.slots[idx].track_base = 0xffff;
                    if idx >= 4 {
                        let music = idx - 4;
                        self.slots[music].masked = 0;
                        let wave_id = self.slots[music].wave_id;
                        self.copy_wave(music, wave_id)?;
                        let freq_port = self.slots[music].freq_port;
                        let divisor = self.slots[music].last_divisor;
                        self.out16(freq_port, divisor)?;
                        if idx == 7 {
                            self.out8(0x90, self.saved_ctrl)?;
                            self.out8(0x8e, self.saved_noise)?;
                        }
                    }
                }
                None
            }
            _ => None,
        };

        if let Some(vol) = updated_vol {
            self.slots[idx].env_vol = vol;
            let out_vol = self.calc_volume(idx, vol)?;
            if self.slots[idx].masked != 1 {
                let port = self.slots[idx].vol_port;
                self.out8(port, out_vol)?;
            }
            if out_vol == 0 {
                self.slots[idx].env_phase = 4;
            }
        }
        Ok(())
    }

    fn calc_volume(&self, idx: usize, raw: u8) -> Result<u8> {
        let s = &self.slots[idx];
        if s.masked == 1 || self.mute_state == 2 {
            return Ok(0);
        }
        let row = u16::from(self.master_vol) << 4;
        let hi_in = u16::from(raw >> 4);
        let lo_in = u16::from(raw & 0x0f);
        let curve_hi = self.seg_u8(
            self.link.driver_segment,
            self.link.volume_curve.wrapping_add(row + hi_in),
        )? as i8;
        let curve_lo = self.seg_u8(
            self.link.driver_segment,
            self.link.volume_curve.wrapping_add(row + lo_in),
        )? as i8;
        let mut l = curve_hi
            .wrapping_add(s.vol_adj_l)
            .wrapping_add(self.balance_l);
        if l < 0 {
            l = 0;
        }
        let mut r = curve_lo
            .wrapping_add(s.vol_adj_r)
            .wrapping_add(self.balance_r);
        if r < 0 {
            r = 0;
        }
        Ok((((l as u8) & 0x0f) << 4) | ((r as u8) & 0x0f))
    }
}
