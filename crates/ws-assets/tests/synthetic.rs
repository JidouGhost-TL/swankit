//! Synthetic 64 KiB images only; no cartridge bytes.
use formatkit_core::Image;
use serde_json::{json, Value};
use ws_assets::{
    decode_asset, extract, fix_checksum, render_tiles, replace, sha256_hex, verify, AssetMap,
    Charmaps, ExtractedFile, ReplaceInput, ReplaceOutcome, Replacement,
};
use ws_gfx::{decode_tile, encode_tile, TileFormat};

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

fn byte_command_map(rom: &[u8], length: Option<usize>) -> AssetMap {
    let mut value = map_json(rom);
    value["assets"][5]["codec"] = json!("ws-lz-u8-zero");
    value["assets"][5]["kind"] = json!("other");
    if let Some(length) = length {
        value["assets"][5]["length"] = json!(length);
    } else {
        value["assets"][5].as_object_mut().unwrap().remove("length");
    }
    load(&value)
}

#[test]
fn map_word_flags_owns_mode_stream_and_preserves_external_prefix() {
    let mut rom = rom();
    rom[PACKED - 2..PACKED].copy_from_slice(&[0x34, 0x12]);
    let stream = [1, 0, 0x80, 42, 0xf0, 0xff, 0, 0, 0xaa];
    rom[PACKED..PACKED + stream.len()].copy_from_slice(&stream);
    let mut value = map_json(&rom);
    value["assets"][5]["codec"] = json!("ws-lz-word16-msb");
    value["assets"][5]["kind"] = json!("other");
    value["assets"][5]["length"] = json!(8);
    let map = load(&value);
    let c = charmaps(&map);
    let decoded = decode_asset(&map, &c, &rom, "z").unwrap();
    assert_eq!(decoded.decoded, [42; 4]);
    assert_eq!(decoded.length, 8);
    for length in [7, 9] {
        value["assets"][5]["length"] = json!(length);
        let wrong = load(&value);
        assert!(decode_asset(&wrong, &charmaps(&wrong), &rom, "z").is_err());
    }
    let changed = replace(
        &map,
        &c,
        &rom,
        &Replacement {
            id: "z".into(),
            input: ReplaceInput::Bytes(vec![7; 4]),
        },
    )
    .unwrap();
    assert_eq!(&changed.rom[PACKED - 2..PACKED], &[0x34, 0x12]);
    assert_eq!(
        decode_asset(&changed.map, &charmaps(&changed.map), &changed.rom, "z")
            .unwrap()
            .decoded,
        [7; 4]
    );
    assert!(verify(&changed.map, &charmaps(&changed.map), &changed.rom)
        .unwrap()
        .ok());
    assert!(checksum_ok(&changed.rom));
    // Literal encoding can grow; ordinary capacity/reference rules still apply.
    assert!(replace(
        &map,
        &c,
        &rom,
        &Replacement {
            id: "z".into(),
            input: ReplaceInput::Bytes(vec![7; 100]),
        }
    )
    .is_err());
}

#[test]
fn map_byte_commands_extract_exact_extent_and_depend_on_source() {
    let mut rom = rom();
    let stream = [3, b'A', b'B', b'C', 0x80, 2, 0, 0xde, 0xad];
    rom[PACKED..PACKED + stream.len()].copy_from_slice(&stream);
    let map = byte_command_map(&rom, None);
    let c = charmaps(&map);
    assert!(map.asset("z").unwrap().codec.terminated());
    let decoded = decode_asset(&map, &c, &rom, "z").unwrap();
    assert_eq!(decoded.length, 7);
    assert_eq!(decoded.decoded, b"ABCABC");
    let extracted = extract(&map, &c, &rom).unwrap();
    assert_eq!(extracted.failures, 0);
    assert!(extracted.files.iter().any(|(name, file)| {
        name == "z.bin" && matches!(file, ExtractedFile::Bytes(bytes) if bytes == b"ABCABC")
    }));

    rom[PACKED + 1] = b'X';
    assert!(extract(&map, &c, &rom).is_err());
    let rebound = byte_command_map(&rom, Some(7));
    assert_eq!(
        decode_asset(&rebound, &charmaps(&rebound), &rom, "z")
            .unwrap()
            .decoded,
        b"XBCXBC"
    );
}

#[test]
fn map_byte_commands_refuse_wrong_extents_unknown_history_and_overflow() {
    let mut rom = rom();
    rom[PACKED..PACKED + 9].copy_from_slice(&[3, b'A', b'B', b'C', 0x80, 2, 0, 7, 8]);
    for length in [6, 9] {
        let map = byte_command_map(&rom, Some(length));
        assert!(decode_asset(&map, &charmaps(&map), &rom, "z").is_err());
    }
    for malformed in [&[2, b'A'][..], &[1, b'A'][..], &[0x80, 0, 0][..]] {
        rom[PACKED..PACKED + malformed.len()].copy_from_slice(malformed);
        let map = byte_command_map(&rom, Some(malformed.len()));
        assert!(decode_asset(&map, &charmaps(&map), &rom, "z").is_err());
    }
    let mut oversized = vec![1, b'A'];
    for _ in 0..505 {
        oversized.extend_from_slice(&[0xff, 0]);
    }
    oversized.push(0);
    rom[PACKED..PACKED + oversized.len()].copy_from_slice(&oversized);
    let map = byte_command_map(&rom, Some(oversized.len()));
    assert!(decode_asset(&map, &charmaps(&map), &rom, "z").is_err());
}

