// SPDX-License-Identifier: MIT OR Apache-2.0
mod common;
use common::{server, split_response};

#[test]
fn bundled_class_ranking_selects_current_course_and_preserves_best() {
    let app = server();
    let definition = app
        .title_definitions()
        .iter()
        .find(|d| d.json()["routes"][0]["id"] == "class-ranking")
        .expect("bundled class ranking");
    let path = definition.json()["routes"][0]["match"]["path"]
        .as_str()
        .unwrap();
    let host = &definition.hosts()[0];
    let post = |body: &str| {
        let request = format!("POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        app.dispatch_http(request.as_bytes())
    };
    for course in 1..=3 {
        let mut request = format!("*=ABCDEF&owner=ALPHA&score={course}|001000|02");
        if course == 3 {
            let list: Vec<u8> = definition.json()["routes"][0]["request"]["list"]["equals"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| b.as_u64().unwrap() as u8)
                .collect();
            request.push_str("&list=");
            request.push_str(std::str::from_utf8(&list).unwrap());
            request.push_str("\r\n");
        }
        let reply = post(&request);
        assert!(reply.starts_with(b"HTTP/1.1 200"));
        let body = split_response(&reply).2;
        assert!(body.starts_with(
            format!(
                "course=1|{}|0|0|0|1\r\n",
                if course == 3 { 1 } else { course }
            )
            .as_bytes()
        ));
        assert!(body.ends_with(b"1|001000|ALPHA|02\r\n"));
        assert_eq!(
            body.windows(7).filter(|s| *s == b"course=").count(),
            if course == 3 { 3 } else { 1 }
        );
    }
    let reply = post("*=ABCDEF&owner=ALPHA&score=1|000010|03");
    assert!(split_response(&reply).2.ends_with(b"1|001000|ALPHA|02\r\n"));
    for body in [
        "*=ABCDEF&owner=A&score=4|000010|03",
        "*=ABCDEF&owner=A&score=1|1000000|03",
        "*=ABCDEF&owner=A&score=1|000010|100",
    ] {
        assert!(post(body).starts_with(b"HTTP/1.1 400"));
    }
}
