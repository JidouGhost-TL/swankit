//! External frozen patch/base/target evidence; synthetic runs skip cleanly.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{corpus_root, hash_bytes, run_dir, Args, Digest};
use serde_json::{json, Value};
use std::{
    cell::Cell,
    path::{Component, Path},
};
use ws_patch::{create, ApplyOptions, GrowthPolicy, OverlapPolicy, Patch};

struct PatchDigest {
    root: std::path::PathBuf,
    recorded: Cell<usize>,
    supplemental: Cell<usize>,
    rle: Cell<usize>,
    records: Cell<usize>,
}

impl PatchDigest {
    fn read(&self, args: &Args, key: &str, directory: &str) -> Result<Vec<u8>> {
        let name = args
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Malformed(format!("patch spec lacks {key}")))?;
        let path = Path::new(name);
        let parts: Vec<_> = path.components().collect();
        if parts.len() != 2
            || parts[0] != Component::Normal(directory.as_ref())
            || !matches!(parts[1], Component::Normal(_))
            || path.extension().is_none_or(|ext| ext != "rom")
        {
            return Err(Error::Malformed(
                "patch spec reference leaves its selection".into(),
            ));
        }
        std::fs::read(self.root.join("ws-patch").join(path))
            .map_err(|error| Error::Malformed(format!("patch reference: {error}")))
    }
}

impl Digest for PatchDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let base = self.read(args, "base", "bases")?;
        let target = self.read(args, "target", "targets")?;
        let patch = Patch::parse(bytes)?;
        let options = ApplyOptions {
            overlaps: OverlapPolicy::LastWriteWins,
            growth: GrowthPolicy::ZeroFill,
            ..ApplyOptions::default()
        };
        let output = patch.apply(&base, options)?;
        if output != target || patch.encode()? != bytes || !ws_patch::catalog::probe(bytes) {
            return Err(Error::Malformed(
                "IPS output/reference, serialization or catalog mismatch".into(),
            ));
        }
        let recreated = create(&base, &target)?;
        let recreated_patch = Patch::parse(&recreated)?;
        if recreated_patch.apply(
            &base,
            ApplyOptions {
                overlaps: OverlapPolicy::Reject,
                ..options
            },
        )? != target
        {
            return Err(Error::Malformed(
                "created IPS is not equivalent to independent target".into(),
            ));
        }
        match args.get("reference_kind").and_then(Value::as_str) {
            Some("published-digest" | "archived-run-digest") => {
                self.recorded.set(self.recorded.get() + 1)
            }
            Some("independent-python") => self.supplemental.set(self.supplemental.get() + 1),
            _ => {
                return Err(Error::Malformed(
                    "unknown patch reference provenance".into(),
                ))
            }
        }
        self.records.set(self.records.get() + patch.records.len());
        self.rle.set(
            self.rle.get()
                + patch
                    .records
                    .iter()
                    .filter(|record| matches!(record.data, ws_patch::RecordData::Rle { .. }))
                    .count(),
        );
        Ok(
            json!({"sha256": hash_bytes(bytes), "base_sha256": hash_bytes(&base),
            "patched_sha256": hash_bytes(&output), "base_size": base.len(),
            "patched_size": output.len(), "records": patch.records.len()}),
        )
    }
}

#[test]
fn corpus_patch_parity() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen patch specs"
    );
    let Some(root) = corpus_root("IPS patch parity") else {
        return;
    };
    let handler = PatchDigest {
        root,
        recorded: Cell::new(0),
        supplemental: Cell::new(0),
        records: Cell::new(0),
        rle: Cell::new(0),
    };
    let count = run_dir("ws-patch", &handler);
    eprintln!("ws-patch corpus: {count} patches; {} records ({} RLE); {} recorded output digests; {} supplemental independent Python expectations; application, byte-preserving serialization, catalog and equivalent create/apply all passed",
        handler.records.get(), handler.rle.get(), handler.recorded.get(), handler.supplemental.get());
}
