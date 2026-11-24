//! Deterministic point-sampled headphone preview at 24 kHz (128 cycles/frame).
use crate::{
    hardware::{HyperVoice, Registers, OUTPUT_HZ},
    WaveBank,
};
use formatkit_core::{Error, Pcm, Result};

/// Hard output bound: ten minutes at 24 kHz, stereo (57.6 MB).
pub const MAX_FRAMES: usize = OUTPUT_HZ as usize * 600;
pub const MAX_EVENTS: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Write { port: u8, value: u8 },
    Waves(WaveBank),
    HyperControl(u16),
    HyperInput { value: u8, dma: bool },
    HyperOutput { left: i16, right: i16 },
}

/// Writes at a frame boundary, applied in slice order before that frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub frame: usize,
    pub action: Action,
}

/// How a sample period counter treats a divisor change. WSdev Sound states
/// the period but not the counter's reload behaviour; both readings are kept
/// explicit for comparison against recorded output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CounterReload {
    /// Elapsed cycles carry over and are compared with the current period.
    #[default]
    Remainder,
    /// A down-counter finishes its current period; a new divisor applies at
    /// the next reload.
    NextPeriod,
}

/// A write or update at an exact CPU cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleEvent {
    pub cycle: u64,
    pub action: Action,
}

#[derive(Debug, Clone, Default)]
pub struct Synth {
    pub registers: Registers,
    pub waves: WaveBank,
    pub hyper: HyperVoice,
    elapsed: [u32; 4],
    preview_phase: [f64; 4],
    index: [usize; 4],
    sweep_cycles: u32,
    sweep_ticks: u8,
    countdown: [u32; 4],
}

impl Synth {
    pub fn apply(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Write { port, value } => {
                self.registers.write(port, value)?;
                if port == 0x8d {
                    self.sweep_ticks = 0;
                }
            }
            Action::Waves(waves) => self.waves = waves,
            Action::HyperControl(value) => self.hyper.write_control(value),
            Action::HyperInput { value, dma } => self.hyper.input(value, dma)?,
            Action::HyperOutput { left, right } => {
                self.hyper.left = left;
                self.hyper.right = right;
            }
        }
        Ok(())
    }

    pub fn frame(&mut self) -> Result<[i16; 2]> {
        self.frame_inner(false)
    }

    fn frame_inner(&mut self, reference_phase: bool) -> Result<[i16; 2]> {
        let control = self.registers.control();
        if control & 0x44 == 0x44 {
            self.sweep_cycles += 128;
            if self.sweep_cycles >= 8192 {
                self.sweep_cycles -= 8192;
                self.sweep_step()?;
            }
        }
        for channel in 0..4 {
            if control & (1 << channel) != 0 {
                let period = u32::from(self.registers.period(channel)?);
                let mut steps = 0;
                if reference_phase {
                    self.preview_phase[channel] += 128.0 / f64::from(period);
                    while self.preview_phase[channel] >= 1.0 {
                        self.preview_phase[channel] -= 1.0;
                        steps += 1;
                    }
                } else {
                    self.elapsed[channel] += 128;
                    steps = self.elapsed[channel] / period;
                    self.elapsed[channel] %= period;
                }
                for _ in 0..steps {
                    self.step_index(channel);
                }
            }
        }
        self.mix()
    }

    fn sweep_step(&mut self) -> Result<()> {
        self.sweep_ticks += 1;
        if self.sweep_ticks > self.registers.read(0x8d)? {
            self.sweep_ticks = 0;
            let value = (i32::from(self.registers.divisor(2)?)
                + i32::from(self.registers.read(0x8c)? as i8))
                & 0x7ff;
            self.registers.set_divisor(2, value as u16)?;
        }
        Ok(())
    }

    fn step_index(&mut self, channel: usize) {
        self.index[channel] = (self.index[channel] + 1) & 31;
        if channel == 3 {
            self.registers.noise.clock(true);
        }
    }

    /// Advance the sample period counters by `cycles` CPU cycles (WSdev Sound:
    /// the index advances every `2048 - divisor` cycles while enabled).
    pub fn advance_cycles(&mut self, cycles: u32, reload: CounterReload) -> Result<()> {
        let control = self.registers.control();
        if control & 0x44 == 0x44 {
            self.sweep_cycles += cycles;
            while self.sweep_cycles >= 8192 {
                self.sweep_cycles -= 8192;
                self.sweep_step()?;
            }
        }
        for channel in 0..4 {
            if control & (1 << channel) == 0 {
                continue;
            }
            let period = u32::from(self.registers.period(channel)?);
            match reload {
                CounterReload::Remainder => {
                    self.elapsed[channel] += cycles;
                    let steps = self.elapsed[channel] / period;
                    self.elapsed[channel] %= period;
                    for _ in 0..steps {
                        self.step_index(channel);
                    }
                }
                CounterReload::NextPeriod => {
                    let mut left = cycles;
                    if self.countdown[channel] == 0 {
                        self.countdown[channel] = period;
                    }
                    while left >= self.countdown[channel] {
                        left -= self.countdown[channel];
                        self.step_index(channel);
                        self.countdown[channel] = u32::from(self.registers.period(channel)?);
                    }
                    self.countdown[channel] -= left;
                }
            }
        }
        Ok(())
    }

    /// Current headphone output (mixing rules as in [`Synth::frame`]).
    pub fn mix(&self) -> Result<[i16; 2]> {
        let control = self.registers.control();
        let mut sums = [0u16; 2];
        for channel in 0..4 {
            if channel == 1 && control & 0x20 != 0 {
                let (left, right) = self.registers.voice_output();
                sums[0] += left;
                sums[1] += right;
            } else if control & (1 << channel) != 0 {
                let sample = if channel == 3 && control & 0x80 != 0 {
                    self.registers.noise.sample()
                } else {
                    self.waves.0[channel].samples()[self.index[channel]]
                };
                let (left, right) = self.registers.volume(channel)?;
                sums[0] += u16::from(sample) * u16::from(left);
                sums[1] += u16::from(sample) * u16::from(right);
            }
        }
        let headphone_enabled = self.registers.read(0x91)? & 8 != 0;
        Ok(std::array::from_fn(|side| {
            if !headphone_enabled {
                return 0;
            }
            let base = i32::from((sums[side] & 0x3ff) << 5);
            let hyper = if side == 0 {
                self.hyper.left
            } else {
                self.hyper.right
            };
            (base + i32::from(hyper)) as i16
        }))
    }
}

