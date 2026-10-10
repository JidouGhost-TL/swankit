// SPDX-License-Identifier: MIT OR Apache-2.0
#![cfg(feature = "net")]
mod common;
use common::*;
use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::Arc,
    time::Duration,
};
use ws_wondergate_server::net::{Config, TcpServer, WireLog};
#[test]
fn trickled_http_bytes_cannot_extend_transaction_deadline() {
    let temp = Temp::new();
    let wire = Arc::new(WireLog::open(temp.0.join("wire")).unwrap());
    let mut server = TcpServer::bind(
        server(),
        Config {
            http: ([127, 0, 0, 1], 0).into(),
            bpl: ([127, 0, 0, 1], 0).into(),
            timeout: Duration::from_millis(150),
            ..Config::default()
        },
        wire.clone(),
    )
    .unwrap();
    let mut stream = TcpStream::connect(server.http_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    // Keep the idle timer refreshed longer than the receiver's timeout.
    stream.write_all(b"POST ").unwrap();
    let mut sender = stream.try_clone().unwrap();
    let writer = std::thread::spawn(move || {
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(50));
            if sender.write_all(b"/").is_err() {
                break;
            }
        }
    });
    let mut reply = [0; 256];
    let n = stream.read(&mut reply).unwrap();
    assert!(reply[..n].starts_with(b"HTTP/1.1 400"));
    writer.join().unwrap();
    server.shutdown().unwrap();
    let line = std::fs::read_to_string(wire.directory().join("wire.jsonl")).unwrap();
    let entry: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert!(entry["error"].is_string());
    assert!((5..35).contains(&entry["request_length"].as_u64().unwrap()));
}
