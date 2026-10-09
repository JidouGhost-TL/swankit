use formatkit_core::{Endian, Error, Result};
use formatkit_layout::{BytesField, U16Field};

/// Documented cartridge EEPROM capacities, in bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EepromSize {
    Kbit1,
    Kbit8,
    Kbit16,
}

impl EepromSize {
    pub const fn bytes(self) -> usize {
        match self {
            Self::Kbit1 => 128,
            Self::Kbit8 => 1024,
            Self::Kbit16 => 2048,
        }
    }
}

/// Physical medium supplied explicitly; ROM declarations alone do not size SRAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Media {
    CartridgeSram { bytes: usize },
    CartridgeEeprom(EepromSize),
    InternalMono,
    InternalColor,
}

impl Media {
    pub fn size(self) -> Result<usize> {
        match self {
            Self::CartridgeSram { bytes }
                if bytes.is_power_of_two() && (8192..=524288).contains(&bytes) =>
            {
                Ok(bytes)
            }
            Self::CartridgeSram { .. } => Err(Error::Malformed("unsupported SRAM size".into())),
            Self::CartridgeEeprom(size) => Ok(size.bytes()),
            Self::InternalMono => Ok(128),
            Self::InternalColor => Ok(2048),
        }
    }
}

/// A borrowed, exactly sized save image.
#[derive(Debug)]
pub struct SaveImage<'a> {
    media: Media,
    bytes: &'a [u8],
}

impl<'a> SaveImage<'a> {
    pub fn parse(media: Media, bytes: &'a [u8]) -> Result<Self> {
        let size = media.size()?;
        if bytes.len() != size {
            return Err(Error::Malformed(format!(
                "save size {}, expected {size}",
                bytes.len()
            )));
        }
        Ok(Self { media, bytes })
    }

    pub const fn media(&self) -> Media {
        self.media
    }
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn word(&self, address: usize) -> Result<u16> {
        if matches!(self.media, Media::CartridgeSram { .. }) {
            return Err(Error::Malformed("SRAM is byte addressed".into()));
        }
        let offset = address
            .checked_mul(2)
            .ok_or_else(|| Error::Malformed("word address overflow".into()))?;
        U16Field::new(offset, Endian::Little).read(self.bytes)
    }

    pub fn program_data(&self) -> Result<&'a [u8]> {
        if !matches!(self.media, Media::InternalMono | Media::InternalColor) {
            return Err(Error::Malformed(
                "program data requires internal EEPROM".into(),
            ));
        }
        Ok(BytesField::<96>::new(0).read(self.bytes)?.as_slice())
    }

    /// Edit one EEPROM word, checking both geometry and extent before mutation.
    pub fn write_word(media: Media, bytes: &mut [u8], address: usize, value: u16) -> Result<()> {
        SaveImage::parse(media, bytes)?;
        if matches!(media, Media::CartridgeSram { .. }) {
            return Err(Error::Malformed("SRAM is byte addressed".into()));
        }
        let offset = address
            .checked_mul(2)
            .ok_or_else(|| Error::Malformed("word address overflow".into()))?;
        U16Field::new(offset, Endian::Little).write(bytes, value)
    }
}
