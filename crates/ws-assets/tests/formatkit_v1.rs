use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::str::FromStr;
use ws_assets::{
    extract, replace_with, sha256_hex, shannon_entropy, to_formatkit_v1, verify, AssetMap,
    Charmaps, FormatkitAssetMapV1, FormatkitMappingKind, FormatkitOpaqueClass, FormatkitStatus,
    LoadedAssetMap, ReplaceInput, ReplaceOptions, Replacement, StringOrVec, FORMATKIT_SCHEMA,
};

fn build_synthetic_rom_and_map() -> (Vec<u8>, AssetMap) {
    let mut rom = vec![0u8; 4096];

    // 0x0040..0x0060: RGB444 palette (16 colors = 32 bytes)
    for i in 0..16u16 {
        let word = (i << 8) | (i << 4) | i;
        rom[0x40 + usize::from(i) * 2..0x40 + usize::from(i) * 2 + 2]
            .copy_from_slice(&word.to_le_bytes());
    }

    // 0x0100..0x0140: 2 raw 4bpp-planar tiles (64 bytes)
    for i in 0..64usize {
        rom[0x100 + i] = ((i * 7 + 3) & 0xff) as u8;
    }

    // 0x0180..0x0188: 2x2 ws-word tilemap (8 bytes)
    for (idx, word) in [0u16, 1u16, 1u16, 0u16].iter().enumerate() {
        rom[0x180 + idx * 2..0x180 + idx * 2 + 2].copy_from_slice(&word.to_le_bytes());
    }

    // 0x0200..: ws-lz-rle-7f compressed tile (32 decoded bytes, self-delimiting)
    let raw_tile: Vec<u8> = (0..32u8).map(|i| i.wrapping_mul(5)).collect();
    let compressed = ws_compress::lz_rle_7f::encode(&raw_tile).unwrap();
    rom[0x200..0x200 + compressed.len()].copy_from_slice(&compressed);

    // 0x0300..: self-delimiting text ("AB" + 0x00 terminator)
    rom[0x300..0x303].copy_from_slice(&[0x41, 0x42, 0x00]);

    // 0x0340..0x0360: PCM unsigned 8-bit audio (32 bytes)
    for i in 0..32usize {
        rom[0x340 + i] = (128u8).wrapping_add((i * 4) as u8);
    }

    // 0x0380..0x0384: pointer table (2 entries of u16 little-endian file offsets)
    rom[0x380..0x382].copy_from_slice(&(0x0100u16).to_le_bytes());
    rom[0x382..0x384].copy_from_slice(&(0x0040u16).to_le_bytes());

    // 0x03A0..0x03B0: data table (16 bytes)
    rom[0x3a0..0x3b0].copy_from_slice(&[0x10; 16]);

    // 0x03C0..0x03C8: other blob (8 bytes)
    rom[0x3c0..0x3c8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);

    let sha = sha256_hex(&rom);
    let doc = json!({
        "schema": "swankit-asset-map/1",
        "rom": { "sha256": sha, "size": rom.len() },
        "charmaps": {
            "ascii": {
                "inline": {
                    "schema": "swankit-charmap/1",
                    "entries": [
                        { "bytes": "41", "text": "A" },
                        { "bytes": "42", "text": "B" }
                    ],
                    "controls": [
                        { "code": 0, "ends": true }
                    ]
                }
            }
        },
        "assets": [
            {
                "id": "pal0",
                "kind": "palette",
                "offset": "0x40",
                "length": "0x20",
                "format": { "palette_format": "rgb444", "palette_index": 0 },
                "confidence": "high",
                "evidence": { "loader": "0x00f000", "notes": "palette table" }
            },
            {
                "id": "tiles0",
                "kind": "tiles",
                "offset": "0x100",
                "length": "0x40",
                "format": { "bpp": 4, "layout": "planar", "width": 2 },
                "relations": { "palette": "pal0" },
                "confidence": "high",
                "evidence": { "loader": "0x00f010" }
            },
            {
                "id": "map0",
                "kind": "tilemap",
                "offset": "0x180",
                "length": "0x8",
                "format": { "width": 2, "height": 2, "entry": "ws-word" },
                "relations": {
                    "tiles": [{ "id": "tiles0", "at": 0 }],
                    "palettes": ["pal0"]
                }
            },
            {
                "id": "tiles_lz",
                "kind": "tiles",
                "offset": "0x200",
                "codec": "ws-lz-rle-7f",
                "format": { "bpp": 4, "layout": "planar", "width": 1 },
                "relations": { "palette": "pal0" }
            },
            {
                "id": "msg0",
                "kind": "text",
                "offset": "0x300",
                "relations": { "charmap": "ascii" }
            },
            {
                "id": "pcm0",
                "kind": "pcm",
                "offset": "0x340",
                "length": "0x20",
                "format": { "sample_rate": 12000 }
            },
            {
                "id": "ptrs0",
                "kind": "pointer-table",
                "offset": "0x380",
                "length": "0x4",
                "format": {
                    "pointer": { "width": 2, "endian": "little", "anchor": "file" },
                    "count": 2
                },
                "relations": { "targets": ["tiles0", "pal0"] }
            },
            {
                "id": "table0",
                "kind": "data-table",
                "offset": "0x3a0",
                "length": "0x10"
            },
            {
                "id": "blob0",
                "kind": "other",
                "offset": "0x3c0",
                "length": "0x8"
            }
        ]
    });

    let bytes = serde_json::to_vec(&doc).unwrap();
    let map = AssetMap::from_json(&bytes).unwrap();
    (rom, map)
}

