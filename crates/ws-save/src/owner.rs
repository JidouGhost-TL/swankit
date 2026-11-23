use crate::{Media, SaveImage};
use formatkit_core::{bcd_to_u8, Endian, Error, Result};
use formatkit_layout::{BytesField, U16Field, U8Field};

/// BCD owner birthday; invalid/unset values remain visible in the raw bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Birthday {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

/// System profile from the documented internal EEPROM layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerProfile {
    pub name_codes: [u8; 16],
    pub name: String,
    pub birthday_raw: [u8; 4],
    pub birthday: Option<Birthday>,
    pub gender: u8,
    pub blood_type: u8,
    pub last_cartridge: [u8; 3],
    pub cartridge_change_counter: u8,
    pub name_change_counter: u8,
    pub startup_counter: u16,
    pub color_configuration: Option<u8>,
    pub name_color: Option<u8>,
}

impl OwnerProfile {
    pub fn parse(image: &SaveImage<'_>) -> Result<Self> {
        if !matches!(image.media(), Media::InternalMono | Media::InternalColor) {
            return Err(Error::Malformed(
                "owner profile requires internal EEPROM".into(),
            ));
        }
        let bytes = image.bytes();
        let name_codes: [u8; 16] = *BytesField::<16>::new(0x60).read(bytes)?;
        let name = name_codes
            .iter()
            .map(|&code| match code {
                0 => " ".into(),
                1..=10 => char::from(b'0' + code - 1).to_string(),
                11..=36 => char::from(b'A' + code - 11).to_string(),
                37 => "♥".into(),
                38 => "♪".into(),
                39 => "+".into(),
                40 => "-".into(),
                41 => "?".into(),
                42 => ".".into(),
                _ => format!("[{code:02x}]"),
            })
            .collect::<String>()
            .trim_end()
            .to_owned();
        let birthday_raw: [u8; 4] = *BytesField::<4>::new(0x70).read(bytes)?;
        let birthday = (|| {
            let [hi, lo, month, day] = birthday_raw;
            let year = u16::from(bcd_to_u8(hi)?) * 100 + u16::from(bcd_to_u8(lo)?);
            let month = bcd_to_u8(month)?;
            let day = bcd_to_u8(day)?;
            let leap =
                year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
            let days = match month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 => 28 + u8::from(leap),
                _ => return None,
            };
            (year != 0 && (1..=days).contains(&day)).then_some(Birthday { year, month, day })
        })();
        let color = image.media() == Media::InternalColor;
        Ok(Self {
            name_codes,
            name,
            birthday_raw,
            birthday,
            gender: U8Field::new(0x74).read(bytes)?,
            blood_type: U8Field::new(0x75).read(bytes)?,
            last_cartridge: *BytesField::<3>::new(0x76).read(bytes)?,
            cartridge_change_counter: U8Field::new(0x7c).read(bytes)?,
            name_change_counter: U8Field::new(0x7d).read(bytes)?,
            startup_counter: U16Field::new(0x7e, Endian::Little).read(bytes)?,
            color_configuration: if color {
                Some(U8Field::new(0x83).read(bytes)?)
            } else {
                None
            },
            name_color: if color {
                Some(U8Field::new(0x84).read(bytes)?)
            } else {
                None
            },
        })
    }

    /// Update profile fields only; preserve counters, reserved bytes and splash data.
    pub fn write(&self, media: Media, bytes: &mut [u8]) -> Result<()> {
        let image = SaveImage::parse(media, bytes)?;
        Self::parse(&image)?;
        BytesField::<16>::new(0x60).write(bytes, &self.name_codes)?;
        BytesField::<4>::new(0x70).write(bytes, &self.birthday_raw)?;
        U8Field::new(0x74).write(bytes, self.gender)?;
        U8Field::new(0x75).write(bytes, self.blood_type)
    }
}