#[test]
fn map_byte_commands_replace_routes_to_existing_encoder() {
    let mut rom = rom();
    rom[PACKED..PACKED + 7].copy_from_slice(&[3, b'A', b'B', b'C', 0x80, 2, 0]);
    let map = byte_command_map(&rom, Some(7));
    let c = charmaps(&map);
    let replacement = vec![b'Z'; 6];
    let changed = replace(
        &map,
        &c,
        &rom,
        &Replacement {
            id: "z".into(),
            input: ReplaceInput::Bytes(replacement.clone()),
        },
    )
    .unwrap();
    let decoded = decode_asset(&changed.map, &charmaps(&changed.map), &changed.rom, "z").unwrap();
    assert_eq!(decoded.decoded, replacement);
    assert_eq!(
        changed.map.asset("z").unwrap().length.unwrap().0,
        decoded.length as u64
    );
    assert!(decoded.length <= 7);
    assert!(verify(&changed.map, &charmaps(&changed.map), &changed.rom)
        .unwrap()
        .ok());
    assert!(checksum_ok(&changed.rom));
}

#[test]
fn map_lzss_codecs_render_and_replace_with_explicit_parameters() {
    for (codec, params, stored, expected) in [
        (
            "ws-lzss-u16-zero",
            json!({"destination": "0x2001", "prehistory": "low5-prefix"}),
            vec![4, 0, 1, b'A', 0x20, 0],
            vec![b'A', 0, 0, 0],
        ),
        (
            "ws-lzss-u16-zero",
            json!({"destination": "0x2001", "prehistory": "low8-prefix"}),
            vec![4, 0, 1, b'A', 0x20, 0],
            vec![b'A', 0, b'A', 0],
        ),
        (
            "ws-lzss-u16-zero",
            json!({"destination": "0x2001", "prehistory": "zero-until-start"}),
            vec![4, 0, 1, b'A', 0x20, 0],
            vec![b'A', 0, b'A', 0],
        ),
        (
            "ws-lzss-12",
            json!({"decoded_length": 9, "history": [65, 66, 67]}),
            vec![0, 2, 0, 0, 0, 0, 0],
            b"ABCCCCCCC".to_vec(),
        ),
    ] {
        let mut rom = rom();
        rom[PACKED..PACKED + stored.len()].copy_from_slice(&stored);
        let mut v = map_json(&rom);
        v["assets"][5]["codec"] = json!(codec);
        v["assets"][5]["codec_params"] = params;
        v["assets"][5]["length"] = json!(stored.len());
        let map = load(&v);
        let c = charmaps(&map);
        assert_eq!(decode_asset(&map, &c, &rom, "z").unwrap().decoded, expected);
        let replacement = vec![0; expected.len()];
        let outcome = replace(
            &map,
            &c,
            &rom,
            &Replacement {
                id: "z".into(),
                input: ReplaceInput::Bytes(replacement.clone()),
            },
        )
        .unwrap();
        assert_eq!(
            decode_asset(&outcome.map, &c, &outcome.rom, "z")
                .unwrap()
                .decoded,
            replacement
        );
        assert!(verify(&outcome.map, &c, &outcome.rom).unwrap().ok());
    }
    let mut rom = rom();
    let tile = rom[TILES..TILES + 32].to_vec();
    let stored = ws_compress::lzss_u16_zero::encode(&tile).unwrap();
    rom[TILES..TILES + stored.len()].copy_from_slice(&stored);
    let mut v = map_json(&rom);
    v["assets"][0]["codec"] = json!("ws-lzss-u16-zero");
    v["assets"][0]["codec_params"] = json!({"destination": "0x2000", "prehistory": "low5-prefix"});
    v["assets"][0]["length"] = json!(stored.len());
    let map = load(&v);
    let c = charmaps(&map);
    let rendered = render_tiles(&map, &c, &rom, "t").unwrap();
    let outcome = replace(
        &map,
        &c,
        &rom,
        &Replacement {
            id: "t".into(),
            input: ReplaceInput::Image(rendered.clone()),
        },
    )
    .unwrap();
    assert_eq!(
        render_tiles(&outcome.map, &c, &outcome.rom, "t").unwrap(),
        rendered
    );
}

