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

#[test]
fn round2_codecs_and_oversize_rle_round_trip_through_formatkit_v1() {
    let mut rom = vec![0u8; 65536];

    let cw_raw: Vec<u8> = (0..64).map(|i| ((i / 4 + i % 7) & 0xff) as u8).collect();
    let cw_enc = ws_compress::counted_word_lz::encode(&cw_raw).unwrap();
    rom[0x100..0x100 + cw_enc.len()].copy_from_slice(&cw_enc);

    let dpcm_raw: Vec<u8> = (0..80_000)
        .map(|i| (128i32 + ((i as i32 % 30) - 15)) as u8)
        .collect();
    let dpcm_enc = ws_compress::dpcm_6bit::encode(&dpcm_raw).unwrap();
    // Use a small clip inside the 64 KiB synthetic ROM and test oversize with a separate ROM below.
    let dpcm_small_raw: Vec<u8> = (0..128)
        .map(|i| (128i32 + ((i as i32 % 20) - 10)) as u8)
        .collect();
    let dpcm_small_enc = ws_compress::dpcm_6bit::encode(&dpcm_small_raw).unwrap();
    rom[0x200..0x200 + dpcm_small_enc.len()].copy_from_slice(&dpcm_small_enc);

    let lz11_raw: Vec<u8> = (0..96).map(|i| ((i / 5 + i % 9) & 0xff) as u8).collect();
    let lz11_enc = ws_compress::lzss_11_5::encode(&lz11_raw).unwrap();
    rom[0x400..0x400 + lz11_enc.len()].copy_from_slice(&lz11_enc);

    // ws.codec.flag16-distance11: mode=1, len=5, flags=0x0002, literal 'A', match(dist=1, len=4)
    let f16_enc = [1u8, 5, 0, 2, 0, b'A', 0, 8];
    rom[0x600..0x600 + f16_enc.len()].copy_from_slice(&f16_enc);

    let prle_raw: Vec<u8> = (0..64).map(|i| ((i * 5) & 0xff) as u8).collect();
    let prle_enc = ws_compress::planar_rle::encode(&prle_raw, 0, 2, 2, 2).unwrap();
    rom[0x700..0x700 + prle_enc.len()].copy_from_slice(&prle_enc);

    let tok_raw: Vec<u8> = (0..96).map(|i| ((i / 3 + i % 11) & 0xff) as u8).collect();
    let tok_enc = ws_compress::lzss_token_be::encode(&tok_raw).unwrap();
    rom[0x800..0x800 + tok_enc.len()].copy_from_slice(&tok_enc);

    // Oversize > 64 KiB RLE-77EE stream (96 KiB of repeated bytes compresses to ~1 KiB)
    let big_rle_raw = vec![0x5au8; 96 * 1024];
    let big_rle_enc = ws_compress::rle_77ee::encode(&big_rle_raw).unwrap();
    assert!(big_rle_enc.len() < 4096);
    rom[0x1000..0x1000 + big_rle_enc.len()].copy_from_slice(&big_rle_enc);

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
                "id": "cw0",
                "offset": 0x100,
                "size": cw_enc.len(),
                "sha256": sha256_hex(&cw_enc),
                "status": "mapped",
                "format_id": "ws.codec.counted-word-lz",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "dpcm0",
                "offset": 0x200,
                "size": dpcm_small_enc.len(),
                "sha256": sha256_hex(&dpcm_small_enc),
                "status": "mapped",
                "format_id": "ws.codec.dpcm-6bit",
                "mapping": {
                    "kind": "compressed",
                    "params": { "format": { "sample_rate": 6000 } },
                    "inner_format": "pcm"
                },
                "verified": true
            },
            {
                "id": "lz11",
                "offset": 0x400,
                "size": lz11_enc.len(),
                "sha256": sha256_hex(&lz11_enc),
                "status": "mapped",
                "format_id": "ws.codec.lzss-11-5",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "f16",
                "offset": 0x600,
                "size": f16_enc.len(),
                "sha256": sha256_hex(&f16_enc),
                "status": "mapped",
                "format_id": "ws.codec.flag16-distance11",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "prle0",
                "offset": 0x700,
                "size": prle_enc.len(),
                "sha256": sha256_hex(&prle_enc),
                "status": "mapped",
                "format_id": "ws.codec.planar-rle",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "tok0",
                "offset": 0x800,
                "size": tok_enc.len(),
                "sha256": sha256_hex(&tok_enc),
                "status": "mapped",
                "format_id": "ws.codec.lzss-token-be",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "rle_big",
                "offset": 0x1000,
                "size": big_rle_enc.len(),
                "sha256": sha256_hex(&big_rle_enc),
                "status": "mapped",
                "format_id": "ws.codec.rle-77ee",
                "mapping": { "kind": "compressed", "inner_format": "other" },
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

    let extracted = extract(&asset_map, &charmaps, &rom).unwrap();
    assert_eq!(extracted.failures, 0);
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
    assert_eq!(file_bytes("cw0.bin"), cw_raw.as_slice());
    assert_eq!(file_bytes("lz11.bin"), lz11_raw.as_slice());
    assert_eq!(file_bytes("f16.bin"), b"AAAAA");
    assert_eq!(file_bytes("prle0.bin"), prle_raw.as_slice());
    assert_eq!(file_bytes("tok0.bin"), tok_raw.as_slice());
    assert_eq!(file_bytes("rle_big.bin"), big_rle_raw.as_slice());

    let expected_wav = ws_sound::render::unsigned_pcm(&dpcm_small_raw, 6_000, dpcm_small_raw.len())
        .unwrap()
        .to_wav_bytes()
        .unwrap();
    assert_eq!(file_bytes("dpcm0.wav"), expected_wav.as_slice());

    // Also verify > 64 KiB dpcm-6bit directly.
    let big_dec = ws_compress::dpcm_6bit::decode(&dpcm_enc, dpcm_raw.len()).unwrap();
    assert_eq!(big_dec.bytes, dpcm_raw);
}

