//! Frozen independent parser outputs and save bytes live outside Git.
use formatkit_core::Result;
use formatkit_corpus_test::{hash_bytes, run_dir, Args, Digest, Spec};
use serde_json::{json, Value};
struct SaveDigest {
    origins: std::collections::HashMap<(String, String, String), Origin>,
}
struct Origin {
    name: String,
    path: std::path::PathBuf,
}
impl Origin {
    fn from_record(record: &Value) -> Self {
        let path = record["path"].as_str().unwrap();
        Self {
            name: record["origin_name"].as_str().unwrap_or(path).into(),
            path: path.into(),
        }
    }
}
impl SaveDigest {
    fn load(root: &std::path::Path, namespace: &str) -> Self {
        let mut origins = std::collections::HashMap::new();
        let path = root.join(format!("{namespace}-enrollment.json"));
        if path.is_file() {
            let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            for r in value["records"].as_array().unwrap() {
                origins.insert(
                    (
                        r["id"].as_str().unwrap().into(),
                        r["sha256"].as_str().unwrap().into(),
                        r["name"].as_str().unwrap().into(),
                    ),
                    Origin::from_record(r),
                );
            }
        }
        Self { origins }
    }
}
impl Digest for SaveDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| formatkit_core::Error::Malformed("missing save identity".into()))?;
        let origin = self.origins.get(&(
            id.into(),
            hash_bytes(bytes),
            args.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
        ));
        let name = origin
            .map(|origin| origin.name.as_str())
            .or_else(|| args.get("name").and_then(Value::as_str))
            .ok_or_else(|| formatkit_core::Error::Malformed("missing save medium name".into()))?;
        let format = ws_saves::format(id)?;
        let mut context = ws_saves::DecodeContext::default();
        if let Some(tables) = args.get("context").and_then(Value::as_object) {
            for (key, bytes) in tables {
                let bytes = bytes
                    .as_array()
                    .ok_or_else(|| formatkit_core::Error::Malformed("context bytes".into()))?
                    .iter()
                    .map(|b| {
                        b.as_u64()
                            .and_then(|n| u8::try_from(n).ok())
                            .ok_or_else(|| formatkit_core::Error::Malformed("context byte".into()))
                    })
                    .collect::<Result<Vec<_>>>()?;
                context.tables.insert(key.clone(), bytes);
            }
        }
        let companion = std::path::Path::new(name).with_extension("ieep");
        // A frozen oracle can retain its original file label after relocation.
        // Read the companion from the current physical path while giving the
        // parser the original logical name; no expectations are regenerated.
        let companion_path = origin.map_or_else(
            || companion.clone(),
            |origin| origin.path.with_extension("ieep"),
        );
        if companion_path.is_file() {
            context.images.insert(
                companion.to_string_lossy().into_owned(),
                std::fs::read(&companion_path)
                    .map_err(|e| formatkit_core::Error::Malformed(e.to_string()))?,
            );
        }
        let decoded = format.decode_with_context(bytes, name, &context)?;
        let mut result = json!({"decoded":decoded,"sha256":hash_bytes(bytes)});
        if let Some(offset) = args.get("corrupt_offset").and_then(Value::as_u64) {
            let mut edited = bytes.to_vec();
            let byte = edited
                .get_mut(usize::try_from(offset).map_err(|_| {
                    formatkit_core::Error::Malformed("corrupt offset overflow".into())
                })?)
                .ok_or_else(|| {
                    formatkit_core::Error::Malformed("corrupt offset outside save".into())
                })?;
            *byte ^= 1;
            format.fix_with_context(&mut edited, name, &context)?;
            result["repaired"] = format.decode_with_context(&edited, name, &context)?;
            result["repaired_sha256"] = json!(hash_bytes(&edited));
        }
        Ok(result)
    }
}
#[test]
fn corpus_saves() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "frozen save specs must never be regenerated"
    );
    let Some(root) = formatkit_corpus_test::corpus_root("save parity") else {
        return;
    };
    let mut count = 0;
    let mut by_title = std::collections::BTreeMap::<String, [usize; 3]>::new();
    for namespace in [
        "ws-saves",
        "ws-saves-parity",
        "ws-saves-play",
        "ws-saves-natural",
    ] {
        if root.join(namespace).join("spec.json").is_file() {
            count += run_dir(namespace, &SaveDigest::load(&root, namespace));
            let spec = Spec::load(&root.join(namespace).join("spec.json")).unwrap();
            for sample in spec.samples.values() {
                let counts = by_title
                    .entry(sample.args["id"].as_str().unwrap().into())
                    .or_default();
                let column = if namespace == "ws-saves-play" {
                    2
                } else if sample.args.contains_key("corrupt_offset") {
                    1
                } else {
                    0
                };
                counts[column] += 1;
            }
        }
    }
    assert!(
        !formatkit_corpus_test::required_mode() || count > 0,
        "no save specs selected"
    );
    if count > 0 {
        for (id, [natural, repair, play]) in by_title {
            eprintln!(
                "SAVE_PARITY {}",
                json!({"id":id,"natural":natural,"repair":repair,"play":play})
            );
        }
        eprintln!("ws-saves corpus: {count} field-tree parity samples and configured repair round trips passed");
    }
}

/// Rejected oracle inputs remain outside the valid-play spec. Check that their
/// primary-medium errors are bounded Rust errors, using the frozen provenance.
#[test]
fn corpus_rejected_primary_media() {
    let Some(root) = formatkit_corpus_test::corpus_root("rejected save media") else {
        return;
    };
    let ledger = root.join("ws-saves-play-enrollment.json");
    if !ledger.is_file() {
        return;
    }
    let enrollment: Value = serde_json::from_slice(&std::fs::read(ledger).unwrap()).unwrap();
    let mut rejected = 0;
    for note in enrollment["source_excluded"].as_array().unwrap() {
        if note["reason"] == "independent oracle does not mark a record/image valid" {
            continue;
        }
        let id = note["id"].as_str().unwrap();
        let origin = Origin::from_record(note);
        let name = origin.name.as_str();
        let bytes = std::fs::read(&origin.path).unwrap();
        assert_eq!(hash_bytes(&bytes), note["sha256"].as_str().unwrap());
        let (kind, error) = match ws_saves::format(id).unwrap().decode(&bytes, name) {
            Err(error) => ("result-error", error.to_string()),
            Ok(value) => (
                "tree-error",
                value["error"]
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .expect("rejected primary input must produce an error")
                    .to_owned(),
            ),
        };
        eprintln!(
            "SAVE_REJECTION {}",
            json!({"id":id,"kind":kind,"error":error})
        );
        rejected += 1;
    }
    eprintln!("ws-saves rejected primary media: {rejected} bounded errors checked");
}