fn assert_string_or_vec(val: &Value, label: &str) {
    match val {
        Value::String(s) => assert!(!s.is_empty(), "{label} string must be non-empty"),
        Value::Array(arr) => {
            assert!(!arr.is_empty(), "{label} array must be non-empty");
            for item in arr {
                let s = item
                    .as_str()
                    .unwrap_or_else(|| panic!("{label} item must be string"));
                assert!(!s.is_empty(), "{label} item must be non-empty");
            }
        }
        other => panic!("{label} must be string or array, got {other}"),
    }
}

/// Strictly validate a serialized `formatkit.asset-map.v1` JSON object against
/// every constraint in `asset-map-schema.json`.
fn assert_matches_asset_map_schema(val: &Value) {
    let root = val.as_object().expect("root must be object");
    let required_root: BTreeSet<&str> = ["schema", "schema_version", "kit", "media", "entries"]
        .into_iter()
        .collect();
    let allowed_root: BTreeSet<&str> = [
        "schema",
        "schema_version",
        "kit",
        "media",
        "free_space",
        "shared",
        "entries",
    ]
    .into_iter()
    .collect();
    for k in root.keys() {
        assert!(allowed_root.contains(k.as_str()), "unexpected root key {k}");
    }
    for req in &required_root {
        assert!(root.contains_key(*req), "missing required root key {req}");
    }
    assert_eq!(root["schema"].as_str(), Some("formatkit.asset-map.v1"));
    assert_eq!(root["schema_version"].as_u64(), Some(1));
    assert!(!root["kit"].as_str().unwrap().is_empty());

    let media = root["media"].as_object().expect("media must be object");
    let allowed_media: BTreeSet<&str> = ["kind", "fingerprint", "total_bytes", "file_count"]
        .into_iter()
        .collect();
    for k in media.keys() {
        assert!(
            allowed_media.contains(k.as_str()),
            "unexpected media key {k}"
        );
    }
    assert!(matches!(
        media["kind"].as_str(),
        Some("flat-rom" | "filesystem-disc" | "filesystem-rom")
    ));

    let fp = media["fingerprint"]
        .as_object()
        .expect("fingerprint must be object");
    let allowed_fp: BTreeSet<&str> = ["rom_sha256", "pvd_sha256", "main_exe_sha256"]
        .into_iter()
        .collect();
    for (k, v) in fp {
        assert!(allowed_fp.contains(k.as_str()), "unexpected fp key {k}");
        let s = v.as_str().unwrap();
        assert_eq!(s.len(), 64);
        assert!(s.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    if let Some(fs) = root.get("free_space") {
        let arr = fs.as_array().expect("free_space must be array");
        let allowed_fs: BTreeSet<&str> = ["offset", "size", "bank", "note"].into_iter().collect();
        for item in arr {
            let obj = item.as_object().expect("free_space item must be object");
            for k in obj.keys() {
                assert!(
                    allowed_fs.contains(k.as_str()),
                    "unexpected free_space key {k}"
                );
            }
            assert!(obj["offset"].as_u64().is_some());
            assert!(obj["size"].as_u64().is_some());
            if let Some(b) = obj.get("bank") {
                assert!(b.as_u64().is_some());
            }
            if let Some(n) = obj.get("note") {
                assert!(n.is_string());
            }
        }
    }

    if let Some(shared) = root.get("shared") {
        let obj = shared.as_object().expect("shared must be object");
        if let Some(cm) = obj.get("charmaps") {
            assert!(cm.is_object(), "shared.charmaps must be object");
        }
        if let Some(pals) = obj.get("palettes") {
            assert!(pals.is_object(), "shared.palettes must be object");
        }
        if let Some(screens) = obj.get("screens") {
            assert!(
                screens.is_object()
                    || screens
                        .as_array()
                        .is_some_and(|a| a.iter().all(Value::is_object)),
                "shared.screens must be object or array of objects"
            );
        }
    }

    let allowed_entry: BTreeSet<&str> = [
        "id",
        "path",
        "offset",
        "lba",
        "size",
        "reserved_capacity",
        "sha256",
        "status",
        "format_id",
        "entropy",
        "code_refs",
        "references",
        "mapping",
        "verified",
    ]
    .into_iter()
    .collect();
    let allowed_ref: BTreeSet<&str> = [
        "at", "width", "endian", "form", "shift", "add", "bank", "note",
    ]
    .into_iter()
    .collect();
    let entries = root["entries"].as_array().expect("entries must be array");
    for entry in entries {
        let obj = entry.as_object().expect("entry must be object");
        for k in obj.keys() {
            assert!(
                allowed_entry.contains(k.as_str()),
                "unexpected entry key {k}"
            );
        }
        assert!(!obj["id"].as_str().unwrap().is_empty());
        assert!(obj["size"].as_u64().is_some());
        if let Some(rc) = obj.get("reserved_capacity") {
            assert!(rc.as_u64().is_some());
        }
        assert!(matches!(
            obj["status"].as_str(),
            Some("identified" | "mapped" | "opaque" | "unknown")
        ));
        if let Some(sha) = obj.get("sha256") {
            let s = sha.as_str().unwrap();
            assert_eq!(s.len(), 64);
            assert!(s.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        if let Some(h) = obj.get("entropy") {
            let f = h.as_f64().unwrap();
            assert!((0.0..=8.0).contains(&f), "entropy out of range: {f}");
        }
        if let Some(refs) = obj.get("references") {
            let arr = refs.as_array().expect("references must be array");
            for r in arr {
                let robj = r.as_object().expect("reference must be object");
                for k in robj.keys() {
                    assert!(
                        allowed_ref.contains(k.as_str()),
                        "unexpected reference key {k}"
                    );
                }
                assert!(robj["at"].as_u64().is_some());
                assert!(robj["width"].as_u64().is_some_and(|w| w >= 1));
                assert!(matches!(robj["endian"].as_str(), Some("le" | "be")));
                assert!(robj["form"].as_str().is_some_and(|s| !s.is_empty()));
            }
        }
        if let Some(mapping) = obj.get("mapping") {
            let m = mapping.as_object().expect("mapping must be object");
            assert!(matches!(
                m["kind"].as_str(),
                Some(
                    "raw-image"
                        | "raw-audio"
                        | "compressed"
                        | "exe-indexed"
                        | "companion"
                        | "opaque"
                )
            ));
            if let Some(pw) = m.get("paired_with") {
                assert_string_or_vec(pw, "mapping.paired_with");
            }
            if let Some(rels) = m.get("relations") {
                let robj = rels.as_object().expect("mapping.relations must be object");
                for (rk, rv) in robj {
                    assert_string_or_vec(rv, &format!("mapping.relations.{rk}"));
                }
            }
            if let Some(oc) = m.get("opaque_class") {
                assert!(matches!(
                    oc.as_str(),
                    Some("script" | "collision" | "table" | "padding" | "bytecode" | "other")
                ));
            }
        }
    }
}

#[test]
fn round_trip_v1_to_formatkit_and_back_is_lossless() {
    let (rom, v1_map) = build_synthetic_rom_and_map();
    let v1_charmaps = Charmaps::load(&v1_map, |_| unreachable!()).unwrap();
    let v1_verify = verify(&v1_map, &v1_charmaps, &rom).unwrap();
    assert!(v1_verify.ok(), "{:?}", v1_verify.errors);
    let v1_extract = extract(&v1_map, &v1_charmaps, &rom).unwrap();
    assert_eq!(v1_extract.failures, 0);

    // Convert swankit-asset-map/1 -> formatkit.asset-map.v1
    let fk = to_formatkit_v1(&v1_map, &rom).unwrap();
    assert_eq!(fk.schema, FORMATKIT_SCHEMA);
    assert_eq!(fk.schema_version, 1);
    assert_eq!(fk.kit, "swankit");
    assert_eq!(fk.entries.len(), v1_map.assets.len());

    // Check exact schema compliance on serialized JSON
    let fk_bytes = fk.to_json().unwrap();
    let fk_json: Value = serde_json::from_slice(&fk_bytes).unwrap();
    assert_matches_asset_map_schema(&fk_json);

    // Check specific mapped fields
    let by_id = |id: &str| fk.entries.iter().find(|e| e.id == id).unwrap();
    assert_eq!(
        by_id("pal0").format_id.as_deref(),
        Some("ws.palette.rgb444")
    );
    assert_eq!(
        by_id("pal0").mapping.as_ref().unwrap().kind,
        FormatkitMappingKind::RawImage
    );
    assert_eq!(by_id("pal0").verified, Some(true));
    assert_eq!(
        by_id("pal0").code_refs.as_deref(),
        Some(&["0x00f000".to_owned()][..])
    );

    assert_eq!(
        by_id("tiles0").format_id.as_deref(),
        Some("ws.tiles.4bpp-planar")
    );
    assert_eq!(
        by_id("tiles0")
            .mapping
            .as_ref()
            .unwrap()
            .paired_with
            .as_ref(),
        Some(&StringOrVec::One("pal0".into()))
    );
    assert_eq!(
        by_id("map0").mapping.as_ref().unwrap().paired_with.as_ref(),
        Some(&StringOrVec::Many(vec!["pal0".into(), "tiles0".into()]))
    );

    assert_eq!(
        by_id("tiles_lz").format_id.as_deref(),
        Some("ws.codec.lz-rle-7f")
    );
    assert_eq!(
        by_id("tiles_lz").mapping.as_ref().unwrap().kind,
        FormatkitMappingKind::Compressed
    );
    assert!(by_id("tiles_lz").size > 0);

    assert_eq!(by_id("msg0").size, 3); // exact consumed length including 0x00 terminator
    assert_eq!(
        by_id("msg0").mapping.as_ref().unwrap().opaque_class,
        Some(FormatkitOpaqueClass::Script)
    );

    assert_eq!(by_id("pcm0").format_id.as_deref(), Some("ws.pcm.u8"));
    assert_eq!(
        by_id("pcm0").mapping.as_ref().unwrap().kind,
        FormatkitMappingKind::RawAudio
    );

    assert_eq!(
        by_id("ptrs0").mapping.as_ref().unwrap().kind,
        FormatkitMappingKind::ExeIndexed
    );

    assert_eq!(
        by_id("table0").mapping.as_ref().unwrap().kind,
        FormatkitMappingKind::Opaque
    );
    assert_eq!(
        by_id("table0").mapping.as_ref().unwrap().opaque_class,
        Some(FormatkitOpaqueClass::Table)
    );
    assert_eq!(by_id("table0").entropy, Some(0.0)); // uniform 0x10 bytes

    assert_eq!(
        by_id("blob0").mapping.as_ref().unwrap().kind,
        FormatkitMappingKind::Opaque
    );
    assert_eq!(
        by_id("blob0").mapping.as_ref().unwrap().opaque_class,
        Some(FormatkitOpaqueClass::Other)
    );
    assert_eq!(by_id("blob0").entropy, Some(3.0)); // 8 distinct bytes -> log2(8) = 3.0

    // Auto-detect via AssetMap::from_json AND FromStr
    let round_tripped = AssetMap::from_json(&fk_bytes).unwrap();
    let round_tripped_str = AssetMap::from_str(std::str::from_utf8(&fk_bytes).unwrap()).unwrap();
    assert_eq!(round_tripped, round_tripped_str);

    // Verify & extract round-tripped AssetMap -> identical outputs!
    let rt_charmaps = Charmaps::load(&round_tripped, |_| unreachable!()).unwrap();
    let rt_verify = verify(&round_tripped, &rt_charmaps, &rom).unwrap();
    assert!(rt_verify.ok(), "{:?}", rt_verify.errors);
    assert_eq!(rt_verify.mapped_bytes, v1_verify.mapped_bytes);
    assert_eq!(rt_verify.references, v1_verify.references);

    let rt_extract = extract(&round_tripped, &rt_charmaps, &rom).unwrap();
    assert_eq!(rt_extract.failures, 0);
    assert_eq!(rt_extract.files, v1_extract.files);

    // LoadedAssetMap verify and markdown
    let loaded = LoadedAssetMap::from_json(&fk_bytes).unwrap();
    assert_eq!(loaded.schema(), FORMATKIT_SCHEMA);
    let loaded_report = loaded.verify(&rt_charmaps, &rom).unwrap();
    assert!(loaded_report.ok(), "{:?}", loaded_report.errors);
    let md = loaded.render_markdown();
    assert!(md.contains("| `tiles0` |"));
    assert!(md.contains("`ws.tiles.4bpp-planar`"));
    assert!(md.contains("`formatkit.asset-map.v1`"));
}

#[test]
fn formatkit_v1_rejects_unknown_fields_and_tampered_sha256() {
    let (rom, v1_map) = build_synthetic_rom_and_map();
    let mut fk = to_formatkit_v1(&v1_map, &rom).unwrap();

    // Reject unknown root field
    let mut bad_root: Value = serde_json::from_slice(&fk.to_json().unwrap()).unwrap();
    bad_root["extra_root"] = json!(true);
    assert!(FormatkitAssetMapV1::from_json(&serde_json::to_vec(&bad_root).unwrap()).is_err());

    // Reject unknown entry field
    let mut bad_entry: Value = serde_json::from_slice(&fk.to_json().unwrap()).unwrap();
    bad_entry["entries"][0]["extra_entry"] = json!(123);
    assert!(FormatkitAssetMapV1::from_json(&serde_json::to_vec(&bad_entry).unwrap()).is_err());

    // Tampered slice SHA-256 fails LoadedAssetMap::verify
    fk.entries[0].sha256 = Some("0".repeat(64));
    let tampered = LoadedAssetMap::FormatkitV1(fk.clone());
    let rt_map = tampered.to_asset_map().unwrap();
    let charmaps = Charmaps::load(&rt_map, |_| unreachable!()).unwrap();
    let report = tampered.verify(&charmaps, &rom).unwrap();
    assert!(!report.ok());
    assert!(report
        .errors
        .iter()
        .any(|e| e.message.contains("slice SHA-256")));

    // Wrong ROM SHA-256 fails LoadedAssetMap::verify with Err
    fk.media.fingerprint.rom_sha256 = Some("f".repeat(64));
    let wrong_rom = LoadedAssetMap::FormatkitV1(fk);
    assert!(wrong_rom.verify(&charmaps, &rom).is_err());

    // Entropy bounds check
    assert_eq!(shannon_entropy(&[]), 0.0);
    assert_eq!(shannon_entropy(&[0xaa; 32]), 0.0);
    let all_bytes: Vec<u8> = (0..=255).collect();
    assert_eq!(shannon_entropy(&all_bytes), 8.0);
    assert_eq!(FormatkitStatus::Mapped.as_str(), "mapped");
}

#[test]
fn editable_map_with_free_space_shared_references_and_relations_round_trips_and_replaces() {
    let mut rom = vec![0u8; 65536];
    // Fill free space region 0x8000..0x9000 with 0xff
    rom[0x8000..0x9000].fill(0xff);

    // 0x0040..0x0060: 16-color RGB444 palette
    for i in 0..16u16 {
        let word = (i << 8) | (i << 4) | i;
        rom[0x40 + usize::from(i) * 2..0x40 + usize::from(i) * 2 + 2]
            .copy_from_slice(&word.to_le_bytes());
    }
    // 0x0100..0x0140: 2 raw 4bpp-planar tiles
    for i in 0..64usize {
        rom[0x100 + i] = ((i * 3 + 1) & 0xff) as u8;
    }
    // 0x0180..0x0188: 2x2 tilemap
    for (idx, word) in [0u16, 1u16, 1u16, 0u16].iter().enumerate() {
        rom[0x180 + idx * 2..0x180 + idx * 2 + 2].copy_from_slice(&word.to_le_bytes());
    }
    // 0x0200..0x0203: text "AB\0" + 5 bytes reserved capacity (0x0200..0x0208)
    rom[0x200..0x203].copy_from_slice(&[0x41, 0x42, 0x00]);
    // 0x0020..0x0022: u16 little-endian file pointer to msg0 (0x0200)
    rom[0x0020..0x0022].copy_from_slice(&(0x0200u16).to_le_bytes());

    let sha = sha256_hex(&rom);
    let doc = json!({
        "schema": "swankit-asset-map/1",
        "rom": { "sha256": sha, "size": rom.len() },
        "charmaps": {
            "ascii": {
                "inline": {
                    "schema": "swankit-charmap/1",
                    "entries": [
                        { "bytes": "41", "text": "A" },
                        { "bytes": "42", "text": "B" }
                    ],
                    "controls": [
                        { "code": 0, "ends": true }
                    ]
                }
            }
        },
        "palettes": {
            "mono_inline": {
                "colors": ["#ffffff", "#aaaaaa", "#555555", "#000000"],
                "evidence": "inline monochrome shades"
            }
        },
        "free_space": [
            { "offset": "0x8000", "length": "0x1000", "fill": "0xff", "note": "unused tail" }
        ],
        "screens": [
            {
                "id": "screen.title",
                "layers": [{ "tilemap": "map0", "x": 0, "y": 0 }],
                "back_color": "#000000"
            }
        ],
        "assets": [
            {
                "id": "pal0",
                "kind": "palette",
                "offset": "0x40",
                "length": "0x20",
                "format": { "palette_format": "rgb444", "palette_index": 0 }
            },
            {
                "id": "tiles0",
                "kind": "tiles",
                "offset": "0x100",
                "length": "0x40",
                "format": { "bpp": 4, "layout": "planar", "width": 2 },
                "relations": { "palette": "pal0" }
            },
            {
                "id": "map0",
                "kind": "tilemap",
                "offset": "0x180",
                "length": "0x8",
                "format": { "width": 2, "height": 2, "entry": "ws-word" },
                "relations": {
                    "tiles": [{ "id": "tiles0", "at": 0 }],
                    "palettes": ["pal0", "mono_inline"]
                }
            },
            {
                "id": "msg0",
                "kind": "text",
                "offset": "0x200",
                "reserved_capacity": "0x8",
                "relations": { "charmap": "ascii" },
                "references": [
                    {
                        "at": "0x20",
                        "width": 2,
                        "endian": "little",
                        "anchor": "file",
                        "kind": "pointer-table-entry",
                        "note": "title string pointer"
                    }
                ],
                "references_complete": true
            }
        ]
    });

    let v1_map = AssetMap::from_json(&serde_json::to_vec(&doc).unwrap()).unwrap();
    let fk = to_formatkit_v1(&v1_map, &rom).unwrap();
    let fk_bytes = fk.to_json().unwrap();
    let fk_json: Value = serde_json::from_slice(&fk_bytes).unwrap();
    assert_matches_asset_map_schema(&fk_json);

    // Verify top-level free_space and shared fields are populated
    assert_eq!(fk.free_space.len(), 1);
    assert_eq!(fk.free_space[0].offset, 0x8000);
    assert_eq!(fk.free_space[0].size, 0x1000);
    assert_eq!(fk.free_space[0].note.as_deref(), Some("unused tail"));
    let shared = fk.shared.as_ref().expect("shared must be present");
    assert!(shared.charmaps.as_ref().unwrap().contains_key("ascii"));
    assert!(shared
        .palettes
        .as_ref()
        .unwrap()
        .contains_key("mono_inline"));
    assert_eq!(shared.screens.as_ref().unwrap().len(), 1);

    // Verify per-entry reserved_capacity, references, multi-paired_with, and relations
    let msg0 = fk.entries.iter().find(|e| e.id == "msg0").unwrap();
    assert_eq!(msg0.reserved_capacity, Some(8));
    assert_eq!(msg0.references.len(), 1);
    assert_eq!(msg0.references[0].at, 0x20);
    assert_eq!(msg0.references[0].width, 2);
    assert_eq!(msg0.references[0].form, "linear");
    let map0 = fk.entries.iter().find(|e| e.id == "map0").unwrap();
    assert_eq!(
        map0.mapping.as_ref().unwrap().paired_with.as_ref(),
        Some(&StringOrVec::Many(vec![
            "pal0".into(),
            "mono_inline".into(),
            "tiles0".into()
        ]))
    );
    assert_eq!(
        map0.mapping.as_ref().unwrap().relations.get("palettes"),
        Some(&StringOrVec::Many(vec![
            "pal0".into(),
            "mono_inline".into()
        ]))
    );

    // Round-trip back to AssetMap and verify 100% lossless preservation
    let rt_map = AssetMap::from_json(&fk_bytes).unwrap();
    assert_eq!(rt_map.free_space, v1_map.free_space);
    assert_eq!(rt_map.charmaps, v1_map.charmaps);
    assert_eq!(rt_map.palettes, v1_map.palettes);
    assert_eq!(rt_map.screens, v1_map.screens);
    assert_eq!(
        rt_map.asset("msg0").unwrap().reserved_capacity,
        v1_map.asset("msg0").unwrap().reserved_capacity
    );
    assert_eq!(
        rt_map.asset("msg0").unwrap().references,
        v1_map.asset("msg0").unwrap().references
    );

    // Verify, extract (including screen.title!), and replace with relocation into free_space!
    let rt_charmaps = Charmaps::load(&rt_map, |_| unreachable!()).unwrap();
    let report = verify(&rt_map, &rt_charmaps, &rom).unwrap();
    assert!(report.ok(), "{:?}", report.errors);
    assert_eq!(report.free_bytes, 0x1000);
    assert_eq!(report.references, 1);

    let extracted = extract(&rt_map, &rt_charmaps, &rom).unwrap();
    assert_eq!(extracted.failures, 0);
    assert!(extracted
        .files
        .iter()
        .any(|(name, _)| name == "screen.title.png"));

    // Grow msg0 beyond reserved_capacity (8 bytes) -> relocates into 0x8000 free_space and patches 0x20!
    let outcome = replace_with(
        &rt_map,
        &rt_charmaps,
        &rom,
        &Replacement {
            id: "msg0".into(),
            input: ReplaceInput::Text("AABBAABBAABB".into()),
        },
        &ReplaceOptions::default(),
    )
    .unwrap();
    assert_eq!(outcome.map.asset("msg0").unwrap().offset.0, 0x8000);
    assert_eq!(
        u16::from_le_bytes([outcome.rom[0x20], outcome.rom[0x21]]),
        0x8000
    );
}

#[test]
fn new_pcm_s8_rle_77ee_and_rle_pcm_00_round_trip() {
    let mut rom = vec![0u8; 65_536];
    let pcm_s8_raw: Vec<u8> = vec![0x80, 0xc0, 0xff, 0x00, 0x01, 0x40, 0x7f, 0x00];
    rom[0x100..0x100 + pcm_s8_raw.len()].copy_from_slice(&pcm_s8_raw);

    let rle77_raw: Vec<u8> = vec![
        0x00, 0x00, 0x00, 0x00, 0x77, 0xee, 0x12, 0x34, 0x12, 0x34, 0x10, 0x00, 0x11, 0x00, 0x12,
        0x00,
    ];
    let rle77_enc = ws_compress::rle_77ee::encode(&rle77_raw).unwrap();
    rom[0x200..0x200 + rle77_enc.len()].copy_from_slice(&rle77_enc);

    let rle_pcm_raw: Vec<u8> = vec![
        0x80, 0x80, 0x80, 0x80, 0x80, 0x00, 0x7f, 0x81, 0x82, 0x00, 0x00,
    ];
    let rle_pcm_enc = ws_compress::rle_pcm_00::encode(&rle_pcm_raw).unwrap();
    rom[0x300..0x300 + rle_pcm_enc.len()].copy_from_slice(&rle_pcm_enc);

    let sha = sha256_hex(&rom);
    let fk_doc = json!({
        "schema": FORMATKIT_SCHEMA,
        "schema_version": 1,
        "kit": "swankit",
        "media": {
            "kind": "flat-rom",
            "fingerprint": { "rom_sha256": sha },
            "total_bytes": rom.len()
        },
        "entries": [
            {
                "id": "voice_s8",
                "offset": 0x100,
                "size": pcm_s8_raw.len(),
                "sha256": sha256_hex(&pcm_s8_raw),
                "status": "mapped",
                "format_id": "ws.pcm.s8",
                "mapping": {
                    "kind": "raw-audio",
                    "params": { "format": { "sample_rate": 12000 } },
                    "inner_format": "pcm"
                },
                "verified": true
            },
            {
                "id": "blob_rle77",
                "offset": 0x200,
                "size": rle77_enc.len(),
                "sha256": sha256_hex(&rle77_enc),
                "status": "mapped",
                "format_id": "ws.codec.rle-77ee",
                "mapping": {
                    "kind": "compressed",
                    "inner_format": "other"
                },
                "verified": true
            },
            {
                "id": "voice_rle_pcm",
                "offset": 0x300,
                "size": rle_pcm_enc.len(),
                "sha256": sha256_hex(&rle_pcm_enc),
                "status": "mapped",
                "format_id": "ws.codec.rle-pcm-00",
                "mapping": {
                    "kind": "compressed",
                    "params": { "format": { "sample_rate": 6000 } },
                    "inner_format": "pcm"
                },
                "verified": true
            }
        ]
    });

    let fk_bytes = serde_json::to_vec(&fk_doc).unwrap();
    let loaded = LoadedAssetMap::from_json(&fk_bytes).unwrap();
    let asset_map = loaded.to_asset_map().unwrap();
    let charmaps = Charmaps::load(&asset_map, |_| unreachable!()).unwrap();

    let report = loaded.verify(&charmaps, &rom).unwrap();
    assert!(report.ok(), "{:?}", report.errors);

    // Format IDs round-trip cleanly through to_formatkit_v1.
    let roundtripped = to_formatkit_v1(&asset_map, &rom).unwrap();
    let fid = |id: &str| {
        roundtripped
            .entries
            .iter()
            .find(|e| e.id == id)
            .unwrap()
            .format_id
            .as_deref()
    };
    assert_eq!(fid("voice_s8"), Some("ws.pcm.s8"));
    assert_eq!(fid("blob_rle77"), Some("ws.codec.rle-77ee"));
    assert_eq!(fid("voice_rle_pcm"), Some("ws.codec.rle-pcm-00"));

    // Extract checks WAV output for s8 PCM and decoded bytes for both codecs.
    let extracted = extract(&asset_map, &charmaps, &rom).unwrap();
    assert_eq!(extracted.failures, 0);
    let expected_s8_wav = ws_sound::render::signed_pcm(&pcm_s8_raw, 12_000, pcm_s8_raw.len())
        .unwrap()
        .to_wav_bytes()
        .unwrap();
    let expected_rle_pcm_wav =
        ws_sound::render::unsigned_pcm(&rle_pcm_raw, 6_000, rle_pcm_raw.len())
            .unwrap()
            .to_wav_bytes()
            .unwrap();
    let file_bytes = |name: &str| -> &[u8] {
        extracted
            .files
            .iter()
            .find_map(|(n, f)| match (n == name, f) {
                (true, ws_assets::ExtractedFile::Bytes(b)) => Some(b.as_slice()),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(file_bytes("voice_s8.wav"), expected_s8_wav.as_slice());
    assert_eq!(file_bytes("blob_rle77.bin"), rle77_raw.as_slice());
    assert_eq!(
        file_bytes("voice_rle_pcm.wav"),
        expected_rle_pcm_wav.as_slice()
    );

    // In-place replacement re-encodes with rle-77ee and rle-pcm-00.
    let new_blob = vec![0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00];
    let replaced = replace_with(
        &asset_map,
        &charmaps,
        &rom,
        &Replacement {
            id: "blob_rle77".into(),
            input: ReplaceInput::Bytes(new_blob.clone()),
        },
        &ReplaceOptions::default(),
    )
    .unwrap();
    let decoded_after = ws_assets::decode_asset(
        &replaced.map,
        &charmaps,
        &replaced.rom,
        "blob_rle77",
    )
    .unwrap();
    assert_eq!(decoded_after.decoded, new_blob);
}
