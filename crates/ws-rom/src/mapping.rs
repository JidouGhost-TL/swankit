//! Mapping documented by WSdev's Memory_map, Mapper and Bandai_2003 pages.
use crate::{Mapper, MAX_ROM_SIZE, MIN_ROM_SIZE};
use formatkit_core::{Error, Result};

/// Bank register state. C2/C3 include the 2003 high-byte extensions at D3/D5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankState {
    /// C0 selects the 1 MiB group for physical addresses 40000..FFFFF.
    pub c0: u8,
    /// C2 selects the 64 KiB bank at physical addresses 20000..2FFFF.
    pub c2: u16,
    /// C3 selects the 64 KiB bank at physical addresses 30000..3FFFF.
    pub c3: u16,
}

impl BankState {
    /// Common 8-bit reset state; callers supply extended high bytes if relevant.
    pub const RESET: Self = Self {
        c0: 0xff,
        c2: 0xff,
        c3: 0xff,
    };

    /// Apply a known mapper's implemented address bits; unknown families error.
    pub fn for_mapper(self, mapper: Mapper) -> Result<Self> {
        match mapper {
            Mapper::Bandai2001 => Ok(Self {
                c0: self.c0 & 0xf,
                c2: self.c2 & 0xff,
                c3: self.c3 & 0xff,
            }),
            Mapper::Bandai2003 => Ok(Self {
                c0: self.c0 & 0x3f,
                c2: self.c2 & 0x3ff,
                c3: self.c3 & 0x3ff,
            }),
            Mapper::Unknown(code) => Err(Error::InvalidField {
                what: "mapper bank rules",
                value: code.into(),
            }),
        }
    }
}

/// File placement in a power-of-two chip, with padding before a short image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomGeometry {
    file_size: usize,
    effective_size: usize,
}

impl RomGeometry {
    /// Validate the size before computing banking masks. No footer code is used.
    pub fn new(file_size: usize) -> Result<Self> {
        if !(MIN_ROM_SIZE..=MAX_ROM_SIZE).contains(&file_size) {
            return Err(Error::InvalidField {
                what: "ROM size",
                value: file_size as u64,
            });
        }
        Ok(Self {
            file_size,
            effective_size: file_size.next_power_of_two(),
        })
    }

    /// Actual bytes in the ROM file.
    pub fn file_size(self) -> usize {
        self.file_size
    }
    /// Smallest power-of-two chip containing this image.
    pub fn effective_size(self) -> usize {
        self.effective_size
    }
    /// Unbacked bytes before the image (normally read as FF).
    pub fn padding_size(self) -> usize {
        self.effective_size - self.file_size
    }
    /// Whether no start padding is needed.
    pub fn is_power_of_two(self) -> bool {
        self.file_size.is_power_of_two()
    }

    /// Effective chip offset for a CPU physical address; RAM and invalid addresses
    /// return None. C0 contributes six bits and C2/C3 contribute ten bits at most.
    /// Use BankState::for_mapper for explicit 2001 register-width normalization.
    pub fn address_to_effective_offset(self, address: u32, banks: BankState) -> Option<usize> {
        let chip = match address {
            0x20000..=0x2ffff => (u32::from(banks.c2 & 0x3ff) << 16) | (address & 0xffff),
            0x30000..=0x3ffff => (u32::from(banks.c3 & 0x3ff) << 16) | (address & 0xffff),
            0x40000..=0xfffff => (u32::from(banks.c0 & 0x3f) << 20) | address,
            _ => return None,
        };
        Some(chip as usize & (self.effective_size - 1))
    }

    /// File offset for a chip offset. Padding and out-of-chip offsets return None.
    pub fn effective_to_file_offset(self, effective: usize) -> Option<usize> {
        if effective >= self.effective_size {
            return None;
        }
        effective.checked_sub(self.padding_size())
    }

    /// ROM file offset backing a CPU physical address in this bank state.
    pub fn address_to_file_offset(self, address: u32, banks: BankState) -> Option<usize> {
        self.effective_to_file_offset(self.address_to_effective_offset(address, banks)?)
    }

    /// All CPU aliases of a file offset in ascending order. An offset can be
    /// unmapped or mirrored into several of the fourteen 64 KiB ROM windows.
    /// Invalid file offsets produce an empty list; padding never acquires an alias.
    pub fn file_offset_to_addresses(self, offset: usize, banks: BankState) -> Vec<u32> {
        if offset >= self.file_size {
            return Vec::new();
        }
        let within = (offset + self.padding_size()) as u32 & 0xffff;
        (2..16)
            .map(|bank| (bank << 16) | within)
            .filter(|&address| self.address_to_file_offset(address, banks) == Some(offset))
            .collect()
    }
}
