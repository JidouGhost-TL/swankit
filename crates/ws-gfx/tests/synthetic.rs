use formatkit_core::Image;
use ws_gfx::{
    decode_tile, render_atlas, render_tile, transparent, Arrangement, AtlasLayout, ColorPalettes,
    MapEntry, MonoPalettes, RenderProfile, Snapshot, Sprite, TileFormat,
};

fn ports() -> [u8; 256] {
    let mut p = [0; 256];
    p[0x1c..0x20].copy_from_slice(&[0x20, 0x64, 0xa8, 0xfe]);
    for pal in 0..16 {
        p[0x20 + pal * 2..0x22 + pal * 2].copy_from_slice(&[0x10, 0x32]);
    }
    p
}
fn pixel(i: &Image, x: usize, y: usize) -> [u8; 4] {
    i.rgba[(y * i.width as usize + x) * 4..(y * i.width as usize + x + 1) * 4]
        .try_into()
        .unwrap()
}
fn fill_tile(ram: &mut [u8], tile: usize, index: u8) {
    for row in ram[0x2000 + tile * 16..0x2010 + tile * 16].chunks_exact_mut(2) {
        row[0] = if index & 1 != 0 { 255 } else { 0 };
        row[1] = if index & 2 != 0 { 255 } else { 0 };
    }
}
fn map(ram: &mut [u8], base: usize, word: u16) {
    for e in ram[base..base + 2048].chunks_exact_mut(2) {
        e.copy_from_slice(&word.to_le_bytes());
    }
}
fn sprite(ram: &mut [u8], i: usize, word: u16, x: u8, y: u8) {
    let o = 0x600 + i * 4;
    ram[o..o + 2].copy_from_slice(&word.to_le_bytes());
    ram[o + 2] = y;
    ram[o + 3] = x;
}
fn scene() -> (Vec<u8>, [u8; 256]) {
    let mut ram = vec![0; 65536];
    for t in 1..4 {
        fill_tile(&mut ram, t, t as u8);
    }
    map(&mut ram, 0, 1);
    map(&mut ram, 0x800, 2);
    let mut p = ports();
    p[0] = 7;
    p[7] = 0x10;
    p[4] = 3;
    p[6] = 1;
    sprite(&mut ram, 0, 3, 0, 0);
    (ram, p)
}
fn render(ram: &[u8], p: &[u8]) -> Image {
    Snapshot::new(ram, p, false)
        .unwrap()
        .render(RenderProfile::Hardware)
        .unwrap()
}

#[test]
fn early_milestone_2bpp_mono_tile_set_to_image() {
    let bytes = [0xaa, 0xcc].repeat(16);
    let indices = decode_tile(&bytes, 0, TileFormat::Bpp2).unwrap();
    assert_eq!(&indices[..8], &[3, 2, 1, 0, 3, 2, 1, 0]);
    let p = ports();
    let mono = MonoPalettes::from_ports(&p).unwrap();
    let out = render_atlas(
        &bytes,
        TileFormat::Bpp2,
        mono.palette(0).unwrap(),
        AtlasLayout {
            arrangement: Arrangement::PLAIN,
            columns: 2,
            scale: 1,
            max_bytes: 512,
        },
    )
    .unwrap();
    assert_eq!((out.width, out.height, out.rgba.len()), (16, 8, 512));
    for (x, g) in [153, 187, 221, 255].into_iter().enumerate() {
        assert_eq!(pixel(&out, x, 0), [g, g, g, 255]);
    }
    assert_eq!(pixel(&out, 0, 0), pixel(&out, 8, 0));
}

#[test]
fn tile_bit_order_and_row_order() {
    let expected = [7, 6, 5, 4, 11, 10, 9, 8].repeat(8);
    for (data, fmt) in [
        ([0xaa, 0xcc, 0xf0, 0x0f].repeat(8), TileFormat::Bpp4Planar),
        ([0x76, 0x54, 0xba, 0x98].repeat(8), TileFormat::Bpp4Packed),
    ] {
        assert_eq!(decode_tile(&data, 0, fmt).unwrap().as_slice(), expected);
    }
    let mut bytes = [0; 16];
    bytes[0] = 0x80;
    bytes[15] = 1;
    let t = decode_tile(&bytes, 0, TileFormat::Bpp2).unwrap();
    assert_eq!((t[0], t[7], t[56], t[63]), (1, 0, 0, 2));
    assert_eq!(TileFormat::Bpp2.tile_count(31), 1);
}

