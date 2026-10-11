//! Synthetic 64 KiB images only; no cartridge bytes.
use formatkit_core::Image;
use serde_json::{json, Value};
use ws_assets::{
    decode_asset, encode_tile, extract, fix_checksum, replace, sha256_hex, verify, AssetMap,
    Charmaps, ExtractedFile, ReplaceInput, Replacement,
};
use ws_gfx::{decode_tile, TileFormat};

const TILES: usize = 0x100;
const MAP: usize = 0x200;
const PAL: usize = 0x300;
const TEXT: usize = 0x400;
const PTR: usize = 0x500;
const FAR: usize = 0x510;
const PACKED: usize = 0x600;
const FREE: usize = 0x1000;

fn tile(index: u8) -> [u8; 16] {
    let mut t = [0u8; 16];
    for row in t.chunks_exact_mut(2) {
        row[0] = if index & 1 != 0 { 0xff } else { 0 };
        row[1] = if index & 2 != 0 { 0xff } else { 0 };
    }
    t
}

fn packed_payload() -> Vec<u8> {
    (0..96u8).map(|i| i % 7).collect()
}

fn rom() -> Vec<u8> {
    let mut rom = vec![0u8; 0x10000];
    rom[TILES..TILES + 16].copy_from_slice(&tile(1));
    rom[TILES + 16..TILES + 32].copy_from_slice(&tile(2));
    // Two cells: tile 0, then tile 1 flipped horizontally; palette 0.
    rom[MAP..MAP + 4].copy_from_slice(&[0x00, 0x00, 0x01, 0x40]);
    for (i, word) in [0x000u16, 0xf00, 0x0f0, 0x00f].iter().enumerate() {
        rom[PAL + 2 * i..PAL + 2 * i + 2].copy_from_slice(&word.to_le_bytes());
    }
    rom[TEXT..TEXT + 4].copy_from_slice(&[0x01, 0x02, 0xff, 0x00]);
    rom[PTR..PTR + 2].copy_from_slice(&(TEXT as u16).to_le_bytes());
    // Far pointer 2000:0400 (segment form, window at file offset 0).
    rom[FAR..FAR + 4].copy_from_slice(&[0x00, 0x04, 0x00, 0x20]);
    let packed = ws_compress::lz_rle_7f::encode(&packed_payload()).unwrap();
    rom[PACKED..PACKED + packed.len()].copy_from_slice(&packed);
    rom[FREE..FREE + 0x1000].fill(0xff);
    rom[0xfff0] = 0xea;
    fix_checksum(&mut rom).unwrap();
    rom
}

fn map_json(rom: &[u8]) -> Value {
    let packed_len = ws_compress::lz_rle_7f::encode(&packed_payload())
        .unwrap()
        .len();
    json!({
        "schema": "swankit-asset-map/1",
        "rom": {"sha256": sha256_hex(rom), "size": rom.len()},
        "charmaps": {"abc": {"inline": {
            "schema": "swankit-charmap/1",
            "entries": [{"bytes": "01", "text": "A"}, {"bytes": "02", "text": "B"}, {"bytes": "03", "text": "C"}],
            "controls": [{"code": 255, "ends": true}, {"code": 254, "argument_bytes": 1, "display": "{x{args}}"}]
        }}},
        "free_space": [{"offset": FREE, "length": "0x1000", "fill": "0xff"}],
        "assets": [
            {"id": "t", "kind": "tiles", "offset": TILES, "length": 32,
             "format": {"bpp": 2, "width": 2, "palette_index": 0}, "relations": {"palette": "p"}},
            {"id": "m", "kind": "tilemap", "offset": MAP, "length": 4,
             "format": {"width": 2, "height": 1},
             "relations": {"tiles": [{"id": "t", "at": 0}], "palettes": ["p"]}},
            {"id": "p", "kind": "palette", "offset": PAL, "length": 32,
             "format": {"palette_format": "rgb444", "palette_index": 0}},
            {"id": "s", "kind": "text", "offset": TEXT, "length": 4,
             "format": {"pad": "0xff"},
             "relations": {"charmap": "abc"},
             "references": [{"at": FAR, "width": 4, "form": "far", "segment_base": "0x2000",
                             "window": 0, "normalize": "segment"}],
             "references_complete": true},
            {"id": "ptrs", "kind": "pointer-table", "offset": PTR, "length": 2,
             "format": {"count": 1, "stride": 2, "pointer": {"width": 2}},
             "relations": {"targets": ["s"]}},
            {"id": "z", "kind": "data-table", "offset": PACKED, "length": packed_len,
             "codec": "ws-lz-rle-7f"}
        ],
        "screens": [{"id": "screen", "layers": [{"tilemap": "m", "x": 0, "y": 0}], "back_color": "#000000"}]
    })
}

