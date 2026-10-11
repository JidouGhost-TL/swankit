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
    services::Services,
    store::JournalStore,
    Definition, FixedClock, SeededRng, Server,
};
fn start(temp: &Temp, timeout: Duration) -> (TcpServer, Arc<WireLog>) {
    let wire = Arc::new(WireLog::open(temp.0.join("wire")).unwrap());
    let app = Server::bundled(
        Box::new(JournalStore::open(temp.0.join("store.jsonl")).unwrap()),
        Box::new(FixedClock(stamp())),
        Box::new(SeededRng::new(123)),
    )
    .unwrap();
    let tcp = TcpServer::bind(
        app,
        Config {
            http: ([127, 0, 0, 1], 0).into(),
            bpl: ([127, 0, 0, 1], 0).into(),
            timeout,
            ..Config::default()
        },
        wire.clone(),
    )
    .unwrap();
    (tcp, wire)
}
fn exchange(address: std::net::SocketAddr, bytes: &[u8]) -> Vec<u8> {
    let mut s = TcpStream::connect(address).unwrap();
    // Hang guard only: replies follow a journal fsync, which can take seconds on a loaded machine.
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s.write_all(bytes).unwrap();
    s.shutdown(Shutdown::Write).unwrap();
    let mut out = vec![];
    s.read_to_end(&mut out).unwrap();
    out
}
fn journal_count(temp: &Temp) -> usize {
    std::fs::read_to_string(temp.0.join("store.jsonl"))
        .unwrap()
        .lines()
        .count()
}
fn ranking_definition() -> Definition {
    let def = Definition::bundled()
        .unwrap()
        .into_iter()
        .find(|d| d.json()["boards"].get("main").is_some())
        .unwrap();
    // Fixtures follow the route grammar: five decoded name bytes, increasing
    // scores within eight decimal digits so each submitter has rank one.
    let fields = &def.json()["routes"][0]["request"];
    assert_eq!(fields["nm"]["bytes"], serde_json::json!([1, 5]));
    assert_eq!(fields["sc"]["regex"], "^[0-9]{1,8}$");
    def
}
fn ranking_request(def: &Definition, i: usize) -> Vec<u8> {
    let host = &def.hosts()[0];
    let route = def.json()["routes"][0]["match"]["path"].as_str().unwrap();
    format!(
        "GET {route}?nm=%26{i:04}&sc={} HTTP/1.1\r\nHost: {host}\r\n\r\n",
        i + 1
    )
    .into_bytes()
}
#[test]
fn network_tokens_refuse_at_cap_and_journal_stops_growing() {
    let temp = Temp::new();
    let (mut tcp, _) = start(&temp, Duration::from_secs(1));
    let host = &Services::bundled().unwrap().charge_hosts[0];
    let request = |i| {
        format!("POST /service/charge/nph-session HTTP/1.1\r\nHost: {host}\r\nX-SWAN-CMND-SID: {i}\r\n\r\n").into_bytes()
    };
    assert!(exchange(tcp.http_addr(), &request(0)).starts_with(b"HTTP/1.1 200"));
    assert_eq!(
        journal_count(&temp),
        1,
        "valid session fixture must be counted"
    );
    let mut issued = 1;
    for i in 1..300 {
        if exchange(tcp.http_addr(), &request(i)).starts_with(b"HTTP/1.1 200") {
            issued += 1;
        }
    }
    assert_eq!(
        issued, 256,
        "per-host token cap must refuse excess network sessions"
    );
    let before = std::fs::metadata(temp.0.join("store.jsonl")).unwrap().len();
    for i in 300..350 {
        assert!(!exchange(tcp.http_addr(), &request(i)).starts_with(b"HTTP/1.1 200"));
    }
    assert_eq!(
        std::fs::metadata(temp.0.join("store.jsonl")).unwrap().len(),
        before
    );
    tcp.shutdown().unwrap();
}
#[test]
fn real_ranking_listener_serves_large_board_under_body_limit() {
    let temp = Temp::new();
    let (mut tcp, _) = start(&temp, Duration::from_secs(1));
    let def = ranking_definition();
    let host = &def.hosts()[0];
    assert!(exchange(tcp.http_addr(), &ranking_request(&def, 0)).starts_with(b"HTTP/1.1 200"));
    assert_eq!(
        journal_count(&temp),
        1,
        "valid ranking fixture must be counted"
    );
    for i in 1..1500 {
        assert!(
            exchange(tcp.http_addr(), &ranking_request(&def, i)).starts_with(b"HTTP/1.1 200"),
            "valid increasing-score submission {i}"
        );
    }
    assert_eq!(journal_count(&temp), 1500);
    for path in ["/wg/", "/wg/rankings"] {
        let response = exchange(
            tcp.http_addr(),
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes(),
        );
        assert!(
            response.starts_with(b"HTTP/1.1 200"),
            "large board failed ranking page"
        );
        assert!(split_response(&response).2.len() <= 65536);
    }
    tcp.shutdown().unwrap();
}
#[test]
fn network_submissions_refuse_at_cap_and_journal_stops_growing() {
    let temp = Temp::new();
    let (mut tcp, _) = start(&temp, Duration::from_secs(1));
    let def = ranking_definition();
    assert!(exchange(tcp.http_addr(), &ranking_request(&def, 0)).starts_with(b"HTTP/1.1 200"));
    assert_eq!(
        journal_count(&temp),
        1,
        "valid ranking fixture must be counted"
    );
    let mut accepted = 1;
    for i in 1..2100 {
        if exchange(tcp.http_addr(), &ranking_request(&def, i)).starts_with(b"HTTP/1.1 200") {
            accepted += 1;
        }
    }
    assert_eq!(
        accepted, 2048,
        "per-board history cap must refuse excess network input"
    );
    let before = std::fs::metadata(temp.0.join("store.jsonl")).unwrap().len();
    for i in 2100..2150 {
        assert!(exchange(tcp.http_addr(), &ranking_request(&def, i)).starts_with(b"HTTP/1.1 500"));
    }
    assert_eq!(
        std::fs::metadata(temp.0.join("store.jsonl")).unwrap().len(),
        before
    );
    tcp.shutdown().unwrap();
}
#[test]
fn bpl_connection_has_total_deadline_despite_active_peer() {
    let temp = Temp::new();
    let (mut tcp, _) = start(&temp, Duration::from_millis(100));
    let mut s = TcpStream::connect(tcp.bpl_addr()).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let packet = [
        1, 24, 0, 0, 24, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    let mut replies = 0;
    for _ in 0..50 {
        if s.write_all(&packet).is_err() {
            break;
        }
        let mut reply = [0; 24];
        if s.read_exact(&mut reply).is_err() {
            break;
        }
        replies += 1;
        if replies == 1 {
            assert_eq!(
                &reply[..8],
                &[1, 24, 16, 0, 24, 0, 0, 0],
                "valid BPL fixture must be accepted"
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        replies <= 32,
        "active BPL peer extended connection indefinitely: {replies} replies"
    );
    tcp.shutdown().unwrap();
    assert_eq!(journal_count(&temp), 0);
}
#[test]
fn wire_log_has_cumulative_quota_across_connections() {
    let temp = Temp::new();
    let (mut tcp, wire) = start(&temp, Duration::from_millis(100));
    let packet = [
        1, 24, 0, 0, 24, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    for _ in 0..300 {
        assert_eq!(exchange(tcp.bpl_addr(), &packet).len(), 24);
    }
    tcp.shutdown().unwrap();
    let files = std::fs::read_dir(wire.directory()).unwrap().count();
    assert!(
        files <= 513,
        "wire files exceeded 256 transaction cap: {files}"
    );
}

#[test]
fn repeated_identity_is_refused_at_64_and_journal_does_not_grow() {
    let temp = Temp::new();
    let (mut tcp, _) = start(&temp, Duration::from_secs(1));
    let definition = ranking_definition();
    let mut request = ranking_request(&definition, 0);
    assert!(exchange(tcp.http_addr(), &request).starts_with(b"HTTP/1.1 200"));
    assert_eq!(journal_count(&temp), 1);
    let mut accepted = 1;
    for score in 2..=80 {
        request = format!(
            "GET {}?nm=SAME&sc={score} HTTP/1.1\r\nHost: {}\r\n\r\n",
            definition.json()["routes"][0]["match"]["path"]
                .as_str()
                .unwrap(),
            definition.hosts()[0]
        )
        .into_bytes();
        if exchange(tcp.http_addr(), &request).starts_with(b"HTTP/1.1 200") {
            accepted += 1;
        }
    }
    // First fixture uses a different identity; SAME has its own allowance.
    assert_eq!(accepted, 65);
    let before = std::fs::metadata(temp.0.join("store.jsonl")).unwrap().len();
    assert!(!exchange(tcp.http_addr(), &request).starts_with(b"HTTP/1.1 200"));
    assert_eq!(
        std::fs::metadata(temp.0.join("store.jsonl")).unwrap().len(),
        before
    );
    tcp.shutdown().unwrap();
}

#[test]
fn wire_byte_quota_refuses_logging_while_http_serving_continues() {
    let temp = Temp::new();
    std::fs::create_dir_all(temp.0.join("wire")).unwrap();
    let seed = std::fs::File::create(temp.0.join("wire/seed.request.bin")).unwrap();
    seed.set_len(16 * 1024 * 1024 - 256).unwrap();
    drop(seed);
    let (mut tcp, _) = start(&temp, Duration::from_secs(1));
    let host = &Services::bundled().unwrap().charge_hosts[0];
    let request=format!("POST /service/charge/nph-session HTTP/1.1\r\nHost: {host}\r\nX-SWAN-CMND-SID: test\r\n\r\n");
    assert!(exchange(tcp.http_addr(), request.as_bytes()).starts_with(b"HTTP/1.1 200"));
    assert!(exchange(tcp.http_addr(), request.as_bytes()).starts_with(b"HTTP/1.1 200"));
    assert_eq!(
        std::fs::read_dir(temp.0.join("wire")).unwrap().count(),
        2,
        "byte quota must leave only seed and journal"
    );
    tcp.shutdown().unwrap();
}

#[test]
fn slow_peers_are_limited_across_http_and_bpl_listeners() {
    let temp = Temp::new();
    let (mut tcp, _) = start(&temp, Duration::from_secs(2));
    let mut peers = Vec::new();
    for index in 0usize..8 {
        let peer = TcpStream::connect(if index.is_multiple_of(2) {
            tcp.http_addr()
        } else {
            tcp.bpl_addr()
        })
        .unwrap();
        peer.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        peers.push(peer);
    }
    std::thread::sleep(Duration::from_millis(100));
    let mut alive = 0;
    for peer in &mut peers {
        let mut byte = [0];
        match peer.read(&mut byte) {
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                alive += 1
            }
            Ok(0) | Err(_) => {}
            other => panic!("idle peer unexpectedly got bytes: {other:?}"),
        }
    }
    for peer in &peers {
        let _ = peer.shutdown(Shutdown::Both);
    }
    tcp.shutdown().unwrap();
    assert_eq!(alive, 4, "per-peer cap must apply across both protocols");
}
