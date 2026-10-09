//! Checked WonderSwan cartridge footer parsing and address mapping.
//! Grammar and hardware policy are owned here; byte mechanisms come from Formatkit.
#![forbid(unsafe_code)]

pub mod catalog;
mod header;
mod mapping;

pub use header::{
    publisher_name, Header, Mapper, Orientation, RomFacts, SaveDeclaration, FOOTER_SIZE,
    MAX_ROM_SIZE, MIN_ROM_SIZE,
};
pub use mapping::{BankState, RomGeometry};
