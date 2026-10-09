use formatkit_catalog::{DetectionContext, FormatCatalog};
use formatkit_core::Error;
use ws_rom::{
    catalog, publisher_name, BankState, Header, Mapper, Orientation, RomFacts, RomGeometry,
    SaveDeclaration, FOOTER_SIZE, MAX_ROM_SIZE,
};

fn image() -> Vec<u8> {
    let mut rom = vec![0; 0x10000];
    let offset = rom.len() - FOOTER_SIZE;
    rom[offset..].copy_from_slice(&[
        0xea, 0x34, 0x12, 0xff, 0xff, 0, 1, 1, 2, 0x80, 2, 0x10, 0x0c, 1, 0x34, 0x12,
    ]);
    rom
}

#[test]
fn synthetic_header() {
    let facts = RomFacts::parse(&image()).unwrap();
    let h = facts.header;
    assert_eq!(h.reset_segment, 0xffff);
    assert_eq!(h.reset_offset, 0x1234);
    assert_eq!(h.reset_linear(), 0x1224);
    assert_eq!(h.maintenance, 0);
    assert_eq!(h.publisher_id, 1);
    assert_eq!(publisher_name(h.publisher_id), Some("Bandai"));
    assert!(h.color());
    assert_eq!((h.game_id, h.version, h.rom_size_code), (2, 0x80, 2));
    assert_eq!(h.checksum, 0x1234);
    assert_eq!(facts.checksum_computed, 0x3d1);
    assert!(!facts.checksum_ok());
    assert_eq!(h.save(), SaveDeclaration::Eeprom { kbit: 1 });
    assert_eq!(h.save().bytes(), Some(128));
    assert_eq!(h.orientation(), Orientation::Horizontal);
    assert_eq!(h.bus_width(), 16);
    assert_eq!(h.rom_speed_cycles(), 2);
    assert_eq!(h.mapper(), Mapper::Bandai2003);
    assert!(h.rtc_interface());
    assert_eq!(h.declared_rom_size(), Some(0x80000));
    assert!(!facts.declared_size_matches());
}

#[test]
fn checksum_wraps_and_excludes_only_the_final_two_bytes() {
    let mut rom = image();
    rom[..0xfff0].fill(0xff);
    let facts = RomFacts::parse(&rom).unwrap();
    // 65520 * 255 plus the explicitly chosen footer bytes, modulo 65536.
    assert_eq!(facts.checksum_computed, 0xf3e1);
    rom[0xfffe..].copy_from_slice(&[0xe1, 0xf3]);
    assert!(RomFacts::parse(&rom).unwrap().checksum_ok());
    rom[0] = 0xfe;
    assert!(!RomFacts::parse(&rom).unwrap().checksum_ok());
}

#[test]
fn width_wait_state_and_orientation_are_independent() {
    let mut rom = image();
    for (flags, width, speed, orientation) in [
        (0, 8, 1, Orientation::Horizontal),
        (4, 16, 1, Orientation::Horizontal),
        (8, 8, 2, Orientation::Horizontal),
        (0x0c, 16, 2, Orientation::Horizontal),
        (3, 8, 1, Orientation::Vertical),
        (5, 16, 1, Orientation::Vertical),
    ] {
        rom[0xfffc] = flags;
        let h = Header::parse(&rom).unwrap();
        assert_eq!(
            (h.bus_width(), h.rom_speed_cycles(), h.orientation()),
            (width, speed, orientation)
        );
    }
}

#[test]
fn nominal_save_declarations_and_unknowns_are_explicit() {
    for (code, save, bytes) in [
        (0, SaveDeclaration::None, 0),
        (1, SaveDeclaration::Sram { kbit: 64 }, 0x2000),
        (2, SaveDeclaration::Sram { kbit: 256 }, 0x8000),
        (3, SaveDeclaration::Sram { kbit: 1024 }, 0x20000),
        (4, SaveDeclaration::Sram { kbit: 2048 }, 0x40000),
        (5, SaveDeclaration::Sram { kbit: 4096 }, 0x80000),
        (0x10, SaveDeclaration::Eeprom { kbit: 1 }, 0x80),
        (0x20, SaveDeclaration::Eeprom { kbit: 16 }, 0x800),
        (0x50, SaveDeclaration::Eeprom { kbit: 8 }, 0x400),
    ] {
        assert_eq!(SaveDeclaration::from_code(code), save);
        assert_eq!(save.bytes(), Some(bytes));
    }
    assert_eq!(
        SaveDeclaration::from_code(0xff),
        SaveDeclaration::Unknown(0xff)
    );
    assert_eq!(SaveDeclaration::Unknown(0xff).bytes(), None);
    assert_eq!(SaveDeclaration::Sram { kbit: u32::MAX }.bytes(), None);
    assert_eq!(Mapper::from_code(2), Mapper::Unknown(2));
    assert_eq!(publisher_name(0xff), None);
}

#[test]
fn declarations_are_distinct_from_file_geometry() {
    let mut h = Header::parse(&image()).unwrap();
    for (code, size) in [
        (0, 0x20000),
        (1, 0x40000),
        (2, 0x80000),
        (3, 0x100000),
        (4, 0x200000),
        (5, 0x300000),
        (6, 0x400000),
        (7, 0x600000),
        (8, 0x800000),
        (9, 0x1000000),
        (10, 0x2000000),
        (11, 0x4000000),
    ] {
        h.rom_size_code = code;
        assert_eq!(h.declared_rom_size(), Some(size));
    }
    h.rom_size_code = 0xff;
    assert_eq!(h.declared_rom_size(), None);
    assert_eq!(
        RomGeometry::new(0xc0000).unwrap().effective_size(),
        0x100000
    );
}

