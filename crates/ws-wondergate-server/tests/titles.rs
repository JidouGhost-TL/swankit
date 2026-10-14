// SPDX-License-Identifier: MIT
// Ported remaining cases from the private server tests/test_services.py.
// Copyright (C) 2026 jidoughost. Evidence is named by client lineage.
mod common;
use common::*;
use serde_json::json;
use ws_wondergate_server::{
    store::{JournalStore, MemoryStore, Scope, Store},
    Definition, FixedClock, SeededRng, Server,
};

fn phase() -> Definition {
    Definition::bundled()
        .unwrap()
        .into_iter()
        .find(|d| d.json()["values"].get("phase").is_some())
        .unwrap()
}
fn rankings() -> Definition {
    Definition::bundled()
        .unwrap()
        .into_iter()
        .find(|d| d.json()["boards"].get("main").is_some())
        .unwrap()
}
fn check_in() -> Definition {
    Definition::bundled()
        .unwrap()
        .into_iter()
        .find(|d| d.json()["routes"][0]["id"] == "check-in")
        .unwrap()
}
fn signup_placeholder() -> Definition {
    Definition::bundled()
        .unwrap()
        .into_iter()
        .find(|d| d.json()["routes"][0]["id"] == "signup-placeholder")
        .unwrap()
}
fn get(d: &Definition, path: &str, query: &str) -> Vec<u8> {
    format!(
        "GET {path}?{query} HTTP/1.1\r\nHost: {}\r\nUser-Agent: SWAN/1.0\r\n\r\n",
        d.hosts()[0]
    )
    .into_bytes()
}
fn literal(d: &Definition) -> Vec<u8> {
    get(
        d,
        d.json()["routes"][0]["match"]["path"].as_str().unwrap(),
        d.json()["routes"][0]["match"]["query_exact"]
            .as_str()
            .unwrap(),
    )
}
fn submit(d: &Definition, name: &str, score: &str) -> Vec<u8> {
    get(
        d,
        d.json()["routes"][0]["match"]["path"].as_str().unwrap(),
        &format!("nm={name}&sc={score}"),
    )
}
fn scope(d: &Definition) -> Scope {
    Scope::new(d.id(), "main", "default")
}
fn app(d: Definition) -> Server {
    Server::from_definitions(
        vec![d],
        Box::new(MemoryStore::new()),
        Box::new(FixedClock(stamp())),
        Box::new(SeededRng::new(123)),
    )
    .unwrap()
}
fn payload(raw: &[u8]) -> Vec<u8> {
    let body = split_response(raw).2;
    assert!(body.starts_with(b"#SUM") && body.ends_with(&[0]));
    let sum = &body[4..8];
    let payload = &body[8..body.len() - 9];
    assert_eq!(&body[body.len() - 9..body.len() - 5], b"#SUM");
    assert_eq!(sum, &body[body.len() - 5..body.len() - 1]);
    assert_eq!(
        u16::from_str_radix(std::str::from_utf8(sum).unwrap(), 16).unwrap(),
        payload
            .iter()
            .fold(0u16, |a, b| a.wrapping_add(u16::from(*b)))
    );
    payload.to_vec()
}
#[test]
fn short_lineage_literal_new_acknowledgment() {
    let d = check_in();
    let app = server();
    let raw = literal(&d);
    for raw in [
        raw.clone(),
        [raw.clone(), vec![0]].concat(),
        String::from_utf8(raw.clone())
            .unwrap()
            .replace("User-Agent: SWAN/1.0\r\n", "")
            .into_bytes(),
    ] {
        let reply = app.dispatch_http(&raw);
        assert!(reply.starts_with(b"HTTP/1.1 200"));
        assert!(split_response(&reply).2.is_empty());
    }
    let query = d.json()["routes"][0]["match"]["query_exact"]
        .as_str()
        .unwrap()
        .replace("666", "667");
    assert!(app
        .dispatch_http(&get(
            &d,
            d.json()["routes"][0]["match"]["path"].as_str().unwrap(),
            &query
        ))
        .starts_with(b"HTTP/1.1 400"));
}

