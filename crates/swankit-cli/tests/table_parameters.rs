use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "table-parameters-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self, n: &str) -> String {
        self.0.join(n).to_str().unwrap().to_owned()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args(args)
        .output()
        .unwrap()
}
fn setup(f: &Fixture) -> Value {
    let mut b = vec![0u8; 0x200000];
    for (n, x) in b.iter_mut().enumerate() {
        *x = (n % 251) as u8;
    }
    b[0x1ffff0] = 0xea;
    b[0x1ffffd] = 0;
    fs::write(f.path("image"), &b).unwrap();
    json!({"schema":"swankit-table-parameters/1","rom":{"sha256":ws_assets::sha256_hex(&b),"size":b.len()},"source_segment":20771,"source_base":9029,"route":{"kind":"aligned-fallthrough"},"context":{"kind":"explicit","banks":[{"c0":1,"c2":0,"c3":0}]}})
}
#[test]
fn normal_cli_resolves_then_uses_existing_raw_extraction() {
    let f = Fixture::new();
    let d = setup(&f);
    fs::write(f.path("definition"), serde_json::to_vec(&d).unwrap()).unwrap();
    let r = run(&[
        "assets",
        "resolve-table",
        &f.path("image"),
        &f.path("definition"),
        "--output",
        &f.path("receipt"),
        "--map-out",
        &f.path("map"),
        "--json",
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let receipt: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(receipt["first_cpu_address"], 0x53575);
    assert_eq!(receipt["slices"][0]["file_offset"], "0x153575");
    let out = f.path("out");
    let r = run(&[
        "assets",
        "extract",
        &f.path("image"),
        &f.path("map"),
        &out,
        "--json",
    ]);
    assert!(r.status.success());
    let b = fs::read(f.path("image")).unwrap();
    assert_eq!(
        fs::read(f.0.join("out/conditional-source-domain.bin")).unwrap(),
        b[0x153575..0x153675]
    );
    let r = run(&[
        "assets",
        "resolve-table",
        &f.path("image"),
        &f.path("definition"),
        "--output",
        &f.path("receipt"),
    ]);
    assert!(!r.status.success());
    let mut b = b;
    b[0] ^= 1;
    fs::write(f.path("changed"), b).unwrap();
    let r = run(&[
        "assets",
        "resolve-table",
        &f.path("changed"),
        &f.path("definition"),
        "--output",
        &f.path("bad"),
    ]);
    assert!(!r.status.success());
    assert!(!f.0.join("bad").exists());
}
#[test]
fn ambiguous_receipt_is_honest_and_does_not_emit_a_single_map() {
    let f = Fixture::new();
    let mut d = setup(&f);
    d["context"]["banks"]
        .as_array_mut()
        .unwrap()
        .push(json!({"c0":0,"c2":0,"c3":0}));
    fs::write(f.path("definition"), serde_json::to_vec(&d).unwrap()).unwrap();
    let r = run(&[
        "assets",
        "resolve-table",
        &f.path("image"),
        &f.path("definition"),
        "--output",
        &f.path("receipt"),
        "--json",
    ]);
    assert!(r.status.success());
    let receipt: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(receipt["decision"], "ambiguous");
    assert_eq!(receipt["slices"].as_array().unwrap().len(), 2);
    let r = run(&[
        "assets",
        "resolve-table",
        &f.path("image"),
        &f.path("definition"),
        "--output",
        &f.path("refused"),
        "--map-out",
        &f.path("map"),
    ]);
    assert!(!r.status.success());
    assert!(!f.0.join("refused").exists());
    assert!(!f.0.join("map").exists());
}