#[test]
fn map_lzss_parameters_and_extents_are_checked() {
    for (codec, params) in [
        ("ws-lzss-u16-zero", json!({})),
        (
            "ws-lzss-u16-zero",
            json!({"destination": 65536, "prehistory": "low5-prefix"}),
        ),
        (
            "ws-lzss-u16-zero",
            json!({"destination": 8192, "prehistory": "invented"}),
        ),
        ("ws-lzss-12", json!({"decoded_length": 3})),
        ("ws-lzss-12", json!({"history": []})),
        ("ws-lzss-12", json!({"decoded_length": 3, "history": [256]})),
    ] {
        let rom = rom();
        let mut v = map_json(&rom);
        v["assets"][5]["codec"] = json!(codec);
        v["assets"][5]["codec_params"] = params;
        let parsed = AssetMap::from_json(&serde_json::to_vec(&v).unwrap());
        assert!(
            parsed.is_err()
                || decode_asset(
                    &parsed.unwrap(),
                    &charmaps(&load(&map_json(&rom))),
                    &rom,
                    "z"
                )
                .is_err()
        );
    }
    let mut rom = rom();
    rom[PACKED..PACKED + 4].copy_from_slice(&[0, 2, 0, 99]);
    let mut v = map_json(&rom);
    v["assets"][5]["codec"] = json!("ws-lzss-12");
    v["assets"][5]["codec_params"] = json!({"decoded_length": 3, "history": [65, 66, 67]});
    v["assets"][5]["length"] = json!(4);
    let map = load(&v);
    assert!(decode_asset(&map, &charmaps(&map), &rom, "z").is_err());
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

#[test]
fn text_edits_keep_hidden_attribute_bits_in_the_edited_span() {
    let mut rom = rom();
    // Word glyphs A B C with attribute bits 0x1800 on B, then end ffff.
    rom[TEXT..TEXT + 8].copy_from_slice(&[0x01, 0x00, 0x02, 0x18, 0x03, 0x00, 0xff, 0xff]);
    fix_checksum(&mut rom).unwrap();
    let mut v = map_json(&rom);
    v["charmaps"]["masked"] = json!({"inline": {
        "schema": "swankit-charmap/1", "unit": "word-le",
        "glyph_mask": {"below": 32768, "mask": 2047},
        "entries": [{"bytes": "0100", "text": "A"}, {"bytes": "0200", "text": "B"}, {"bytes": "0300", "text": "C"}],
        "controls": [{"code": 65535, "ends": true}]
    }});
    v["assets"][3]["length"] = json!(8);
    v["assets"][3]["relations"]["charmap"] = json!("masked");
    let map = load(&v);
    let out = replace(
        &map,
        &charmaps(&map),
        &rom,
        &Replacement {
            id: "s".into(),
            input: ReplaceInput::Text("ACC".into()),
        },
    )
    .unwrap();
    assert_eq!(
        out.rom[TEXT..TEXT + 8],
        [0x01, 0x00, 0x03, 0x18, 0x03, 0x00, 0xff, 0xff]
    );
}

const OAM: usize = 0x700;
const SPAL: usize = 0x340;

/// Two sprites over the two-cell map: a high-priority tile-2 sprite at x=8
/// and a low-priority tile-1 sprite at x=4, h-flipped (WSdev Display/Sprites).
fn sprite_rom() -> (Vec<u8>, Value) {
    let mut rom = rom();
    for (i, word) in [0x000u16, 0x00f, 0x0f0, 0xfff].iter().enumerate() {
        rom[SPAL + 2 * i..SPAL + 2 * i + 2].copy_from_slice(&word.to_le_bytes());
    }
    // Records: attributes (tile | palette-8 << 9 | 0x2000 high | 0x4000 h-flip), y, x.
    rom[OAM..OAM + 8].copy_from_slice(&[0x01, 0x20, 0x00, 0x08, 0x00, 0x40, 0x00, 0x04]);
    fix_checksum(&mut rom).unwrap();
    let mut v = map_json(&rom);
    let assets = v["assets"].as_array_mut().unwrap();
    assets.push(
        json!({"id": "sp", "kind": "palette", "offset": SPAL, "length": 32,
        "format": {"palette_format": "rgb444", "palette_index": 8}}),
    );
    assets.push(
        json!({"id": "oam", "kind": "sprite-table", "offset": OAM, "length": 8,
        "relations": {"tiles": [{"id": "t", "at": 0}], "palettes": ["sp"]}}),
    );
    v["screens"][0]["sprites"] = json!([{"table": "oam"}]);
    (rom, v)
}

#[test]
fn screens_compose_sprite_tables_with_priority_and_flips() {
    let (rom, v) = sprite_rom();
    let map = load(&v);
    let cm = charmaps(&map);
    let image = ws_assets::render_screen(&map, &cm, &rom, "screen").unwrap();
    let blue = [0, 0, 255, 255];
    let green = [0, 255, 0, 255];
    let red = [255, 0, 0, 255];
    // Slot 1 (tile index 2 everywhere) drawn by the high sprite at x 8..15.
    assert_eq!(pixel(&image, 8, 0), green);
    assert_eq!(pixel(&image, 15, 7), green);
    // The low sprite (slot 0, index 1) covers x 4..7 where no other sprite is:
    // with the map on screen 1 it is drawn over the map.
    assert_eq!(pixel(&image, 4, 0), blue);
    // Uncovered map pixels stay the map's (slot 0, palette 0 index 1 = red).
    assert_eq!(pixel(&image, 0, 0), red);
    // On screen 2 the map hides low-priority sprites but not high ones.
    let mut v2 = v.clone();
    v2["screens"][0]["layers"][0]["screen"] = json!(2);
    let map2 = load(&v2);
    let image = ws_assets::render_screen(&map2, &cm, &rom, "screen").unwrap();
    assert_eq!(pixel(&image, 4, 0), red);
    assert_eq!(pixel(&image, 8, 0), green);
    // Extraction writes the sprite table as an image of its own.
    let out = extract(&map, &cm, &rom).unwrap();
    assert!(out.files.iter().any(|(n, _)| n == "oam.png"));
    assert_eq!(out.failures, 0);
}

#[test]
fn wrapped_layers_scroll_around_the_256_pixel_plane() {
    let rom = rom();
    let mut v = map_json(&rom);
    v["screens"][0]["layers"][0]["x"] = json!(250);
    let plain = load(&v);
    let cm = charmaps(&plain);
    let image = ws_assets::render_screen(&plain, &cm, &rom, "screen").unwrap();
    assert_eq!(pixel(&image, 0, 0), [0, 0, 0, 255]);
    v["screens"][0]["layers"][0]["wrap"] = json!(true);
    let wrapped = load(&v);
    let image = ws_assets::render_screen(&wrapped, &cm, &rom, "screen").unwrap();
    // Pixel 256 of the plane (cell 0, column 6) lands on screen x 0.
    assert_eq!(pixel(&image, 0, 0), [255, 0, 0, 255]);
}

const FONT: usize = 0x800;

/// A dialogue string in a 12-cell box of 16-pixel glyphs, 2 lines per page
/// (fd = new line, fc = new page, ff = end).
fn dialogue(text: &[u8]) -> (Vec<u8>, AssetMap) {
    let mut rom = rom();
    rom[TEXT..TEXT + text.len()].copy_from_slice(text);
    fix_checksum(&mut rom).unwrap();
    let mut v = map_json(&rom);
    v["charmaps"]["dlg"] = json!({"inline": {
        "schema": "swankit-charmap/1",
        "entries": [{"bytes": "01", "text": "Ａ"}, {"bytes": "02", "text": "ｉ"}],
        "controls": [{"code": 255, "ends": true}, {"code": 253, "display": "\n"},
                     {"code": 252, "display": "\n{page}\n"}]
    }});
    let s = &mut v["assets"][3];
    s["length"] = json!(text.len());
    s["relations"] = json!({"charmap": "dlg", "font": "f"});
    s["format"] = json!({"pad": "0xff", "line_width": 192, "lines": 2, "page_break": "{page}"});
    v["assets"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "f", "kind": "font", "offset": FONT,
        "length": 64, "format": {"bpp": 1, "tiles_wide": 2, "tiles_high": 2, "count": 2,
        "glyph_widths": {"ｉ": 8}}}));
    (rom, load(&v))
}

