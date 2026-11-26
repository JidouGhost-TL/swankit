use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;
use ws_assets::{sha256_hex, to_formatkit_v1, AssetMap};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "swankit-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn synthetic_rom_and_maps() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut rom = vec![0u8; 2048];
    for i in 0..16u16 {
        let word = (i << 8) | (i << 4) | i;
        rom[0x40 + usize::from(i) * 2..0x40 + usize::from(i) * 2 + 2]
            .copy_from_slice(&word.to_le_bytes());
    }
    for i in 0..64usize {
        rom[0x100 + i] = ((i * 5 + 1) & 0xff) as u8;
    }
    let sha = sha256_hex(&rom);
    let v1_json = serde_json::to_vec_pretty(&json!({
        "schema": "swankit-asset-map/1",
        "rom": { "sha256": sha, "size": rom.len() },
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
            }
        ]
    }))
    .unwrap();
    let v1_map = AssetMap::from_json(&v1_json).unwrap();
    let fk_json = to_formatkit_v1(&v1_map, &rom).unwrap().to_json().unwrap();
    (rom, v1_json, fk_json)
}

#[test]
fn map_verify_markdown_and_assets_export_cli() {
    let dir = temp_dir("map-cli");
    let (rom, v1_json, fk_json) = synthetic_rom_and_maps();
    let rom_path = dir.join("test.wsc");
    let v1_path = dir.join("map_v1.json");
    let fk_path = dir.join("map_fk.json");
    std::fs::write(&rom_path, &rom).unwrap();
    std::fs::write(&v1_path, &v1_json).unwrap();
    std::fs::write(&fk_path, &fk_json).unwrap();

    // 1. `swankit map verify` passes on both schemas (human + --json)
    for (label, path, expected_schema) in [
        ("v1", &v1_path, "swankit-asset-map/1"),
        ("fk", &fk_path, "formatkit.asset-map.v1"),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_swankit"))
            .args([
                "map",
                "verify",
                rom_path.to_str().unwrap(),
                path.to_str().unwrap(),
                "--json",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{label} map verify failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let parsed: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(parsed["schema"].as_str(), Some(expected_schema));
        assert_eq!(parsed["ok"].as_bool(), Some(true));
        assert_eq!(parsed["mapped_bytes"].as_u64(), Some(96));
        assert_eq!(parsed["rom_bytes"].as_u64(), Some(2048));
        assert!(parsed["coverage_pct"].as_f64().unwrap() > 4.0);
    }

    // 2. `swankit map verify` fails on tampered slice SHA-256
    let mut tampered: Value = serde_json::from_slice(&fk_json).unwrap();
    tampered["entries"][0]["sha256"] = json!("0".repeat(64));
    let tampered_path = dir.join("tampered_slice.json");
    std::fs::write(
        &tampered_path,
        serde_json::to_vec_pretty(&tampered).unwrap(),
    )
    .unwrap();
    let bad_slice = Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args([
            "map",
            "verify",
            rom_path.to_str().unwrap(),
            tampered_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!bad_slice.status.success());
    let stdout = String::from_utf8_lossy(&bad_slice.stdout);
    assert!(stdout.contains("slice SHA-256"), "{stdout}");

    // 3. `swankit map verify` fails on wrong ROM SHA-256
    let mut wrong_rom: Value = serde_json::from_slice(&fk_json).unwrap();
    wrong_rom["media"]["fingerprint"]["rom_sha256"] = json!("f".repeat(64));
    let wrong_rom_path = dir.join("wrong_rom.json");
    std::fs::write(
        &wrong_rom_path,
        serde_json::to_vec_pretty(&wrong_rom).unwrap(),
    )
    .unwrap();
    let bad_rom = Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args([
            "map",
            "verify",
            rom_path.to_str().unwrap(),
            wrong_rom_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!bad_rom.status.success());
    let stderr = String::from_utf8_lossy(&bad_rom.stderr);
    assert!(stderr.contains("ROM SHA-256"), "{stderr}");

    // 4. `swankit map markdown` works on both schemas
    for path in [&v1_path, &fk_path] {
        let md_out = Command::new(env!("CARGO_BIN_EXE_swankit"))
            .args(["map", "markdown", path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            md_out.status.success(),
            "{}",
            String::from_utf8_lossy(&md_out.stderr)
        );
        let md = String::from_utf8_lossy(&md_out.stdout);
        assert!(md.contains("| ID | Offset | Size | Format ID | Kind | Status | Verified |"));
        assert!(md.contains("| `pal0` |"));
        assert!(md.contains("| `tiles0` |"));
    }

    // 5. `swankit assets export --asset-map` matches `assets extract` on both schemas
    let out_v1 = dir.join("export_v1");
    let out_fk = dir.join("export_fk");
    for (map_p, out_p) in [(&v1_path, &out_v1), (&fk_path, &out_fk)] {
        let exp = Command::new(env!("CARGO_BIN_EXE_swankit"))
            .args([
                "assets",
                "export",
                rom_path.to_str().unwrap(),
                "--asset-map",
                map_p.to_str().unwrap(),
                "-o",
                out_p.to_str().unwrap(),
                "--json",
            ])
            .output()
            .unwrap();
        assert!(
            exp.status.success(),
            "{}",
            String::from_utf8_lossy(&exp.stderr)
        );
        let parsed: Value = serde_json::from_slice(&exp.stdout).unwrap();
        assert_eq!(parsed["failures"].as_u64(), Some(0));
        assert_eq!(parsed["files"].as_u64(), Some(2));
    }
    assert_eq!(
        std::fs::read(out_v1.join("tiles0.png")).unwrap(),
        std::fs::read(out_fk.join("tiles0.png")).unwrap()
    );
    assert_eq!(
        std::fs::read(out_v1.join("pal0.png")).unwrap(),
        std::fs::read(out_fk.join("pal0.png")).unwrap()
    );

    // 6. `map upgrade` must NOT exist on the public CLI
    let no_upgrade = Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args(["map", "upgrade", fk_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!no_upgrade.status.success());

    // 7. Root `--help` lists `[--json]` on `save set`, `save fix`, `patch apply`, plus `map verify`/`markdown` and `assets export`
    let help = Command::new(env!("CARGO_BIN_EXE_swankit"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    let help_str = String::from_utf8_lossy(&help.stdout);
    for needle in [
        "save set DEF_OR_AUTO SAVE --field INDEX --value JSON --output SAVE [--all-copies] [--skip-validation] [--no-repair] [--context JSON] [--json]",
        "save fix DEF_OR_AUTO SAVE --output SAVE [--context JSON] [--json]",
        "patch apply PATCH BASE --output FILE [--overlap reject|last-write-wins] [--growth reject|zero-fill] [--json]",
        "map verify ROM MAP [--json]",
        "map markdown MAP",
        "export|extract|replace|verify",
    ] {
        assert!(help_str.contains(needle), "missing {needle:?} in --help");
    }

    std::fs::remove_dir_all(dir).unwrap();
}
