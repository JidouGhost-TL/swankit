// SPDX-License-Identifier: MIT
// Ported framing/wire case from private server tests/test_services.py (MIT).
// Copyright (C) 2026 jidoughost. Additional lifecycle/error cases are new.
#![cfg(feature = "net")]
mod common;
use common::*;
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    sync::Arc,
    time::Duration,
};
use ws_wondergate_server::{
    net::{Config, TcpServer, WireLog},
    services::bpl,
    Limits,
};

fn start(temp: &Temp) -> (TcpServer, Arc<WireLog>) {
    let wire = Arc::new(WireLog::open(temp.0.join("wire")).unwrap());
    let tcp = TcpServer::bind(
        server(),
        Config {
            http: ([127, 0, 0, 1], 0).into(),
            bpl: ([127, 0, 0, 1], 0).into(),
            timeout: Duration::from_millis(300),
            ..Config::default()
        },
        wire.clone(),
    )
    .unwrap();
    (tcp, wire)
}
fn exchange(address: std::net::SocketAddr, packet: &[u8], fragment: bool) -> Vec<u8> {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    if fragment {
        for byte in packet {
            stream.write_all(&[*byte]).unwrap();
        }
    } else {
        stream.write_all(packet).unwrap();
    }
    stream.shutdown(Shutdown::Write).unwrap();
    let mut reply = vec![];
    stream.read_to_end(&mut reply).unwrap();
    reply
}
fn entries(wire: &WireLog) -> Vec<serde_json::Value> {
    std::fs::read_to_string(wire.directory().join("wire.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn verify(wire: &WireLog, entries: &[serde_json::Value]) {
    use formatkit_corpus_test::hash_bytes;
    for entry in entries {
        let keys = [
            "id",
            "time",
            "protocol",
            "peer",
            "request",
            "response",
            "request_length",
            "response_length",
            "error",
            "request_sha256",
            "response_sha256",
        ];
        assert_eq!(entry.as_object().unwrap().len(), keys.len());
        for key in keys {
            assert!(entry.get(key).is_some());
        }
        assert_eq!(entry["id"].as_str().unwrap().len(), 32);
        assert_eq!(entry["time"], "2026-10-05T20:04:03+00:00");
        assert!(entry["peer"][0].is_string());
        assert!(entry["peer"][1].is_u64());
        for direction in ["request", "response"] {
            let bytes =
                std::fs::read(wire.directory().join(entry[direction].as_str().unwrap())).unwrap();
            assert_eq!(entry[format!("{direction}_length")], bytes.len());
            assert_eq!(entry[format!("{direction}_sha256")], hash_bytes(&bytes));
        }
    }
}
#[test]
fn socket_fragmentation_close_and_full_wire_logs() {
    let temp = Temp::new();
    let (mut tcp, wire) = start(&temp);
    let http = [post("nph-gettime", 0, false, &[]), vec![0]].concat();
    let packet = packet();
    assert_eq!(
        exchange(tcp.http_addr(), &http, true),
        server().dispatch_http(&http)
    );
    assert_eq!(
        exchange(tcp.bpl_addr(), &packet, true),
        bpl::reply(&packet, &Limits::default()).unwrap()
    );
    tcp.shutdown().unwrap();
    let entries = entries(&wire);
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry["error"].is_null()));
    verify(&wire, &entries);
    assert_eq!(
        std::fs::read(
            wire.directory()
                .join(entries[0]["request"].as_str().unwrap())
        )
        .unwrap(),
        http
    );
    assert_eq!(
        std::fs::read(
            wire.directory()
                .join(entries[1]["request"].as_str().unwrap())
        )
        .unwrap(),
        packet
    );
}
#[test]
fn consecutive_bpl_packets_get_individual_records_and_http_delayed_nul() {
    let temp = Temp::new();
    let (mut tcp, wire) = start(&temp);
    let packet = packet();
    let pair = [packet.clone(), packet.clone()].concat();
    let reply = exchange(tcp.bpl_addr(), &pair, false);
    assert_eq!(
        reply,
        bpl::reply(&packet, &Limits::default()).unwrap().repeat(2)
    );
    let raw = post("nph-gettime", 0, false, &[]);
    let mut stream = TcpStream::connect(tcp.http_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.write_all(&raw).unwrap();
    std::thread::sleep(Duration::from_millis(10));
    stream.write_all(&[0]).unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut reply = vec![];
    stream.read_to_end(&mut reply).unwrap();
    assert_eq!(reply, server().dispatch_http(&raw));
    tcp.shutdown().unwrap();
    let entries = entries(&wire);
    assert_eq!(entries.len(), 3);
    verify(&wire, &entries);
    assert_eq!(
        std::fs::read(
            wire.directory()
                .join(entries[2]["request"].as_str().unwrap())
        )
        .unwrap(),
        [raw, vec![0]].concat()
    );
}
#[test]
fn malformed_and_early_eof_log_complete_received_bytes() {
    let temp = Temp::new();
    let (mut tcp, wire) = start(&temp);
    for raw in [
        b"GET / HTTP/1.1\r\nBad_Key: 1\r\n\r\n".to_vec(),
        post("nph-gettime", 0, false, b"Content-Length: 3\r\n"),
    ] {
        assert!(exchange(tcp.http_addr(), &raw, false).starts_with(b"HTTP/1.1 400"));
    }
    for raw in [vec![1, 24, 0], vec![1, 24, 0, 0, 255, 255, 255, 255]] {
        assert!(exchange(tcp.bpl_addr(), &raw, false).is_empty());
    }
    tcp.shutdown().unwrap();
    let entries = entries(&wire);
    assert_eq!(entries.len(), 4);
    assert!(entries.iter().all(|entry| entry["error"].is_string()));
    verify(&wire, &entries);
    assert_eq!(entries[2]["request_length"], 3);
    assert_eq!(entries[3]["request_length"], 8);
}
#[test]
fn thread_shutdown_and_wire_reopen_preserve_all_prior_files() {
    let temp = Temp::new();
    let (mut tcp, wire) = start(&temp);
    assert!(WireLog::open(wire.directory()).is_err());
    exchange(tcp.http_addr(), &post("nph-gettime", 0, false, &[]), false);
    tcp.shutdown().unwrap();
    let entries = entries(&wire);
    let original = std::fs::read(
        wire.directory()
            .join(entries[0]["response"].as_str().unwrap()),
    )
    .unwrap();
    let directory = wire.directory().to_owned();
    drop(wire);
    drop(tcp);
    let wire = WireLog::open(directory).unwrap();
    wire.record(
        "HTTP",
        ([127, 0, 0, 1], 123).into(),
        b"new",
        b"reply",
        None,
        stamp(),
    )
    .unwrap();
    let reopened = self::entries(&wire);
    assert_eq!(reopened.len(), 2);
    assert_ne!(reopened[0]["id"], reopened[1]["id"]);
    assert_eq!(
        std::fs::read(
            wire.directory()
                .join(reopened[0]["response"].as_str().unwrap())
        )
        .unwrap(),
        original
    );
    verify(&wire, &reopened);
}