/// Event sequence to PCM. No implicit clipping, DC filter, resampling or DMA.
pub fn render(events: &[Event], frames: usize) -> Result<Pcm> {
    render_inner(events, frames, false)
}

/// Listening preview with the reference tool's floating sample phase and
/// one-pole DC filter (0.0005). These are preview conventions, not hardware.
pub fn render_coupled(events: &[Event], frames: usize) -> Result<Pcm> {
    render_inner(events, frames, true)
}

fn render_inner(events: &[Event], frames: usize, coupled: bool) -> Result<Pcm> {
    if frames > MAX_FRAMES {
        return Err(Error::ResourceLimit {
            resource: "PCM frames",
            requested: frames as u64,
            limit: MAX_FRAMES as u64,
        });
    }
    if events.len() > MAX_EVENTS {
        return Err(Error::ResourceLimit {
            resource: "sound events",
            requested: events.len() as u64,
            limit: MAX_EVENTS as u64,
        });
    }
    let mut previous = 0;
    for event in events {
        if event.frame < previous || event.frame >= frames {
            return Err(Error::Malformed(
                "sound events must be ordered and inside output extent".into(),
            ));
        }
        previous = event.frame;
    }
    let mut synth = Synth::default();
    let mut samples = Vec::with_capacity(frames * 2);
    let mut position = 0;
    let mut dc = [0.0f64; 2];
    for frame in 0..frames {
        while position < events.len() && events[position].frame == frame {
            synth.apply(events[position].action)?;
            position += 1;
        }
        let mut pair = synth.frame_inner(coupled)?;
        if coupled {
            for side in 0..2 {
                let raw = f64::from(pair[side]);
                dc[side] += (raw - dc[side]) * 0.0005;
                pair[side] = (raw - dc[side]).clamp(-32768.0, 32767.0) as i16;
            }
        }
        samples.extend(pair);
    }
    Pcm::try_interleaved(OUTPUT_HZ, 2, samples)
}

/// Unsigned 8-bit sample extraction; the driver supplies the actual rate.
pub fn unsigned_pcm(bytes: &[u8], sample_rate: u32, max_samples: usize) -> Result<Pcm> {
    if sample_rate == 0 {
        return Err(Error::InvalidField {
            what: "PCM sample rate",
            value: 0,
        });
    }
    let limit = max_samples.min(MAX_FRAMES);
    if bytes.len() > limit {
        return Err(Error::ResourceLimit {
            resource: "PCM samples",
            requested: bytes.len() as u64,
            limit: limit as u64,
        });
    }
    Ok(Pcm::new(
        sample_rate,
        bytes.iter().map(|&s| (i16::from(s) - 128) * 256).collect(),
    ))
}

/// Cycle-timed rendering: each event is applied at its own CPU cycle and the
/// output is sampled at the end of every 128-cycle frame (24 kHz).
pub fn render_cycles(events: &[CycleEvent], frames: usize, reload: CounterReload) -> Result<Pcm> {
    if frames > MAX_FRAMES {
        return Err(Error::ResourceLimit {
            resource: "PCM frames",
            requested: frames as u64,
            limit: MAX_FRAMES as u64,
        });
    }
    if events.len() > MAX_EVENTS {
        return Err(Error::ResourceLimit {
            resource: "sound events",
            requested: events.len() as u64,
            limit: MAX_EVENTS as u64,
        });
    }
    if events.windows(2).any(|w| w[1].cycle < w[0].cycle) {
        return Err(Error::Malformed("cycle events must be ordered".into()));
    }
    let mut synth = Synth::default();
    let mut samples = Vec::with_capacity(frames * 2);
    let mut position = 0;
    let mut now = 0u64;
    for frame in 0..frames {
        let end = (frame as u64 + 1) * 128;
        while position < events.len() && events[position].cycle < end {
            let at = events[position].cycle.max(now);
            synth.advance_cycles((at - now) as u32, reload)?;
            now = at;
            synth.apply(events[position].action)?;
            position += 1;
        }
        synth.advance_cycles((end - now) as u32, reload)?;
        now = end;
        samples.extend(synth.mix()?);
    }
    Pcm::try_interleaved(OUTPUT_HZ, 2, samples)
}
