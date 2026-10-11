//! Re-runnable draft verification and independently captured decoded witnesses.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{corpus_root, hash_bytes, run_dir, Args, Digest};
use serde_json::{json, Map, Value};
use std::path::{Component, Path, PathBuf};
use ws_assets::{AssetMap, Charmaps};
struct DiscoveryDigest {
    root: PathBuf,
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
        if got.map != expected {
            return Err(Error::Malformed("re-run differs from frozen draft".into()));
        }
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
    eprintln!("ws-discover: {n} frozen drafts re-run and verified");
}
