//! Sound register policy from WSdev Sound and Hyper Voice; see README sources.
use formatkit_core::{bytes_at, Error, Result};

pub const CPU_HZ: u32 = 3_072_000;
pub const OUTPUT_HZ: u32 = 24_000;
pub const NOISE_TAPS: [u8; 8] = [14, 10, 13, 4, 8, 6, 9, 11];
pub const DMA_RATES: [u32; 4] = [4_000, 6_000, 12_000, 24_000];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Noise {
    lfsr: u16,
    control: u8,
}

impl Noise {
    pub const fn state(self) -> u16 {
        self.lfsr
    }
    pub const fn control(self) -> u8 {
        self.control
    }
    pub fn write(&mut self, value: u8) {
        if value & 8 != 0 {
            self.lfsr = 0;
        }
        self.control = value & 0x17;
    }
    /// Advance even in wave output mode, when channel 4 and LFSR are enabled.
    pub fn clock(&mut self, channel_enabled: bool) {
        if channel_enabled && self.control & 0x10 != 0 {
            let tap = NOISE_TAPS[(self.control & 7) as usize];
            let bit = 1 ^ (((self.lfsr >> 7) ^ (self.lfsr >> tap)) & 1);
            self.lfsr = ((self.lfsr << 1) | bit) & 0x7fff;
        }
    }
    pub const fn sample(self) -> u8 {
        if self.lfsr & 1 == 0 {
            0
        } else {
            15
        }
    }
}

/// Ports $80..$94. Reset here means a deterministic preview state, not power-on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registers {
    ports: [u8; 21],
    pub noise: Noise,
}

impl Registers {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let data = bytes_at(bytes, 0, 21)?;
        let mut state = Self::default();
        for (i, &value) in data.iter().enumerate() {
            if i != 18 && i != 19 {
                state.write(0x80 + i as u8, value)?;
            }
        }
        state.noise.lfsr = (u16::from(data[18]) | (u16::from(data[19]) << 8)) & 0x7fff;
        state.set_headphones_connected(data[17] & 0x80 != 0);
        Ok(state)
    }
    pub fn read(&self, port: u8) -> Result<u8> {
        match port {
            0x8e => Ok(self.noise.control()),
            0x92 => Ok(self.noise.state() as u8),
            0x93 => Ok((self.noise.state() >> 8) as u8),
            0x80..=0x94 => Ok(self.ports[(port - 0x80) as usize]),
            _ => Err(Error::InvalidField {
                what: "sound register port",
                value: port.into(),
            }),
        }
    }
    pub fn write(&mut self, port: u8, value: u8) -> Result<()> {
        let value = match port {
            0x81 | 0x83 | 0x85 | 0x87 => value & 7,
            0x8d => value & 31,
            0x8e => {
                self.noise.write(value);
                self.noise.control()
            }
            0x90 => value & 0xef,
            0x91 => (self.ports[17] & 0x80) | (value & 0x0f),
            0x92 | 0x93 => {
                return Err(Error::Unsupported(
                    "noise state registers are read-only".into(),
                ))
            }
            0x94 => value & 15,
            0x80..=0x8c | 0x8f => value,
            _ => {
                return Err(Error::InvalidField {
                    what: "sound register port",
                    value: port.into(),
                })
            }
        };
        self.ports[(port - 0x80) as usize] = value;
        Ok(())
    }
    pub fn set_headphones_connected(&mut self, connected: bool) {
        self.ports[17] = (self.ports[17] & 0x0f) | if connected { 0x80 } else { 0 };
    }
    pub const fn control(&self) -> u8 {
        self.ports[16]
    }
    pub fn divisor(&self, channel: usize) -> Result<u16> {
        if channel >= 4 {
            return Err(Error::InvalidField {
                what: "sound channel index",
                value: channel as u64,
            });
        }
        Ok(u16::from(self.ports[channel * 2]) | (u16::from(self.ports[channel * 2 + 1]) << 8))
    }
    pub fn period(&self, channel: usize) -> Result<u16> {
        Ok(2048 - self.divisor(channel)?)
    }
    pub fn set_divisor(&mut self, channel: usize, divisor: u16) -> Result<()> {
        if channel >= 4 {
            return Err(Error::InvalidField {
                what: "sound channel index",
                value: channel as u64,
            });
        }
        self.write(0x80 + channel as u8 * 2, divisor as u8)?;
        self.write(0x81 + channel as u8 * 2, (divisor >> 8) as u8)
    }
    pub fn volume(&self, channel: usize) -> Result<(u8, u8)> {
        if channel >= 4 {
            return Err(Error::InvalidField {
                what: "sound channel index",
                value: channel as u64,
            });
        }
        let volume = self.ports[8 + channel];
        Ok((volume >> 4, volume & 15))
    }
    /// Left/right unsigned voice values. Full volume overrides the half bit.
    pub fn voice_output(&self) -> (u16, u16) {
        let sample = u16::from(self.ports[9]);
        let volume = self.ports[20];
        let out = |full, half| {
            if volume & full != 0 {
                sample
            } else if volume & half != 0 {
                sample >> 1
            } else {
                0
            }
        };
        (out(4, 8), out(1, 2))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HyperVoice {
    pub control: u16,
    pub left: i16,
    pub right: i16,
    next_right: bool,
}

impl HyperVoice {
    pub fn write_control(&mut self, value: u16) {
        if value & 0x1000 != 0 {
            self.next_right = false;
        }
        self.control = value & !0x1000;
    }
    pub const fn rate(self) -> u32 {
        let divisor = [1, 2, 3, 4, 5, 6, 8, 12][((self.control >> 4) & 7) as usize];
        OUTPUT_HZ / divisor
    }
    /// Conversion for the three documented extension modes. Mode 3's upper
    /// bits are unspecified; return an error instead of inventing a value.
    pub fn extend(self, value: u8) -> Result<i16> {
        let value = match (self.control >> 2) & 3 {
            0 => i32::from(value) << 8,
            1 => (i32::from(value) << 8) | !0xffff,
            2 => i32::from(value as i8) << 8,
            _ => {
                return Err(Error::Unsupported(
                    "Hyper Voice ignored extension mode".into(),
                ))
            }
        };
        // Extend the documented 19-bit representation, shift, retain low 16.
        Ok((value >> (self.control & 3)) as i16)
    }
    pub fn input(&mut self, value: u8, dma: bool) -> Result<()> {
        if self.control & 0x80 == 0 {
            return Ok(());
        }
        let sample = self.extend(value)?;
        let layout = if dma { (self.control >> 13) & 3 } else { 0 };
        match layout {
            1 => self.left = sample,
            2 => self.right = sample,
            3 => {
                self.left = sample;
                self.right = sample;
            }
            _ => {
                if self.next_right {
                    self.right = sample;
                } else {
                    self.left = sample;
                }
                self.next_right = !self.next_right;
            }
        }
        Ok(())
    }
}
