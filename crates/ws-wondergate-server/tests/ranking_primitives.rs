// SPDX-License-Identifier: MIT OR Apache-2.0
mod common;
use common::{split_response, stamp};
use serde_json::{json, Value};
use ws_wondergate_server::{
    store::{JournalStore, MemoryStore, Scope, Store},
    Definition, FixedClock, SeededRng, Server,
};
fn program() -> Value {
    json!({
 "schema":"swankit-wondergate-title/1","id":"synthetic-table","provenance":{"server":"new","notes":"Synthetic delimited ranking grammar"},"hosts":["synthetic.test"],
 "boards":{"first":{"identity":"*","order":"desc","ties":"first-best-submission","course":"1"},"second":{"identity":"*","order":"asc","ties":"first-best-submission","course":"2"}},
 "records":{"row":[{"field":"rank","fmt":"dec"},{"text":"|"},{"field":"score","fmt":"dec","divide":10},{"text":"|"},{"field":"name","fmt":"raw","width":4,"empty_bytes":[32]},{"text":"|"},{"field":"extra","fmt":"dec"},{"text":"\r\n"}]},
 "routes":[{"id":"submit","method":"POST","match":{"path":"/submit"},"source":"form","form_trim_end":[0,13,10],
 "request":{"*":{"bytes":[0,8],"alphabet":"0123456789ABCDEF"},"owner":{"decode":"percent-plus","min_byte":32,"deny":[124,127],"trim_end":32,"last_bytes":[32,65]},"mode":{"default":[49],"choices":[[49],[50]]},"list":{"optional":true,"equals":[89]}},
 "tables":{"score":{"delimiter":124,"optional":true,"rows":{"1":[{"as":"u64","regex":"^[0-9]{3}$","prefix":"0","suffix":"0"},{"as":"u64","max_number":127}],"2":[{"as":"u64","max_number":999}]}}},
 "table_actions":[{"field":"score","identity":"*","name":"owner","score":0,"extra":1,"boards":{"1":"first"}},{"field":"score","identity":"*","name":"owner","score":0,"boards":{"2":"second"}}],
 "respond":{"status":200,"body":[{"date":14},{"text":"|"},{"board":"first","metric":"count","fmt":"dec"},{"text":"|"},{"board":"first","metric":"position","identity":"*","fmt":"dec","max":1,"overflow_zero":true},{"text":"|"},{"board":"first","metric":"length","limit":3,"pad_to":3,"fmt":"dec"},{"text":"\r\n"},{"when":{"equals":{"mode":[49]},"absent":["list"]},"segments":[{"text":"default\r\n"}]},{"each":{"leaders":{"board":"first","limit":3,"pad_to":3}},"record":"row"}]}}],"limits":{"max_body":2048,"max_response":2200}})
}
fn load(d: &Value) -> Definition {
    Definition::from_json(&d.to_string()).unwrap()
}
fn server(d: &Value, store: Box<dyn Store>) -> Server {
    Server::from_definitions(
        vec![load(d)],
        store,
        Box::new(FixedClock(stamp())),
        Box::new(SeededRng::new(1)),
    )
    .unwrap()
}
fn post(body: &[u8]) -> Vec<u8> {
    let mut r=format!("POST /submit HTTP/1.1\r\nHost: synthetic.test\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n",body.len()).into_bytes();
    r.extend(body);
    r
}
#[test]
fn tables_dates_padding_optional_fields_and_conditions() {
    let app = server(&program(), Box::new(MemoryStore::new()));
    let r = app.dispatch_http(&post(
        b"*=ABCDEF&owner=NEWA+++&score=1|070|127&score=2|321\0\r\n",
    ));
    assert_eq!(
        split_response(&r).2,
        b"20261005200403|1|1|3\r\ndefault\r\n1|7|NEWA|127\r\n0|0|    |0\r\n0|0|    |0\r\n"
    );
    let r = app.dispatch_http(&post(b"*=&owner=A&score=1|090|1&mode=2&list=Y"));
    assert_eq!(
        split_response(&r).2,
        b"20261005200403|2|1|3\r\n1|9|A   |1\r\n2|7|NEWA|127\r\n0|0|    |0\r\n"
    );
    let r = app.dispatch_http(&post(b"*=ABCDEF&owner=A&list=Y"));
    assert!(split_response(&r)
        .2
        .starts_with(b"20261005200403|2|0|3\r\n"));
}
#[test]
fn malformed_rows_validate_before_all_actions_and_journal_survives_restart() {
    let temp = common::Temp::new();
    let path = temp.0.join("journal.jsonl");
    let app = server(&program(), Box::new(JournalStore::open(&path).unwrap()));
    for body in [
        b"*=A&owner=A&score=1|070|1&score=2|bad".as_slice(),
        b"*=A&owner=A&score=1|070|1&score=1|080|2",
        b"*=A&owner=A&score=1|170|1",
        b"*=A&owner=A&score=1|071|1",
        b"*=A&owner=A&score=1|070|128",
        b"*=A&owner=B&score=1|070|1",
        b"*=G&owner=A&score=1|070|1",
        b"*=A&owner=A&score=3|1",
        b"*=A&owner=A&score=2|1000",
        b"*=A&owner=A&score=2|%QQ",
        b"*=A&owner=A&mode=3",
        b"*=A&owner=A&owner=A",
        b"*=A&score=2|1",
    ] {
        assert!(
            app.dispatch_http(&post(body)).starts_with(b"HTTP/1.1 400"),
            "{body:?}"
        );
    }
    drop(app);
    assert_eq!(
        JournalStore::open(&path)
            .unwrap()
            .submissions(&Scope::new("synthetic-table", "first", "1"))
            .unwrap()
            .len(),
        0
    );
    let app = server(&program(), Box::new(JournalStore::open(&path).unwrap()));
    assert!(app
        .dispatch_http(&post(b"*=A&owner=A&score=1|070|1&score=2|321"))
        .starts_with(b"HTTP/1.1 200"));
    drop(app);
    let store = JournalStore::open(path).unwrap();
    assert_eq!(
        store
            .submissions(&Scope::new("synthetic-table", "second", "2"))
            .unwrap()[0]
            .submission
            .score,
        321
    );
}
#[test]
fn shared_hosts_dispatch_disjoint_paths_and_reject_collisions() {
    let first = program();
    let mut second = program();
    second["id"] = json!("synthetic-other");
    second["routes"][0]["match"]["path"] = json!("/other");
    let app = Server::from_definitions(
        vec![load(&first), load(&second)],
        Box::new(MemoryStore::new()),
        Box::new(FixedClock(stamp())),
        Box::new(SeededRng::new(1)),
    )
    .unwrap();
    let request = String::from_utf8(post(b"*=A&owner=A"))
        .unwrap()
        .replace("/submit", "/other");
    assert!(app
        .dispatch_http(request.as_bytes())
        .starts_with(b"HTTP/1.1 200"));
    second["routes"][0]["match"]["path"] = json!("/submit");
    assert!(Server::from_definitions(
        vec![load(&first), load(&second)],
        Box::new(MemoryStore::new()),
        Box::new(FixedClock(stamp())),
        Box::new(SeededRng::new(1))
    )
    .is_err());
    second["routes"][0]["method"] = json!("GET");
    second["routes"][0]["source"] = json!("query");
    assert!(Server::from_definitions(
        vec![load(&first), load(&second)],
        Box::new(MemoryStore::new()),
        Box::new(FixedClock(stamp())),
        Box::new(SeededRng::new(1)),
    )
    .is_err());
}
#[test]
fn new_grammar_references_and_bounds_are_checked_at_load() {
    for (pointer, value) in [
        ("/routes/0/table_actions/0/extra", json!(7)),
        ("/routes/0/table_actions/0/boards/1", json!("unknown")),
        ("/routes/0/tables/score/rows/1", json!([])),
        ("/records/row/2/divide", json!(0)),
        ("/routes/0/respond/body/0/date", json!(10)),
        ("/routes/0/respond/body/2/board", json!("unknown")),
        ("/routes/0/respond/body/4/identity", json!("unknown")),
        ("/routes/0/request/mode/choices", json!([])),
    ] {
        let mut d = program();
        *d.pointer_mut(pointer).unwrap() = value;
        assert!(Definition::from_json(&d.to_string()).is_err(), "{pointer}");
    }
    let mut d = program();
    d["routes"][0]["request"]["owner"]["optional"] = json!(true);
    assert!(Definition::from_json(&d.to_string()).is_err());
}

#[test]
fn decimal_table_tags_normalize_before_duplicate_detection() {
    let mut d = program();
    d["routes"][0]["tables"]["score"]["tag_decimal_width"] = json!([1, 2]);
    let app = server(&d, Box::new(MemoryStore::new()));
    assert!(app
        .dispatch_http(&post(b"*=A&owner=A&score=01|070|1"))
        .starts_with(b"HTTP/1.1 200"));
    for body in [
        b"*=A&owner=A&score=01|070|1&score=1|080|1".as_slice(),
        b"*=A&owner=A&score=001|070|1",
        b"*=A&owner=A&score=+1|070|1",
    ] {
        assert!(app.dispatch_http(&post(body)).starts_with(b"HTTP/1.1 400"));
    }
    d["routes"][0]["tables"]["score"]["tag_decimal_width"] = json!([0, 2]);
    assert!(Definition::from_json(&d.to_string()).is_err());
    d["routes"][0]["tables"]["score"]["tag_decimal_width"] = json!([1, 2]);
    d["routes"][0]["tables"]["score"]["rows"]["01"] = json!([{"as":"u64"}]);
    assert!(Definition::from_json(&d.to_string()).is_err());
}
