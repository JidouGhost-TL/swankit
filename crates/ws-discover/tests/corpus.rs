//! Re-runnable draft verification and independently captured decoded witnesses.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{corpus_root, hash_bytes, run_dir, Args, Digest};
use serde_json::{json, Map, Value};
use std::path::{Component, Path, PathBuf};
use ws_assets::{AssetMap, Charmaps};
struct DiscoveryDigest {
    root: PathBuf,
}
// Immutable drafts are verified baselines, not claims of a complete inventory.
// New codec recognition may add assets and overlap declarations, but it must
// preserve every prior entry, relation, reference and non-asset map field.
fn check_baseline(got: &AssetMap, expected: &AssetMap) -> Result<()> {
    for prior in &expected.assets {
        let current = got.asset(&prior.id)?;
        if !prior
            .overlaps
            .iter()
            .all(|id| current.overlaps.contains(id))
        {
            return Err(Error::Malformed("frozen overlap removed".into()));
        }
        let mut current = current.clone();
        current.overlaps = prior.overlaps.clone();
        if current != *prior {
            return Err(Error::Malformed("frozen asset changed".into()));
        }
    }
    let mut baseline = got.clone();
    baseline.assets = expected.assets.clone();
    if baseline != *expected {
        return Err(Error::Malformed("frozen map metadata changed".into()));
    }
    Ok(())
}
fn path_arg(root: &Path, args: &Args, key: &str) -> Result<PathBuf> {
    let name = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Malformed(format!("missing {key}")))?;
    let p = Path::new(name);
    if p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(Error::Malformed("evidence path leaves selection".into()));
    }
    Ok(root.join("ws-discover").join(p))
}
impl Digest for DiscoveryDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let evidence: ws_discover::Evidence = serde_json::from_slice(
            &std::fs::read(path_arg(&self.root, args, "evidence")?)
                .map_err(|e| Error::Malformed(e.to_string()))?,
        )
        .map_err(|e| Error::Malformed(e.to_string()))?;
        let got = ws_discover::discover(bytes, &evidence)?;
        let expected = AssetMap::from_json(
            &std::fs::read(path_arg(&self.root, args, "map")?)
                .map_err(|e| Error::Malformed(e.to_string()))?,
        )?;
        check_baseline(&got.map, &expected)?;
        let cm = Charmaps::load(&got.map, |_| {
            Err(Error::Malformed("external charmap in frozen map".into()))
        })?;
        let report = ws_assets::verify(&got.map, &cm, bytes)?;
        let mut independent = Map::new();
        if let Some(Value::Object(ids)) = args.get("independent_ids") {
            for id in ids.keys() {
                let a = ws_assets::decode_asset(&got.map, &cm, bytes, id)?;
                independent.insert(id.clone(), json!(hash_bytes(&a.decoded)));
            }
        }
        Ok(
            json!({"rom_sha256":hash_bytes(bytes),"verify_errors":report.errors.len(),"independent":independent}),
        )
    }
}
#[test]
fn corpus_discovery() {
    assert_ne!(std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(), Ok("1"));
    let Some(root) = corpus_root("discovery drafts") else {
        return;
    };
    let n = run_dir("ws-discover", &DiscoveryDigest { root });
    eprintln!("ws-discover: {n} frozen baselines retained; extended drafts re-run and verified");
}

#[test]
fn baseline_allows_additions_but_rejects_loss_and_changes() {
    let make = |assets| {
        AssetMap::from_json(
            &serde_json::to_vec(&json!({
                "schema":"swankit-asset-map/1", "rom":{"sha256":"00".repeat(32),"size":65536},
                "assets":assets
            }))
            .unwrap(),
        )
        .unwrap()
    };
    let a = json!({"id":"prior","kind":"other","offset":100,"length":32});
    let expected = make(json!([a]));
    let mut got = make(json!([a,{"id":"additional","kind":"other","offset":200,"length":32}]));
    check_baseline(&got, &expected).unwrap();
    got.assets[0].overlaps.push("additional".into());
    check_baseline(&got, &expected).unwrap();
    got.assets[0].offset.0 = 101;
    assert!(check_baseline(&got, &expected).is_err());
    got.assets.remove(0);
    assert!(check_baseline(&got, &expected).is_err());
    got = expected.clone();
    got.rom.sha256 = "11".repeat(32);
    assert!(check_baseline(&got, &expected).is_err());
}