fn edit(map: &AssetMap, rom: &[u8], text: &str, force: bool) -> Result<ReplaceOutcome, String> {
    ws_assets::replace_with(
        map,
        &charmaps(map),
        rom,
        &Replacement {
            id: "s".into(),
            input: ReplaceInput::Text(text.into()),
        },
        &ws_assets::ReplaceOptions {
            force,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())
}

#[test]
fn too_wide_lines_are_refused_unless_forced() {
    let (rom, map) = dialogue(&[1, 0xfd, 1, 0xff, 0xff, 0xff]);
    // Twelve 16-pixel cells fill the 192-pixel line exactly.
    assert!(edit(&map, &rom, &"Ａ".repeat(12), false).is_ok());
    // The 19-cell line (304 pixels) is refused with its measure...
    let err = edit(&map, &rom, &"Ａ".repeat(19), false).unwrap_err();
    assert!(err.contains("304") && err.contains("192"), "{err}");
    // ...but narrow glyphs count at their own width: 11 * 16 + 2 * 8 = 192.
    assert!(edit(&map, &rom, &format!("{}ｉｉ", "Ａ".repeat(11)), false).is_ok());
    // Three lines on one page exceed the 2-line box; a page break resets it.
    assert!(edit(&map, &rom, "Ａ\nＡ\nＡ", false)
        .unwrap_err()
        .contains("lines"));
    assert!(edit(&map, &rom, "Ａ\nＡ\n{page}\nＡ", false).is_ok());
    // --force writes it and reports the overflow as a warning.
    let forced = edit(&map, &rom, &"Ａ".repeat(19), true).unwrap();
    assert!(forced.warnings.iter().any(|w| w.contains("304")));
}

/// Three cells all drawn by tile slot 0; tile index 2 of `t` is declared free.
fn shared_tile_map() -> (Vec<u8>, AssetMap) {
    let mut rom = rom();
    rom[MAP..MAP + 6].fill(0);
    rom[TILES + 32..TILES + 48].copy_from_slice(&tile(0));
    fix_checksum(&mut rom).unwrap();
    let mut v = map_json(&rom);
    v["assets"][0]["length"] = json!(48);
    v["assets"][0]["format"]["free_tiles"] = json!([{"first": 2, "count": 1}]);
    v["assets"][1]["length"] = json!(6);
    v["assets"][1]["format"]["width"] = json!(3);
    (rom, load(&v))
}

fn paint_cells(cells: &[(usize, [u8; 4])]) -> Image {
    let mut image = Image::try_new(24, 8, [255, 0, 0, 255].repeat(192)).unwrap();
    for &(cell, colour) in cells {
        for y in 0..8 {
            for x in cell * 8..cell * 8 + 8 {
                image.rgba[(y * 24 + x) * 4..(y * 24 + x) * 4 + 4].copy_from_slice(&colour);
            }
        }
    }
    image
}

fn edit_map(
    map: &AssetMap,
    rom: &[u8],
    image: Image,
    allocate: bool,
) -> Result<ReplaceOutcome, String> {
    ws_assets::replace_with(
        map,
        &charmaps(map),
        rom,
        &Replacement {
            id: "m".into(),
            input: ReplaceInput::Image(image),
        },
        &ws_assets::ReplaceOptions {
            allocate,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())
}

#[test]
fn allocate_gives_differently_edited_shared_cells_free_tiles() {
    let (rom, map) = shared_tile_map();
    let blue = [0, 0, 255, 255];
    // Without --allocate the shared tile cannot take two different edits.
    let err = edit_map(&map, &rom, paint_cells(&[(2, blue)]), false).unwrap_err();
    assert!(err.contains("--allocate"), "{err}");
    // With it, cell 2 gets free tile 2 (VRAM slot 2); cells 0 and 1 keep slot 0.
    let out = edit_map(&map, &rom, paint_cells(&[(2, blue)]), true).unwrap();
    assert_eq!(out.rom[MAP..MAP + 6], [0, 0, 0, 0, 2, 0]);
    assert_eq!(out.rom[TILES..TILES + 32], rom[TILES..TILES + 32]);
    let t2 = decode_tile(&out.rom, TILES + 32, TileFormat::Bpp2).unwrap();
    assert!(t2.iter().all(|&v| v == 3));
    assert!(checksum_ok(&out.rom));
    // The patched map no longer lists the used tile as free and renders the edit.
    let t = out.map.asset("t").unwrap();
    assert!(t.format.free_tiles.as_ref().is_none_or(|f| f.is_empty()));
    let cm = charmaps(&out.map);
    let image = ws_assets::render_tilemap(&out.map, &cm, &out.rom, "m").unwrap();
    assert_eq!(pixel(&image, 16, 0), blue);
    assert_eq!(pixel(&image, 0, 0), [255, 0, 0, 255]);
    // Two new versions need two free tiles: refused clearly.
    let err = edit_map(
        &map,
        &rom,
        paint_cells(&[(1, [0, 255, 0, 255]), (2, blue)]),
        true,
    )
    .unwrap_err();
    assert!(err.contains("free tile"), "{err}");
}

const BLOB: usize = 0x900;
const BLOB_REF: usize = 0x520;
const HMAP: usize = 0x980;

/// A tile blob whose u16 tile count precedes the tiles, referenced by a
/// 16-bit pointer at the count word, and a map with a u16 width/height header.
fn header_map() -> (Vec<u8>, AssetMap) {
    let mut rom = rom();
    rom[BLOB..BLOB + 2].copy_from_slice(&2u16.to_le_bytes());
    rom[BLOB + 2..BLOB + 18].copy_from_slice(&tile(1));
    rom[BLOB + 18..BLOB + 34].copy_from_slice(&tile(2));
    rom[BLOB_REF..BLOB_REF + 2].copy_from_slice(&(BLOB as u16).to_le_bytes());
    rom[HMAP..HMAP + 8].copy_from_slice(&[2, 0, 1, 0, 1, 0, 0, 0]);
    fix_checksum(&mut rom).unwrap();
    let mut v = map_json(&rom);
    let assets = v["assets"].as_array_mut().unwrap();
    assets.push(
        json!({"id": "blob", "kind": "tiles", "offset": BLOB, "length": 34,
        "format": {"bpp": 2, "width": 2, "header": "u16-count", "palette_index": 0},
        "relations": {"palette": "p"},
        "references": [{"at": BLOB_REF, "width": 2}], "references_complete": true}),
    );
    assets.push(
        json!({"id": "hm", "kind": "tilemap", "offset": HMAP, "length": 8,
        "format": {"width": 2, "height": 1, "header": "u16-width-height"},
        "relations": {"tiles": [{"id": "blob", "at": 0}], "palettes": ["p"]}}),
    );
    (rom, load(&v))
}

#[test]
fn header_counts_belong_to_the_asset_and_tiles_can_grow() {
    let (rom, map) = header_map();
    let cm = charmaps(&map);
    // The decoded payload is the tiles only; the header is checked against it.
    assert_eq!(
        decode_asset(&map, &cm, &rom, "blob").unwrap().decoded.len(),
        32
    );
    let image = ws_assets::render_tilemap(&map, &cm, &rom, "hm").unwrap();
    assert_eq!((image.width, image.height), (16, 8));
    // A sheet one row taller adds a third (non-blank) tile: the count word
    // becomes 3, the blob outgrows its slot and moves to free space.
    let mut sheet = Image::try_new(16, 16, [0, 0, 0, 255].repeat(256)).unwrap();
    let old = ws_assets::render_tiles(&map, &cm, &rom, "blob").unwrap();
    sheet.rgba[..old.rgba.len()].copy_from_slice(&old.rgba);
    for p in sheet.rgba[16 * 8 * 4..16 * 8 * 4 + 8 * 4].chunks_exact_mut(4) {
        p.copy_from_slice(&[0, 0, 255, 255]);
    }
    let out = replace(
        &map,
        &cm,
        &rom,
        &Replacement {
            id: "blob".into(),
            input: ReplaceInput::Image(sheet),
        },
    )
    .unwrap();
    let moved = out.map.asset("blob").unwrap().offset.usize().unwrap();
    assert_eq!(moved, FREE);
    assert_eq!(out.rom[BLOB_REF..BLOB_REF + 2], (FREE as u16).to_le_bytes());
    assert_eq!(out.rom[FREE..FREE + 2], 3u16.to_le_bytes());
    let cm2 = charmaps(&out.map);
    let grown = decode_asset(&out.map, &cm2, &out.rom, "blob").unwrap();
    assert_eq!(grown.decoded.len(), 48);
    assert_eq!(grown.decoded[..32], rom[BLOB + 2..BLOB + 34]);
    // A header that disagrees with the declared length is refused.
    let mut bad = rom.clone();
    bad[BLOB] = 5;
    fix_checksum(&mut bad).unwrap();
    let mut v = serde_json::to_value(&map).unwrap();
    v["rom"]["sha256"] = json!(sha256_hex(&bad));
    let bad_map = load(&v);
    assert!(decode_asset(&bad_map, &cm, &bad, "blob").is_err());
}

#[test]
fn references_refuse_stored_values_the_code_does_not_accept() {
    // A loader that switches banks only for bank bytes below 0x74.
    let bank: ws_assets::map::Encoding =
        serde_json::from_value(json!({"width": 1, "shift": 16, "below": "0x74"})).unwrap();
    let len = 0x80_0000;
    assert_eq!(
        ws_assets::encode_reference(&bank, 0x73_1234, 0x07_0000, len).unwrap(),
        [0x73]
    );
    let err = ws_assets::encode_reference(&bank, 0x74_0000, 0x07_0000, len).unwrap_err();
    assert!(err.to_string().contains("0x74"), "{err}");
    // A 16-bit offset whose bank comes from that companion bank byte may move
    // to another bank; without the declaration its implied bank is fixed.
    let fixed: ws_assets::map::Encoding = serde_json::from_value(json!({"width": 2})).unwrap();
    assert!(ws_assets::encode_reference(&fixed, 0x73_1234, 0x07_0000, len).is_err());
    let paired: ws_assets::map::Encoding =
        serde_json::from_value(json!({"width": 2, "companion": true})).unwrap();
    assert_eq!(
        ws_assets::encode_reference(&paired, 0x73_1234, 0x07_0000, len).unwrap(),
        [0x34, 0x12]
    );
}

#[test]
fn font_image_edit_preserves_padding_and_column_order() {
    let mut rom = rom();
    for i in 0..80 {
        rom[0x2000 + i] = ((i * 37) & 255) as u8;
    }
    fix_checksum(&mut rom).unwrap();
    let map = AssetMap::from_json(
        &serde_json::to_vec(&json!({
            "schema":"swankit-asset-map/1","rom":{"sha256":sha256_hex(&rom),"size":rom.len()},
            "assets":[{"id":"font","kind":"font","offset":0x2000,"length":80,
            "format":{"bpp":1,"count":2,"stride":40,"tiles_wide":2,"tiles_high":2,
            "order":"column","tile_offsets":[0,16,8,24],"width":2}}]
        }))
        .unwrap(),
    )
    .unwrap();
    let cm = Charmaps::load(&map, |_| unreachable!()).unwrap();
    let extracted = extract(&map, &cm, &rom).unwrap();
    let (_, ExtractedFile::Image(image)) = extracted
        .files
        .iter()
        .find(|(n, _)| n == "font.png")
        .unwrap()
    else {
        panic!()
    };
    let unchanged = replace(
        &map,
        &cm,
        &rom,
        &Replacement {
            id: "font".into(),
            input: ReplaceInput::Image(image.clone()),
        },
    )
    .unwrap();
    assert_eq!(unchanged.rom, rom);
    let mut edited = image.clone();
    // Tile at x=8,y=0 is column-major tile 2, explicitly stored at byte 8.
    let pixel = 8 * 4;
    let original = edited.rgba[pixel];
    edited.rgba[pixel..pixel + 4].copy_from_slice(&[
        255 - original,
        255 - original,
        255 - original,
        255,
    ]);
    let changed = replace(
        &map,
        &cm,
        &rom,
        &Replacement {
            id: "font".into(),
            input: ReplaceInput::Image(edited),
        },
    )
    .unwrap();
    assert_eq!(changed.rom[0x2008], rom[0x2008] ^ 0x80);
    assert_eq!(&changed.rom[0x2020..0x2028], &rom[0x2020..0x2028]);
    assert!(verify(&changed.map, &cm, &changed.rom)
        .unwrap()
        .errors
        .is_empty());
}

#[test]
fn tile_sheet_padding_edits_are_refused_without_changing_tile_count() {
    let rom = rom();
    let mut value = map_json(&rom);
    value["assets"][0]["format"]["width"] = json!(3);
    let map = load(&value);
    let cm = charmaps(&map);
    let sheet = render_tiles(&map, &cm, &rom, "t").unwrap();
    assert_eq!((sheet.width, sheet.height), (24, 8));
    let noop = replace(
        &map,
        &cm,
        &rom,
        &Replacement {
            id: "t".into(),
            input: ReplaceInput::Image(sheet.clone()),
        },
    )
    .unwrap();
    assert_eq!(noop.rom, rom);
    // The third cell is transparent sheet padding, outside the two tiles.
    for alpha in [1, 255] {
        let mut edited = sheet.clone();
        edited.rgba[16 * 4..16 * 4 + 4].copy_from_slice(&[0, 0, 0, alpha]);
        let error = replace(
            &map,
            &cm,
            &rom,
            &Replacement {
                id: "t".into(),
                input: ReplaceInput::Image(edited),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("padding"), "{error}");
    }
}

#[test]
fn compressed_edit_undo_redo_retains_verified_allocation() {
    let original = rom();
    let map = load(&map_json(&original));
    let c = charmaps(&map);
    let original_len = decode_asset(&map, &c, &original, "z").unwrap().length;
    let edit = |m: &AssetMap, r: &[u8], data: Vec<u8>| {
        replace(
            m,
            &c,
            r,
            &Replacement {
                id: "z".into(),
                input: ReplaceInput::Bytes(data),
            },
        )
        .unwrap()
    };
    let short = edit(&map, &original, vec![0; 96]);
    assert!(short.map.asset("z").unwrap().length.unwrap().0 < original_len as u64);
    assert_eq!(
        short.map.asset("z").unwrap().reserved_capacity.unwrap().0,
        original_len as u64
    );
    let restored = edit(&short.map, &short.rom, packed_payload());
    let again = edit(&restored.map, &restored.rom, vec![0; 96]);
    let redo = edit(&again.map, &again.rom, packed_payload());
    assert_eq!(redo.rom, original);
    assert!(checksum_ok(&short.rom) && checksum_ok(&redo.rom));
    for output in [&short, &restored, &again, &redo] {
        assert!(verify(&output.map, &c, &output.rom).unwrap().ok());
        assert_eq!(&output.rom[..PACKED], &original[..PACKED]);
        assert_eq!(
            &output.rom[PACKED + original_len..original.len() - 2],
            &original[PACKED + original_len..original.len() - 2]
        );
    }
    let unchanged = edit(&short.map, &short.rom, vec![0; 96]);
    assert_eq!(unchanged.rom, short.rom);
    assert_eq!(unchanged.map, short.map);
    let mut padded = short.map.clone();
    padded.assets[5].length = Some(ws_assets::map::Num(original_len as u64));
    assert!(decode_asset(&padded, &c, &short.rom, "z").is_err());
    let mut no_reservation = short.map.clone();
    no_reservation.assets[5].reserved_capacity = None;
    assert!(replace(
        &no_reservation,
        &c,
        &short.rom,
        &Replacement {
            id: "z".into(),
            input: ReplaceInput::Bytes(packed_payload())
        }
    )
    .is_err());
}

#[test]
fn reservation_rejects_invalid_bounds_and_conflicting_ownership() {
    let original = rom();
    let base = map_json(&original);
    let stored_len = base["assets"][5]["length"].as_u64().unwrap();
    for capacity in [0, stored_len - 1, 0x10000, u64::MAX] {
        let mut value = base.clone();
        value["assets"][5]["reserved_capacity"] = json!(capacity);
        let map = load(&value);
        let c = charmaps(&map);
        assert!(!verify(&map, &c, &original).unwrap().ok(), "{capacity}");
        assert!(
            replace(
                &map,
                &c,
                &original,
                &Replacement {
                    id: "z".into(),
                    input: ReplaceInput::Bytes(vec![0; 96])
                }
            )
            .is_err(),
            "{capacity}"
        );
    }
    for conflict in ["asset", "reservation", "free"] {
        let mut value = base.clone();
        value["assets"][5]["reserved_capacity"] = json!(64);
        match conflict {
            "asset" => value["assets"].as_array_mut().unwrap().push(json!({
                "id":"tail", "kind":"other", "offset":PACKED+32, "length":1,
                "overlaps":["z"]
            })),
            "reservation" => value["assets"].as_array_mut().unwrap().push(json!({
                "id":"before", "kind":"other", "offset":PACKED-8, "length":1,
                "reserved_capacity":16
            })),
            _ => value["free_space"]
                .as_array_mut()
                .unwrap()
                .push(json!({"offset":PACKED+32,"length":1})),
        }
        let map = load(&value);
        let c = charmaps(&map);
        assert!(!verify(&map, &c, &original).unwrap().ok(), "{conflict}");
        assert!(
            replace(
                &map,
                &c,
                &original,
                &Replacement {
                    id: "z".into(),
                    input: ReplaceInput::Bytes(vec![0; 96])
                }
            )
            .is_err(),
            "{conflict}"
        );
    }
}

#[test]
fn relocation_does_not_inherit_old_reservation() {
    let original = rom();
    let mut value = map_json(&original);
    let stored_len = value["assets"][5]["length"].as_u64().unwrap();
    value["assets"][5]["reserved_capacity"] = json!(stored_len + 1);
    value["assets"][5]["references_complete"] = json!(true);
    value["assets"][5]["references"] = json!([{"at":0x520,"width":2}]);
    let map = load(&value);
    let c = charmaps(&map);
    let output = replace(
        &map,
        &c,
        &original,
        &Replacement {
            id: "z".into(),
            input: ReplaceInput::Bytes((0..96u8).collect()),
        },
    )
    .unwrap();
    assert_ne!(
        output.map.asset("z").unwrap().offset,
        map.asset("z").unwrap().offset
    );
    assert_eq!(output.map.asset("z").unwrap().reserved_capacity, None);
    assert_eq!(
        output
            .map
            .free_space
            .iter()
            .find(|f| f.offset.0 == PACKED as u64)
            .unwrap()
            .length
            .0,
        stored_len
    );
    let no_op = replace(
        &output.map,
        &c,
        &output.rom,
        &Replacement {
            id: "z".into(),
            input: ReplaceInput::Bytes((0..96u8).collect()),
        },
    )
    .unwrap();
    assert_eq!(no_op.rom, output.rom);
    assert!(checksum_ok(&output.rom));
}

#[test]
fn cross_bank_implicit_and_explicit_streams_retain_original_allocation() {
    let payload: Vec<u8> = (0..32).collect();
    let stored = ws_compress::lz_rle_7f::encode(&payload).unwrap();
    assert!(stored.len() > 16);
    for explicit_length in [false, true] {
        let mut original = vec![0u8; 0x20000];
        original[0xfff0..0xfff0 + stored.len()].copy_from_slice(&stored);
        fix_checksum(&mut original).unwrap();
        let mut value = json!({
            "schema":"swankit-asset-map/1",
            "rom":{"sha256":sha256_hex(&original),"size":original.len()},
            "assets":[{"id":"stream","kind":"other","offset":0xfff0,"codec":"ws-lz-rle-7f"}]
        });
        if explicit_length {
            value["assets"][0]["length"] = json!(stored.len());
        }
        let map = load(&value);
        let c = charmaps(&map);
        let short = replace(
            &map,
            &c,
            &original,
            &Replacement {
                id: "stream".into(),
                input: ReplaceInput::Bytes(vec![0; 32]),
            },
        )
        .unwrap();
        assert_eq!(
            short.map.assets[0].reserved_capacity.unwrap().0,
            stored.len() as u64
        );
        let restored = replace(
            &short.map,
            &c,
            &short.rom,
            &Replacement {
                id: "stream".into(),
                input: ReplaceInput::Bytes(payload.clone()),
            },
        )
        .unwrap();
        assert_eq!(restored.rom, original);
        assert!(verify(&restored.map, &c, &restored.rom).unwrap().ok());
    }
}

#[test]
fn explicit_reservation_allows_growth_and_preserves_unwritten_tail() {
    let original = rom();
    let mut value = map_json(&original);
    value["assets"][5]["reserved_capacity"] = json!(64);
    let map = load(&value);
    let c = charmaps(&map);
    let changed = replace(
        &map,
        &c,
        &original,
        &Replacement {
            id: "z".into(),
            input: ReplaceInput::Bytes((0..40).collect()),
        },
    )
    .unwrap();
    let extent = decode_asset(&changed.map, &c, &changed.rom, "z")
        .unwrap()
        .length;
    assert!(extent > map.assets[5].length.unwrap().0 as usize);
    assert_eq!(changed.map.assets[5].reserved_capacity.unwrap().0, 64);
    assert_eq!(
        &changed.rom[PACKED + extent..original.len() - 2],
        &original[PACKED + extent..original.len() - 2]
    );
    assert!(checksum_ok(&changed.rom));
    assert!(verify(&changed.map, &c, &changed.rom).unwrap().ok());
}

#[test]
fn map_numbers_report_unsigned_limits_and_accept_decimal_and_hex() {
    use ws_assets::map::Num;
    for (text, expected) in [
        ("0", 0),
        ("48", 48),
        ("\"48\"", 48),
        ("\"0x30\"", 48),
        ("18446744073709551615", u64::MAX),
        ("\"0xffffffffffffffff\"", u64::MAX),
    ] {
        assert_eq!(serde_json::from_str::<Num>(text).unwrap().0, expected);
    }
    for text in [
        "-1",
        "\"-1\"",
        "\"-0x1\"",
        "\"not-a-number\"",
        "18446744073709551616",
        "\"18446744073709551616\"",
        "\"0x10000000000000000\"",
        "1.5",
        "true",
        "null",
        "[]",
    ] {
        let error = serde_json::from_str::<Num>(text).unwrap_err().to_string();
        assert!(error.contains("unsigned"), "{text}: {error}");
        assert!(!error.contains("Raw"), "{text}: {error}");
    }
}

#[test]
fn returned_library_maps_inline_all_compiled_file_formats_even_when_unchanged() {
    let original = rom();
    let initial = map_json(&original);
    let alphabet = initial["charmaps"]["abc"]["inline"].clone();
    for (name, bytes, unit) in [
        (
            "alphabet.json",
            serde_json::to_vec(&alphabet).unwrap(),
            None,
        ),
        (
            "definition.json",
            serde_json::to_vec(&json!({
                "schema":"swankit-text-def/1", "id":"synthetic", "map":alphabet
            }))
            .unwrap(),
            None,
        ),
        (
            "alphabet.tsv",
            b"bytes\ttext\n01\t\"A\"\n02\t\"B\"\nff\t\"!\"\n00\t\"Z\"\n".to_vec(),
            Some("byte"),
        ),
    ] {
        let mut value = initial.clone();
        value["charmaps"]["abc"] = json!({"file":name});
        if let Some(unit) = unit {
            value["charmaps"]["abc"]["unit"] = json!(unit);
        }
        let map = load(&value);
        let compiled = Charmaps::load(&map, |path| {
            assert_eq!(path, name);
            Ok(bytes.clone())
        })
        .unwrap();
        let outcome = replace(
            &map,
            &compiled,
            &original,
            &Replacement {
                id: "s".into(),
                input: ReplaceInput::Bytes(original[TEXT..TEXT + 4].to_vec()),
            },
        )
        .unwrap();
        assert_eq!(outcome.rom, original);
        let returned = AssetMap::from_json(&outcome.map.to_json().unwrap()).unwrap();
        assert!(returned.charmaps["abc"].file.is_none());
        assert!(returned.charmaps["abc"].inline.is_some());
        let portable =
            Charmaps::load(&returned, |_| panic!("returned map must be self-contained")).unwrap();
        assert_eq!(
            compiled
                .decode(map.asset("s").unwrap(), &original[TEXT..TEXT + 4])
                .unwrap()
                .text,
            portable
                .decode(returned.asset("s").unwrap(), &original[TEXT..TEXT + 4])
                .unwrap()
                .text
        );
    }
}

#[test]
fn map_counted_words_tracks_consumption_and_preserves_carrier_tail() {
    let mut rom = rom();
    let stream = [2, 0, 1, 0, 1, 0x80, 0x34, 0x12, 0xaa];
    rom[PACKED..PACKED + stream.len()].copy_from_slice(&stream);
    let mut value = map_json(&rom);
    value["assets"][5]["codec"] = json!("ws-lz-word-counted");
    value["assets"][5]["kind"] = json!("other");
    value["assets"][5]["length"] = json!(8);
    let map = load(&value);
    let decoded = decode_asset(&map, &charmaps(&map), &rom, "z").unwrap();
    assert_eq!(decoded.decoded, [0x34, 0x12].repeat(3));
    assert_eq!(decoded.length, 8);
    for length in [7, 9] {
        value["assets"][5]["length"] = json!(length);
        let wrong = load(&value);
        assert!(decode_asset(&wrong, &charmaps(&wrong), &rom, "z").is_err());
    }
    value["assets"][5].as_object_mut().unwrap().remove("length");
    let map = load(&value);
    assert_eq!(
        decode_asset(&map, &charmaps(&map), &rom, "z")
            .unwrap()
            .length,
        8
    );
}
