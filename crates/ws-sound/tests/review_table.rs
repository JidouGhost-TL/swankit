use ws_rom::BankState;
use ws_sound::drivers::{
    byte::{self, Dialect},
    RomView,
};
#[test]
fn byte_c_table_accepts_pcm_entries_without_aborting() {
    let mut rom = vec![0; 65536];
    rom[..2].copy_from_slice(&0x100u16.to_le_bytes());
    rom[0x100..0x109].copy_from_slice(&[0, 0xb7, 1, 0x34, 0x12, 0x78, 0x56, 0xbc, 0x9a]);
    let headers = byte::table(
        RomView::new(&rom, BankState::RESET).unwrap(),
        0,
        1,
        0xf000,
        Dialect::C,
        0,
    )
    .unwrap();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].mask, 0);
    assert!(headers[0].tracks.is_empty());
}
