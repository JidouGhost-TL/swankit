// SPDX-License-Identifier: MIT OR Apache-2.0
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use ws_wondergate::{
    backend::{Tcp, TcpOptions},
    frame::Frame,
    Adapter, Config, NetError, Network, Recv,
};

fn command(a: &mut Adapter<Tcp>, cmd: u8, params: &[u8]) -> Vec<u8> {
    a.try_handle(&Frame {
        kind: 0x11,
        cmd,
        params: params.to_vec(),
    })
    .unwrap()
    .unwrap()
    .params
}
fn connected(parts: Vec<Vec<u8>>) -> (Adapter<Tcp>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        for part in parts {
            peer.write_all(&part).unwrap();
        }
        peer.shutdown(Shutdown::Write).unwrap();
    });
    let mut a = Adapter::new(Config::default(), Tcp::default());
    assert_eq!(command(&mut a, 1, &[0]), [0]);
    let mut address = vec![0, 0, 2];
    address.extend_from_slice(&port.to_le_bytes());
    address.extend_from_slice(&[127, 0, 0, 1]);
    address.extend_from_slice(&[0; 8]);
    assert_eq!(command(&mut a, 3, &address), [0, 1]);
    (a, worker)
}

// Four variants of the Scripted/harness cases run over real loopback TCP.
#[test]
fn readline_leaves_binary_body_for_block_read() {
    let lines: [&[u8]; 4] = [
        b"HTTP/1.1 200 OK\r\n",
        b"Content-Length: 5\r\n",
        b"Connection: close\r\n",
        b"\r\n",
    ];
    let (mut a, worker) = connected(vec![[lines.concat(), b"\0\xff\r\nX".to_vec()].concat()]);
    worker.join().unwrap();
    for line in lines {
        assert_eq!(
            command(&mut a, 0xd, &[0, 120, 0]),
            [vec![0, line.len() as u8], line.to_vec()].concat()
        );
    }
    assert_eq!(command(&mut a, 0xf, &[0, 120, 0]), b"\0\x05\0\xff\r\nX");
    assert_eq!(command(&mut a, 0xf, &[0, 120, 0]), [0, 255]);
}
#[test]
fn fragmented_crlf_and_long_line() {
    let (mut a, worker) = connected(vec![
        [vec![b'a'; 150], vec![b'\r']].concat(),
        b"\n\r".to_vec(),
        b"\nbody".to_vec(),
    ]);
    worker.join().unwrap();
    assert_eq!(
        command(&mut a, 0xd, &[0, 80, 0]),
        [vec![0, 80], vec![b'a'; 80]].concat()
    );
    assert_eq!(
        command(&mut a, 0xd, &[0, 80, 0]),
        [vec![0, 72], vec![b'a'; 70], b"\r\n".to_vec()].concat()
    );
    assert_eq!(command(&mut a, 0xd, &[0, 120, 0]), b"\0\x02\r\n");
    assert_eq!(command(&mut a, 0xf, &[0, 120, 0]), b"\0\x04body");
}
#[test]
fn readline_eof_and_unterminated_tail() {
    let (mut a, worker) = connected(vec![b"tail".to_vec()]);
    worker.join().unwrap();
    assert_eq!(command(&mut a, 0xd, &[0, 120, 0]), b"\0\x04tail");
    assert_eq!(command(&mut a, 0xd, &[0, 120, 0]), [0, 255]);
}
#[test]
fn zero_length_read_preserves_data() {
    let (mut a, worker) = connected(vec![b"line\r\nbody".to_vec()]);
    worker.join().unwrap();
    for cmd in [0xd, 0xf] {
        assert_eq!(command(&mut a, cmd, &[0, 0, 0]), [0, 0]);
    }
    assert_eq!(command(&mut a, 0xd, &[0, 120, 0]), b"\0\x06line\r\n");
    assert_eq!(command(&mut a, 0xf, &[0, 120, 0]), b"\0\x04body");
}
#[test]
fn dns_is_denied_by_default_with_explicit_maps_numeric_and_opt_in() {
    let mut tcp = Tcp::default();
    assert_eq!(tcp.resolve(b"unlisted.invalid"), None);
    assert_eq!(tcp.resolve(b"localhost"), None);
    assert_eq!(tcp.resolve_redirect(b"localhost"), Some([127, 0, 0, 1]));
    assert_eq!(tcp.resolve(b"localhost"), None); // Redirect authorization is not retained.
    assert_eq!(tcp.resolve(b"127.0.0.1"), Some([127, 0, 0, 1]));
    assert_eq!(tcp.resolve(b"0.0.0.0"), None);
    let mut options = TcpOptions {
        resolve_all: Some([127, 0, 0, 2]),
        ..TcpOptions::default()
    };
    options
        .resolutions
        .insert(b"exact.invalid".to_vec(), [127, 0, 0, 1]);
    let mut tcp = Tcp::new(options).unwrap();
    assert_eq!(tcp.resolve(b"exact.invalid"), Some([127, 0, 0, 1]));
    assert_eq!(tcp.resolve(b"Exact.invalid"), Some([127, 0, 0, 2]));
    assert_eq!(tcp.resolve(b"\xff"), None);
    assert_eq!(tcp.resolve(b"bad\0name"), None);
    let mut tcp = Tcp::new(TcpOptions {
        allow_system_dns: true,
        ..TcpOptions::default()
    })
    .unwrap();
    assert_eq!(tcp.resolve(b"localhost"), Some([127, 0, 0, 1]));
}
#[test]
fn recv_timeout_close_and_failed_connect_are_bounded() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (release, wait) = mpsc::channel();
    let worker = thread::spawn(move || {
        let (_peer, _) = listener.accept().unwrap();
        let _ = wait.recv_timeout(Duration::from_secs(3));
    });
    let mut tcp = Tcp::new(TcpOptions {
        recv_timeout: Duration::from_millis(30),
        connect_timeout: Duration::from_millis(100),
        ..TcpOptions::default()
    })
    .unwrap();
    let mut conn = tcp.connect([127, 0, 0, 1], port).unwrap();
    assert!(matches!(conn.recv(0), Recv::Data(v) if v.is_empty()));
    let start = Instant::now();
    assert!(matches!(conn.recv(usize::MAX), Recv::Error));
    assert!(start.elapsed() < Duration::from_secs(2));
    conn.close();
    conn.close();
    assert!(conn.send(b"x").is_err());
    assert!(matches!(conn.recv(1), Recv::Error));
    release.send(()).unwrap();
    worker.join().unwrap();
    assert!(tcp.connect([127, 0, 0, 1], port).is_err());
    assert!(matches!(tcp.connect([0; 4], port), Err(NetError::Denied)));
    assert!(Tcp::new(TcpOptions {
        recv_timeout: Duration::ZERO,
        ..TcpOptions::default()
    })
    .is_err());
}
#[test]
fn tcp_send_reaches_peer() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = thread::spawn(move || {
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut bytes = [0; 4];
        peer.read_exact(&mut bytes).unwrap();
        bytes
    });
    let mut conn = Tcp::default().connect([127, 0, 0, 1], port).unwrap();
    let data = b"\0\xffxy";
    let mut n = 0;
    while n < data.len() {
        let sent = conn.send(&data[n..]).unwrap();
        assert!(sent > 0);
        n += sent;
    }
    assert_eq!(worker.join().unwrap(), *data);
}
