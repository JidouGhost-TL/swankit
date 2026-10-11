// SPDX-License-Identifier: MIT OR Apache-2.0
use ws_wondergate_server::{http, Limits};
#[test]
fn zero_body_crlf_is_fragment_independent_bounded_and_lossless() {
    for method in ["GET", "HEAD"] {
        let raw =
            format!("{method} /index HTTP/1.1\r\nHost: synthetic.test\r\n\r\n\r\n").into_bytes();
        let parsed = http::parse(&raw, &Limits::default()).unwrap();
        assert_eq!(parsed.raw, raw);
        assert!(parsed.body.is_empty());
        for split in 0..=raw.len() {
            let mut input = http::Assembler::new(Limits::default());
            input.feed(&raw[..split]).unwrap();
            input.feed(&raw[split..]).unwrap();
            input.finish().unwrap();
            assert_eq!(input.raw(), raw);
        }
        for trailer in [
            b"\r".as_slice(),
            b"\n",
            b"\r\n\r\n",
            b"\0\r\n",
            b"GET /next HTTP/1.1\r\n\r\n",
        ] {
            let bad = [&raw[..raw.len() - 2], trailer].concat();
            assert!(http::parse(&bad, &Limits::default()).is_err());
            let mut input = http::Assembler::new(Limits::default());
            let result = input.feed(&bad);
            assert!(result.is_err() || input.finish().is_err());
        }
    }
    assert!(http::parse(
        b"POST / HTTP/1.1\r\nHost: synthetic.test\r\n\r\n\r\n",
        &Limits::default()
    )
    .is_err());
    let body = b"POST / HTTP/1.1\r\nHost: synthetic.test\r\nContent-Length: 2\r\n\r\n\r\n";
    assert_eq!(http::parse(body, &Limits::default()).unwrap().body, b"\r\n");
}
