//! CLI ROM facts and detection use the same external frozen product selection.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{hash_bytes, run_dir, Args, Digest};
use serde_json::{json, Value};
use std::{fs, io::Write, process::Command};
struct CliDigest;
impl Digest for CliDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let path = std::env::temp_dir().join(format!("swankit-cli-corpus-{}", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| Error::Malformed(e.to_string()))?;
        file.write_all(bytes)
            .map_err(|e| Error::Malformed(e.to_string()))?;
        drop(file);
        let check = || -> Result<Value> {
            let output = Command::new(env!("CARGO_BIN_EXE_swankit"))
                .arg("detect")
                .arg(&path)
                .arg("--json")
                .output()
                .map_err(|e| Error::Malformed(e.to_string()))?;
            if !output.status.success() {
                return Err(Error::Malformed(
                    String::from_utf8_lossy(&output.stderr).into_owned(),
                ));
            }
            let found: Value = serde_json::from_slice(&output.stdout)
                .map_err(|e| Error::Malformed(e.to_string()))?;
            if args.get("kind").and_then(Value::as_str) == Some("ws-rom") {
                let info = Command::new(env!("CARGO_BIN_EXE_swankit"))
                    .arg("info")
                    .arg(&path)
                    .arg("--json")
                    .output()
                    .map_err(|e| Error::Malformed(e.to_string()))?;
                if !info.status.success() {
                    return Err(Error::Malformed("corpus CLI info failed".into()));
                }
                let info: Value = serde_json::from_slice(&info.stdout)
                    .map_err(|e| Error::Malformed(e.to_string()))?;
                let reference = args
                    .get("reference")
                    .ok_or_else(|| Error::Malformed("missing independent facts".into()))?;
                for key in ["size_bytes", "color", "checksum_ok", "orientation"] {
                    if info.get(key) != reference.get(key) {
                        return Err(Error::Malformed(format!("CLI fact mismatch: {key}")));
                    }
                }
            }
            Ok(json!({"selected": found[0]["detection"]["selected"], "sha256": hash_bytes(bytes)}))
        };
        let result = check();
        fs::remove_file(path).map_err(|e| Error::Malformed(e.to_string()))?;
        result
    }
}
#[test]
fn corpus_cli_detection_and_info() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never rewrite frozen CLI expectations"
    );
    let count = run_dir("swankit-detection", &CliDigest);
    if count > 0 {
        eprintln!("swankit CLI: {count} files classified through binary; ROM info agrees with frozen owner facts");
    }
}

#[cfg(feature = "saves")]
#[derive(Default)]
struct SaveCliDigest {
    detection: std::cell::RefCell<std::collections::BTreeMap<String, Value>>,
}
#[cfg(feature = "saves")]
fn invoke_save_cli(words: &[&std::ffi::OsStr]) -> Result<Value> {
    let result = Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args(words)
        .arg("--json")
        .output()
        .map_err(|e| Error::Malformed(e.to_string()))?;
    if !result.status.success() {
        return Err(Error::Malformed(
            String::from_utf8_lossy(&result.stderr).into_owned(),
        ));
    }
    serde_json::from_slice(&result.stdout).map_err(|e| Error::Malformed(e.to_string()))
}
#[cfg(feature = "saves")]
impl Digest for SaveCliDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let path = std::env::temp_dir().join(format!("swankit-save-corpus-{}", std::process::id()));
        fs::create_dir(&path).map_err(|e| Error::Malformed(e.to_string()))?;
        let directory = Directory(path);
        let input = directory.0.join("input");
        let context = directory.0.join("context.json");
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Malformed("missing save identity".into()))?;
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Malformed("missing logical name".into()))?;
        let context_value = args
            .get("context")
            .ok_or_else(|| Error::Malformed("missing save context".into()))?;
        fs::write(&input, bytes).map_err(|e| Error::Malformed(e.to_string()))?;
        fs::write(
            &context,
            serde_json::to_vec(context_value).map_err(|e| Error::Malformed(e.to_string()))?,
        )
        .map_err(|e| Error::Malformed(e.to_string()))?;
        use std::ffi::OsStr;
        let hash = hash_bytes(bytes);
        let key = format!("{hash}:{name}:{context_value}");
        let cached = self.detection.borrow().get(&key).cloned();
        let detected = if let Some(value) = cached {
            value
        } else {
            let value = invoke_save_cli(&[
                OsStr::new("detect"),
                input.as_os_str(),
                OsStr::new("--name"),
                OsStr::new(name),
                OsStr::new("--context"),
                context.as_os_str(),
            ])?;
            self.detection.borrow_mut().insert(key, value.clone());
            value
        };
        let decoded = invoke_save_cli(&[
            OsStr::new("save"),
            OsStr::new("decode"),
            OsStr::new(id),
            input.as_os_str(),
            OsStr::new("--name"),
            OsStr::new(name),
            OsStr::new("--context"),
            context.as_os_str(),
        ])?;
        if detected[0]["detection"]["save_definitions"]
            .as_array()
            .is_some_and(|c| c.len() == 1)
        {
            let automatic = invoke_save_cli(&[
                OsStr::new("save"),
                OsStr::new("decode"),
                OsStr::new("auto"),
                input.as_os_str(),
                OsStr::new("--name"),
                OsStr::new(name),
                OsStr::new("--context"),
                context.as_os_str(),
            ])?;
            if automatic != decoded {
                return Err(Error::Malformed(
                    "CLI unique auto selection differs from independent decode".into(),
                ));
            }
        }
        let matched = detected[0]["detection"]["save_definitions"]
            .as_array()
            .is_some_and(|candidates| candidates.iter().any(|c| c["id"] == id));
        let mut result = json!({"selected":detected[0]["detection"]["selected"],"matched":matched,"decoded":decoded,"sha256":hash});
        if let Some(offset) = args.get("corrupt_offset").and_then(Value::as_u64) {
            let mut corrupted = bytes.to_vec();
            *corrupted
                .get_mut(
                    usize::try_from(offset)
                        .map_err(|_| Error::Malformed("corruption offset overflow".into()))?,
                )
                .ok_or_else(|| Error::Malformed("corruption outside image".into()))? ^= 1;
            fs::write(&input, corrupted).map_err(|e| Error::Malformed(e.to_string()))?;
            let repaired = directory.0.join("repaired");
            invoke_save_cli(&[
                OsStr::new("save"),
                OsStr::new("fix"),
                OsStr::new(id),
                input.as_os_str(),
                OsStr::new("--name"),
                OsStr::new(name),
                OsStr::new("--context"),
                context.as_os_str(),
                OsStr::new("--output"),
                repaired.as_os_str(),
            ])?;
            let fixed = fs::read(&repaired).map_err(|e| Error::Malformed(e.to_string()))?;
            result["repaired_sha256"] = json!(hash_bytes(&fixed));
            result["repaired"] = invoke_save_cli(&[
                OsStr::new("save"),
                OsStr::new("decode"),
                OsStr::new(id),
                repaired.as_os_str(),
                OsStr::new("--name"),
                OsStr::new(name),
                OsStr::new("--context"),
                context.as_os_str(),
            ])?;
        }
        Ok(result)
    }
}
#[cfg(feature = "saves")]
#[test]
fn corpus_cli_saves() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never rewrite frozen CLI save expectations"
    );
    let count = run_dir("swankit-save-detection", &SaveCliDigest::default());
    if count > 0 {
        eprintln!("swankit CLI saves: {count} detection/decode cases and configured repair bytes/trees passed against independent expectations");
    }
}

