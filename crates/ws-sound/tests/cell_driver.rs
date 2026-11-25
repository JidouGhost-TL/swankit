//! Synthetic unit tests for the tick-level four-byte-cell ADSR driver.
use ws_rom::BankState;
use ws_sound::drivers::{
    cell_driver::{CellCommand, CellDriver, CellLink},
    sdk_driver::Target,
};

fn build_synthetic_cell_rom() -> (Vec<u8>, CellLink) {
    // 256 KiB ROM: bank 0xC0=0xFF maps segments C000..FFFF to 0x00000..0x3FFFF.
    // Segment D000 -> file offset 0x10000, segment C000 -> file offset 0x00000.
    let mut rom = vec![0u8; 0x40000];
    let d_base = 0x10000usize;
    let c_base = 0x00000usize;

    let link = CellLink {
        driver_segment: 0xd000,
        directory: 0x0100,
        group_count: 1,
        wave_table: 0x0200,
        volume_curve: 0x0300,
        wave_ram: 0x0400,
    };

    // Group 0: segment 0xC000, base 0
    rom[d_base + 0x0100..d_base + 0x0104].copy_from_slice(&[0x00, 0xc0, 0x00, 0x00]);

    // Wave table: wave 0 = 0x11.., wave 1 = 0x22..
    rom[d_base + 0x0200..d_base + 0x0210].fill(0x11);
    rom[d_base + 0x0210..d_base + 0x0220].fill(0x22);

    // Volume curve: row r (master_vol), col c -> (r * c) / 15
    for r in 0..16usize {
        for c in 0..16usize {
            rom[d_base + 0x0300 + r * 16 + c] = ((r * c) / 15) as u8;
        }
    }

    // Song 0 entry (13 bytes at C000:0000):
    // tracks = [0x0100, 0x0200, 0x0200, 0x0200], tempo = 0x0300, wavetab = 0x0020, flag = 0 (music)
    let entry: [u8; 13] = [
        0x00, 0x01, // t0
        0x00, 0x02, // t1 (empty)
        0x00, 0x02, // t2 (empty)
        0x00, 0x02, // t3 (empty)
        0x00, 0x03, // tempo (ADSR base)
        0x20, 0x00, // wavetab
        0x00, // flag = music
    ];
    rom[c_base..c_base + 13].copy_from_slice(&entry);
    // Wavetab at C000:0020: waves [1, 0, 0, 0]
    rom[c_base + 0x20..c_base + 0x24].copy_from_slice(&[1, 0, 0, 0]);
    // Empty track marker at C000:0204 = 0xFFFF
    rom[c_base + 0x0204..c_base + 0x0206].copy_from_slice(&[0xff, 0xff]);
    // ADSR descriptor 0 at C000:0300:
    // attack: period=0, mask=0xff, count=1 -> step 0 sets key_vol, step 1 advances to decay
    // decay: period=0, step=0x11, count=2
    // sustain: period=0, step=0x00, count=0 -> note_end
    // release: period=0, step=0x22, count=1
    rom[c_base + 0x0300..c_base + 0x0310].copy_from_slice(&[
        0, 0xff, 1, 0, // attack
        0, 0x11, 2, 0, // decay
        0, 0x00, 0, 0, // sustain
        0, 0x22, 1, 0, // release
    ]);
    // Track 0 at C000:0100 (pre-incremented by 4 -> first cell at +4):
    // +4: cmd 2 (env 0): 02 80 00 00
    // +8: note dur=3, key=0x88, div=0x0500: 03 88 00 05
    // +12: end: ff ff ff ff
    rom[c_base + 0x0104..c_base + 0x0110].copy_from_slice(&[
        0x02, 0x80, 0x00, 0x00, // cmd 2 -> env_ptr = 0x0300
        0x03, 0x88, 0x00, 0x05, // note dur=3, key=0x88, div=0x0500
        0xff, 0xff, 0xff, 0xff, // end
    ]);

    (rom, link)
}

#[test]
fn cell_driver_init_play_adsr_and_stop() {
    let (rom, link) = build_synthetic_cell_rom();
    let mut drv = CellDriver::new(link, &rom, BankState::RESET).unwrap();
    drv.init().unwrap();
    drv.dispatch(CellCommand::Play { song_id: 0 }).unwrap();
    drv.writes.clear();

    for _ in 0..5 {
        drv.tick().unwrap();
    }

    let tick0_ports: Vec<(u8, u16)> = drv
        .writes
        .iter()
        .filter(|w| w.tick == 0)
        .filter_map(|w| match w.target {
            Target::Port(p) => Some((p, w.value)),
            Target::WaveWord(_) => None,
        })
        .collect();
    // Tick 0: note divisor 0x0500 on ch0 (80h=00, 81h=05), attack vol 0x88 on 88h.
    assert_eq!(tick0_ports, vec![(0x80, 0x00), (0x81, 0x05), (0x88, 0x88)]);
    // Tick 1: attack reaches count=1 -> outputs key_vol 0x88 and advances to decay.
    let tick1_vol: Vec<u16> = drv
        .writes
        .iter()
        .filter(|w| w.tick == 1 && w.target == Target::Port(0x88))
        .map(|w| w.value)
        .collect();
    assert_eq!(tick1_vol, vec![0x88]);
    // Tick 2: decay steps down by 0x11 -> 0x77.
    let tick2_vol: Vec<u16> = drv
        .writes
        .iter()
        .filter(|w| w.tick == 2 && w.target == Target::Port(0x88))
        .map(|w| w.value)
        .collect();
    assert_eq!(tick2_vol, vec![0x77]);

    drv.stop().unwrap();
    assert!(drv
        .writes
        .iter()
        .any(|w| w.target == Target::Port(0x94) && w.value == 0));
}
