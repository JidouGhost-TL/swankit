// SPDX-License-Identifier: MIT OR Apache-2.0
mod common;
use common::{server, split_response};

fn request(method: &str, path: &str, host: &str) -> Vec<u8> {
    format!("{method} {path} HTTP/1.0\r\nHost: {host}\r\n\r\n").into_bytes()
}

#[test]
fn bundled_local_home_page_is_bounded_html_with_route_errors() {
    let server = server();
    let reply = server.dispatch_http(&request("GET", "/", "www.mopera.ne.jp"));
    assert!(reply.starts_with(b"HTTP/1.1 200"));
    let (_, headers, body) = split_response(&reply);
    assert_eq!(headers["Content-Type"], "text/html");
    assert!(reply.len() <= 256);
    let message = b"Local service is working.";
    assert!(body.windows(message.len()).any(|part| part == message));
    for (method, path, host, status) in [
        ("POST", "/", "www.mopera.ne.jp", 405),
        ("GET", "/missing", "www.mopera.ne.jp", 404),
        ("GET", "/", "unknown.test", 404),
    ] {
        let reply = server.dispatch_http(&request(method, path, host));
        assert!(reply.starts_with(format!("HTTP/1.1 {status}").as_bytes()));
        assert!(split_response(&reply).2.is_empty());
    }
}
