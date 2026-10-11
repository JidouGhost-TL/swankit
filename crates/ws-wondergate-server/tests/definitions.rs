// SPDX-License-Identifier: MIT OR Apache-2.0
mod common;
use common::*;
use serde_json::{json, Value};
use ws_wondergate_server::{
    engine::sum16_envelope,
    form, schema,
    store::{JournalStore, MemoryStore, Scope, Store},
    Definition, FixedClock, SeededRng, Server,
};

fn program() -> Value {
    json!({
        "schema":"swankit-wondergate-title/1", "id":"synthetic",
        "provenance":{"server":"new","notes":"Synthetic definition"},
        "hosts":["synthetic.test"],
        "boards":{"main":{"identity":"nm","order":"desc","ties":"first-submission","course":"default"}},
        "records":{"row":[{"field":"rank","fmt":"dec"},{"text":":"},{"field":"name","fmt":"raw"},{"text":":"},{"field":"score","fmt":"dec"}]},
        "routes":[{"id":"submit","method":"GET","match":{"path":"/submit","query_keys":["nm","sc"]},
            "request":{"nm":{"decode":"percent-plus","bytes":[1,5]},"sc":{"decode":"percent-plus","regex":"^[0-9]{1,20}$","as":"u64"}},
            "actions":[{"record":{"board":"main","identity":"nm","name":"nm","score":"sc"}}],
            "respond":{"status":200,"body":[{"row":{"submitter":"main"},"record":"row"}]}}],
        "limits":{"max_body":1024,"max_response":2048}
    })
}
fn set(value: &mut Value, pointer: &str, replacement: Value) {
    if let Some(slot) = value.pointer_mut(pointer) {
        *slot = replacement;
    } else {
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        value.pointer_mut(parent).unwrap()[key] = replacement;
    }
}
fn load(value: &Value) -> Definition {
    Definition::from_json(&value.to_string()).unwrap()
}
fn app(value: &Value, store: Box<dyn Store>) -> Server {
    Server::from_definitions(
        vec![load(value)],
        store,
        Box::new(FixedClock(stamp())),
        Box::new(SeededRng::new(1)),
    )
    .unwrap()
}
fn request(method: &str, path: &str, body: &[u8], content_type: &str) -> Vec<u8> {
    [format!("{method} {path} HTTP/1.1\r\nHost: synthetic.test\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\n\r\n",body.len()).into_bytes(),body.to_vec()].concat()
}
fn dispatch(app: &Server, query: &str) -> Vec<u8> {
    app.dispatch_http(&request(
        "GET",
        &format!("/submit?{query}"),
        &[],
        "text/plain",
    ))
}
#[test]
fn submitted_condition_uses_only_the_current_request() {
    let mut value = program();
    value["boards"]["other"] = value["boards"]["main"].clone();
    value["routes"][0]["respond"]["body"] = json!([
        {"when":{"submitted":["main"]},"segments":[{"text":"current"}]},
        {"when":{"submitted":["other"]},"segments":[{"text":"wrong"}]}
    ]);
    let server = app(&value, Box::new(MemoryStore::new()));
    assert_eq!(
        split_response(&dispatch(&server, "nm=A&sc=4")).2,
        b"current"
    );
    value["routes"][0]["actions"] = json!([]);
    let server = app(&value, Box::new(MemoryStore::new()));
    assert!(split_response(&dispatch(&server, "nm=A&sc=4")).2.is_empty());
    value["routes"][0]["respond"]["body"][0]["when"]["submitted"] = json!(["missing"]);
    assert!(Definition::from_json(&value.to_string()).is_err());
}
#[test]
fn explicit_keep_alive_policy_respects_client_close() {
    let mut value = program();
    value["routes"][0]["respond"]["keep_alive"] = json!(true);
    let server = app(&value, Box::new(MemoryStore::new()));
    let reply = dispatch(&server, "nm=A&sc=4");
    assert!(reply
        .windows(24)
        .any(|s| s == b"Connection: keep-alive\r\n"));
    let raw = request("GET", "/submit?nm=A&sc=4", &[], "text/plain");
    let raw = String::from_utf8(raw).unwrap().replace(
        "Host: synthetic.test\r\n",
        "Host: synthetic.test\r\nConnection: close\r\n",
    );
    let reply = server.dispatch_http(raw.as_bytes());
    assert!(reply.windows(19).any(|s| s == b"Connection: close\r\n"));
}
#[test]
fn schema_is_closed_and_bundled_definitions_roundtrip() {
    for definition in Definition::bundled().unwrap() {
        schema::validate(definition.json()).unwrap();
        assert_eq!(
            load(&serde_json::from_str::<Value>(&definition.to_json().unwrap()).unwrap()).json(),
            definition.json()
        );
    }
    let original = program();
    for (pointer, value) in [
        ("/schema", json!("unknown")),
        ("/provenance/server", json!("original")),
        ("/hosts", json!([])),
        ("/routes/0/method", json!("PUT")),
        ("/limits/max_body", json!(65537)),
        ("/limits/max_body", json!(1.5)),
        ("/routes/0/respond/body", json!([{"zero":-1}])),
        ("/routes/0/respond/body", json!([{"zero":1,"text":"x"}])),
        (
            "/routes/0/respond/body",
            json!([{"plugin":"unknown","params":{}}]),
        ),
        ("/routes/0/request/nm/repeats", json!("all")),
        ("/routes/0/request/nm/deny", json!([0, 0])),
    ] {
        let mut bad = original.clone();
        set(&mut bad, pointer, value);
        assert!(schema::validate(&bad).is_err(), "{pointer}");
        assert!(Definition::from_json(&bad.to_string()).is_err());
    }
    let mut unknown = original;
    unknown["routes"][0]["extra"] = json!(true);
    assert!(schema::validate(&unknown).is_err());
    assert!(Definition::from_json(&" ".repeat(262145)).is_err());
    assert!(Definition::from_json("{").is_err());
}
#[test]
fn semantic_validation_rejects_ambiguous_or_unexecutable_programs() {
    for (pointer, value) in [
        ("/id", json!("bad/id")),
        ("/hosts", json!(["SYNTHETIC.test"])),
        ("/provenance/notes", json!(" ")),
        ("/routes/0/request/nm/bytes", json!([5, 1])),
        ("/routes/0/request/sc/regex", json!(".*")),
        ("/routes/0/request/sc/as", Value::Null),
        ("/routes/0/match/query_keys", json!(["other"])),
        ("/routes/0/actions/0/record/board", json!("absent")),
        ("/routes/0/actions/0/record/identity", json!("sc")),
        ("/routes/0/actions", json!([])),
        ("/routes/0/respond/expect_lines", json!(7)),
        ("/routes/0/respond/headers", json!([["Connection", "open"]])),
        ("/routes/0/respond/headers", json!([["X", "1"], ["x", "2"]])),
        (
            "/routes/0/respond/headers",
            json!([["X", "ok\r\nBad: yes"]]),
        ),
        ("/records/row/0/field", json!("unknown")),
        ("/records/row/0/fmt", json!("raw")),
        ("/records/row/2/width", json!(513)),
        (
            "/routes/0/respond/body",
            json!([{"row":{"place":{"board":"missing","n":1}},"record":"row"}]),
        ),
        (
            "/routes/0/respond/body",
            json!([{"value":"missing.mode","as":"u8"}]),
        ),
        ("/routes/0/respond/body", json!([{"zero":800},{"zero":800}])),
        ("/limits/max_response", json!(1)),
        ("/routes/0/source", json!("form")),
    ] {
        let mut bad = program();
        set(&mut bad, pointer, value);
        assert!(
            Definition::from_json(&bad.to_string()).is_err(),
            "{pointer}"
        );
    }
    let mut bad = program();
    let route = bad["routes"][0].clone();
    bad["routes"].as_array_mut().unwrap().push(route);
    assert!(Definition::from_json(&bad.to_string()).is_err());
    bad["routes"][1]["id"] = json!("overlap");
    assert!(Definition::from_json(&bad.to_string()).is_err());
    bad["routes"][1]["match"] = json!({"path":"/submit","query_exact":"sc=1&nm=a"});
    assert!(Definition::from_json(&bad.to_string()).is_err());
    let d = load(&program());
    assert!(Server::from_definitions(
        vec![d.clone(), d],
        Box::new(MemoryStore::new()),
        Box::new(FixedClock(stamp())),
        Box::new(SeededRng::new(1))
    )
    .is_err());
}
#[test]
fn primitives_decimal_percent_shift_jis_and_checksum_bounds() {
    assert_eq!(form::percent_plus(b"a+%82%A0%2b").unwrap(), b"a \x82\xa0+");
    for bytes in [b"%".as_slice(), b"%0", b"%ZZ"] {
        assert!(form::percent_plus(bytes).is_err());
    }
    assert_eq!(
        form::number(b"18446744073709551615", 1, 20, u64::MAX).unwrap(),
        u64::MAX
    );
    assert!(form::number(b"18446744073709551616", 1, 20, u64::MAX).is_err());
    assert!(form::number(b"+1", 1, 20, u64::MAX).is_err());
    assert!(form::fields(b"x=1&").is_err());
    assert!(form::fields(b"%78=1").is_err());
    assert!(form::fields(&vec![0; 65537]).is_err());
    assert_eq!(form::shift_jis(&[0x82, 0xa0], false).unwrap(), "あ");
    assert_eq!(
        form::shift_jis(&[0x81, 0x60, 0x80, 0x87, 0x40], true).unwrap(),
        "〜��@"
    );
    assert!(form::shift_jis(&[0x87, 0x40], false).is_err());
    let payload = vec![255; 300];
    let wrapped = sum16_envelope(&payload, "@", true, &[0], 311).unwrap();
    assert_eq!(&wrapped[..5], b"@2AD4");
    assert_eq!(&wrapped[305..], b"@2AD4\0");
    assert!(sum16_envelope(&payload, "@", false, &[], 309).is_err());
    assert!(sum16_envelope(&[], "", false, &[], 100).is_err());
}
#[test]
fn forms_repeats_charsets_and_invalid_fields_never_store() {
    let mut d = program();
    d["routes"][0]["method"] = json!("POST");
    d["routes"][0]["source"] = json!("form");
    d["routes"][0]["request"]["nm"]["charset"] = json!("shift-jis");
    d["routes"][0]["request"]["nm"]["repeats"] = json!("last");
    let temp = Temp::new();
    let path = temp.0.join("journal.jsonl");
    let app = app(&d, Box::new(JournalStore::open(&path).unwrap()));
    let raw = request(
        "POST",
        "/submit",
        b"nm=old&nm=%82%A0&sc=5",
        "application/x-www-form-urlencoded; charset=shift_jis",
    );
    assert_eq!(split_response(&app.dispatch_http(&raw)).2, b"1:\x82\xa0:5");
    for (body, content_type) in [
        (
            b"nm=%ZZ&nm=ok&sc=1".as_slice(),
            "application/x-www-form-urlencoded",
        ),
        (b"nm=%80&sc=1", "application/x-www-form-urlencoded"),
        (b"nm=ok&sc=1&extra=2", "application/x-www-form-urlencoded"),
        (b"nm=ok&sc=1", "text/plain"),
        (
            b"nm=ok&sc=18446744073709551616",
            "application/x-www-form-urlencoded",
        ),
    ] {
        assert!(app
            .dispatch_http(&request("POST", "/submit", body, content_type))
            .starts_with(b"HTTP/1.1 400"));
    }
    drop(app);
    assert_eq!(
        JournalStore::open(&path)
            .unwrap()
            .submissions(&Scope::new("synthetic", "main", "default"))
            .unwrap()
            .len(),
        1
    );
    d["routes"][0]["request"]["nm"]["repeats"] = json!("first");
    let app = app_for(&d);
    assert_eq!(split_response(&app.dispatch_http(&raw)).2, b"1:old:5");
}
fn app_for(d: &Value) -> Server {
    app(d, Box::new(MemoryStore::new()))
}
#[test]
fn increments_formats_padding_and_runtime_response_limits() {
    let mut d = program();
    let record = d["routes"][0]["actions"][0]["record"].clone();
    d["routes"][0]["actions"][0] = json!({"increment":record});
    d["records"]["row"][4] =
        json!({"field":"score","fmt":"hex-upper","width":4,"zero_pad":true,"min":1,"max":255});
    let app = app_for(&d);
    assert_eq!(split_response(&dispatch(&app, "nm=a&sc=1")).2, b"1:a:0001");
    assert_eq!(
        split_response(&dispatch(&app, "nm=a&sc=254")).2,
        b"1:a:00FF"
    );
    assert_eq!(split_response(&dispatch(&app, "nm=a&sc=1")).2, b"1:a:00FF");
    assert!(dispatch(&app, "nm=a&sc=18446744073709551615").starts_with(b"HTTP/1.1 400"));
    let mut d = program();
    d["records"]["row"] = json!([{"field":"score","fmt":"hex-lower","width":4,"zero_pad":true}]);
    d["routes"][0]["respond"]["body"] =
        json!([{"each":{"leaders":{"board":"main","limit":1,"pad_to":2}},"record":"row"}]);
    assert_eq!(
        split_response(&dispatch(&app_for(&d), "nm=a&sc=15")).2,
        b"000f0000"
    );
    let mut d = program();
    d["records"]["row"] = json!([{"field":"score","fmt":"u8"}]);
    assert_eq!(
        split_response(&dispatch(&app_for(&d), "nm=a&sc=255")).2,
        &[255]
    );
    assert!(dispatch(&app_for(&d), "nm=a&sc=256").starts_with(b"HTTP/1.1 500"));
    let mut d = program();
    d["limits"]["max_body"] = json!(4);
    assert!(dispatch(&app_for(&d), "nm=abcde&sc=99").starts_with(b"HTTP/1.1 500"));
}
#[test]
fn html_escapes_raw_names_and_method_policy_precedes_query() {
    let mut d = program();
    d["records"]["row"] = json!([{"field":"name","fmt":"html-shift-jis"}]);
    let app = app_for(&d);
    assert_eq!(
        split_response(&dispatch(&app, "nm=%26%3C%3E%22%27&sc=1")).2,
        b"&amp;&lt;&gt;&quot;&#x27;"
    );
    assert_eq!(
        split_response(&dispatch(&app, "nm=%81%60%87%40&sc=1")).2,
        "〜�@".as_bytes()
    );
    d["routes"][0]["errors"] = json!({"method":405,"method_first":true});
    assert!(app_for(&d)
        .dispatch_http(&request("POST", "/submit?bad=1", &[], "text/plain"))
        .starts_with(b"HTTP/1.1 405"));
    d["errors"] = json!({"method":405,"method_first":true});
    assert!(app_for(&d)
        .dispatch_http(&request("POST", "/unknown", &[], "text/plain"))
        .starts_with(b"HTTP/1.1 405"));
}