#[test]
fn round3_codecs_round_trip_through_formatkit_v1() {
    let mut rom = vec![0u8; 65536];

    // 1. ws.codec.lzss-be-units (unit_size=16, source_width=16): 1 unit = 16x 'A'
    let beu_enc = [0u8, 1, 1, b'A', 0, 28];
    rom[0x100..0x100 + beu_enc.len()].copy_from_slice(&beu_enc);

    // 2. ws.codec.lzss-12-ext (decoded_length=5): "ABABA"
    let l12e_enc = [4u8, b'A', b'B', 1, 0x10];
    rom[0x200..0x200 + l12e_enc.len()].copy_from_slice(&l12e_enc);

    // 3. ws.codec.nibble-lzss-ring8
    let nib_raw: Vec<u8> = (0..128).map(|i| ((i / 3 + i % 13) & 0xff) as u8).collect();
    let nib_enc = ws_compress::nibble_lzss_ring8::encode(&nib_raw).unwrap();
    rom[0x300..0x300 + nib_enc.len()].copy_from_slice(&nib_enc);

    // 4. ws.codec.byte-lzss-ring8
    let byt_raw: Vec<u8> = (0..160).map(|i| ((i / 5 + i % 17) & 0xff) as u8).collect();
    let byt_enc = ws_compress::byte_lzss_ring8::encode(&byt_raw).unwrap();
    rom[0x500..0x500 + byt_enc.len()].copy_from_slice(&byt_enc);

    // 5. ws.codec.zero-rle16 (672 bytes = 42 2bpp tiles)
    let mut zrle_raw = vec![0u8; 672];
    zrle_raw[0..4].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
    zrle_raw[320..324].copy_from_slice(&[0xaa, 0xbb, 0xcc, 0xdd]);
    let zrle_enc = ws_compress::zero_rle16::encode(&zrle_raw).unwrap();
    rom[0x700..0x700 + zrle_enc.len()].copy_from_slice(&zrle_enc);

    // 6. ws.codec.lz-tile-7b (4 tiles = 128 bytes)
    let tile7_raw: Vec<u8> = (0..128).map(|i| ((i / 4) & 0xff) as u8).collect();
    let tile7_enc = ws_compress::lz_tile_7b::encode(&tile7_raw).unwrap();
    rom[0x900..0x900 + tile7_enc.len()].copy_from_slice(&tile7_enc);

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
                "id": "beu0",
                "offset": 0x100,
                "size": beu_enc.len(),
                "sha256": sha256_hex(&beu_enc),
                "status": "mapped",
                "format_id": "ws.codec.lzss-be-units",
                "mapping": {
                    "kind": "compressed",
                    "params": { "codec_params": { "unit_size": 16, "source_width": 16 } },
                    "inner_format": "other"
                },
                "verified": true
            },
            {
                "id": "l12e0",
                "offset": 0x200,
                "size": l12e_enc.len(),
                "sha256": sha256_hex(&l12e_enc),
                "status": "mapped",
                "format_id": "ws.codec.lzss-12-ext",
                "mapping": {
                    "kind": "compressed",
                    "params": { "codec_params": { "decoded_length": 5 } },
                    "inner_format": "other"
                },
                "verified": true
            },
            {
                "id": "nib0",
                "offset": 0x300,
                "size": nib_enc.len(),
                "sha256": sha256_hex(&nib_enc),
                "status": "mapped",
                "format_id": "ws.codec.nibble-lzss-ring8",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "byt0",
                "offset": 0x500,
                "size": byt_enc.len(),
                "sha256": sha256_hex(&byt_enc),
                "status": "mapped",
                "format_id": "ws.codec.byte-lzss-ring8",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "zrle0",
                "offset": 0x700,
                "size": zrle_enc.len(),
                "sha256": sha256_hex(&zrle_enc),
                "status": "mapped",
                "format_id": "ws.codec.zero-rle16",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "tile7",
                "offset": 0x900,
                "size": tile7_enc.len(),
                "sha256": sha256_hex(&tile7_enc),
                "status": "mapped",
                "format_id": "ws.codec.lz-tile-7b",
                "mapping": { "kind": "compressed", "inner_format": "other" },
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

    let extracted = extract(&asset_map, &charmaps, &rom).unwrap();
    assert_eq!(extracted.failures, 0);
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
    assert_eq!(file_bytes("beu0.bin"), vec![b'A'; 16].as_slice());
    assert_eq!(file_bytes("l12e0.bin"), b"ABABA");
    assert_eq!(file_bytes("nib0.bin"), nib_raw.as_slice());
    assert_eq!(file_bytes("byt0.bin"), byt_raw.as_slice());
    assert_eq!(file_bytes("zrle0.bin"), zrle_raw.as_slice());
    assert_eq!(file_bytes("tile7.bin"), tile7_raw.as_slice());
}