fn load(value: &Value) -> AssetMap {
    AssetMap::from_json(&serde_json::to_vec(value).unwrap()).unwrap()
}

fn charmaps(map: &AssetMap) -> Charmaps {
    Charmaps::load(map, |_| unreachable!("inline charmaps only")).unwrap()
}

fn pixel(image: &Image, x: usize, y: usize) -> [u8; 4] {
    let at = (y * image.width as usize + x) * 4;
    image.rgba[at..at + 4].try_into().unwrap()
}

fn checksum_ok(rom: &[u8]) -> bool {
    let n = rom.len() - 2;
    let sum = rom[..n].iter().fold(0u16, |s, &b| s.wrapping_add(b.into()));
    rom[n..] == sum.to_le_bytes()
}

#[test]
fn tile_encoder_inverts_the_hardware_decoder() {
    for format in [
        TileFormat::Bpp2,
        TileFormat::Bpp4Planar,
        TileFormat::Bpp4Packed,
    ] {
        let mut indices = [0u8; 64];
        for (i, v) in indices.iter_mut().enumerate() {
            *v = (i * 7 % format.colors()) as u8;
        }
        let bytes = encode_tile(&indices, format);
        assert_eq!(decode_tile(&bytes, 0, format).unwrap(), indices);
    }
}

#[test]
fn schema_rejects_unknown_fields_duplicates_and_dangling_relations() {
    let rom = rom();
    let good = map_json(&rom);
    load(&good);
    let mut v = good.clone();
    v["assets"][0]["colour"] = json!(1);
    assert!(AssetMap::from_json(&serde_json::to_vec(&v).unwrap()).is_err());
    let mut v = good.clone();
    v["assets"][1]["id"] = json!("t");
    assert!(AssetMap::from_json(&serde_json::to_vec(&v).unwrap()).is_err());
    let mut v = good.clone();
    v["assets"][1]["relations"]["tiles"][0]["id"] = json!("missing");
    assert!(AssetMap::from_json(&serde_json::to_vec(&v).unwrap()).is_err());
    let mut v = good;
    v["schema"] = json!("swankit-asset-map/2");
    assert!(AssetMap::from_json(&serde_json::to_vec(&v).unwrap()).is_err());
}

#[test]
fn identity_mismatch_is_refused() {
    let rom = rom();
    let map = load(&map_json(&rom));
    let mut other = rom.clone();
    other[0x10] ^= 1;
    assert!(verify(&map, &charmaps(&map), &other).is_err());
    assert!(extract(&map, &charmaps(&map), &other).is_err());
}

#[test]
fn verify_accepts_the_synthetic_map_and_reports_coverage() {
    let rom = rom();
    let map = load(&map_json(&rom));
    let report = verify(&map, &charmaps(&map), &rom).unwrap();
    assert!(report.ok(), "{:?}", report.errors);
    assert_eq!(report.references, 2);
    assert_eq!(report.free_bytes, 0x1000);
    assert!(report.mapped_bytes >= 32 + 4 + 32 + 4 + 2);
}

#[test]
fn verify_finds_wrong_references_overlaps_and_dirty_free_space() {
    let mut rom = rom();
    rom[PTR] ^= 1;
    rom[FREE + 5] = 0;
    let mut v = map_json(&rom);
    v["assets"][2]["length"] = json!(0x110); // palette now runs into the text
    let map = load(&v);
    let report = verify(&map, &charmaps(&map), &rom).unwrap();
    let text: Vec<String> = report.errors.iter().map(|f| f.message.clone()).collect();
    assert!(
        text.iter().any(|m| m.contains("reference at 0x500")),
        "{text:?}"
    );
    assert!(text.iter().any(|m| m.contains("overlaps s")), "{text:?}");
    assert!(text.iter().any(|m| m.contains("declared fill")), "{text:?}");
}

