#![cfg(unix)]

fn run(case: &str) {
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/assets_text_framing.py"
        ))
        .arg(env!("CARGO_BIN_EXE_swankit"))
        .arg(case)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn explicit_lf_framing_preserves_content_and_complete_rom() {
    run("roundtrip");
}

#[test]
fn default_text_import_keeps_legacy_optional_lf_then_cr_behavior() {
    run("legacy");
}

#[test]
fn text_framing_refusals_preserve_fresh_and_forced_destinations() {
    run("refusals");
}