#[test]
fn round4_and_remaining_wired_codecs_round_trip_through_formatkit_v1() {
    let mut rom = vec![0u8; 65536];

    // 1. ws-lz-command16-lsb: 2 commands, literal 'A', then distance 1 length 3 => "AAAA"
    let cmd16_enc = [0x00u8, 0x02, 0x06, b'A', 0x00, 0x10];
    rom[0x100..0x100 + cmd16_enc.len()].copy_from_slice(&cmd16_enc);

    // 2. ws-lzss-msb-distance12: "ABABA" (decoded_length=5)
    let msb12_enc = [0x20u8, b'A', b'B', 0x10, 0x00];
    rom[0x200..0x200 + msb12_enc.len()].copy_from_slice(&msb12_enc);

    // 3. ws-mask16-fill: 1 group, fill 0xee, mask 0x8001 (all fill except 14 literals)
    let mut mask16_enc = vec![1u8, 0xee, 0x01, 0x80];
    mask16_enc.extend(0u8..14);
    rom[0x300..0x300 + mask16_enc.len()].copy_from_slice(&mask16_enc);
    let mut mask16_expected: Vec<u8> = (0..7).collect();
    mask16_expected.extend_from_slice(&[0xee, 0xee]);
    mask16_expected.extend(7..14);

    // 4. ws-dpcm-4bit-nibble -> WAV export
    let dpcm4_raw: Vec<u8> = vec![
        0x20, 0x20, 0x20, 0x21, 0x23, 0x20, 0x1e, 0x60, 0x60, 0x60, 0x5f, 0x80,
    ];
    let dpcm4_enc = ws_compress::dpcm_4bit_nibble::encode(&dpcm4_raw).unwrap();
    rom[0x400..0x400 + dpcm4_enc.len()].copy_from_slice(&dpcm4_enc);

    // 5. ws-lz-rle-pat
    let lrp_raw = b"AAAABBBBCCCC_pattern_pattern_01234567".to_vec();
    let lrp_enc = ws_compress::lz_rle_pat::encode(&lrp_raw).unwrap();
    rom[0x500..0x500 + lrp_enc.len()].copy_from_slice(&lrp_enc);

    // 6a. ws-lzss-bandai-g (1-plane)
    let bg1_raw = b"BandaiG_1plane_BandaiG_1plane_01234567".to_vec();
    let bg1_enc = ws_compress::lzss_bandai_g::encode(&bg1_raw).unwrap();
    rom[0x600..0x600 + bg1_enc.len()].copy_from_slice(&bg1_enc);

    // 6b. ws-lzss-bandai-g (2-plane with 2-byte prefix header)
    let bg2_raw: Vec<u8> = (0..64u8).map(|i| if i % 2 == 0 { i & 7 } else { 0xc0 | (i & 3) }).collect();
    let mut bg2_enc = vec![0xaa, 0x55];
    bg2_enc.extend_from_slice(&ws_compress::lzss_bandai_g::encode_2plane(&bg2_raw).unwrap());
    rom[0x700..0x700 + bg2_enc.len()].copy_from_slice(&bg2_enc);

    // 7. ws-rle16-bit15
    let rle16_raw = vec![0x34, 0x12, 0x34, 0x12, 0x34, 0x12, 0x78, 0x56, 0xbc, 0x9a];
    let rle16_enc = ws_compress::rle16_bit15::encode(&rle16_raw).unwrap();
    rom[0x800..0x800 + rle16_enc.len()].copy_from_slice(&rle16_enc);

    // 8. ws-tile-xor-pack
    let mut txp_raw = vec![0u8; 32];
    txp_raw.extend((0..32u8).map(|i| if i % 4 == 0 { i + 1 } else { 0 }));
    let txp_enc = ws_compress::tile_xor_pack::encode(&txp_raw).unwrap();
    rom[0x900..0x900 + txp_enc.len()].copy_from_slice(&txp_enc);

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
                "id": "cmd16",
                "offset": 0x100,
                "size": cmd16_enc.len(),
                "sha256": sha256_hex(&cmd16_enc),
                "status": "mapped",
                "format_id": "ws.codec.lz-command16-lsb",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "msb12",
                "offset": 0x200,
                "size": msb12_enc.len(),
                "sha256": sha256_hex(&msb12_enc),
                "status": "mapped",
                "format_id": "ws.codec.lzss-msb-distance12",
                "mapping": {
                    "kind": "compressed",
                    "params": { "codec_params": { "decoded_length": 5 } },
                    "inner_format": "other"
                },
                "verified": true
            },
            {
                "id": "mask16",
                "offset": 0x300,
                "size": mask16_enc.len(),
                "sha256": sha256_hex(&mask16_enc),
                "status": "mapped",
                "format_id": "ws.codec.mask16-fill",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "dpcm4",
                "offset": 0x400,
                "size": dpcm4_enc.len(),
                "sha256": sha256_hex(&dpcm4_enc),
                "status": "mapped",
                "format_id": "ws.codec.dpcm-4bit-nibble",
                "mapping": {
                    "kind": "compressed",
                    "params": { "format": { "sample_rate": 12000 } },
                    "inner_format": "pcm"
                },
                "verified": true
            },
            {
                "id": "lrp0",
                "offset": 0x500,
                "size": lrp_enc.len(),
                "sha256": sha256_hex(&lrp_enc),
                "status": "mapped",
                "format_id": "ws.codec.lz-rle-pat",
                "mapping": {
                    "kind": "compressed",
                    "params": { "codec_params": { "decoded_length": lrp_raw.len() } },
                    "inner_format": "other"
                },
                "verified": true
            },
            {
                "id": "bg1",
                "offset": 0x600,
                "size": bg1_enc.len(),
                "sha256": sha256_hex(&bg1_enc),
                "status": "mapped",
                "format_id": "ws.codec.lzss-bandai-g",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "bg2",
                "offset": 0x700,
                "size": bg2_enc.len(),
                "sha256": sha256_hex(&bg2_enc),
                "status": "mapped",
                "format_id": "ws.codec.lzss-bandai-g",
                "mapping": {
                    "kind": "compressed",
                    "params": { "codec_params": { "planes": 2, "header_bytes": 2 } },
                    "inner_format": "other"
                },
                "verified": true
            },
            {
                "id": "rle16",
                "offset": 0x800,
                "size": rle16_enc.len(),
                "sha256": sha256_hex(&rle16_enc),
                "status": "mapped",
                "format_id": "ws.codec.rle16-bit15",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "txp0",
                "offset": 0x900,
                "size": txp_enc.len(),
                "sha256": sha256_hex(&txp_enc),
                "status": "mapped",
                "format_id": "ws.codec.tile-xor-pack",
                "mapping": { "kind": "compressed", "inner_format": "other" },
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

    let extracted = extract(&asset_map, &charmaps, &rom).unwrap();
    assert_eq!(extracted.failures, 0);
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
    assert_eq!(file_bytes("cmd16.bin"), b"AAAA");
    assert_eq!(file_bytes("msb12.bin"), b"ABABA");
    assert_eq!(file_bytes("mask16.bin"), mask16_expected.as_slice());
    let expected_wav = ws_sound::render::unsigned_pcm(&dpcm4_raw, 12_000, dpcm4_raw.len())
        .unwrap()
        .to_wav_bytes()
        .unwrap();
    assert_eq!(file_bytes("dpcm4.wav"), expected_wav.as_slice());
    assert_eq!(file_bytes("lrp0.bin"), lrp_raw.as_slice());
    assert_eq!(file_bytes("bg1.bin"), bg1_raw.as_slice());
    assert_eq!(file_bytes("bg2.bin"), bg2_raw.as_slice());
    assert_eq!(file_bytes("rle16.bin"), rle16_raw.as_slice());
    assert_eq!(file_bytes("txp0.bin"), txp_raw.as_slice());
}

