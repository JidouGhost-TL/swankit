//! Sprite priority is relative to screen 2; earlier sprites win across groups.
//! <https://ws.nesdev.org/wiki/Display/Sprites>
use ws_gfx::{RenderProfile, Snapshot};

#[test]
fn earlier_low_sprite_occludes_later_high_sprite() {
    let mut ram = [0; 65536];
    // Tiles 1, 2, 3 are solid indices 1, 2, 3.
    for (tile, row) in [(1, [255, 0]), (2, [0, 255]), (3, [255, 255])] {
        for y in 0..8 {
            ram[0x2000 + tile * 16 + y * 2..0x2002 + tile * 16 + y * 2].copy_from_slice(&row);
        }
    }
    for e in ram[0x800..0x1000].chunks_exact_mut(2) {
        e.copy_from_slice(&3u16.to_le_bytes());
    }
    ram[0x600..0x608].copy_from_slice(&[1, 0, 0, 0, 2, 0x20, 0, 0]);
    let mut p = [0; 256];
    p[0] = 4;
    p[4] = 3;
    p[6] = 2;
    p[7] = 0x10;
    p[0x1c] = 0x20;
    p[0x1d] = 0x64;
    p[0x30] = 0x10;
    p[0x31] = 0x32;
    let s = Snapshot::new(&ram, &p, false).unwrap();
    assert_eq!(
        &s.render(RenderProfile::Hardware).unwrap().rgba[..4],
        &[221, 221, 221, 255]
    );
    assert_eq!(
        &s.render(RenderProfile::JavaDiagnostic).unwrap().rgba[..4],
        &[187, 187, 187, 255]
    );
    // Screen 2 hides the earlier low sprite; the later high sprite stays occluded.
    p[0] |= 2;
    p[0x20] = 0x10;
    p[0x21] = 0x32;
    let s = Snapshot::new(&ram, &p, false).unwrap();
    assert_eq!(
        &s.render(RenderProfile::Hardware).unwrap().rgba[..4],
        &[153, 153, 153, 255]
    );
    assert_eq!(
        &s.render(RenderProfile::JavaDiagnostic).unwrap().rgba[..4],
        &[187, 187, 187, 255]
    );
}
