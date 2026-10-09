//! Product detection against external independent owner facts and digests.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{hash_bytes, run_dir, Args, Digest};
use serde_json::{json, Value};
use swankit::Toolkit;
struct DetectionDigest;
impl Digest for DetectionDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let toolkit = Toolkit::new()?;
        let detected = toolkit.detect(bytes, None);
        match args.get("kind").and_then(Value::as_str) {
            Some("ws-rom") => {
                let info = toolkit.info(bytes)?;
                let reference = args
                    .get("reference")
                    .ok_or_else(|| Error::Malformed("missing ROM reference".into()))?;
                for (key, got) in [
                    ("size_bytes", json!(info.size_bytes)),
                    ("color", json!(info.color)),
                    ("checksum_ok", json!(info.checksum_ok)),
                    ("orientation", json!(info.orientation)),
                    ("publisher", json!(info.publisher.unwrap_or("?"))),
                    (
                        "checksum_computed",
                        json!(format!("{:04x}", info.checksum_computed)),
                    ),
                    (
                        "checksum_footer",
                        json!(format!("{:04x}", info.checksum_footer)),
                    ),
                ] {
                    if reference.get(key) != Some(&got) {
                        return Err(Error::Malformed(format!(
                            "product facts differ from independent {key}"
                        )));
                    }
                }
                let facts = swankit::rom::RomFacts::parse(bytes)?;
                if !swankit::rom::catalog::probe(bytes) || facts.header.game_id != info.game_id {
                    return Err(Error::Malformed("product/ws-rom mismatch".into()));
                }
            }
            Some("ws-patch") => {
                swankit::patch::Patch::parse(bytes)?;
            }
            _ => return Err(Error::Malformed("unknown detection reference kind".into())),
        }
        Ok(json!({"selected": detected.selected, "sha256": hash_bytes(bytes)}))
    }
}
#[test]
fn corpus_composed_detection() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never rewrite frozen product expectations"
    );
    let count = run_dir("swankit-detection", &DetectionDigest);
    if count > 0 {
        eprintln!("swankit: {count} owner samples classified; composed ROM facts and IPS parser agreement passed");
    }
}

#[cfg(feature = "saves")]
#[derive(Default)]
struct SaveDigest {
    cache: std::cell::RefCell<std::collections::BTreeMap<String, swankit::Detection>>,
    natural: std::cell::Cell<usize>,
    unique: std::cell::Cell<usize>,
    ambiguous: std::cell::Cell<usize>,
}
#[cfg(feature = "saves")]
impl Digest for SaveDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let toolkit = Toolkit::new()?;
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Malformed("missing save identity".into()))?;
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Malformed("missing save logical name".into()))?;
        let context_json = args
            .get("context")
            .ok_or_else(|| Error::Malformed("missing save context".into()))?;
        let context = swankit::save::context_from_json(
            &serde_json::to_vec(context_json).map_err(|e| Error::Malformed(e.to_string()))?,
        )?;
        let hash = hash_bytes(bytes);
        let key = format!("{hash}:{name}:{context_json}");
        let previous = self.cache.borrow().get(&key).cloned();
        let detection = if let Some(d) = previous {
            d
        } else {
            let detection = toolkit.detect_with_save_context(bytes, None, name, &context)?;
            self.cache.borrow_mut().insert(key, detection.clone());
            detection
        };
        let format = swankit::saves::format(id)?;
        let mut result = json!({"selected":detection.selected,"matched":detection.save_definitions.iter().any(|c| c.id == id),"sha256":hash,
            "decoded":toolkit.decode_save(format,bytes,name,&context)?});
        if detection.save_definitions.len() == 1 {
            let automatic = toolkit.auto_save(bytes, name, &context)?;
            if automatic.id != id
                || toolkit.decode_save(automatic, bytes, name, &context)? != result["decoded"]
            {
                return Err(Error::Malformed(
                    "unique automatic save selection disagrees with independent definition".into(),
                ));
            }
        }
        if let Some(offset) = args.get("corrupt_offset").and_then(Value::as_u64) {
            let mut corrupted = bytes.to_vec();
            *corrupted
                .get_mut(
                    usize::try_from(offset)
                        .map_err(|_| Error::Malformed("corruption offset overflow".into()))?,
                )
                .ok_or_else(|| Error::Malformed("corruption outside image".into()))? ^= 1;
            let fixed = toolkit.fix_save(format, &corrupted, name, &context)?;
            result["repaired"] = toolkit.decode_save(format, &fixed, name, &context)?;
            result["repaired_sha256"] = json!(hash_bytes(&fixed));
        } else {
            self.natural.set(self.natural.get() + 1);
            if detection.save_definitions.len() == 1 {
                self.unique.set(self.unique.get() + 1);
            } else {
                self.ambiguous.set(self.ambiguous.get() + 1);
            }
        }
        Ok(result)
    }
}
#[cfg(feature = "saves")]
#[test]
fn corpus_save_detection_and_dispatch() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never rewrite frozen save detection expectations"
    );
    let handler = SaveDigest::default();
    let count = run_dir("swankit-save-detection", &handler);
    if count > 0 {
        eprintln!("swankit saves: {count} cases; {} natural images recognized, {} unique definitions, {} ambiguous candidate sets; independent decode and configured repair byte/tree parity passed",handler.natural.get(),handler.unique.get(),handler.ambiguous.get());
    }
}
