use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::str::FromStr;
use ws_assets::{
    extract, sha256_hex, shannon_entropy, to_formatkit_v1, verify, AssetMap, Charmaps,
    FormatkitAssetMapV1, FormatkitMappingKind, FormatkitOpaqueClass, FormatkitStatus,
    LoadedAssetMap, FORMATKIT_SCHEMA,
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

/// Strictly validate a serialized `formatkit.asset-map.v1` JSON object against
/// every constraint in `asset-map-schema.json`.
fn assert_matches_asset_map_schema(val: &Value) {
    let root = val.as_object().expect("root must be object");
    let allowed_root: BTreeSet<&str> = ["schema", "schema_version", "kit", "media", "entries"]
        .into_iter()
        .collect();
    for k in root.keys() {
        assert!(allowed_root.contains(k.as_str()), "unexpected root key {k}");
    }
    for req in &allowed_root {
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

    let allowed_entry: BTreeSet<&str> = [
        "id",
        "path",
        "offset",
        "lba",
        "size",
        "sha256",
        "status",
        "format_id",
        "entropy",
        "code_refs",
        "mapping",
        "verified",
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
            .as_deref(),
        Some("pal0")
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
