use std::process::Command;

fn run(case: &str) {
    let result = Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/review_link.py"))
        .arg(env!("CARGO_BIN_EXE_swankit"))
        .arg(case)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn permanent_stdio_end_exits_with_listen_partner() {
    run("Link.test_stdio_listen_exit");
}
#[test]
fn permanent_exec_end_exits_with_tcp_partner() {
    run("Link.test_exec_tcp_exit_and_children");
}
#[test]
fn sighup_reaps_child_group() {
    run("Link.test_sighup_reaps_child_group");
}
#[test]
fn shared_pty_flags_are_restored() {
    run("Link.test_pty_shared_flags_restored");
}
#[test]
fn early_bytes_survive_half_close_before_partner() {
    run("Link.test_early_bytes_then_close_before_partner");
}
#[test]
fn wondergate_sessions_are_fresh() {
    run("Link.test_wondergate_two_sessions");
}