#[test]
fn round5_to_7_codecs_round_trip() {
    let mut rom = vec![0u8; 65_536];

    // 1. ws.codec.lzss-11-5-direct (both zero tag and non-zero tag)
    let lz11d_raw = b"ABCDABCDABCDABCD01234567890123456789tail".to_vec();
    let lz11d_enc0 = ws_compress::lzss_11_5_direct::encode(&lz11d_raw).unwrap();
    let lz11d_enc1 = ws_compress::lzss_11_5_direct::encode_with_tag(&lz11d_raw, 0x7361).unwrap();
    rom[0x100..0x100 + lz11d_enc0.len()].copy_from_slice(&lz11d_enc0);
    rom[0x200..0x200 + lz11d_enc1.len()].copy_from_slice(&lz11d_enc1);

    // 2. ws.codec.huffman-static (both ROM tree_offset and inline tree)
    let huff_tree = vec![0u8, b'A', 1, 0, b'B', 1, 0, b'C', 0, b'D'];
    rom[0x1000..0x1000 + huff_tree.len()].copy_from_slice(&huff_tree);
    let huff_raw = b"ABACADABBAABCDDCBA".to_vec();
    let huff_enc = ws_compress::huffman_static::encode(&huff_raw, &huff_tree).unwrap();
    rom[0x300..0x300 + huff_enc.len()].copy_from_slice(&huff_enc);
    rom[0x400..0x400 + huff_enc.len()].copy_from_slice(&huff_enc);

    // 3. ws.codec.rle-lz4-cmd
    let mut rle_lz4_raw = b"PATTERN_01234567".to_vec();
    rle_lz4_raw.extend_from_slice(&[0x55; 24]);
    rle_lz4_raw.extend_from_slice(b"PATTERN_01234567");
    let rle_lz4_enc = ws_compress::rle_lz4_cmd::encode(&rle_lz4_raw).unwrap();
    rom[0x500..0x500 + rle_lz4_enc.len()].copy_from_slice(&rle_lz4_enc);

    // 4. ws.codec.word-rle-00 (carrier-bounded and explicit decoded_length)
    let wrle_words: [u16; 8] = [0x0123, 0x0000, 0x0000, 0x0000, 0x8042, 0x1234, 0x1234, 0x00ff];
    let mut wrle_raw = Vec::new();
    for w in wrle_words {
        wrle_raw.extend_from_slice(&w.to_le_bytes());
    }
    let wrle_enc = ws_compress::word_rle_00::encode(&wrle_raw).unwrap();
    rom[0x600..0x600 + wrle_enc.len()].copy_from_slice(&wrle_enc);
    rom[0x700..0x700 + wrle_enc.len()].copy_from_slice(&wrle_enc);

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
                "id": "lz11d0",
                "offset": 0x100,
                "size": lz11d_enc0.len(),
                "sha256": sha256_hex(&lz11d_enc0),
                "status": "mapped",
                "format_id": "ws.codec.lzss-11-5-direct",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "lz11d1",
                "offset": 0x200,
                "size": lz11d_enc1.len(),
                "sha256": sha256_hex(&lz11d_enc1),
                "status": "mapped",
                "format_id": "ws.codec.lzss-11-5-direct",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "huff_rom",
                "offset": 0x300,
                "size": huff_enc.len(),
                "sha256": sha256_hex(&huff_enc),
                "status": "mapped",
                "format_id": "ws.codec.huffman-static",
                "mapping": {
                    "kind": "compressed",
                    "params": {
                        "codec_params": {
                            "tree_offset": 0x1000,
                            "tree_length": huff_tree.len(),
                            "decoded_length": huff_raw.len()
                        }
                    },
                    "inner_format": "other"
                },
                "verified": true
            },
            {
                "id": "huff_inline",
                "offset": 0x400,
                "size": huff_enc.len(),
                "sha256": sha256_hex(&huff_enc),
                "status": "mapped",
                "format_id": "ws.codec.huffman-static",
                "mapping": {
                    "kind": "compressed",
                    "params": {
                        "codec_params": {
                            "tree": huff_tree,
                            "decoded_length": huff_raw.len()
                        }
                    },
                    "inner_format": "other"
                },
                "verified": true
            },
            {
                "id": "rle_lz4",
                "offset": 0x500,
                "size": rle_lz4_enc.len(),
                "sha256": sha256_hex(&rle_lz4_enc),
                "status": "mapped",
                "format_id": "ws.codec.rle-lz4-cmd",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "wrle_carrier",
                "offset": 0x600,
                "size": wrle_enc.len(),
                "sha256": sha256_hex(&wrle_enc),
                "status": "mapped",
                "format_id": "ws.codec.word-rle-00",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "wrle_sized",
                "offset": 0x700,
                "size": wrle_enc.len(),
                "sha256": sha256_hex(&wrle_enc),
                "status": "mapped",
                "format_id": "ws.codec.word-rle-00",
                "mapping": {
                    "kind": "compressed",
                    "params": { "codec_params": { "decoded_length": wrle_raw.len() } },
                    "inner_format": "other"
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

    let extracted = extract(&asset_map, &charmaps, &rom).unwrap();
    assert_eq!(extracted.failures, 0);
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
    assert_eq!(file_bytes("lz11d0.bin"), lz11d_raw.as_slice());
    assert_eq!(file_bytes("lz11d1.bin"), lz11d_raw.as_slice());
    assert_eq!(file_bytes("huff_rom.bin"), huff_raw.as_slice());
    assert_eq!(file_bytes("huff_inline.bin"), huff_raw.as_slice());
    assert_eq!(file_bytes("rle_lz4.bin"), rle_lz4_raw.as_slice());
    assert_eq!(file_bytes("wrle_carrier.bin"), wrle_raw.as_slice());
    assert_eq!(file_bytes("wrle_sized.bin"), wrle_raw.as_slice());
}

#[test]
fn round10_codecs_and_clamped_lzss_round_trip_through_formatkit_v1() {
    let mut rom = vec![0u8; 65536];

    // 1. ws.codec.dpcm-4bit-sm (6 kHz, scale 2, WAV export without explicit sample_rate)
    let dpcm_sm_pcm: Vec<u8> = vec![128, 130, 134, 132, 126, 128, 130];
    let dpcm_sm_enc = ws_compress::dpcm_4bit_sm::encode_with_params(&dpcm_sm_pcm, 2, 2).unwrap();
    rom[0x100..0x100 + dpcm_sm_enc.len()].copy_from_slice(&dpcm_sm_enc);

    // 2. ws.codec.wave-4bit-packed (12 kHz, WAV export without explicit sample_rate)
    let wave4_pcm: Vec<u8> = vec![0x40, 0x80, 0xc0, 0x80, 0x00, 0xf0];
    let wave4_enc = ws_compress::wave_4bit_packed::encode_with_rate(&wave4_pcm, 1).unwrap();
    rom[0x200..0x200 + wave4_enc.len()].copy_from_slice(&wave4_enc);

    // 3. ws.codec.lzss-be-units with clamp_output: true (overshoot 17 > 16 clamped to 16)
    let beu_clamped = [0u8, 1, 1, b'A', 0, 29];
    rom[0x300..0x300 + beu_clamped.len()].copy_from_slice(&beu_clamped);

    // 4. ws.codec.tilemap-2bit (3x2 = 6 words)
    let tm_words: [u16; 6] = [0x0000, 0x0001, 0x0123, 0x0124, 0x0124, 0x0500];
    let mut tm_raw = Vec::new();
    for w in tm_words {
        tm_raw.extend_from_slice(&w.to_le_bytes());
    }
    let tm_enc = ws_compress::tilemap_2bit::encode(3, 2, &tm_raw).unwrap();
    rom[0x400..0x400 + tm_enc.len()].copy_from_slice(&tm_enc);

    // 5. ws.codec.lzss-12-4-msb
    let msb_raw: Vec<u8> = (0..128).map(|i| ((i / 4 + i % 7) & 0xff) as u8).collect();
    let msb_enc = ws_compress::lzss_12_4_msb::encode(&msb_raw).unwrap();
    rom[0x500..0x500 + msb_enc.len()].copy_from_slice(&msb_enc);

    // 6. ws.codec.strided-rle (4 planes and 2 planes)
    let srle4_raw: Vec<u8> = (0..64).map(|i| ((i / 4 + (i % 4) * 17) & 0xff) as u8).collect();
    let srle4_enc = ws_compress::strided_rle::encode(&srle4_raw, 4).unwrap();
    rom[0x700..0x700 + srle4_enc.len()].copy_from_slice(&srle4_enc);

    let srle2_raw: Vec<u8> = (0..32).map(|i| if i % 2 == 0 { 0xaa } else { i as u8 }).collect();
    let srle2_enc = ws_compress::strided_rle::encode(&srle2_raw, 2).unwrap();
    rom[0x800..0x800 + srle2_enc.len()].copy_from_slice(&srle2_enc);

    // 7. ws.codec.dict16-rle
    let dict_raw: Vec<u8> = (0..96).map(|i| ((i / 3 + i % 11) & 0xff) as u8).collect();
    let dict_enc = ws_compress::dict16_rle::encode(&dict_raw).unwrap();
    rom[0x900..0x900 + dict_enc.len()].copy_from_slice(&dict_enc);

    // 8. ws.codec.lz44
    let lz44_raw: Vec<u8> = (0..96).map(|i| ((i / 5 + i % 4) & 0xff) as u8).collect();
    let lz44_enc = ws_compress::lz44::encode(&lz44_raw).unwrap();
    rom[0xb00..0xb00 + lz44_enc.len()].copy_from_slice(&lz44_enc);

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
                "id": "dpcm_sm0",
                "offset": 0x100,
                "size": dpcm_sm_enc.len(),
                "sha256": sha256_hex(&dpcm_sm_enc),
                "status": "mapped",
                "format_id": "ws.codec.dpcm-4bit-sm",
                "mapping": { "kind": "compressed", "inner_format": "pcm" },
                "verified": true
            },
            {
                "id": "wave4_0",
                "offset": 0x200,
                "size": wave4_enc.len(),
                "sha256": sha256_hex(&wave4_enc),
                "status": "mapped",
                "format_id": "ws.codec.wave-4bit-packed",
                "mapping": { "kind": "compressed", "inner_format": "pcm" },
                "verified": true
            },
            {
                "id": "beu_clamp",
                "offset": 0x300,
                "size": beu_clamped.len(),
                "sha256": sha256_hex(&beu_clamped),
                "status": "mapped",
                "format_id": "ws.codec.lzss-be-units",
                "mapping": {
                    "kind": "compressed",
                    "params": {
                        "codec_params": {
                            "unit_size": 16,
                            "source_width": 16,
                            "clamp_output": true
                        }
                    },
                    "inner_format": "other"
                },
                "verified": true
            },
            {
                "id": "tm2b",
                "offset": 0x400,
                "size": tm_enc.len(),
                "sha256": sha256_hex(&tm_enc),
                "status": "mapped",
                "format_id": "ws.codec.tilemap-2bit",
                "mapping": {
                    "kind": "compressed",
                    "params": { "format": { "width": 3, "height": 2 } },
                    "inner_format": "other"
                },
                "verified": true
            },
            {
                "id": "lz_msb",
                "offset": 0x500,
                "size": msb_enc.len(),
                "sha256": sha256_hex(&msb_enc),
                "status": "mapped",
                "format_id": "ws.codec.lzss-12-4-msb",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "srle4",
                "offset": 0x700,
                "size": srle4_enc.len(),
                "sha256": sha256_hex(&srle4_enc),
                "status": "mapped",
                "format_id": "ws.codec.strided-rle",
                "mapping": {
                    "kind": "compressed",
                    "params": { "codec_params": { "planes": 4 } },
                    "inner_format": "other"
                },
                "verified": true
            },
            {
                "id": "srle2",
                "offset": 0x800,
                "size": srle2_enc.len(),
                "sha256": sha256_hex(&srle2_enc),
                "status": "mapped",
                "format_id": "ws.codec.strided-rle",
                "mapping": {
                    "kind": "compressed",
                    "params": { "codec_params": { "planes": 2 } },
                    "inner_format": "other"
                },
                "verified": true
            },
            {
                "id": "dict16",
                "offset": 0x900,
                "size": dict_enc.len(),
                "sha256": sha256_hex(&dict_enc),
                "status": "mapped",
                "format_id": "ws.codec.dict16-rle",
                "mapping": { "kind": "compressed", "inner_format": "other" },
                "verified": true
            },
            {
                "id": "lz44_0",
                "offset": 0xb00,
                "size": lz44_enc.len(),
                "sha256": sha256_hex(&lz44_enc),
                "status": "mapped",
                "format_id": "ws.codec.lz44",
                "mapping": {
                    "kind": "compressed",
                    "params": { "codec_params": { "decoded_length": lz44_raw.len() } },
                    "inner_format": "other"
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

    let extracted = extract(&asset_map, &charmaps, &rom).unwrap();
    assert_eq!(extracted.failures, 0);
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

    let expected_dpcm_wav =
        ws_sound::render::unsigned_pcm(&dpcm_sm_pcm, 6_000, dpcm_sm_pcm.len())
            .unwrap()
            .to_wav_bytes()
            .unwrap();
    assert_eq!(file_bytes("dpcm_sm0.wav"), expected_dpcm_wav.as_slice());

    let expected_wave4_wav =
        ws_sound::render::unsigned_pcm(&wave4_pcm, 12_000, wave4_pcm.len())
            .unwrap()
            .to_wav_bytes()
            .unwrap();
    assert_eq!(file_bytes("wave4_0.wav"), expected_wave4_wav.as_slice());

    assert_eq!(file_bytes("beu_clamp.bin"), vec![b'A'; 16].as_slice());
    assert_eq!(file_bytes("tm2b.bin"), tm_raw.as_slice());
    assert_eq!(file_bytes("lz_msb.bin"), msb_raw.as_slice());
    assert_eq!(file_bytes("srle4.bin"), srle4_raw.as_slice());
    assert_eq!(file_bytes("srle2.bin"), srle2_raw.as_slice());
    assert_eq!(file_bytes("dict16.bin"), dict_raw.as_slice());
    assert_eq!(file_bytes("lz44_0.bin"), lz44_raw.as_slice());
}