#[test]
fn extract_renders_maps_with_flips_and_writes_text_codes_and_decoded_streams() {
    let rom = rom();
    let map = load(&map_json(&rom));
    let out = extract(&map, &charmaps(&map), &rom).unwrap();
    assert_eq!(out.failures, 0);
    let file = |name: &str| out.files.iter().find(|f| f.0 == name).unwrap().1.clone();
    let ExtractedFile::Image(screen) = file("screen.png") else {
        panic!()
    };
    assert_eq!((screen.width, screen.height), (224, 144));
    let ExtractedFile::Image(m) = file("m.png") else {
        panic!()
    };
    assert_eq!((m.width, m.height), (16, 8));
    assert_eq!(pixel(&m, 0, 0), [255, 0, 0, 255]); // tile 0 = index 1 = red
    assert_eq!(pixel(&m, 15, 7), [0, 255, 0, 255]); // tile 1 = index 2 = green
    let ExtractedFile::Bytes(text) = file("s.txt") else {
        panic!()
    };
    assert_eq!(text, b"AB\n");
    let ExtractedFile::Bytes(codes) = file("s.codes.tsv") else {
        panic!()
    };
    assert!(String::from_utf8(codes)
        .unwrap()
        .starts_with("01\t\"A\"\n02\t\"B\"\nff\t\"\"\n"));
    let ExtractedFile::Bytes(z) = file("z.bin") else {
        panic!()
    };
    assert_eq!(z, packed_payload());
    assert_eq!(out.index["assets"].as_array().unwrap().len(), 6);
}

#[test]
fn tilemap_image_edits_rewrite_the_tiles_and_fix_the_checksum() {
    let rom = rom();
    let map = load(&map_json(&rom));
    let cm = charmaps(&map);
    let ExtractedFile::Image(mut image) = extract(&map, &cm, &rom)
        .unwrap()
        .files
        .into_iter()
        .find(|f| f.0 == "m.png")
        .unwrap()
        .1
    else {
        panic!()
    };
    // Paint the second (flipped) cell's leftmost column blue: tile 1's rightmost column.
    for y in 0..8 {
        let at = (y * 16 + 8) * 4;
        image.rgba[at..at + 4].copy_from_slice(&[0, 0, 255, 255]);
    }
    let out = replace(
        &map,
        &cm,
        &rom,
        &Replacement {
            id: "m".into(),
            input: ReplaceInput::Image(image),
        },
    )
    .unwrap();
    assert!(checksum_ok(&out.rom));
    assert_eq!(out.rom[MAP..MAP + 4], rom[MAP..MAP + 4]);
    let t1 = decode_tile(&out.rom, TILES + 16, TileFormat::Bpp2).unwrap();
    assert!((0..8).all(|y| t1[y * 8 + 7] == 3 && t1[y * 8] == 2));
    assert_eq!(out.rom[TILES..TILES + 16], rom[TILES..TILES + 16]);
    assert_eq!(out.map.rom.sha256, sha256_hex(&out.rom));
}

