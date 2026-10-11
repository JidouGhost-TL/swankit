#![cfg(unix)]
fn run(case: &str) {
    let output = std::process::Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/review_io.py"))
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
fn cli_fifo_input_returns_without_waiting_for_a_writer() {
    run("cli");
}
#[test]
fn interop_fifo_error_allows_the_next_request() {
    run("interop");
}
