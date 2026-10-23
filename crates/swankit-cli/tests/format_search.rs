use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "swankit-format-search-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().into()
    }
    fn put(&self, name: &str, data: &[u8]) -> String {
        let p = self.path(name);
        fs::write(&p, data).unwrap();
        p
    }
    fn json(&self, name: &str, data: Value) -> String {
        self.put(name, &serde_json::to_vec(&data).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn command(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args(args)
        .output()
        .unwrap()
}
fn ok(args: &[&str]) -> Value {
    let got = command(args);
    assert!(
        got.status.success(),
        "{}",
        String::from_utf8_lossy(&got.stderr)
    );
    serde_json::from_slice(&got.stdout).unwrap()
}
#[test]
fn batch_parallel_checkpoints_resume_and_failure_are_actionable() {
    let f = Fixture::new();
    f.put("a.bin", &(0u8..96).collect::<Vec<_>>());
    f.put("b.bin", &(0u8..128).collect::<Vec<_>>());
    let config=f.json("config.json",json!({"families":["reader-signature"],"signatures":[{"id":"sample","bytes":[16,17,18,19]}]}));
    let manifest=f.json("inputs.json",json!({"schema":"swankit-format-search-inputs/1","inputs":[{"id":"one","path":"a.bin"},{"id":"two","path":"b.bin"}]}));
    let out = f.path("out");
    let args = [
        "assets",
        "search-corpus",
        &manifest,
        "--config",
        &config,
        "--output",
        &out,
        "--jobs",
        "2",
        "--json",
    ];
    let completed = command(&args);
    assert!(completed.status.success());
    let progress = String::from_utf8_lossy(&completed.stderr);
    let progress: Vec<_> = progress
        .lines()
        .filter(|line| line.starts_with("format search "))
        .collect();
    assert_eq!(progress.len(), 2);
    assert!(progress[0].starts_with("format search 1/2:"));
    assert!(progress[1].starts_with("format search 2/2:"));
    let first: Value = serde_json::from_slice(&completed.stdout).unwrap();
    assert_eq!(first["succeeded"], 2);
    assert_eq!(first["failed"], 0);
    assert_eq!(first["leads"], 2);
    let mut resume = args.to_vec();
    resume.push("--resume");
    let second = ok(&resume);
    assert_eq!(second["resumed"], 2);
    assert!(!command(&args).status.success());
    // Tampering is a failed input, not silent regeneration or a successful batch.
    let checkpoint = PathBuf::from(&out).join(second["entries"][0]["report"].as_str().unwrap());
    let mut stored: Value = serde_json::from_slice(&fs::read(&checkpoint).unwrap()).unwrap();
    stored["report"]["rom_bytes"] = json!(999);
    fs::write(&checkpoint, serde_json::to_vec(&stored).unwrap()).unwrap();
    assert!(!command(&resume).status.success());
    let summary: Value =
        serde_json::from_slice(&fs::read(PathBuf::from(&out).join("summary.json")).unwrap())
            .unwrap();
    assert_eq!(summary["failed"], 1);
    assert!(summary["entries"][0]["error"]
        .as_str()
        .unwrap()
        .contains("differs"));
    f.put("b.bin", &(0u8..127).collect::<Vec<_>>());
    assert!(!command(&resume).status.success());
    let summary: Value =
        serde_json::from_slice(&fs::read(PathBuf::from(&out).join("summary.json")).unwrap())
            .unwrap();
    assert!(summary["entries"][1]["error"]
        .as_str()
        .unwrap()
        .contains("input changed"));
}
#[test]
fn single_search_refuses_source_output_alias_and_config_typoes() {
    let f = Fixture::new();
    let rom = f.put("source.bin", &(0u8..96).collect::<Vec<_>>());
    let before = fs::read(&rom).unwrap();
    assert!(!command(&["assets", "search", &rom, "--output", &rom])
        .status
        .success());
    assert_eq!(before, fs::read(&rom).unwrap());
    let config = f.json("bad.json", json!({"max_probe":1}));
    assert!(!command(&["assets", "search", &rom, "--config", &config])
        .status
        .success());
}

#[test]
fn failed_batch_without_checkpoints_still_binds_manifest_on_resume() {
    let f = Fixture::new();
    let source = f.put("source.bin", &(0u8..96).collect::<Vec<_>>());
    let manifest=f.json("inputs.json",json!({"schema":"swankit-format-search-inputs/1","inputs":[{"id":"missing","path":"absent.bin"}]}));
    let out = f.path("out");
    assert!(
        !command(&["assets", "search-corpus", &manifest, "--output", &out])
            .status
            .success()
    );
    f.json(
        "inputs.json",
        json!({"schema":"swankit-format-search-inputs/1","inputs":[{"id":"new","path":source}]}),
    );
    let result = command(&[
        "assets",
        "search-corpus",
        &manifest,
        "--output",
        &out,
        "--resume",
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr)
        .contains("manifest/configuration/executable changed"));
}