#[test]
fn palettes_all_shades_rgb444_and_transparency() {
    let mut p = ports();
    for pal in 0..16 {
        p[0x20 + pal * 2] = (pal as u8 & 7) | 0x80;
    }
    let mono = MonoPalettes::from_ports(&p).unwrap();
    for (i, g) in [255, 221, 187, 153, 119, 85, 17, 0].into_iter().enumerate() {
        assert_eq!(mono.shade(i as u8).unwrap(), [g, g, g, 255]);
    }
    for pal in 0..16 {
        assert_eq!(mono.palette(pal).unwrap()[0], mono.shade(pal & 7).unwrap());
    }
    let mut bytes = Vec::new();
    for p in 0u16..16 {
        for i in 0u16..16 {
            bytes.extend_from_slice(&(0xf000 | (p << 8) | (i << 4) | (15 - i)).to_le_bytes());
        }
    }
    let colors = ColorPalettes::parse(&bytes).unwrap();
    for p in 0..16 {
        for i in 0..16 {
            assert_eq!(
                colors.palette(p).unwrap()[i],
                [p * 17, i as u8 * 17, (15 - i) as u8 * 17, 255]
            );
        }
    }
    for p in 0..16 {
        assert_eq!(transparent(p, 0, TileFormat::Bpp2), p & 4 != 0);
        assert!(transparent(p, 0, TileFormat::Bpp4Packed));
        assert!(!transparent(p, 1, TileFormat::Bpp4Planar));
    }
    assert!(mono.shade(8).is_err());
    assert!(mono.palette(16).is_err());
    assert!(colors.palette(16).is_err());
}

#[test]
fn arrangements_scale_and_pad() {
    let tiles = [0x11u8, 0x22, 0x33, 0x44, 0x55]
        .into_iter()
        .flat_map(|b| [b; 32])
        .collect::<Vec<_>>();
    let pal: Vec<_> = (0..16).map(|i| [i, 0, 0, 255]).collect();
    for (arr, expected) in [
        (Arrangement::GLYPH_ROW, [1, 2, 3, 4]),
        (Arrangement::GLYPH_COLUMN, [1, 3, 2, 4]),
    ] {
        let out = render_atlas(
            &tiles,
            TileFormat::Bpp4Packed,
            &pal,
            AtlasLayout {
                arrangement: arr,
                columns: 2,
                scale: 2,
                max_bytes: 8192,
            },
        )
        .unwrap();
        assert_eq!((out.width, out.height), (64, 32));
        for ((x, y), e) in [(0, 0), (16, 0), (0, 16), (16, 16)]
            .into_iter()
            .zip(expected)
        {
            assert_eq!(pixel(&out, x, y)[0], e);
            assert_eq!(pixel(&out, x + 1, y + 1)[0], e);
        }
        assert_eq!(pixel(&out, 32, 0)[0], 5);
        assert_eq!(pixel(&out, 48, 16), [0, 0, 0, 255]);
    }
}

#[test]
fn attributes_and_priority() {
    let e = MapEntry::from_word(0xffff, true);
    assert_eq!(
        (e.tile, e.palette, e.bank, e.h_flip, e.v_flip),
        (1023, 15, true, true, true)
    );
    assert_eq!(MapEntry::from_word(0xffff, false).tile, 511);
    assert_eq!(MapEntry::parse(&[0xff, 0xff], true).unwrap(), [e]);
    let s = Sprite::parse(&[0xff, 0xff, 200, 252]).unwrap();
    assert_eq!((s.tile, s.palette, s.y, s.x), (511, 15, 200, 252));
    assert!(s.high_priority && s.h_flip && s.v_flip && !s.inside_window);
    let (mut ram, mut p) = scene();
    assert_eq!(pixel(&render(&ram, &p), 0, 0)[0], 187);
    sprite(&mut ram, 0, 0x2003, 0, 0);
    assert_eq!(pixel(&render(&ram, &p), 0, 0)[0], 153);
    p[6] = 2;
    sprite(&mut ram, 1, 0x2001, 0, 0);
    assert_eq!(pixel(&render(&ram, &p), 0, 0)[0], 153);
    p[0] = 1;
    assert_eq!(pixel(&render(&ram, &p), 0, 0)[0], 221);
    p[0] = 0;
    p[1] = 7;
    assert_eq!(pixel(&render(&ram, &p), 0, 0)[0], 0);
}