#[test]
fn malformed_images_return_errors_with_source_coordinates() {
    for length in [0, 1, 15, 16, 0xffff] {
        assert!(Header::parse(&vec![0; length]).is_err());
    }
    for length in [0, 0xffff, MAX_ROM_SIZE + 1, usize::MAX] {
        assert!(RomGeometry::new(length).is_err());
    }
    let mut rom = image();
    rom[0xfff0] = 0;
    let error = Header::parse(&rom).unwrap_err();
    assert_eq!(
        error.leaf(),
        &Error::BadMagic {
            offset: 0xfff0,
            expected: 0xea,
            found: 0
        }
    );
    assert_eq!(error.contexts().count(), 1);
    assert!(!catalog::probe(&rom));
}

#[test]
fn catalog_composes_and_preserves_damaged_checksum_recognition() {
    let catalog = FormatCatalog::builder()
        .add_descriptor(catalog::ROM_FORMAT)
        .available_decoders(["ws-rom"])
        .build()
        .unwrap();
    let mut rom = image();
    assert_eq!(
        catalog
            .identify(DetectionContext::from_bytes(&rom))
            .unwrap()
            .format_id(),
        catalog::ROM_ID
    );
    assert!(!RomFacts::parse(&rom).unwrap().checksum_ok());
    rom[0xfffd] = 0xff;
    assert!(Header::parse(&rom).is_ok());
    assert!(catalog
        .identify(DetectionContext::from_bytes(&rom))
        .is_none());
    rom[0xfffd] = 0;
    rom[0xfff5] = 1;
    assert!(!catalog::probe(&rom));
}

#[test]
fn c0_c2_c3_windows_and_reverse_aliases() {
    let g = RomGeometry::new(0x400000).unwrap();
    let banks = BankState {
        c0: 2,
        c2: 0x12,
        c3: 0x23,
    };
    assert_eq!(g.address_to_file_offset(0x23456, banks), Some(0x123456));
    assert_eq!(g.address_to_file_offset(0x34567, banks), Some(0x234567));
    assert_eq!(g.address_to_file_offset(0x45678, banks), Some(0x245678));
    assert_eq!(
        g.address_to_file_offset(0xfffff, BankState::RESET),
        Some(0x3fffff)
    );
    assert_eq!(g.file_offset_to_addresses(0x123456, banks), vec![0x23456]);
    assert_eq!(
        g.file_offset_to_addresses(0x200000, banks),
        Vec::<u32>::new()
    );
    let mirrored = RomGeometry::new(0x10000).unwrap();
    assert_eq!(mirrored.file_offset_to_addresses(0x3456, banks).len(), 14);
    assert_eq!(g.address_to_file_offset(0x1ffff, banks), None);
    assert_eq!(g.address_to_file_offset(0x100000, banks), None);
    assert!(g.file_offset_to_addresses(g.file_size(), banks).is_empty());
}

#[test]
fn non_power_of_two_images_use_start_padding() {
    let g = RomGeometry::new(0xc0000).unwrap();
    let banks = BankState {
        c0: 0,
        c2: 3,
        c3: 4,
    };
    assert_eq!(g.padding_size(), 0x40000);
    assert!(!g.is_power_of_two());
    assert_eq!(g.address_to_file_offset(0x23456, banks), None);
    assert_eq!(g.address_to_file_offset(0x30000, banks), Some(0));
    assert_eq!(g.address_to_file_offset(0x40000, banks), Some(0));
    assert_eq!(g.address_to_file_offset(0xfffff, banks), Some(0xbffff));
    assert_eq!(g.file_offset_to_addresses(0, banks), vec![0x30000, 0x40000]);
    assert_eq!(g.effective_to_file_offset(0x100000), None);
    assert_eq!(g.effective_to_file_offset(0x3ffff), None);
}

#[test]
fn mapper_widths_and_maximum_extended_banks() {
    let state = BankState {
        c0: 0xff,
        c2: 0x312,
        c3: 0xffff,
    };
    let old = state.for_mapper(Mapper::Bandai2001).unwrap();
    let extended = state.for_mapper(Mapper::Bandai2003).unwrap();
    assert_eq!(
        old,
        BankState {
            c0: 0xf,
            c2: 0x12,
            c3: 0xff
        }
    );
    assert_eq!(
        extended,
        BankState {
            c0: 0x3f,
            c2: 0x312,
            c3: 0x3ff
        }
    );
    assert!(state.for_mapper(Mapper::Unknown(2)).is_err());
    let g = RomGeometry::new(MAX_ROM_SIZE).unwrap();
    assert_eq!(g.address_to_file_offset(0x23456, extended), Some(0x3123456));
    assert_eq!(
        g.address_to_file_offset(0xfffff, extended),
        Some(MAX_ROM_SIZE - 1)
    );
}

#[test]
fn mapping_roundtrips_all_bank_boundaries_without_padding_aliases() {
    for size in [0x10000, 0x20000, 0xc0000, 0x300000, 0x400000, MAX_ROM_SIZE] {
        let g = RomGeometry::new(size).unwrap();
        for banks in [
            BankState::RESET,
            BankState {
                c0: 0,
                c2: 0,
                c3: 1,
            },
            BankState {
                c0: 0x21,
                c2: 0x312,
                c3: 0x3fe,
            },
        ] {
            for bank in 2..16 {
                for within in [0, 1, 0x7fff, 0xffff] {
                    let address = (bank << 16) | within;
                    if let Some(offset) = g.address_to_file_offset(address, banks) {
                        assert!(offset < size);
                        assert!(g.file_offset_to_addresses(offset, banks).contains(&address));
                    }
                }
            }
        }
    }
}
