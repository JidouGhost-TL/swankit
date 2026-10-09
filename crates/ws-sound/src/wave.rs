use formatkit_core::{bytes_at, Error, Pcm, Reader, Result};

pub const WAVE_BYTES: usize = 16;
pub const WAVE_SAMPLES: usize = 32;

/// One hardware wave, with low nybble preceding high nybble in each byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wave([u8; WAVE_SAMPLES]);

impl Wave {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes_at(bytes, 0, WAVE_BYTES)?);
        let mut samples = [0; WAVE_SAMPLES];
        for pair in samples.chunks_exact_mut(2) {
            let value = reader.u8()?;
            pair[0] = value & 15;
            pair[1] = value >> 4;
        }
        Ok(Self(samples))
    }

    pub const fn samples(&self) -> &[u8; WAVE_SAMPLES] {
        &self.0
    }

    /// A single wave cycle, centered at 8 and scaled by 4096. No resampling.
    pub fn to_pcm(&self, sample_rate: u32) -> Result<Pcm> {
        if sample_rate == 0 {
            return Err(Error::InvalidField {
                what: "wave sample rate",
                value: 0,
            });
        }
        Ok(Pcm::new(
            sample_rate,
            self.0.iter().map(|&s| (i16::from(s) - 8) * 4096).collect(),
        ))
    }
}

/// Four 16-byte waves in the RAM block selected by port $8F.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaveBank(pub [Wave; 4]);

impl WaveBank {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes_at(bytes, 0, 64)?);
        Ok(Self([
            Wave::decode(reader.bytes(16)?)?,
            Wave::decode(reader.bytes(16)?)?,
            Wave::decode(reader.bytes(16)?)?,
            Wave::decode(reader.bytes(16)?)?,
        ]))
    }

    pub fn from_ram(ram: &[u8], address_register: u8) -> Result<Self> {
        Self::decode(bytes_at(ram, Self::ram_base(address_register), 64)?)
    }

    pub const fn ram_base(address_register: u8) -> usize {
        (address_register as usize) << 6
    }
}

impl Default for WaveBank {
    fn default() -> Self {
        Self([Wave([0; WAVE_SAMPLES]); 4])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_ramp_decodes_to_one_pcm_cycle() {
        let encoded: Vec<u8> = (0..16).map(|n| ((15 - n) << 4) | n).collect();
        let wave = Wave::decode(&encoded).unwrap();
        let pcm = wave.to_pcm(24_000).unwrap();
        assert_eq!(pcm.frame_count(), 32);
        for n in 0..16 {
            assert_eq!(wave.samples()[n * 2], n as u8);
            assert_eq!(wave.samples()[n * 2 + 1], (15 - n) as u8);
            assert_eq!(pcm.samples()[n * 2], (n as i16 - 8) * 4096);
        }
    }

    #[test]
    fn ram_block_bounds_and_channel_order() {
        let mut ram = vec![0; 0x4000];
        for c in 0..4 {
            ram[0x3fc0 + c * 16..0x3fd0 + c * 16].fill(c as u8 * 0x11);
        }
        let bank = WaveBank::from_ram(&ram, 255).unwrap();
        assert_eq!(bank.0[3].samples(), &[3; 32]);
        assert!(WaveBank::from_ram(&ram[..0x3fff], 255).is_err());
        assert!(Wave::decode(&[0; 15]).is_err());
        assert!(Wave::decode(&[0; 16]).unwrap().to_pcm(0).is_err());
    }
}