#[test]
fn inclusive_windows_and_outside_modes() {
    let (mut ram, mut p) = scene();
    p[0] = 0x23;
    p[8..12].copy_from_slice(&[2, 3, 4, 5]);
    let out = render(&ram, &p);
    assert_eq!(pixel(&out, 2, 3)[0], 187);
    assert_eq!(pixel(&out, 4, 5)[0], 187);
    assert_eq!(pixel(&out, 1, 3)[0], 221);
    assert_eq!(pixel(&out, 4, 6)[0], 221);
    p[0] |= 0x10;
    let out = render(&ram, &p);
    assert_eq!(pixel(&out, 2, 3)[0], 221);
    assert_eq!(pixel(&out, 1, 3)[0], 187);
    p[0] = 0x0d;
    p[12..16].copy_from_slice(&[2, 3, 4, 5]);
    sprite(&mut ram, 0, 0x2003, 0, 0);
    let out = render(&ram, &p);
    assert_eq!(pixel(&out, 2, 3)[0], 153);
    assert_eq!(pixel(&out, 1, 3)[0], 221);
    sprite(&mut ram, 0, 0x3003, 0, 0);
    let out = render(&ram, &p);
    assert_eq!(pixel(&out, 2, 3)[0], 221);
    assert_eq!(pixel(&out, 1, 3)[0], 153);
    p[8..12].copy_from_slice(&[4, 5, 2, 3]);
    p[0] = 0x23;
    assert_eq!(pixel(&render(&ram, &p), 2, 3)[0], 221);
}

#[test]
fn scroll_flips_sprite_wrap_range_and_alpha() {
    let (mut ram, mut p) = scene();
    p[0] = 1;
    p[0x10] = 255;
    p[0x11] = 255;
    ram[0x2010..0x2020].fill(0);
    ram[0x2010] = 0x80;
    map(&mut ram, 0, 0xc001);
    let out = render(&ram, &p);
    assert_eq!(pixel(&out, 0, 0)[0], 221);
    assert_eq!(pixel(&out, 1, 0)[0], 255);
    let (mut ram, mut p) = scene();
    p[0] = 5;
    sprite(&mut ram, 0, 0x2003, 252, 252);
    let out = render(&ram, &p);
    assert_eq!(pixel(&out, 0, 0)[0], 153);
    assert_eq!(pixel(&out, 4, 0)[0], 221);
    p[5] = 127;
    p[6] = 255;
    sprite(&mut ram, 127, 0x2003, 0, 0);
    let s = Snapshot::new(&ram, &p, false).unwrap();
    assert_eq!(s.sprites(RenderProfile::Hardware).unwrap().len(), 1);
    let mut sprite = s.sprites(RenderProfile::Hardware).unwrap()[0];
    sprite.tile = 0;
    sprite.palette = 12;
    assert_eq!(
        pixel(
            &s.render_sprite(sprite, RenderProfile::Hardware).unwrap(),
            0,
            0
        ),
        [0; 4]
    );
}

#[test]
fn documented_bank_masks_and_scanline_limit_vs_java() {
    let mut ram = vec![0; 65536];
    fill_tile(&mut ram, 0, 1);
    fill_tile(&mut ram, 512, 2);
    map(&mut ram, 0, 0x2000);
    ram[0xfe02..0xfe04].copy_from_slice(&0x0f00u16.to_le_bytes());
    ram[0xfe04..0xfe06].copy_from_slice(&0x000fu16.to_le_bytes());
    let mut p = ports();
    p[0] = 1;
    p[0x60] = 0x80;
    let s = Snapshot::new(&ram, &p, true).unwrap();
    assert_eq!(
        pixel(&s.render(RenderProfile::Hardware).unwrap(), 0, 0),
        [0, 0, 255, 255]
    );
    assert_eq!(
        pixel(&s.render(RenderProfile::JavaDiagnostic).unwrap(), 0, 0),
        [255, 0, 0, 255]
    );
    let (mut ram, mut p) = scene();
    p[0] = 1;
    p[7] = 8;
    map(&mut ram, 0x4000, 2);
    let s = Snapshot::new(&ram, &p, false).unwrap();
    assert_eq!(
        pixel(&s.render(RenderProfile::Hardware).unwrap(), 0, 0)[0],
        221
    );
    assert_eq!(
        pixel(&s.render(RenderProfile::JavaDiagnostic).unwrap(), 0, 0)[0],
        187
    );
    p[0] = 5;
    p[7] = 0;
    p[6] = 33;
    for i in 0..32 {
        sprite(&mut ram, i, 0x2003, 240, 0);
    }
    sprite(&mut ram, 32, 0x2003, 0, 0);
    let s = Snapshot::new(&ram, &p, false).unwrap();
    assert_eq!(
        pixel(&s.render(RenderProfile::Hardware).unwrap(), 0, 0)[0],
        221
    );
    assert_eq!(
        pixel(&s.render(RenderProfile::JavaDiagnostic).unwrap(), 0, 0)[0],
        153
    );
}

