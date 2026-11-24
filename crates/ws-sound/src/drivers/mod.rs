//! Decoded driver grammars. Linked table locations are caller-supplied.
pub mod byte;
pub mod cell;
pub mod sdk;
pub mod sdk_discover;
pub mod sdk_driver;

use formatkit_core::{bytes_at, Error, Result};
use ws_rom::{BankState, RomGeometry};

pub const MAX_RECORDS: usize = 65_536;

pub(crate) fn check_count(count: usize, width: usize) -> Result<usize> {
    if count > MAX_RECORDS {
        return Err(Error::ResourceLimit {
            resource: "sound records",
            requested: count as u64,
            limit: MAX_RECORDS as u64,
        });
    }
    count
        .checked_mul(width)
        .ok_or_else(|| Error::Malformed("sound table size overflow".into()))
}

/// Resolved ROM window. Mapping uses ws-rom's padded-chip policy, not file modulo.
#[derive(Debug, Clone, Copy)]
pub struct RomView<'a> {
    bytes: &'a [u8],
    geometry: RomGeometry,
    banks: BankState,
}

impl<'a> RomView<'a> {
    pub fn new(bytes: &'a [u8], banks: BankState) -> Result<Self> {
        Ok(Self {
            bytes,
            geometry: RomGeometry::new(bytes.len())?,
            banks,
        })
    }
    pub fn offset(self, segment: u16, offset: u16) -> Result<usize> {
        let address = ((u32::from(segment) << 4) + u32::from(offset)) & 0xfffff;
        self.geometry
            .address_to_file_offset(address, self.banks)
            .ok_or_else(|| Error::Malformed("sound pointer has no ROM backing".into()))
    }
    /// Resident span; rejects mapping discontinuities and segment offset wrapping.
    pub fn span(self, segment: u16, offset: u16, len: usize) -> Result<&'a [u8]> {
        if len > 0x10000 - usize::from(offset) {
            return Err(Error::Malformed("sound span wraps its segment".into()));
        }
        let start = self.offset(segment, offset)?;
        if len > 0 {
            let last = self.offset(segment, offset + (len - 1) as u16)?;
            if start.checked_add(len - 1) != Some(last) {
                return Err(Error::Malformed(
                    "sound span crosses a mapping discontinuity".into(),
                ));
            }
        }
        bytes_at(self.bytes, start, len)
    }
    pub fn file_span(self, offset: usize, len: usize) -> Result<&'a [u8]> {
        bytes_at(self.bytes, offset, len)
    }
    pub fn with_bank(self, segment: u16, bank: u8) -> Result<Self> {
        let mut view = self;
        match segment {
            0x2000 => view.banks.c2 = u16::from(bank),
            0x3000 => view.banks.c3 = u16::from(bank),
            0x4000..=0xffff => {}
            _ => return Err(Error::Unsupported("byte driver sequence window".into())),
        }
        Ok(view)
    }
}
