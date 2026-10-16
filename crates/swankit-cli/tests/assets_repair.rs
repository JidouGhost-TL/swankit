#![cfg(unix)]

fn run(case: &str) {
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/assets_repair.py"
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
fn asset_destinations_fail_without_partial_files_or_sentinel_changes() {
    run("destinations");
}

#[test]
fn asset_outputs_refuse_aliases_and_protect_all_inputs() {
    run("aliases");
}

#[test]
fn returned_maps_preserve_every_external_charmap_format_and_repeated_edits() {
    run("charmaps");
}

#[test]
fn asset_refusals_preserve_identity_reservations_and_free_space() {
    run("safety");
}

#[test]
fn asset_extract_preflights_the_complete_output_set() {
    run("extract");
}