#[test]
fn conflicting_edits_of_a_shared_tile_are_refused() {
    let mut rom = rom();
    rom[MAP + 2..MAP + 4].copy_from_slice(&[0x00, 0x00]); // both cells use tile 0
    fix_checksum(&mut rom).unwrap();
    let map = load(&map_json(&rom));
    let mut image = Image::try_new(16, 8, [255, 0, 0, 255].repeat(128)).unwrap();
    image.rgba[0..4].copy_from_slice(&[0, 0, 255, 255]);
    let err = replace(
        &map,
        &charmaps(&map),
        &rom,
        &Replacement {
            id: "m".into(),
            input: ReplaceInput::Image(image),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("different edits"), "{err}");
}

#[test]
fn shorter_text_is_padded_in_place() {
    let rom = rom();
    let map = load(&map_json(&rom));
    let out = replace(
        &map,
        &charmaps(&map),
        &rom,
        &Replacement {
            id: "s".into(),
            input: ReplaceInput::Text("C".into()),
        },
    )
    .unwrap();
    assert_eq!(out.rom[TEXT..TEXT + 4], [0x03, 0xff, 0xff, 0xff]);
    assert_eq!(out.rom[PTR..PTR + 2], rom[PTR..PTR + 2]);
    assert!(checksum_ok(&out.rom));
}

#[test]
fn longer_text_relocates_into_free_space_and_rewrites_every_reference() {
    let rom = rom();
    let map = load(&map_json(&rom));
    let out = replace(
        &map,
        &charmaps(&map),
        &rom,
        &Replacement {
            id: "s".into(),
            input: ReplaceInput::Text("ABCABC".into()),
        },
    )
    .unwrap();
    assert_eq!(out.rom[FREE..FREE + 7], [1, 2, 3, 1, 2, 3, 0xff]);
    assert_eq!(out.rom[PTR..PTR + 2], (FREE as u16).to_le_bytes());
    assert_eq!(out.rom[FAR..FAR + 4], [0x00, 0x10, 0x00, 0x20]);
    assert!(checksum_ok(&out.rom));
    // The rewritten map verifies against the patched image.
    let report = verify(&out.map, &charmaps(&out.map), &out.rom).unwrap();
    assert!(report.ok(), "{:?}", report.errors);
    assert_eq!(out.map.asset("s").unwrap().offset.0, FREE as u64);
}

#[test]
fn relocation_is_refused_without_complete_references_or_outside_implied_bits() {
    let rom = rom();
    let mut v = map_json(&rom);
    v["assets"][3]["references_complete"] = json!(false);
    let map = load(&v);
    let grow = Replacement {
        id: "s".into(),
        input: ReplaceInput::Text("ABCABC".into()),
    };
    let err = replace(&map, &charmaps(&map), &rom, &grow).unwrap_err();
    assert!(err.to_string().contains("references_complete"), "{err}");
    // A one-byte pointer implies the 256-byte page: free space at 0x1000 cannot be reached.
    let mut v = map_json(&rom);
    v["assets"][4]["format"]["pointer"]["width"] = json!(1);
    v["assets"][4]["format"]["stride"] = json!(2);
    let map = load(&v);
    let err = replace(&map, &charmaps(&map), &rom, &grow).unwrap_err();
    assert!(err.to_string().contains("no declared free range"), "{err}");
}

#[test]
fn text_edits_keep_unchanged_tokens_exactly() {
    let mut rom = rom();
    // A, hidden-operand control fe 07, B, end.
    rom[TEXT..TEXT + 5].copy_from_slice(&[0x01, 0xfe, 0x07, 0x02, 0xff]);
    fix_checksum(&mut rom).unwrap();
    let mut v = map_json(&rom);
    v["assets"][3]["length"] = json!(5);
    v["charmaps"]["abc"]["inline"]["controls"][1]["display"] = json!("|");
    let map = load(&v);
    let out = replace(
        &map,
        &charmaps(&map),
        &rom,
        &Replacement {
            id: "s".into(),
            input: ReplaceInput::Text("A|C".into()),
        },
    )
    .unwrap();
    assert_eq!(out.rom[TEXT..TEXT + 5], [0x01, 0xfe, 0x07, 0x03, 0xff]);
}

#[test]
fn compressed_assets_are_reencoded_and_decode_to_the_new_payload() {
    let rom = rom();
    let map = load(&map_json(&rom));
    let cm = charmaps(&map);
    let new: Vec<u8> = vec![0; 96];
    let out = replace(
        &map,
        &cm,
        &rom,
        &Replacement {
            id: "z".into(),
            input: ReplaceInput::Bytes(new.clone()),
        },
    )
    .unwrap();
    let stored = decode_asset(&out.map, &charmaps(&out.map), &out.rom, "z").unwrap();
    assert_eq!(stored.decoded, new);
    assert!(stored.length <= decode_asset(&map, &cm, &rom, "z").unwrap().length);
    assert!(verify(&out.map, &charmaps(&out.map), &out.rom)
        .unwrap()
        .ok());
}

#[test]
fn published_schema_lists_the_same_kinds_and_codecs() {
    let schema: Value =
        serde_json::from_str(include_str!("../../../docs/asset-map.schema.json")).unwrap();
    let asset = &schema["$defs"]["asset"]["properties"];
    let kinds: Vec<&str> = asset["kind"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let ours: Vec<&str> = ws_assets::Kind::ALL.iter().map(|k| k.name()).collect();
    assert_eq!(kinds, ours);
    for codec in asset["codec"]["enum"].as_array().unwrap() {
        let parsed: ws_assets::map::Codec = serde_json::from_value(codec.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), *codec);
    }
}
