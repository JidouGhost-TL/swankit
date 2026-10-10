// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(dead_code)]
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use ws_wondergate_server::{
    services::Services, store::MemoryStore, FixedClock, SeededRng, Server, Timestamp,
};
pub fn stamp() -> Timestamp {
    Timestamp::parse_fixed("20261005200403").unwrap()
}
pub fn server() -> Server {
    Server::bundled(
        Box::new(MemoryStore::new()),
        Box::new(FixedClock(stamp())),
        Box::new(SeededRng::new(123)),
    )
    .unwrap()
}
pub fn post(action: &str, host: usize, absolute: bool, extra: &[u8]) -> Vec<u8> {
    let services = Services::bundled().unwrap();
    let path = format!("/service/charge/{action}");
    let target = if absolute {
        format!("http://{}{path}", services.charge_hosts[0])
    } else {
        path
    };
    let mut raw = format!(
        "POST {target} HTTP/1.1\r\nHost: {}\r\nUser-Agent: SWAN/1.0\r\n",
        services.charge_hosts[host]
    )
    .into_bytes();
    raw.extend(extra);
    raw.extend(b"\r\n");
    raw
}
pub fn split_response(raw: &[u8]) -> (String, std::collections::BTreeMap<String, String>, &[u8]) {
    let end = raw.windows(4).position(|part| part == b"\r\n\r\n").unwrap();
    let head = std::str::from_utf8(&raw[..end]).unwrap().to_owned();
    let fields: std::collections::BTreeMap<String, String> = head
        .split("\r\n")
        .skip(1)
        .map(|line| {
            let (k, v) = line.split_once(": ").unwrap();
            (k.into(), v.into())
        })
        .collect();
    let body = &raw[end + 4..];
    assert_eq!(
        fields["Content-Length"].parse::<usize>().unwrap(),
        body.len()
    );
    assert_eq!(fields["Connection"], "close");
    (head, fields, body)
}
pub fn packet() -> Vec<u8> {
    let mut packet: Vec<_> = (0..24).collect();
    packet[..8].copy_from_slice(&[1, 24, 0, 0, 45, 0, 0, 0]);
    packet.extend([21, 0, 1, 0]);
    packet.extend([b'U'; 17]);
    assert_eq!(packet.len(), 45);
    packet
}
pub struct Temp(pub PathBuf);
impl Temp {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "swankit-wg-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("temporary directory: {e}"),
            }
        }
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
