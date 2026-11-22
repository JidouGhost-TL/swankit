//! Footer grammar: <https://ws.nesdev.org/wiki/ROM_header> (revision 680).
use crate::RomGeometry;
use formatkit_core::{Endian, Error, ErrorContext, Reader, Result};
use formatkit_layout::{U16Field, U8Field};

/// Cartridge footer width in bytes.
pub const FOOTER_SIZE: usize = 16;
/// Smallest supported image: one 64 KiB bank.
pub const MIN_ROM_SIZE: usize = 0x1_0000;
/// Largest ROM supported by the 2003 mapper.
pub const MAX_ROM_SIZE: usize = 0x400_0000;

const JMP: U8Field = U8Field::new(0);
const RESET_OFFSET: U16Field = U16Field::new(1, Endian::Little);
const RESET_SEGMENT: U16Field = U16Field::new(3, Endian::Little);
const MAINTENANCE: U8Field = U8Field::new(5);
const PUBLISHER: U8Field = U8Field::new(6);
const COLOR: U8Field = U8Field::new(7);
const GAME_ID: U8Field = U8Field::new(8);
const VERSION: U8Field = U8Field::new(9);
const ROM_SIZE: U8Field = U8Field::new(10);
const SAVE: U8Field = U8Field::new(11);
const FLAGS: U8Field = U8Field::new(12);
const MAPPER: U8Field = U8Field::new(13);
const CHECKSUM: U16Field = U16Field::new(14, Endian::Little);

/// Raw footer metadata. Unknown declaration codes are preserved, never guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Offset of the far jump target.
    pub reset_offset: u16,
    /// Segment of the far jump target.
    pub reset_segment: u16,
    /// Maintenance/splash control byte.
    pub maintenance: u8,
    /// Developer/publisher identifier.
    pub publisher_id: u8,
    /// Unmodified color byte; only bit zero declares color support.
    pub color_code: u8,
    /// Raw game identifier, conventionally binary-coded decimal.
    pub game_id: u8,
    /// Version and internal EEPROM protection byte.
    pub version: u8,
    /// ROM size declaration code, independent of image geometry.
    pub rom_size_code: u8,
    /// Save media/size declaration code.
    pub save_code: u8,
    /// Orientation, bus width and wait state flags.
    pub flags: u8,
    /// Mapper code, historically called the RTC flag.
    pub mapper_code: u8,
    /// Little-endian sum stored in the footer.
    pub checksum: u16,
}

impl Header {
    /// Parse a cartridge image with valid geometry and an EA far-jump footer.
    /// Checksum and declaration mismatches are facts rather than fatal errors.
    pub fn parse(rom: &[u8]) -> Result<Self> {
        RomGeometry::new(rom.len())?;
        let offset = rom.len() - FOOTER_SIZE;
        let mut reader = Reader::new(rom);
        reader.skip(offset)?;
        let footer = reader.bytes(FOOTER_SIZE)?;
        let parse = || -> Result<Self> {
            let opcode = JMP.read(footer)?;
            if opcode != 0xea {
                return Err(Error::BadMagic {
                    offset,
                    expected: 0xea,
                    found: opcode.into(),
                });
            }
            Ok(Self {
                reset_offset: RESET_OFFSET.read(footer)?,
                reset_segment: RESET_SEGMENT.read(footer)?,
                maintenance: MAINTENANCE.read(footer)?,
                publisher_id: PUBLISHER.read(footer)?,
                color_code: COLOR.read(footer)?,
                game_id: GAME_ID.read(footer)?,
                version: VERSION.read(footer)?,
                rom_size_code: ROM_SIZE.read(footer)?,
                save_code: SAVE.read(footer)?,
                flags: FLAGS.read(footer)?,
                mapper_code: MAPPER.read(footer)?,
                checksum: CHECKSUM.read(footer)?,
            })
        };
        parse().map_err(|error| {
            error.context(ErrorContext::SourceRange {
                offset: offset as u64,
                length: FOOTER_SIZE as u64,
            })
        })
    }

    /// CPU physical address, wrapping the segment:offset sum to 20 bits.
    pub fn reset_linear(self) -> u32 {
        ((u32::from(self.reset_segment) << 4) + u32::from(self.reset_offset)) & 0xf_ffff
    }

    /// Whether color operation is declared (independent of a filename).
    pub fn color(self) -> bool {
        self.color_code & 1 != 0
    }

    /// Splash screen's initial orientation.
    pub fn orientation(self) -> Orientation {
        if self.flags & 1 == 0 {
            Orientation::Horizontal
        } else {
            Orientation::Vertical
        }
    }

    /// ROM bus width, from flag bit 2 (not bit 1).
    pub fn bus_width(self) -> u8 {
        if self.flags & 4 == 0 {
            8
        } else {
            16
        }
    }

    /// Duration of a byte ROM access, including the flag bit 3 wait state.
    /// A word on an 8-bit bus requires two byte transactions.
    pub fn rom_speed_cycles(self) -> u8 {
        1 + u8::from(self.flags & 8 != 0)
    }

    /// Decoded mapper family.
    pub fn mapper(self) -> Mapper {
        Mapper::from_code(self.mapper_code)
    }

    /// RTC *interface* support. The footer does not prove a populated RTC chip.
    pub fn rtc_interface(self) -> bool {
        self.mapper() == Mapper::Bandai2003
    }

    /// Nominal save declaration; SRAM code 01 has uncertain physical capacity.
    pub fn save(self) -> SaveDeclaration {
        SaveDeclaration::from_code(self.save_code)
    }