#[test]
fn lineage_route_error_precedence_matches_handlers() {
    for (d, unknown_path, bad_query) in [
        (phase(), 404, 404),
        (check_in(), 404, 405),
        (rankings(), 405, 405),
    ] {
        let app = app(d.clone());
        let raw = get(&d, "/unknown", "");
        let post = String::from_utf8(raw).unwrap().replacen("GET ", "POST ", 1);
        assert!(app
            .dispatch_http(post.as_bytes())
            .starts_with(format!("HTTP/1.1 {unknown_path}").as_bytes()));
        let path = d.json()["routes"][0]["match"]["path"].as_str().unwrap();
        let post = String::from_utf8(get(&d, path, "bad=1"))
            .unwrap()
            .replacen("GET ", "POST ", 1);
        assert!(app
            .dispatch_http(post.as_bytes())
            .starts_with(format!("HTTP/1.1 {bad_query}").as_bytes()));
    }
}
#[cfg(feature = "net")]
#[test]
fn short_lineage_fragmented_request_over_tcp() {
    use std::{
        io::{Read, Write},
        net::{Shutdown, TcpStream},
        sync::Arc,
        time::Duration,
    };
    use ws_wondergate_server::net::{Config, TcpServer, WireLog};
    let temp = Temp::new();
    let wire = Arc::new(WireLog::open(temp.0.join("wire")).unwrap());
    let mut tcp = TcpServer::bind(
        server(),
        Config {
            http: ([127, 0, 0, 1], 0).into(),
            bpl: ([127, 0, 0, 1], 0).into(),
            ..Config::default()
        },
        wire.clone(),
    )
    .unwrap();
    let raw = literal(&check_in());
    let mut stream = TcpStream::connect(tcp.http_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    for byte in &raw {
        stream.write_all(&[*byte]).unwrap();
    }
    stream.shutdown(Shutdown::Write).unwrap();
    let mut received = vec![];
    stream.read_to_end(&mut received).unwrap();
    assert_eq!(received, server().dispatch_http(&raw));
    tcp.shutdown().unwrap();
    let entries = std::fs::read_to_string(wire.directory().join("wire.jsonl")).unwrap();
    let entry: serde_json::Value = serde_json::from_str(entries.lines().next().unwrap()).unwrap();
    assert_eq!(entry["request_length"], raw.len());
    assert_eq!(entry["response_length"], received.len());
    assert_eq!(
        std::fs::read(wire.directory().join(entry["request"].as_str().unwrap())).unwrap(),
        raw
    );
    assert_eq!(
        std::fs::read(wire.directory().join(entry["response"].as_str().unwrap())).unwrap(),
        received
    );
}
#[test]
fn vt_lineage_literal_and_seven_lines() {
    let reply = server().dispatch_http(&[literal(&phase()), vec![0]].concat());
    let (head, _, body) = split_response(&reply);
    assert_eq!(head.split("\r\n").count() + 1, 7);
    assert_eq!(body.len(), 11);
    assert_eq!(&body[8..11], &[1, 0, 0]);
    let frame = [
        vec![0x11, 0, body.len() as u8 + 3, 0x0f, 0, body.len() as u8],
        body.to_vec(),
    ]
    .concat();
    assert_eq!(&frame[8..11], &[1, 0, 0]);
}
#[test]
fn vt_lineage_nonzero_phase_and_rollover() {
    let mut d = phase().json().clone();
    d["values"]["phase"]["fixed"] = json!({"phase":2,"elapsed_s":719});
    let d = Definition::from_json(&d.to_string()).unwrap();
    let reply = app(d.clone()).dispatch_http(&literal(&d));
    let body = split_response(&reply).2;
    assert_eq!(&body[2..5], &body[8..11]);
    assert_eq!(u16::from(body[3]) * 60 + u16::from(body[4]), 719);
    let mut invalid = d.json().clone();
    invalid["values"]["phase"]["fixed"]["elapsed_s"] = json!(720);
    assert!(Definition::from_json(&invalid.to_string()).is_err());
    let mut clock = d.json().clone();
    clock["values"]["phase"]["policy"] = json!("clock");
    clock["values"]["phase"]
        .as_object_mut()
        .unwrap()
        .remove("fixed");
    let seconds: i64 = stamp().into();
    for (delta, mode, minute, second) in [
        (0, 1, 0, 0),
        (719, 1, 11, 59),
        (720, 2, 0, 0),
        (1440, 1, 0, 0),
        (-1, 2, 11, 59),
    ] {
        clock["values"]["phase"]["epoch_s"] = json!(seconds - delta);
        let d = Definition::from_json(&clock.to_string()).unwrap();
        let reply = app(d.clone()).dispatch_http(&literal(&d));
        assert_eq!(&split_response(&reply).2[2..5], &[mode, minute, second]);
    }
}
#[test]
fn bank13_lineage_literal_stores_and_returns_ranking() {
    let d = rankings();
    let temp = Temp::new();
    let path = temp.0.join("journal.jsonl");
    let app = Server::from_definitions(
        vec![d.clone()],
        Box::new(JournalStore::open(&path).unwrap()),
        Box::new(FixedClock(stamp())),
        Box::new(SeededRng::new(1)),
    )
    .unwrap();
    let reply = app.dispatch_http(&submit(&d, "ken22", "12345678"));
    let body = payload(&reply);
    let prefix = d.json()["routes"][0]["respond"]["body"]["segments"][0]["text"]
        .as_str()
        .unwrap();
    assert!(body.starts_with(&[prefix.as_bytes(), b"1#ken22_12345678#"].concat()));
    drop(app);
    let store = JournalStore::open(&path).unwrap();
    let rows = store.submissions(&scope(&d)).unwrap();
    assert_eq!(rows[0].submission.score, 12345678);
    assert_eq!(rows[0].submission.name, b"ken22");
    assert_eq!(rows[0].provenance, "new");
}
#[test]
fn bank13_lineage_top_three_current_and_fourth() {
    let d = rankings();
    let app = server();
    for i in 0..6 {
        app.dispatch_http(&submit(&d, &format!("user{i}"), &i.to_string()));
    }
    let reply = app.dispatch_http(&submit(&d, "user0", "0"));
    let body = payload(&reply);
    let prefix = d.json()["routes"][0]["respond"]["body"]["segments"][0]["text"]
        .as_str()
        .unwrap();
    assert_eq!(
        body,
        [
            prefix.as_bytes(),
            b"1#user5_5#2#user4_4#3#user3_3#6#user0_0#4#user2_2#"
        ]
        .concat()
    );
    // Current request score is preserved even when the best score is older.
    let reply = app.dispatch_http(&submit(&d, "user5", "0"));
    assert!(payload(&reply)
        .windows(b"1#user5_0#".len())
        .any(|p| p == b"1#user5_0#"));
    let page = get(&d, "/wg/rankings", "");
    let reply = app.dispatch_http(&page);
    let body = split_response(&reply).2;
    assert_eq!(body.windows(4).filter(|p| *p == b"<tr>").count(), 7);
    assert!(!body.windows(19).any(|p| p == b"<td>0</td><td></td>"));
}
#[test]
fn bank13_lineage_percent_bytes_invalid_queries_do_not_store() {
    let d = rankings();
    let temp = Temp::new();
    let path = temp.0.join("journal.jsonl");
    let app = Server::from_definitions(
        vec![d.clone()],
        Box::new(JournalStore::open(&path).unwrap()),
        Box::new(FixedClock(stamp())),
        Box::new(SeededRng::new(1)),
    )
    .unwrap();
    assert!(app
        .dispatch_http(&submit(&d, "%82%A0", "12345678"))
        .starts_with(b"HTTP/1.1 200"));
    for (name, score) in [
        ("toolong", "1"),
        ("a_b", "1"),
        ("a%00", "1"),
        ("%ZZ", "1"),
        ("ok", "-1"),
        ("ok", "100000000"),
        ("%", "1"),
        ("%8", "1"),
        ("a#", "1"),
    ] {
        // A raw fragment is stripped by the HTTP parser, leaving missing fields.
        assert!(app
            .dispatch_http(&submit(&d, name, score))
            .starts_with(b"HTTP/1.1 400"));
    }
    for query in [
        "nm=ok&nm=bad&sc=1",
        "sc=1&nm=ok&other=1",
        "nm=ok",
        "nm=%20&sc=%ZZ",
    ] {
        assert!(app
            .dispatch_http(&get(
                &d,
                d.json()["routes"][0]["match"]["path"].as_str().unwrap(),
                query
            ))
            .starts_with(b"HTTP/1.1 400"));
    }
    drop(app);
    let rows = JournalStore::open(path)
        .unwrap()
        .submissions(&scope(&d))
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].submission.name, [0x82, 0xa0]);
}
#[test]
fn signup_placeholder_accepts_binary_post_and_preserves_reply_layout() {
    // A synthetic binary account request with the observed MIME spelling.
    // The authored reply is a flag followed by an eight-byte identifier.
    let d = signup_placeholder();
    let app = server();
    let body = [0xFFu8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
    let raw = [
        format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: SWAN/1.0\r\nContent-Type: application/x-www-from-urlencoded\r\nContent-Length: 9\r\n\r\n",
            d.json()["routes"][0]["match"]["path"].as_str().unwrap(),
            d.hosts()[0],
        )
        .into_bytes(),
        body.to_vec(),
    ]
    .concat();
    let reply = app.dispatch_http(&raw);
    assert!(reply.starts_with(b"HTTP/1.1 200"));
    let (status, headers, payload) = split_response(&reply);
    assert!(status.starts_with("HTTP/1.1 200"));
    assert_eq!(headers.get("Content-Length").map(String::as_str), Some("9"));
    assert_eq!(payload, &[0, 1, 2, 3, 4, 5, 6, 7, 8]);
}
#[test]
fn bank13_lineage_new_stock_and_unimplemented_content() {
    let d = rankings();
    let app = server();
    let path = d.json()["routes"][0]["match"]["path"].as_str().unwrap();
    for (id, query, status) in [
        ("stock", "it=kure", 200),
        ("maze", "mp=kure", 501),
        ("stock", "it=%6bure", 200),
    ] {
        let reply = app.dispatch_http(&get(&d, path, query));
        assert!(reply.starts_with(format!("HTTP/1.1 {status}").as_bytes()));
        let route = d.json()["routes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap();
        assert_eq!(
            payload(&reply),
            route["respond"]["body"]["segments"][0]["text"]
                .as_str()
                .unwrap()
                .as_bytes()
        );
    }
}
