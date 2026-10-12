// SPDX-License-Identifier: MIT OR Apache-2.0
mod common;
use common::*;
use ws_wondergate_server::{Definition, Server};

fn rating_definition() -> Definition {
    Definition::bundled()
        .unwrap()
        .into_iter()
        .find(|d| {
            d.json()["routes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["id"] == "getrate")
        })
        .expect("bundled rating download route")
}
fn request(d: &Definition, route: &str, body: &str) -> Vec<u8> {
    let path = d.json()["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == route)
        .unwrap()["match"]["path"]
        .as_str()
        .unwrap();
    format!("POST {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}", d.hosts()[0], body.len()).into_bytes()
}
fn rating_server() -> (Definition, Server) {
    (rating_definition(), server())
}
#[test]
fn rating_download_has_bounded_authored_reply() {
    let (d, app) = rating_server();
    let reply = app.dispatch_http(&request(&d, "getrate", "c=new&d=demo"));
    assert!(reply.starts_with(b"HTTP/1.1 200"));
    let (_, fields, body) = split_response(&reply);
    assert_eq!(fields["Content-Type"], "text/plain");
    assert_eq!(body, b"c=1&e=150000&f=00001");
    for body in [
        "c=&d=demo",
        "c=new",
        "c=new&d=demo&extra=1",
        "c=new&c=new&d=demo",
        "c=%GG&d=demo",
        "c=123456789012&d=demo",
        "c=new&d=12345678901234",
    ] {
        assert!(
            app.dispatch_http(&request(&d, "getrate", body))
                .starts_with(b"HTTP/1.1 400"),
            "{body}"
        );
    }
}
#[test]
fn rating_upload_checks_decimal_width_and_word_bounds() {
    let (d, app) = rating_server();
    for body in [
        "c=new&d=demo&e=150000&f=1&g=0",
        "c=new&d=demo&e=65535255&f=65535&g=65535",
    ] {
        let reply = app.dispatch_http(&request(&d, "setrate", body));
        assert!(reply.starts_with(b"HTTP/1.1 200"));
        assert_eq!(split_response(&reply).2, b"c=1");
    }
    for body in [
        "c=new&d=demo",
        "c=new&d=demo&e=12&f=1&g=0",
        "c=new&d=demo&e=123456789&f=1&g=0",
        "c=new&d=demo&e=150000&f=65536&g=0",
        "c=new&d=demo&e=150000&f=1&g=-1",
    ] {
        assert!(
            app.dispatch_http(&request(&d, "setrate", body))
                .starts_with(b"HTTP/1.1 400"),
            "{body}"
        );
    }
}