#[test]
fn clock_rollover_uses_one_timestamp_for_the_entire_transaction() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use ws_wondergate_server::{Clock, Timestamp};
    struct CrossingClock(AtomicUsize);
    impl Clock for CrossingClock {
        fn now_utc(&self) -> Timestamp {
            let seconds: i64 = stamp().into();
            Timestamp::try_from(seconds + i64::from(self.0.fetch_add(1, Ordering::Relaxed) != 0))
                .unwrap()
        }
    }
    let mut d = program();
    let seconds: i64 = stamp().into();
    d["values"] = json!({"phase":{"primitive":"cyclic-phase","period_s":720,
        "phases":[1,2],"policy":"clock","epoch_s":seconds-719}});
    d["routes"][0]["respond"]["body"] = json!([
        {"value":"phase.mode","as":"u8"}, {"value":"phase.minute","as":"u8"},
        {"value":"phase.second","as":"u8"}, {"value":"phase.mode","as":"u8"},
        {"value":"phase.minute","as":"u8"}, {"value":"phase.second","as":"u8"}
    ]);
    let app = Server::from_definitions(
        vec![load(&d)],
        Box::new(MemoryStore::new()),
        Box::new(CrossingClock(AtomicUsize::new(0))),
        Box::new(SeededRng::new(1)),
    )
    .unwrap();
    assert_eq!(
        split_response(&dispatch(&app, "nm=a&sc=1")).2,
        &[1, 11, 59, 1, 11, 59]
    );
}

