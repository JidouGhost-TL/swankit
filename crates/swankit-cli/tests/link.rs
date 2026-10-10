use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn stdio_exec_forwards_binary_without_stdout_diagnostics() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args(["link", "stdio", "exec:cat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let bytes: Vec<u8> = (0..=255).collect();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&bytes).unwrap();
    // Keep stdin open until the response returns; EOF ends the stdio endpoint.
    use std::io::Read;
    let mut stdout = child.stdout.take().unwrap();
    let mut response = vec![0; 256];
    stdout.read_exact(&mut response).unwrap();
    assert_eq!(response, bytes);
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("both endpoints ended"));
}

#[test]
fn help_and_invalid_options_are_configuration_errors() {
    let binary = env!("CARGO_BIN_EXE_swankit");
    let out = Command::new(binary)
        .args(["link", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8(out.stdout).unwrap();
    assert!(help.contains("127.0.0.1"));
    assert!(help.contains("192000"));
    for args in [
        vec!["link", "stdio", "exec:cat", "--jitter-ms", "1"],
        vec!["link", "stdio", "exec:cat", "--baud", "123"],
        vec!["link", "stdio", "stdio"],
        vec!["link", "bad", "exec:cat"],
    ] {
        assert!(!Command::new(binary)
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    }
}
