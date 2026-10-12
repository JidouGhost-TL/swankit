// SPDX-License-Identifier: MIT OR Apache-2.0
// Synthetic ports of the fourteen adapter harness cases; no live networking.
use ws_wondergate::{
    backend::{Scripted, ScriptedPeer},
    frame::Frame,
    Adapter, Config, Recv,
};
const IP: [u8; 4] = [127, 0, 0, 1];
fn command(adapter: &mut Adapter<Scripted>, kind: u8, cmd: u8, params: &[u8]) -> Vec<u8> {
    let frame = Frame {
        kind,
        cmd,
        params: params.to_vec(),
    };
    let mut out = Vec::new();
    adapter.push(
        &frame.encode(Config::default().profile.header).unwrap(),
        &mut out,
        &mut |_| {},
    );
    out
}
fn connected(fragments: &[&[u8]]) -> Adapter<Scripted> {
    let mut net = Scripted::default();
    net.queue(ScriptedPeer::new(
        IP,
        80,
        fragments.iter().map(|b| Recv::Data(b.to_vec())),
    ));
    let mut adapter = Adapter::new(Config::default(), net);
    assert_eq!(command(&mut adapter, 0x11, 1, &[0]), [0x11, 0, 2, 1, 0]);
    assert_eq!(
        command(
            &mut adapter,
            0x11,
            3,
            &[0, 0, 2, 80, 0, 127, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]
        ),
        [0x11, 0, 3, 3, 0, 1]
    );
    adapter
}
fn read(adapter: &mut Adapter<Scripted>, cmd: u8, size: u8, socket: u8) -> (u8, Vec<u8>) {
    let reply = command(adapter, 0x11, cmd, &[socket, size, 0]);
    assert_eq!(&reply[..5], &[0x11, 0, (reply.len() - 3) as u8, cmd, 0]);
    if reply[5] != 255 {
        assert_eq!(usize::from(reply[5]), reply.len() - 6);
    } else {
        assert_eq!(reply.len(), 6);
    }
    (reply[5], reply[6..].to_vec())
}
#[test]
fn init_and_status() {
    let mut a = Adapter::new(Config::default(), Scripted::default());
    assert_eq!(command(&mut a, 1, 2, &[]), [1, 0, 3, 2, b'1', b'0']);
    assert_eq!(command(&mut a, 2, 1, &[0]), [2, 0, 2, 2, 2]);
    assert_eq!(command(&mut a, 1, 0, &[]), [1, 0, 3, 0, 0, 15]);
}
#[test]
fn readline_leaves_binary_body_for_block_read() {
    let lines: [&[u8]; 4] = [
        b"HTTP/1.1 200 OK\r\n",
        b"Content-Length: 5\r\n",
        b"Connection: close\r\n",
        b"\r\n",
    ];
    let response = [lines.concat(), b"\0\xff\r\nX".to_vec()].concat();
    let mut a = connected(&[&response]);
    for line in lines {
        assert_eq!(read(&mut a, 0xd, 120, 0), (line.len() as u8, line.to_vec()));
    }
    assert_eq!(read(&mut a, 0xf, 120, 0), (5, b"\0\xff\r\nX".to_vec()));
}
#[test]
fn seven_header_lines_and_time_body() {
    let mut lines = vec![b"HTTP/1.1 200 OK\r\n".to_vec()];
    for i in 0..5 {
        lines.push(format!("X-{i}: 0\r\n").into_bytes());
    }
    lines.push(b"\r\n".to_vec());
    let body = [0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0];
    let response = [lines.concat(), body.to_vec()].concat();
    let mut a = connected(&[&response]);
    for line in lines {
        assert_eq!(read(&mut a, 0xd, 120, 0), (line.len() as u8, line));
    }
    assert_eq!(read(&mut a, 0xf, 120, 0), (11, body.to_vec()));
}
#[test]
fn fragmented_crlf_and_long_line() {
    let mut first = vec![b'a'; 150];
    first.push(b'\r');
    let mut a = connected(&[&first, b"\n\r", b"\nbody"]);
    assert_eq!(read(&mut a, 0xd, 80, 0), (80, vec![b'a'; 80]));
    let mut rest = vec![b'a'; 70];
    rest.extend_from_slice(b"\r\n");
    assert_eq!(read(&mut a, 0xd, 80, 0), (72, rest));
    assert_eq!(read(&mut a, 0xd, 120, 0), (2, b"\r\n".to_vec()));
    assert_eq!(read(&mut a, 0xf, 120, 0), (4, b"body".to_vec()));
}
#[test]
fn readline_eof_and_unterminated_tail() {
    let mut a = connected(&[b"tail"]);
    assert_eq!(read(&mut a, 0xd, 120, 0), (4, b"tail".to_vec()));
    assert_eq!(read(&mut a, 0xd, 120, 0), (255, vec![]));
}
#[test]
fn block_eof_after_short_read() {
    let mut a = connected(&[b"binary\0tail"]);
    assert_eq!(read(&mut a, 0xf, 120, 0), (11, b"binary\0tail".to_vec()));
    assert_eq!(read(&mut a, 0xf, 120, 0), (255, vec![]));
}
#[test]
fn zero_length_read_preserves_data() {
    let mut a = connected(&[b"line\r\nbody"]);
    for cmd in [0xd, 0xf] {
        assert_eq!(read(&mut a, cmd, 0, 0), (0, vec![]));
    }
    assert_eq!(read(&mut a, 0xd, 120, 0), (6, b"line\r\n".to_vec()));
    assert_eq!(read(&mut a, 0xf, 120, 0), (4, b"body".to_vec()));
}
#[test]
fn invalid_socket_error_has_no_uninitialized_payload() {
    let mut a = Adapter::new(Config::default(), Scripted::default());
    for cmd in [0xd, 0xf] {
        assert_eq!(read(&mut a, cmd, 120, 254), (255, vec![]));
    }
}
#[test]
fn close_six_bytes_and_id_reuse() {
    let mut a = connected(&[]);
    assert_eq!(command(&mut a, 0x11, 7, &[0]), [0x11, 0, 3, 7, 0, 1]);
    assert_eq!(command(&mut a, 0x11, 7, &[0]), [0x11, 0, 3, 7, 0, 0]);
    assert_eq!(command(&mut a, 0x11, 1, &[0]), [0x11, 0, 2, 1, 0]);
}
#[test]
fn poll_six_bytes_and_invalid_socket() {
    let mut a = connected(&[]);
    assert_eq!(command(&mut a, 0x11, 6, &[254, 2]), [0x11, 0, 3, 6, 0, 0]);
    assert_eq!(command(&mut a, 0x11, 6, &[0, 2]), [0x11, 0, 3, 6, 0, 1]);
}
#[test]
fn f_stack_metadata_and_answer() {
    let mut a = Adapter::new(Config::default(), Scripted::default());
    for cmd in [1, 3, 4] {
        assert_eq!(command(&mut a, 1, cmd, &[0]), [1, 0, 2, cmd, 0]);
    }
    assert_eq!(command(&mut a, 1, 9, &[0]), [1, 0, 2, 0xb, 0]);
}
#[test]
fn both_dial_prefixes() {
    let mut a = Adapter::new(Config::default(), Scripted::default());
    for mode in [1, 3] {
        assert_eq!(
            command(&mut a, 1, 8, &[0, 1, mode, 2, b'#', b'#']),
            [1, 0, 2, 0xb, 0]
        );
    }
}
#[test]
fn dial_long_success_for_gather_beat_length_gate() {
    // ROM 38C887 returns 99 unless dial success has BX>=0x18 (24 bytes
    // incl. 3-byte header) with result 00. Short 5/6-byte replies fail.
    let cfg = Config::from_ini("[swankit]\ndial_long_success = true\n").unwrap();
    assert!(cfg.profile.dial_long_success);
    let mut a = Adapter::new(cfg, Scripted::default());
    for phone in [vec![], vec![b'1']] {
        let mut params = vec![0, 1, 1, phone.len() as u8];
        params.extend_from_slice(&phone);
        let reply = command(&mut a, 1, 8, &params);
        let mut expected = vec![1, 0, 22, 0xb];
        expected.extend_from_slice(&[0; 21]);
        assert_eq!(reply, expected);
        assert_eq!(reply.len(), 25);
    }
    // Default profile keeps short replies for other titles.
    let mut short = Adapter::new(Config::default(), Scripted::default());
    assert_eq!(command(&mut short, 1, 8, &[0, 1, 1, 0]), [1, 0, 2, 0xb, 0]);
}
#[test]
fn interframe_nul_padding() {
    let mut a = Adapter::new(Config::default(), Scripted::default());
    let mut out = Vec::new();
    a.push(&[0], &mut out, &mut |_| {});
    assert!(out.is_empty());
    assert_eq!(command(&mut a, 1, 2, &[]), [1, 0, 3, 2, b'1', b'0']);
}
#[test]
fn poweroff_four_byte_reply() {
    let mut a = Adapter::new(Config::default(), Scripted::default());
    assert_eq!(
        command(&mut a, 0xf, 0xff, &[0x55, 0xaa, 0x55, 0xaa]),
        [0xf, 0, 1, 0xff]
    );
}