    /// ROM size declaration in bytes, including inferred codes 0A and 0B.
    pub fn declared_rom_size(self) -> Option<usize> {
        match self.rom_size_code {
            0 => Some(0x2_0000),
            1 => Some(0x4_0000),
            2 => Some(0x8_0000),
            3 => Some(0x10_0000),
            4 => Some(0x20_0000),
            5 => Some(0x30_0000),
            6 => Some(0x40_0000),
            7 => Some(0x60_0000),
            8 => Some(0x80_0000),
            9 => Some(0x100_0000),
            10 => Some(0x200_0000),
            11 => Some(0x400_0000),
            _ => None,
        }
    }
}

/// Initial LCD/splash orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Landscape orientation.
    Horizontal,
    /// Portrait orientation.
    Vertical,
}

/// Mapper family; code zero cannot distinguish 2001 from KARNAK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mapper {
    /// Common 2001 register set (or KARNAK with external model evidence).
    Bandai2001,
    /// Extended 2003 register set and RTC interface.
    Bandai2003,
    /// Unrecognized value retained verbatim.
    Unknown(u8),
}

impl Mapper {
    /// Interpret the complete byte without treating unknown values as 2001.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Bandai2001,
            1 => Self::Bandai2003,
            other => Self::Unknown(other),
        }
    }
}

/// Nominal footer declaration, rather than a measurement of the installed chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveDeclaration {
    /// No cartridge save media declared.
    None,
    /// Battery-backed SRAM capacity in kilobits.
    Sram { kbit: u32 },
    /// Serial EEPROM capacity in kilobits.
    Eeprom { kbit: u32 },
    /// Unrecognized code retained without silently assuming no media.
    Unknown(u8),
}

impl SaveDeclaration {
    /// Decode documented nominal declarations. See docs/hardware.md for code 01.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::None,
            1 => Self::Sram { kbit: 64 },
            2 => Self::Sram { kbit: 256 },
            3 => Self::Sram { kbit: 1024 },
            4 => Self::Sram { kbit: 2048 },
            5 => Self::Sram { kbit: 4096 },
            0x10 => Self::Eeprom { kbit: 1 },
            0x20 => Self::Eeprom { kbit: 16 },
            0x50 => Self::Eeprom { kbit: 8 },
            other => Self::Unknown(other),
        }
    }

    /// Nominal byte capacity; unknown declarations return None.
    pub fn bytes(self) -> Option<u32> {
        match self {
            Self::None => Some(0),
            Self::Sram { kbit } | Self::Eeprom { kbit } => kbit.checked_mul(128),
            Self::Unknown(_) => None,
        }
    }
}

/// Parsed metadata and derived facts for one image; no image bytes are retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomFacts {
    /// Cartridge footer.
    pub header: Header,
    /// File size and power-of-two banking geometry.
    pub geometry: RomGeometry,
    /// Sum of all bytes except the final two, modulo 65536.
    pub checksum_computed: u16,
}

impl RomFacts {
    /// Parse with checked Formatkit reads. A bad checksum remains reportable.
    pub fn parse(rom: &[u8]) -> Result<Self> {
        let header = Header::parse(rom)?;
        let geometry = RomGeometry::new(rom.len())?;
        let covered = Reader::new(rom).bytes(rom.len() - 2)?;
        let checksum_computed = formatkit_core::checksum::sum_bytes_u16(covered, 0);
        Ok(Self {
            header,
            geometry,
            checksum_computed,
        })
    }

    /// Whether the stored checksum matches the computed sum.
    pub fn checksum_ok(self) -> bool {
        self.header.checksum == self.checksum_computed
    }

    /// Whether the declared ROM size exactly equals the file size.
    pub fn declared_size_matches(self) -> bool {
        self.header.declared_rom_size() == Some(self.geometry.file_size())
    }
}

/// Known publisher labels. None means unknown; labels retain established aliases.
pub fn publisher_name(id: u8) -> Option<&'static str> {
    Some(match id {
        0x01 => "Bandai",
        0x02 => "Taito",
        0x03 => "Tomy",
        0x04 => "Koei",
        0x05 => "Data East",
        0x06 => "Asmik Ace",
        0x07 => "Media Entertainment",
        0x08 => "Nichibutsu",
        0x0a => "Coconuts Japan",
        0x0b => "Sammy",
        0x0c => "Sunsoft",
        0x0d => "Mebius",
        0x0e => "Banpresto",
        0x10 => "Jaleco",
        0x11 => "Imagineer",
        0x12 => "Konami",
        0x16 => "Kobunsha",
        0x17 => "Bottom Up",
        0x18 => "Kaga Tech",
        0x19 => "Sunrise",
        0x1a => "Cyber Front",
        0x1b => "Megahouse",
        0x1d => "Interbec",
        0x1e => "Nihon Application",
        0x1f => "Bandai Visual",
        0x20 => "Athena",
        0x21 => "KID",
        0x22 => "HAL",
        0x23 => "Yuki Enterprise",
        0x24 => "Omega Micott",
        0x25 => "Layup",
        0x26 => "Kadokawa Shoten",
        0x27 => "Shall Luck",
        0x28 => "Squaresoft",
        0x2a => "SCC?",
        0x2b => "Tom Create",
        0x2d => "Namco",
        0x2e => "Soeishinsha",
        0x2f => "Hearty Robin",
        0x31 => "Vanguard",
        0x32 => "Megatron",
        0x33 => "Wiz",
        0x35 => "Tanita",
        0x36 => "Capcom",
        _ => return None,
    })
}
