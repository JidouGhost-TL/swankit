//! External frozen asset maps with independent decodes; synthetic runs skip cleanly.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{corpus_root, hash_bytes, required_mode, run_dir, Args, Digest};
use serde_json::{json, Map, Value};
use std::{
    cell::Cell,
    path::{Component, Path, PathBuf},
};
use ws_assets::{decode_asset, verify, AssetMap, Charmaps};

struct AssetDigest {
    /// The selection directory (`ws-assets` or `ws-assets-<suffix>`).
    root: PathBuf,
    assets: Cell<usize>,
    references: Cell<usize>,
    independent: Cell<usize>,
}

impl Digest for AssetDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let name = args
            .get("map")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Malformed("asset spec lacks map".into()))?;
        let path = Path::new(name);
        let parts: Vec<_> = path.components().collect();
        if parts.len() != 2
            || parts[0] != Component::Normal("maps".as_ref())
            || path.extension().is_none_or(|ext| ext != "json")
        {
            return Err(Error::Malformed(
                "asset map reference leaves its selection".into(),
            ));
        }
        let document = std::fs::read(self.root.join(path))
            .map_err(|error| Error::Malformed(format!("asset map: {error}")))?;
        let map = AssetMap::from_json(&document)?;
        let charmaps = Charmaps::load(&map, |_| {
            Err(Error::Malformed(
                "staged maps use builtin or inline charmaps".into(),
            ))
        })?;
        let report = verify(&map, &charmaps, bytes)?;
        for finding in &report.errors {
            eprintln!("{}: {}", finding.id, finding.message);
        }
        // Decode every asset with independent evidence named in the frozen expectation.
        let mut independent = Map::new();
        if let Some(Value::Object(expected)) = args.get("independent_ids") {
            for id in expected.keys() {
                let stored = decode_asset(&map, &charmaps, bytes, id)?;
                independent.insert(id.clone(), json!(hash_bytes(&stored.decoded)));
            }
        }
        self.assets.set(self.assets.get() + report.assets);
        self.references
            .set(self.references.get() + report.references);
        self.independent
            .set(self.independent.get() + independent.len());
        Ok(
            json!({"rom_sha256": hash_bytes(bytes), "verify_errors": report.errors.len(),
            "independent": independent}),
        )
    }
}

#[test]
fn corpus_asset_map_parity() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen asset specs"
    );
    let Some(root) = corpus_root("asset map parity") else {
        return;
    };
    // The first selection and every later map revision (ws-assets-<suffix>).
    let mut selections: Vec<String> = std::fs::read_dir(&root)
        .expect("read asset corpus root")
        .map(|entry| {
            entry
                .expect("read asset corpus entry")
                .file_name()
                .into_string()
                .expect("asset corpus entry must have a UTF-8 name")
        })
        .filter(|name| name == "ws-assets" || name.starts_with("ws-assets-"))
        .collect();
    selections.sort();
    let mut inspected = 0;
    for selection in selections {
        let handler = AssetDigest {
            root: root.join(&selection),
            assets: Cell::new(0),
            references: Cell::new(0),
            independent: Cell::new(0),
        };
        let count = run_dir(&selection, &handler);
        inspected += count;
        eprintln!(
            "{selection} corpus: {count} maps; {} assets and {} references verified; {} decodes equal independent extractor files",
            handler.assets.get(),
            handler.references.get(),
            handler.independent.get()
        );
    }
    assert!(
        !required_mode() || inspected > 0,
        "required asset corpus inspected no samples"
    );
}

#[test]
fn required_asset_maps_refuse_missing_or_empty_selections() {
    for case in ["empty-root", "not-directory", "empty-selection"] {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "asset-corpus-required-{}-{nonce}-{case}",
            std::process::id()
        ));
        if case == "not-directory" {
            std::fs::write(&root, []).unwrap();
        } else {
            std::fs::create_dir(&root).unwrap();
            if case == "empty-selection" {
                let selection = root.join("ws-assets");
                std::fs::create_dir(&selection).unwrap();
                std::fs::write(selection.join("spec.json"), br#"{"samples":{}}"#).unwrap();
            }
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "corpus_asset_map_parity", "--nocapture"])
            .env("FORMATKIT_CORPUS_DIR", &root)
            .env("FORMATKIT_CORPUS_REQUIRED", "1")
            .env("FORMATKIT_CORPUS_RUNNER_LOCKED", "1")
            .env("FORMATKIT_CORPUS_WRITE", "0")
            .output()
            .unwrap();
        if case == "not-directory" {
            std::fs::remove_file(&root).unwrap();
        } else {
            std::fs::remove_dir_all(&root).unwrap();
        }
        assert_eq!(
            output.status.code(),
            Some(101),
            "required asset corpus accepted {case}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let expected = match case {
            "empty-root" => "required asset corpus inspected no samples",
            "not-directory" => "read asset corpus root",
            _ => "spec present but lists no samples",
        };
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "wrong failure for {case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
