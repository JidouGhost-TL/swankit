// SPDX-License-Identifier: MIT OR Apache-2.0
//! Independent shared-service bytes, frozen externally; no title engine goldens.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{hash_bytes, run_dir, Args, Digest};
use serde_json::{json, Value};
use ws_wondergate_server::{
    http, services::bpl, store::MemoryStore, FixedClock, Limits, SeededRng, Server, Timestamp,
};
struct ServicesDigest;
impl Digest for ServicesDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let clock = Timestamp::parse_fixed(
            args.str("clock")
                .ok_or_else(|| Error::Malformed("missing corpus clock".into()))?,
        )?;
        let seed = args
            .get("seed")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::Malformed("missing corpus seed".into()))?;
        let (reply, error) = match args.str("protocol") {
            Some("HTTP") => {
                let app = Server::bundled(
                    Box::new(MemoryStore::new()),
                    Box::new(FixedClock(clock)),
                    Box::new(SeededRng::new(seed)),
                )?;
                (
                    app.dispatch_http(bytes),
                    http::parse(bytes, &Limits::default()).is_err(),
                )
            }
            Some("BPL") => match bpl::reply(bytes, &Limits::default()) {
                Ok(reply) => (reply, false),
                Err(_) => (vec![], true),
            },
            _ => return Err(Error::Malformed("unknown corpus protocol".into())),
        };
        Ok(
            json!({"sha256":hash_bytes(bytes),"response_sha256":hash_bytes(&reply),"response_length":reply.len(),"error":error}),
        )
    }
}
#[test]
fn corpus_shared_services() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen service expectations"
    );
    let count = run_dir("ws-wondergate-services", &ServicesDigest);
    eprintln!("ws-wondergate-services: {count} independent HTTP/BPL request/reply vectors passed");
}
