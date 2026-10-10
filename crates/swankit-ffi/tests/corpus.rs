//! External Java oracles and frozen adapter expectations, run only by the safe runner.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{corpus_root, hash_bytes, run_dir, Args, Digest};
use serde_json::{json, Value};

struct InteropDigest;
impl Digest for InteropDigest {
    fn digest(&self, input: &[u8], args: &Args) -> Result<Value> {
        let operation = args
            .get("operation")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| Error::Malformed("interop spec operation".into()))?;
        let output = swankit_ffi::decode(operation, input)?;
        Ok(
            json!({"sha256":hash_bytes(input),"output_sha256":hash_bytes(&output),"output_bytes":output.len()}),
        )
    }
}

#[test]
fn corpus_interop() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate expectations"
    );
    let Some(_) = corpus_root("Java interop") else {
        return;
    };
    assert_eq!(
        std::env::var("FORMATKIT_CORPUS_RUNNER_LOCKED").as_deref(),
        Ok("1"),
        "interop requires safe runner"
    );
    // Bootstrap enrolls new independent Java vectors before the frozen Rust selection exists.
    if std::env::var("SWANKIT_INTEROP_BOOTSTRAP").as_deref() != Ok("1") {
        let count = run_dir("swankit-interop", &InteropDigest);
        eprintln!("interop adapter: {count} independent Java vectors passed");
    }
    if let Some(path) = std::env::var_os("SWANKIT_INTEROP_PARITY_CONFIG") {
        let config: Vec<Vec<String>> =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(!config.is_empty(), "Java parity commands must not be empty");
        for command in config {
            assert!(!command.is_empty(), "empty Java command");
            let status = std::process::Command::new(&command[0])
                .args(&command[1..])
                .status()
                .unwrap();
            assert!(status.success(), "Java parity exited {status}");
        }
    } else {
        eprintln!("Java live parity not requested; set SWANKIT_INTEROP_PARITY_CONFIG to opt in");
    }
}