#[test]
fn raster_ports_and_layer_alpha() {
    let (ram, mut p) = scene();
    p[0] = 0;
    let mut rows = vec![p; 144];
    rows[1][1] = 7;
    let s = Snapshot::new(&ram, &p, false)
        .unwrap()
        .with_line_ports(&rows)
        .unwrap();
    let out = s.render(RenderProfile::Hardware).unwrap();
    assert_eq!(pixel(&out, 0, 0)[0], 255);
    assert_eq!(pixel(&out, 0, 1)[0], 0);
    let (mut ram, p) = scene();
    map(&mut ram, 0, 0x800);
    let out = Snapshot::new(&ram, &p, false)
        .unwrap()
        .render_layer(0, RenderProfile::Hardware, [0; 4])
        .unwrap();
    assert_eq!(pixel(&out, 0, 0), [0; 4]);
}

#[test]
fn malformed_inputs_and_allocation_limits_return_errors() {
    for format in [
        TileFormat::Bpp2,
        TileFormat::Bpp4Planar,
        TileFormat::Bpp4Packed,
    ] {
        for n in 0..format.bytes_per_tile() {
            assert!(decode_tile(&[0; 32][..n], 0, format).is_err());
        }
        assert!(decode_tile(&[0; 32], usize::MAX, format).is_err());
    }
    for n in 0..512 {
        assert!(ColorPalettes::parse(&[0; 512][..n]).is_err());
    }
    assert!(ColorPalettes::parse(&[0; 513]).is_err());
    for n in 0..4 {
        assert!(Sprite::parse(&[0; 4][..n]).is_err());
    }
    assert!(MapEntry::parse(&[0; 3], true).is_err());
    assert!(MapEntry::parse(&[0; 2050], true).is_err());
    assert!(MonoPalettes::from_ports(&[0; 63]).is_err());
    assert!(Snapshot::new(&[0; 65535], &ports(), false).is_err());
    assert!(Snapshot::new(&[0; 65536], &[0; 255], false).is_err());
    assert!(Snapshot::new(&[0; 65536], &ports(), false)
        .unwrap()
        .with_line_ports(&[[0; 256]; 143])
        .is_err());
    assert!(render_tile(&[15; 64], &[[0; 4]; 4]).is_err());
    let mut layout = AtlasLayout {
        arrangement: Arrangement::PLAIN,
        columns: 1,
        scale: 1,
        max_bytes: 255,
    };
    assert!(render_atlas(&[0; 16], TileFormat::Bpp2, &[[0; 4]; 4], layout).is_err());
    layout.columns = u32::MAX;
    assert!(render_atlas(&[0; 16], TileFormat::Bpp2, &[[0; 4]; 4], layout).is_err());
    layout.columns = 1;
    layout.arrangement.tiles_wide = 0;
    assert!(render_atlas(&[0; 16], TileFormat::Bpp2, &[[0; 4]; 4], layout).is_err());
    let ram = [0; 65536];
    let p = ports();
    let s = Snapshot::new(&ram, &p, false).unwrap();
    assert!(s.map(16, RenderProfile::Hardware).is_err());
    let mut e = MapEntry::from_word(0, false);
    e.tile = u16::MAX;
    assert!(s
        .render_map(&[e], 1, RenderProfile::Hardware, [0; 4], 256)
        .is_err());
}
