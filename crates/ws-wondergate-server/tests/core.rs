// SPDX-License-Identifier: MIT OR Apache-2.0
mod common;
use common::*;
use std::sync::Arc;
use ws_wondergate_server::{
    board::{Board, Order, Ties},
    http,
    services::{self, Services},
    store::{JournalStore, MemoryStore, Scope, Store, Submission},
    FixedClock, Limits, Progress, Rng, SeededRng, Server, Timestamp,
};

#[test]
fn http_latin1_and_binary_body_are_lossless() {
    let services = Services::bundled().unwrap();
    let mut raw = format!(
        "POST http://{}/raw?nm=%82%A0 HTTP/1.0\r\nHost: {}.:80\r\nContent-Length: 4\r\nX-Bytes: ",
        services.charge_hosts[0].to_ascii_uppercase(),
        services.charge_hosts[0]
    )
    .into_bytes();
    raw.extend([0x82, 0xa0, b'X']);
    raw.extend(b"\r\n\r\n");
    raw.extend([0x82, 0xa0, 0, 255]);
    let request = http::parse(&raw, &Limits::default()).unwrap();
    assert_eq!(request.raw, raw);
    assert_eq!(request.body, [0x82, 0xa0, 0, 255]);
    assert_eq!(
        request.headers["x-bytes"]
            .chars()
            .map(|c| c as u8)
            .collect::<Vec<_>>(),
        [0x82, 0xa0, b'X']
    );
    assert_eq!(request.host, services.charge_hosts[0]);
    assert_eq!(request.path, "/raw");
    assert_eq!(request.query, "nm=%82%A0");
    let response = http::response(
        200,
        &[
            ("X-Extra", "value"),
            ("Content-Type", "application/octet-stream"),
        ],
        &request.body,
    )
    .unwrap();
    assert_eq!(response, [b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nX-Extra: value\r\n\r\n".as_slice(),request.body.as_slice()].concat());
}
#[test]
fn http_rejects_invalid_headers_targets_and_lengths() {
    let host = &Services::bundled().unwrap().charge_hosts[0];
    let base = format!("POST / HTTP/1.1\r\nHost: {host}\r\n");
    for line in [
        "NoColon",
        ": missing",
        "Bad_Key: 1",
        "Host: duplicate",
        "Transfer-Encoding: identity",
        "Content-Length: -1",
        "Content-Length: 0000000",
        "Content-Length: 65537",
        "Content-Length: 1x",
    ] {
        assert!(
            http::parse(
                format!("{base}{line}\r\n\r\n").as_bytes(),
                &Limits::default()
            )
            .is_err(),
            "{line}"
        );
    }
    for target in [
        "relative",
        "https://untrusted/",
        "http://untrusted/",
        "//untrusted/",
        "/",
    ] {
        let req = format!("GET {target} HTTP/2.0\r\nHost: {host}\r\n\r\n");
        assert!(http::parse(req.as_bytes(), &Limits::default()).is_err());
        if target != "/" {
            assert!(http::parse(
                req.replace("HTTP/2.0", "HTTP/1.1").as_bytes(),
                &Limits::default()
            )
            .is_err());
        }
    }
    for authority in [
        "",
        "...",
        "host:abc",
        "host:",
        "host:1:2",
        "user@host",
        "[::1]",
        "host/path",
    ] {
        let req = format!("GET / HTTP/1.1\r\nHost: {authority}\r\n\r\n");
        assert!(http::parse(req.as_bytes(), &Limits::default()).is_err());
    }
    let raw = post("nph-gettime", 0, false, &[]);
    assert!(http::parse(&[raw.clone(), vec![0]].concat(), &Limits::default()).is_ok());
    assert!(http::parse(&[raw, vec![0, 0]].concat(), &Limits::default()).is_err());
    let body = format!("{base}Content-Length: 1\r\n\r\n");
    assert!(http::parse(body.as_bytes(), &Limits::default()).is_err());
    assert!(http::parse(&[body.as_bytes(), &[1, 0]].concat(), &Limits::default()).is_err());
    assert!(http::response(200, &[("X", "value\r\nInjected: yes")], &[]).is_err());
    assert!(http::response(200, &[("X\nBad", "yes")], &[]).is_err());
    assert!(http::response(299, &[], &[]).is_err());
}
#[test]
fn assemblers_fragment_bounds_eof_and_consecutive_bpl() {
    let raw = post("nph-gettime", 0, false, b"content-LENGTH: 3\r\n");
    let full = [raw.clone(), vec![0x82, 0xa0, 0]].concat();
    let mut input = http::Assembler::new(Limits::default());
    for (i, byte) in full.iter().enumerate() {
        assert_eq!(input.feed(&[*byte]).unwrap(), i + 1 == full.len());
    }
    input.finish().unwrap();
    assert_eq!(input.raw(), full);
    assert_eq!(input.body_size(), Some(3));
    assert!(input.feed(&[0]).is_err());
    assert!(input.feed(&[]).is_err());
    let mut missing = http::Assembler::new(Limits::default());
    missing.feed(&raw).unwrap();
    assert!(missing.finish().is_err());
    let limits = Limits {
        max_header: 8,
        ..Limits::default()
    };
    assert!(http::Assembler::new(limits).feed(b"012345678901").is_err());
    let mut input = http::Assembler::new(Limits::default());
    input.feed(&post("nph-gettime", 0, false, &[])).unwrap();
    assert!(input.feed(&[0]).unwrap());
    input.finish().unwrap();
    let packet = packet();
    let pair = [packet.clone(), packet.clone()].concat();
    for size in [1, 2, 7, 24, 45, 90] {
        let mut input = services::bpl::Assembler::new(Limits::default());
        let mut out = vec![];
        for chunk in pair.chunks(size) {
            out.extend(input.feed(chunk).unwrap());
        }
        assert_eq!(out, [packet.clone(), packet.clone()]);
        input.finish().unwrap();
    }
    let mut input = services::bpl::Assembler::new(Limits::default());
    input.feed(&packet[..44]).unwrap();
    assert!(input.finish().is_err());
    for length in [0u32, 8, 23, 65537, u32::MAX] {
        let mut raw = [0u8; 8];
        raw[4..].copy_from_slice(&length.to_le_bytes());
        let mut input = services::bpl::Assembler::new(Limits::default());
        assert!(input.feed(&raw).is_err());
        assert!(input.feed(&packet).is_err());
    }
    let mut maximum = vec![0; 65536];
    maximum[..8].copy_from_slice(&[1, 24, 0, 0, 0, 0, 1, 0]);
    assert_eq!(
        services::bpl::reply(&maximum, &Limits::default())
            .unwrap()
            .len(),
        24
    );
}
fn submission(scope: &Scope, identity: &[u8], name: &[u8], score: u64, extra: i64) -> Submission {
    Submission {
        scope: scope.clone(),
        identity: identity.into(),
        name: name.into(),
        score,
        extra,
        submitted_at: stamp(),
    }
}
#[test]
fn boards_best_scores_metadata_ties_padding_and_isolation() {
    let store = MemoryStore::new();
    let scope = Scope::new("synthetic", "rank", "1");
    let down = Board::new(&store, scope.clone(), Order::Descending);
    assert_eq!(down.count().unwrap(), 0);
    assert_eq!(down.position(b"absent").unwrap(), 0);
    assert_eq!(down.slots(3).unwrap(), vec![None, None, None]);
    for (id, name, score, extra) in [
        (b"a".as_slice(), b"old".as_slice(), 20, 1),
        (b"b", b"second", 30, 2),
        (b"a", b"best", 30, 3),
        (b"a", b"equal", 30, 4),
        (b"a", b"worse", 10, 5),
        (b"", b"zero-profile", 25, 0),
    ] {
        store
            .submit(submission(&scope, id, name, score, extra))
            .unwrap();
    }
    let rows = down.leaders(10).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].submission.identity, b"a");
    assert_eq!(rows[0].submission.name, b"best");
    assert_eq!(rows[0].submission.extra, 3);
    assert_eq!(down.position(b"a").unwrap(), 1);
    let later =
        Board::new(&store, scope.clone(), Order::Descending).with_ties(Ties::FirstBestSubmission);
    assert_eq!(later.leaders(1).unwrap()[0].submission.identity, b"b");
    assert_eq!(down.position(b"").unwrap(), 3);
    assert_eq!(down.slots(4).unwrap()[3], None);
    assert!(down.slots(4097).is_err());
    let up = Board::new(&store, scope.clone(), Order::Ascending);
    assert_eq!(up.leaders(1).unwrap()[0].submission.name, b"worse");
    for other in [
        Scope::new("other", "rank", "1"),
        Scope::new("synthetic", "other", "1"),
        Scope::new("synthetic", "rank", "2"),
    ] {
        assert_eq!(
            Board::new(&store, other, Order::Descending)
                .count()
                .unwrap(),
            0
        );
    }
}
#[test]
fn journal_replays_binary_submissions_and_refuses_corrupt_tails() {
    let temp = Temp::new();
    let path = temp.0.join("journal.jsonl");
    let scope = Scope::new("synthetic", "rank", "1");
    let store = JournalStore::open(&path).unwrap();
    assert!(JournalStore::open(&path).is_err());
    store
        .submit(submission(&scope, b"", &[0x82, 0xa0], 42, 7))
        .unwrap();
    let first = store
        .aid("platform", "sid", &"ab".repeat(40), stamp())
        .unwrap();
    assert_eq!(
        first,
        store
            .aid("platform", "sid", &"cd".repeat(40), stamp())
            .unwrap()
    );
    assert_ne!(
        first,
        store
            .aid("platform", "sid2", &"cd".repeat(40), stamp())
            .unwrap()
    );
    let before = store.submissions(&scope).unwrap();
    drop(store);
    let frozen = std::fs::read(&path).unwrap();
    let restored = JournalStore::open(&path).unwrap();
    assert_eq!(restored.submissions(&scope).unwrap(), before);
    assert_eq!(
        restored
            .aid("platform", "sid", &"cd".repeat(40), stamp())
            .unwrap(),
        first
    );
    drop(restored);
    assert_eq!(std::fs::read(&path).unwrap(), frozen);
    for tail in [b"{".as_slice(), b"\n", b"{}\n"] {
        let corrupt = [frozen.clone(), tail.to_vec()].concat();
        std::fs::write(&path, &corrupt).unwrap();
        assert!(JournalStore::open(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
    }
    let mut record: serde_json::Value =
        serde_json::from_slice(frozen.split(|b| *b == b'\n').next().unwrap()).unwrap();
    record["provenance"] = "original".into();
    std::fs::write(&path, format!("{record}\n")).unwrap();
    assert!(JournalStore::open(&path).is_err());
    std::fs::write(&path, vec![b'x'; 131073]).unwrap();
    assert!(JournalStore::open(&path).is_err());
}
#[test]
fn concurrent_tokens_have_one_durable_winner() {
    let temp = Temp::new();
    let path = temp.0.join("journal.jsonl");
    let store = Arc::new(JournalStore::open(&path).unwrap());
    let threads: Vec<_> = (0..16)
        .map(|i| {
            let store = store.clone();
            std::thread::spawn(move || {
                store
                    .aid("platform", "sid", &format!("{i:080x}"), stamp())
                    .unwrap()
            })
        })
        .collect();
    let aids: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(aids.iter().all(|aid| aid == &aids[0]));
    drop(store);
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
    assert_eq!(
        JournalStore::open(path)
            .unwrap()
            .aid("platform", "sid", &"ff".repeat(40), stamp())
            .unwrap(),
        aids[0]
    );
}
#[test]
fn clocks_calendar_rollover_and_seeded_rng_chunking() {
    for (utc, jst) in [
        ("19991231180000", "20000101030000"),
        ("20000228180000", "20000229030000"),
        ("21000228180000", "21000301030000"),
        ("19691231180000", "19700101030000"),
    ] {
        assert_eq!(Timestamp::parse_fixed(utc).unwrap().jst_digits(), jst);
    }
    for bad in [
        "20260229000000",
        "20261301000000",
        "20261009246000",
        "00001009000000",
        "20261009",
        "2026100900000é",
    ] {
        assert!(Timestamp::parse_fixed(bad).is_err());
    }
    assert!(Timestamp::try_from(i64::MAX).is_err());
    assert!(Timestamp::try_from(i64::MIN).is_err());
    let mut rng = SeededRng::new(0);
    let mut full = [0; 80];
    rng.fill(&mut full).unwrap();
    assert_eq!(&full[..8], &0xe220_a839_7b1d_cdafu64.to_le_bytes());
    let mut chunked = [0; 80];
    let mut rng = SeededRng::new(0);
    for part in chunked.chunks_mut(3) {
        rng.fill(part).unwrap();
    }
    assert_eq!(full, chunked);
}
#[test]
fn sessions_route_methods_errors_and_multiple_packets() {
    let app = server();
    let req = post("nph-gettime", 0, false, &[]);
    let mut session = app.session(80);
    assert_eq!(session.feed(&req[..5]).unwrap(), Progress::NeedMore);
    assert_eq!(
        session.feed(&req[5..]).unwrap(),
        Progress::Reply {
            bytes: app.dispatch_http(&req),
            close: true
        }
    );
    session.finish().unwrap();
    assert!(session.feed(&[]).is_err());
    let packet = packet();
    let mut session = app.session(5555);
    assert_eq!(
        session
            .feed(&[packet.clone(), packet.clone()].concat())
            .unwrap(),
        Progress::Reply {
            bytes: services::bpl::reply(&packet, &Limits::default())
                .unwrap()
                .repeat(2),
            close: false
        }
    );
    session.finish().unwrap();
    let get = String::from_utf8(req).unwrap().replacen("POST", "GET", 1);
    assert!(app
        .dispatch_http(get.as_bytes())
        .starts_with(b"HTTP/1.1 405"));
    let missing = post("missing", 0, false, &[]);
    assert!(app.dispatch_http(&missing).starts_with(b"HTTP/1.1 404"));
    assert!(app.resolves(&app.services().charge_hosts[0]));
    assert!(!app.resolves("untrusted"));
}
#[test]
fn services_definitions_validate_and_injected_failures_become_500() {
    let bundled = include_str!("../data/services.json");
    let mut value: serde_json::Value = serde_json::from_str(bundled).unwrap();
    value["provenance"]["server"] = "original".into();
    assert!(Services::from_json(&value.to_string()).is_err());
    let mut services = Services::bundled().unwrap();
    services.charge_hosts.push(services.charge_hosts[0].clone());
    assert!(services.validate().is_err());
    let mut services = Services::bundled().unwrap();
    services.http_aliases[0][0] = "untrusted".into();
    assert!(services.validate().is_err());
    struct BrokenRng;
    impl Rng for BrokenRng {
        fn fill(&mut self, _: &mut [u8]) -> std::io::Result<()> {
            Err(std::io::Error::other("synthetic failure"))
        }
    }
    let app = Server::bundled(
        Box::new(MemoryStore::new()),
        Box::new(FixedClock(stamp())),
        Box::new(BrokenRng),
    )
    .unwrap();
    assert!(app
        .dispatch_http(&post("nph-session", 0, false, &[]))
        .starts_with(b"HTTP/1.1 500"));
}
#[test]
fn deterministic_malformed_input_loops_never_panic() {
    let mut rng = SeededRng::new(77);
    let limits = Limits::default();
    for length in 0..512 {
        let mut raw = vec![0; length];
        rng.fill(&mut raw).unwrap();
        let _ = http::parse(&raw, &limits);
        let _ = services::bpl::reply(&raw, &limits);
        let mut http = http::Assembler::new(limits);
        let mut bpl = services::bpl::Assembler::new(limits);
        for chunk in raw.chunks(7) {
            if http.feed(chunk).is_err() {
                break;
            }
        }
        for chunk in raw.chunks(7) {
            if bpl.feed(chunk).is_err() {
                break;
            }
        }
        let _ = http.finish();
        let _ = bpl.finish();
    }
    let valid = post("nph-gettime", 0, false, &[]);
    for i in 0..valid.len() {
        let mut raw = valid.clone();
        raw[i] = 255;
        let _ = http::parse(&raw, &limits);
    }
}
