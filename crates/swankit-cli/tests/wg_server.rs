// SPDX-License-Identifier: MIT OR Apache-2.0
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};
use ws_wondergate_server::services::Services;
struct Owned {
    child: Child,
    directory: PathBuf,
}
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
#[test]
fn wg_server_cli_starts_serves_logs_and_persists_tokens() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = loop {
        let path = std::env::temp_dir().join(format!(
            "swankit-wg-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::create_dir(&path) {
            Ok(()) => break path,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => panic!("{e}"),
        }
    };
    let child = Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args([
            "wg-server",
            "--http-port",
            "0",
            "--bpl-port",
            "0",
            "--clock",
            "fixed:20261005200403",
            "--seed",
            "7",
            "--data",
        ])
        .arg(directory.join("data"))
        .arg("--wire")
        .arg(directory.join("wire"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut owned = Owned { child, directory };
    let mut startup = String::new();
    BufReader::new(owned.child.stdout.take().unwrap())
        .read_line(&mut startup)
        .unwrap();
    let parts: Vec<_> = startup.split_whitespace().collect();
    assert_eq!(parts[0], "WonderGate");
    let address: SocketAddr = parts[1].strip_prefix("HTTP:").unwrap().parse().unwrap();
    let host = &Services::bundled().unwrap().charge_hosts[0];
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    stream.write_all(format!("POST /service/charge/nph-session HTTP/1.1\r\nHost: {host}\r\nX-swan-cmnd-sid: synthetic\r\n\r\n").as_bytes()).unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut reply = vec![];
    stream.read_to_end(&mut reply).unwrap();
    assert!(reply.starts_with(b"HTTP/1.1 200 OK"));
    let records = std::fs::read_to_string(owned.directory.join("data/journal.jsonl")).unwrap();
    assert_eq!(records.lines().count(), 1);
    let record: serde_json::Value = serde_json::from_str(records.trim()).unwrap();
    assert_eq!(record["provenance"], "new");
    assert_eq!(record["data"]["aid"].as_str().unwrap().len(), 80);
    let wire = std::fs::read_to_string(owned.directory.join("wire/wire.jsonl")).unwrap();
    assert_eq!(wire.lines().count(), 1);
    let entry: serde_json::Value = serde_json::from_str(wire.trim()).unwrap();
    assert!(entry["error"].is_null());
    assert_eq!(
        std::fs::read(
            owned
                .directory
                .join("wire")
                .join(entry["response"].as_str().unwrap())
        )
        .unwrap(),
        reply
    );
}
