// SPDX-License-Identifier: MIT OR Apache-2.0
use std::{
    fs,
    io::Write,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};
use ws_wondergate::{
    frame::{Frame, HeaderMode},
    transcript,
};

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "wg-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn invoke(mut command: Command, input: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}
fn adapter() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_wg-adapter"));
    command.env_remove("WG_INI").env_remove("WG_SERIAL_LOG");
    command
}

#[test]
fn excomm_sentinel_environment_config_precedence_and_eof() {
    let scratch = Scratch::new();
    let config = scratch.0.join("device.ini");
    fs::write(&config, "[swankit]\nprofile=upstream\n").unwrap();
    let poweroff = [15, 0, 5, 255, 85, 170, 85, 170];
    let mut command = adapter();
    command.arg("ASDF").env("WG_INI", &config);
    let out = invoke(command, &poweroff);
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    assert_eq!(out.stdout, [15, 0, 2, 255, 0]);
    let revival = scratch.0.join("revival.ini");
    fs::write(&revival, "").unwrap();
    let mut command = adapter();
    command
        .args(["ASDF", "--config"])
        .arg(&revival)
        .env("WG_INI", &config);
    assert_eq!(invoke(command, &poweroff).stdout, [15, 0, 1, 255]);
    let mut command = adapter();
    command.arg(&config).env("WG_INI", "missing.ini");
    assert_eq!(invoke(command, &poweroff).stdout, [15, 0, 2, 255, 0]);
    assert!(invoke(adapter(), &[]).status.success());
    let out = invoke(adapter(), &[1, 0, 3, 2]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}

#[test]
fn tap_is_lossless_and_standalone_decode_matches_product() {
    let scratch = Scratch::new();
    let tap = scratch.0.join("serial.log");
    let input = [0, 1, 0, 1, 2, 2, 0, 2, 1, 0];
    let mut command = adapter();
    command.arg("--tap").arg(&tap);
    let out = invoke(command, &input);
    assert!(out.status.success());
    assert_eq!(out.stdout, [1, 0, 3, 2, 49, 48, 2, 0, 2, 2, 2]);
    let streams = transcript::parse(&fs::read_to_string(&tap).unwrap()).unwrap();
    assert_eq!(streams.input, input);
    assert_eq!(streams.output, out.stdout);
    let decoded = Command::new(env!("CARGO_BIN_EXE_wg-decode"))
        .arg(&tap)
        .output()
        .unwrap();
    assert!(decoded.status.success());
    let listing = String::from_utf8(decoded.stdout.clone()).unwrap();
    assert!(listing.contains("=== EMU->ADAPTER (10 bytes) ==="));
    assert!(listing.contains("+1 01:02 init/poweron"));
    let product = Command::new(env!("CARGO_BIN_EXE_swankit"))
        .arg("wg-decode")
        .arg(&tap)
        .output()
        .unwrap();
    assert_eq!(decoded.stdout, product.stdout);
    let mut command = adapter();
    command.arg("--tap").arg(&tap);
    assert!(!invoke(command, &[]).status.success());
    let mut command = adapter();
    command.env("WG_SERIAL_LOG", &tap);
    assert!(invoke(command, &[1, 0, 1, 2]).status.success());
    assert_eq!(
        transcript::parse(&fs::read_to_string(&tap).unwrap())
            .unwrap()
            .input
            .len(),
        14
    );
}

#[test]
fn decode_checks_full_u16_lengths_and_rejects_truncated_frames() {
    let scratch = Scratch::new();
    let tap = scratch.0.join("large.log");
    let mut bytes = Vec::new();
    transcript::Tap::new(&mut bytes)
        .write_at(
            0,
            transcript::Direction::ToAdapter,
            &Frame {
                kind: 17,
                cmd: 14,
                params: vec![65; 300],
            }
            .encode(HeaderMode::Len16Be)
            .unwrap(),
        )
        .unwrap();
    fs::write(&tap, &bytes).unwrap();
    let decoded = Command::new(env!("CARGO_BIN_EXE_wg-decode"))
        .arg(&tap)
        .output()
        .unwrap();
    assert!(decoded.status.success());
    assert!(String::from_utf8_lossy(&decoded.stdout).contains("params[300]"));
    fs::write(&tap, "T 0 > 01000302\n").unwrap();
    let bad = Command::new(env!("CARGO_BIN_EXE_wg-decode"))
        .arg(&tap)
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(bad.stdout.is_empty());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("frame at 0"));
}

#[test]
fn product_raw_command_and_argument_errors() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_swankit"));
    command
        .arg("wg-adapter")
        .env_remove("WG_INI")
        .env_remove("WG_SERIAL_LOG");
    assert_eq!(invoke(command, &[1, 0, 1, 2]).stdout, [1, 0, 3, 2, 49, 48]);
    for words in [
        vec!["--net", "other"],
        vec!["--tap"],
        vec!["ASDF", "ASDF"],
        vec!["--net", "tcp", "--net", "tcp"],
        vec!["missing.ini"],
    ] {
        let mut command = adapter();
        command.args(words);
        assert!(!invoke(command, &[]).status.success());
    }
}