/// Retained save inputs replayed through the binary's `save set`, compared with
/// independently recorded outcomes: exact output bytes for qualified edits and
/// the refusal class for inputs the selected definition does not accept.
#[cfg(feature = "saves")]
struct SaveEditDigest;
#[cfg(feature = "saves")]
impl Digest for SaveEditDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Malformed(format!("missing save edit {key}")))
        };
        let name = text("name")?;
        if name.contains(['/', '\\']) || name == "." || name == ".." {
            return Err(Error::Malformed(
                "save edit name must be a bare file name".into(),
            ));
        }
        let field = args
            .get("field")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::Malformed("missing save edit field".into()))?
            .to_string();
        let options = args
            .get("options")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Malformed("missing save edit options".into()))?
            .iter()
            .map(|o| {
                o.as_str()
                    .filter(|o| matches!(*o, "--all-copies" | "--no-repair" | "--skip-validation"))
                    .ok_or_else(|| Error::Malformed("unknown save edit option".into()))
            })
            .collect::<Result<Vec<_>>>()?;
        let path = std::env::temp_dir().join(format!("swankit-save-edit-{}", std::process::id()));
        fs::create_dir(&path).map_err(|e| Error::Malformed(e.to_string()))?;
        let directory = Directory(path);
        let input = directory.0.join(name);
        let output = directory.0.join("edited");
        fs::write(&input, bytes).map_err(|e| Error::Malformed(e.to_string()))?;
        let result = Command::new(env!("CARGO_BIN_EXE_swankit"))
            .args(["save", "set", text("definition")?])
            .arg(&input)
            .args(["--field", &field, "--value", text("value")?])
            .args(&options)
            .arg("--output")
            .arg(&output)
            .arg("--json")
            .output()
            .map_err(|e| Error::Malformed(e.to_string()))?;
        if fs::read(&input).map_err(|e| Error::Malformed(e.to_string()))? != bytes {
            return Err(Error::Malformed("save edit changed its input".into()));
        }
        let mut digest = json!({"input_sha256": hash_bytes(bytes)});
        if result.status.success() {
            let edited = fs::read(&output).map_err(|e| Error::Malformed(e.to_string()))?;
            digest["outcome"] = json!("written");
            digest["output_sha256"] = json!(hash_bytes(&edited));
        } else {
            if output.exists() {
                return Err(Error::Malformed(
                    "refused save edit created an output".into(),
                ));
            }
            let error: Value = serde_json::from_slice(&result.stderr)
                .map_err(|e| Error::Malformed(e.to_string()))?;
            let message = error["error"].as_str().unwrap_or_default();
            digest["outcome"] = json!("refused");
            digest["reason"] = json!(if message.starts_with("input save has no usable record") {
                "input-unusable".to_string()
            } else if message == "save has no usable record after repair" {
                "post-edit-unusable".to_string()
            } else {
                format!("other: {message}")
            });
        }
        Ok(digest)
    }
}
#[cfg(feature = "saves")]
#[test]
fn corpus_cli_save_edits() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never rewrite frozen CLI save edit expectations"
    );
    let count = run_dir("swankit-save-edits", &SaveEditDigest);
    if count > 0 {
        eprintln!("swankit CLI save edits: {count} retained inputs matched recorded output bytes or refusal classes");
    }
}
