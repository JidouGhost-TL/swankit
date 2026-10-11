use super::*;
use crate::{store::MemoryStore, FixedClock, SeededRng, Timestamp};
#[test]
fn inherited_nonblocking_stream_waits_for_delayed_http_bytes() {
    let make_server = || {
        Server::bundled(
            Box::new(MemoryStore::new()),
            Box::new(FixedClock(
                Timestamp::parse_fixed("20261005200403").unwrap(),
            )),
            Box::new(SeededRng::new(123)),
        )
        .unwrap()
    };
    let path = std::env::temp_dir().join(format!("swankit-flags-{}", std::process::id()));
    let wire = Arc::new(WireLog::open(&path).unwrap());
    let mut tcp = TcpServer::bind(
        make_server(),
        Config {
            http: ([127, 0, 0, 1], 0).into(),
            bpl: ([127, 0, 0, 1], 0).into(),
            ..Default::default()
        },
        wire.clone(),
    )
    .unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let (stream, peer) = listener.accept().unwrap();
    stream.set_nonblocking(true).unwrap();
    let worker = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        let _ = client.write_all(b"GET /unknown HTTP/1.1\r\nHost: synthetic.invalid\r\n\r\n");
        let _ = client.shutdown(std::net::Shutdown::Write);
        let mut response = Vec::new();
        let _ = client.read_to_end(&mut response);
        response
    });
    serve(
        stream,
        peer,
        "HTTP",
        &make_server(),
        &wire,
        Duration::from_secs(1),
        &tcp.shared,
    )
    .unwrap();
    let response = worker.join().unwrap();
    tcp.shutdown().unwrap();
    std::fs::remove_dir_all(path).unwrap();
    assert!(
        response.starts_with(b"HTTP/1.1 404"),
        "inherited nonblocking mode skipped delayed valid request: {}",
        String::from_utf8_lossy(&response)
    );
}
