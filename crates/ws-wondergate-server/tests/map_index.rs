// SPDX-License-Identifier: MIT OR Apache-2.0
mod common;
use common::{server, split_response};

#[test]
fn bundled_map_index_has_bounded_names_and_valid_dates() {
    let app = server();
    let definition = app
        .title_definitions()
        .iter()
        .find(|d| d.json()["routes"][0]["id"] == "map-index")
        .expect("bundled map index");
    let path = definition.json()["routes"][0]["match"]["path"]
        .as_str()
        .unwrap();
    let host = &definition.hosts()[0];
    let request = |method: &str, target: &str| {
        app.dispatch_http(format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes())
    };
    let reply = request("GET", path);
    assert!(reply.starts_with(b"HTTP/1.1 200"));
    let body = std::str::from_utf8(split_response(&reply).2).unwrap();
    let rows: Vec<_> = body.split_terminator("\r\n").collect();
    assert!(!rows.is_empty() && rows.len() <= 50);
    for row in rows {
        let fields: Vec<_> = row.split(',').collect();
        assert_eq!(fields.len(), 4);
        assert!(!fields[0].is_empty() && fields[0].len() <= 12);
        assert!(fields[0].bytes().all(|b| b.is_ascii_alphanumeric()));
        for (field, bound) in fields[1..].iter().zip([9999, 12, 31]) {
            assert!((1..=bound).contains(&field.parse::<u32>().unwrap()));
        }
    }
    assert!(reply.len() <= 256);
    assert!(request("POST", path).starts_with(b"HTTP/1.1 405"));
    assert!(request("GET", &format!("{path}/missing")).starts_with(b"HTTP/1.1 404"));
}

#[test]
fn bundled_map_payload_matches_client_dimensions_and_tile_bounds() {
    let app = server();
    let definition = app
        .title_definitions()
        .iter()
        .find(|d| d.json()["routes"][0]["id"] == "map-index")
        .unwrap();
    let route = definition.json()["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "local-map")
        .expect("local map payload");
    let path = route["match"]["path"].as_str().unwrap();
    let reply = app.dispatch_http(
        format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\n\r\n",
            definition.hosts()[0]
        )
        .as_bytes(),
    );
    assert!(reply.starts_with(b"HTTP/1.1 200"));
    let body = split_response(&reply).2;
    assert_eq!(body.len(), 28 * 18);
    assert!(body.iter().all(|b| *b <= 0x1e));
    assert!(body[..28 * 17].iter().all(|b| *b == 0));
    assert_eq!(&body[28 * 17..], &[1; 28]);
    assert!(reply.len() <= 640);
}
