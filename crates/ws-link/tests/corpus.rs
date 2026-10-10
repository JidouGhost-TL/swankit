//! Recorded byte payloads with independently frozen output expectations.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{hash_bytes, run_dir, Args, Digest};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::{atomic::AtomicBool, atomic::Ordering, Arc},
    thread,
    time::Duration,
};
use ws_link::{Endpoint, Options, Relay};

fn malformed(error: impl std::fmt::Display) -> Error {
    Error::Malformed(error.to_string())
}
struct LinkDigest;
impl Digest for LinkDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        if bytes.is_empty() || bytes.len() > 1024 * 1024 {
            return Err(malformed("link corpus payload exceeds bounds"));
        }
        let number = |key| {
            args.get(key)
                .and_then(Value::as_u64)
                .ok_or_else(|| malformed("invalid link schedule argument"))
        };
        let options = Options {
            latency: Duration::from_millis(number("latency_ms")?),
            jitter: Duration::from_millis(number("jitter_ms")?),
            seed: number("seed")?,
            ..Options::default()
        };
        let direction = match args.get("direction").and_then(Value::as_str) {
            Some("A>B") => 0,
            Some("B>A") => 1,
            _ => return Err(malformed("invalid link direction")),
        };
        let relay = Relay::new(
            [
                Endpoint::Listen("127.0.0.1:0".into()),
                Endpoint::Listen("127.0.0.1:0".into()),
            ],
            options,
            Box::new(std::io::sink()),
        )
        .map_err(malformed)?;
        let mut peers = Vec::new();
        for end in 0..2 {
            let address = relay
                .listen_address(end)
                .ok_or_else(|| malformed("missing relay listener"))?;
            let peer = TcpStream::connect(address).map_err(malformed)?;
            peer.set_nodelay(true).map_err(malformed)?;
            peer.set_read_timeout(Some(Duration::from_secs(10)))
                .map_err(malformed)?;
            peer.set_write_timeout(Some(Duration::from_secs(10)))
                .map_err(malformed)?;
            peers.push(peer);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = thread::spawn(move || relay.run(&flag));
        let mut received = vec![0; bytes.len()];
        // Keep the sending stream open: EOF ends that endpoint and may drop
        // already-read pending bytes. Both operations have bounded waits.
        let transferred = thread::scope(|scope| {
            let (a, b) = peers.split_at_mut(1);
            let (sender, receiver) = if direction == 0 {
                (&mut a[0], &mut b[0])
            } else {
                (&mut b[0], &mut a[0])
            };
            let writer = scope.spawn(|| sender.write_all(bytes));
            receiver.read_exact(&mut received).map_err(malformed)?;
            writer
                .join()
                .map_err(|_| malformed("link corpus writer panicked"))?
                .map_err(malformed)
        });
        stop.store(true, Ordering::Relaxed);
        let stats = worker
            .join()
            .map_err(|_| malformed("link corpus worker panicked"))?
            .map_err(malformed)?;
        transferred?;
        Ok(
            json!({"sha256": hash_bytes(bytes), "output_sha256": hash_bytes(&received),
            "output_bytes": received.len(), "forwarded": stats.forwarded,
            "dropped": stats.dropped}),
        )
    }
}

#[test]
fn corpus_recorded_link_payloads() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen link expectations"
    );
    let count = run_dir("ws-link", &LinkDigest);
    eprintln!("ws-link corpus: {count} recorded payloads delivered through the relay against independent byte digests, order and zero-drop expectations");
}