#[test]
fn explicit_head_preserves_rendered_length_without_body_or_actions() {
    let mut value = program();
    value["routes"][0]["method"] = json!("HEAD");
    value["routes"][0]["actions"] = json!([]);
    value["routes"][0]["respond"]["body"] = json!([{"text":"placeholder"}]);
    let server = app(&value, Box::new(MemoryStore::new()));
    let reply = server.dispatch_http(&request("HEAD", "/submit?nm=A&sc=4", &[], "text/plain"));
    let header = std::str::from_utf8(&reply).unwrap();
    assert!(header.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(header.contains("Content-Length: 11\r\n"));
    assert!(header.ends_with("\r\n\r\n"));
    assert!(!header.contains("placeholder"));
    value["routes"][0]["actions"] = program()["routes"][0]["actions"].clone();
    assert!(Definition::from_json(&value.to_string()).is_err());
}

#[test]
fn bare_get_requires_explicit_exact_static_route_and_unambiguous_host() {
    let mut value = program();
    value["routes"][0]["match"] = json!({"path":"/index"});
    value["routes"][0]["request"] = json!({});
    value["routes"][0]["actions"] = json!([]);
    value["routes"][0]["respond"]["body"] = json!([{"text":"placeholder"}]);
    let raw = b"GET /index\r\n\r\n";
    assert!(app(&value, Box::new(MemoryStore::new()))
        .try_dispatch_http(raw)
        .is_err());
    value["routes"][0]["bare_get"] = json!(true);
    let server = app(&value, Box::new(MemoryStore::new()));
    assert_eq!(server.try_dispatch_http(raw).unwrap(), b"placeholder");
    for bad in [
        b"GET /other\r\n\r\n".as_slice(),
        b"GET /index?x=1\r\n\r\n",
        b"GET /index\r\n\r\nGET /index\r\n\r\n",
        b"POST /index\r\n\r\n",
        b"GET /index\r\n\r\n\0",
    ] {
        assert!(server.try_dispatch_http(bad).is_err());
    }
    value["routes"][0]["method"] = json!("HEAD");
    assert!(Definition::from_json(&value.to_string()).is_err());
    value["routes"][0]["method"] = json!("GET");
    let first = load(&value);
    value["id"] = json!("another");
    value["hosts"] = json!(["another.test"]);
    let ambiguous = Server::from_definitions(
        vec![first, load(&value)],
        Box::new(MemoryStore::new()),
        Box::new(FixedClock(stamp())),
        Box::new(SeededRng::new(1)),
    )
    .unwrap();
    assert!(ambiguous.try_dispatch_http(raw).is_err());
}
