// SPDX-License-Identifier: MIT
// Ported shared/framing cases from the private server tests/test_services.py.
// Copyright (C) 2026 jidoughost. BPL expected bytes corrected by PHASE0.
mod common;
use common::*;
use ws_wondergate_server::{
    http,
    services::{self, Services},
    store::JournalStore,
    FixedClock, Limits, SeededRng, Server,
};

#[test]
fn bpl_exact_allow_and_length() {
    let packet = packet();
    let reply = services::bpl::reply(&packet, &Limits::default()).unwrap();
    assert_eq!(reply.len(), 24);
    assert_eq!(&reply[..8], &[1, 24, 16, 0, 24, 0, 0, 0]);
    assert_eq!(&reply[8..], &packet[8..24]);
    assert!(services::bpl::reply(&packet[..44], &Limits::default()).is_err());
    assert!(services::bpl::reply(
        &[1, 8, 0, 0, 12, 0, 0, 0, 85, 82, 76, 0],
        &Limits::default()
    )
    .is_err());
}
#[test]
fn shared_exact_r_lineage_requests() {
    let app = server();
    for action in [
        "nph-srvccount",
        "nph-srvcinfo",
        "nph-session",
        "nph-gettime",
    ] {
        let extra = if action == "nph-session" {
            format!("X-swan-cmnd-sid: {}\r\n", "0".repeat(88)).into_bytes()
        } else if action != "nph-gettime" {
            b"X-swan-srvc-id: BAN00A01\r\n".to_vec()
        } else {
            vec![]
        };
        let reply = app.dispatch_http(&post(action, 0, true, &extra));
        assert!(reply.starts_with(b"HTTP/1.1 200 OK"));
        assert!(split_response(&reply).2.is_empty());
    }
}
#[test]
fn time_current_jst_digits_and_host_quirk() {
    let app = server();
    let reply = app.dispatch_http(&post("nph-gettime", 1, false, &[]));
    let fields = split_response(&reply).1;
    assert_eq!(fields["X-SWAN-TIME-NOW"], "20261006050403");
    assert_eq!(fields["X-SWAN-TIME-RET"], "0");
    let request = http::parse(&post("nph-gettime", 1, true, &[]), &Limits::default()).unwrap();
    assert_eq!(request.host, Services::bundled().unwrap().charge_hosts[1]);
}
#[test]
fn all_service_ids_available_and_zero_cost() {
    let app = server();
    for service in ["BAN00A01", "BAN00C01", "NEW00001"] {
        let reply = app.dispatch_http(&post(
            "nph-srvcinfo",
            0,
            true,
            format!("X-swan-srvc-id: {service}\r\n").as_bytes(),
        ));
        let fields = split_response(&reply).1;
        assert_eq!(fields["X-SWAN-SRVC-RET"], "0");
        for suffix in ["PRC", "WAY", "TIM", "TRM", "LCHRG"] {
            assert_eq!(fields[&format!("X-SWAN-SRVC-{suffix}")], "0");
        }
        assert!(fields["X-SWAN-SRVC-OPEN"] < fields["X-SWAN-SRVC-CLOSE"]);
    }
}
#[test]
fn session_aid_retry_persistence_and_billing() {
    let temp = Temp::new();
    let path = temp.0.join("journal.jsonl");
    let make = || {
        Server::bundled(
            Box::new(JournalStore::open(&path).unwrap()),
            Box::new(FixedClock(stamp())),
            Box::new(SeededRng::new(123)),
        )
        .unwrap()
    };
    let app = make();
    let req = post(
        "nph-session",
        0,
        true,
        format!("X-swan-cmnd-sid: {}\r\n", "a".repeat(88)).as_bytes(),
    );
    let reply = app.dispatch_http(&req);
    let first = split_response(&reply).1;
    assert_eq!(first["X-SWAN-CMND-AID"].len(), 80);
    assert_eq!(reply, app.dispatch_http(&req));
    drop(app);
    let restored = make();
    assert_eq!(reply, restored.dispatch_http(&req));
    let host = &restored.services().charge_hosts[2];
    let req = format!("POST /cgi-bin/kakin.cgi HTTP/1.1\r\nHost: {host}\r\nUser-Agent: SWAN/1.0\r\nX-swan-cnts-aid: {}\r\n\r\n",first["X-SWAN-CMND-AID"]);
    let response = restored.dispatch_http(req.as_bytes());
    let fields = split_response(&response).1;
    assert_eq!(fields["X-SWAN-CNTS-RET"], "0");
    assert_eq!(fields["X-SWAN-CNTS-AID"], first["X-SWAN-CMND-AID"]);
    let lines = std::fs::read_to_string(path).unwrap();
    assert_eq!(lines.lines().count(), 2);
    for line in lines.lines() {
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(line).unwrap()["provenance"],
            "new"
        );
    }
}
#[test]
fn host_routing_and_framing() {
    let app = server();
    let req = post("nph-gettime", 0, false, &[]);
    let unknown = String::from_utf8(req.clone())
        .unwrap()
        .replace(&app.services().charge_hosts[0], "unknown");
    assert!(app
        .dispatch_http(unknown.as_bytes())
        .starts_with(b"HTTP/1.1 404"));
    for raw in [
        b"bad".to_vec(),
        [req, b"data".to_vec()].concat(),
        post("nph-gettime", 0, true, b"Host: duplicate\r\n"),
        post("nph-gettime", 0, true, b"Transfer-Encoding: chunked\r\n"),
    ] {
        assert!(app.dispatch_http(&raw).starts_with(b"HTTP/1.1 400"));
    }
}
