// SPDX-License-Identifier: MIT OR Apache-2.0
mod common;
use common::{server, split_response};

// This checks the HTTP record shape. Prepared pull and full-wire previews
// succeeded; a short-wire preview lost status-header bytes during a UART
// overrun. Transport timing and repeatability remain unresolved.
#[test]
fn bundled_supply_list_serves_traced_record_shape() {
    let app = server();
    let definition = app
        .title_definitions()
        .iter()
        .find(|d| d.json()["routes"][0]["id"] == "supply-list")
        .expect("bundled supply list");
    let path = definition.json()["routes"][0]["match"]["path"]
        .as_str()
        .unwrap();
    let host = &definition.hosts()[0];
    let request = |method: &str, target: &str| {
        app.dispatch_http(format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes())
    };
    let reply = request("GET", path);
    assert!(reply.starts_with(b"HTTP/1.1 200"));
    let body = split_response(&reply).2.to_vec();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.ends_with("\r\n"));
    let rows: Vec<_> = text.split_terminator("\r\n").collect();
    assert!(!rows.is_empty() && rows.len() <= 50);
    let mut last_section = b'0';
    for row in rows {
        let bytes = row.as_bytes();
        assert_eq!(bytes.len(), 8);
        assert!((b'1'..=b'4').contains(&bytes[0]));
        assert!(bytes[0] >= last_section);
        last_section = bytes[0];
        assert!(bytes[1..].iter().all(|b| b.is_ascii_digit()));
    }
    assert!(request("POST", path).starts_with(b"HTTP/1.1 405"));
    assert!(request("GET", &format!("{path}/missing")).starts_with(b"HTTP/1.1 404"));
}
