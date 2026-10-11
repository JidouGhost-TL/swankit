//! External frozen asset maps with independent decodes; synthetic runs skip cleanly.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{corpus_root, hash_bytes, run_dir, Args, Digest};
use serde_json::{json, Map, Value};
use std::{
    cell::Cell,
    path::{Component, Path, PathBuf},
};
use ws_assets::{decode_asset, verify, AssetMap, Charmaps};

struct AssetDigest {
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
        let document = std::fs::read(self.root.join("ws-assets").join(path))
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
    let handler = AssetDigest {
        root,
        assets: Cell::new(0),
        references: Cell::new(0),
        independent: Cell::new(0),
    };
    let count = run_dir("ws-assets", &handler);
    eprintln!(
        "ws-assets corpus: {count} maps; {} assets and {} references verified; {} decodes equal independent extractor files",
        handler.assets.get(),
        handler.references.get(),
        handler.independent.get()
    );
}
