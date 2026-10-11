// SPDX-License-Identifier: MIT OR Apache-2.0
//! Independent shared-service vectors and captured HTTP flows, frozen externally.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{corpus_root, hash_bytes, run_dir, Args, Digest, Spec};
use serde_json::{json, Value};
use ws_wondergate_server::{
    http,
    services::bpl,
    store::{MemoryStore, Store, Submission},
    FixedClock, Limits, SeededRng, Server, Timestamp,
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
                let store = MemoryStore::new();
                if let Some(rows) = args.get("store") {
                    let rows = rows
                        .as_array()
                        .ok_or_else(|| Error::Malformed("invalid corpus starting store".into()))?;
                    if rows.len() > 4096 {
                        return Err(Error::Malformed("oversized corpus starting store".into()));
                    }
                    for row in rows {
                        let row: Submission = serde_json::from_value(row.clone())
                            .map_err(|e| Error::Malformed(e.to_string()))?;
                        store
                            .submit(row)
                            .map_err(|e| Error::Malformed(e.to_string()))?;
                    }
                }
                if let Some(path) = args.str("store_path") {
                    if std::fs::metadata(path)
                        .map_err(|e| Error::Malformed(e.to_string()))?
                        .len()
                        > 16 * 1024 * 1024
                    {
                        return Err(Error::Malformed("oversized corpus starting history".into()));
                    }
                    let bytes = std::fs::read(path).map_err(|e| Error::Malformed(e.to_string()))?;
                    if Some(hash_bytes(&bytes).as_str()) != args.str("store_sha256") {
                        return Err(Error::Malformed(
                            "corpus starting history digest mismatch".into(),
                        ));
                    }
                    let rows: Vec<Submission> = serde_json::from_slice(&bytes)
                        .map_err(|e| Error::Malformed(e.to_string()))?;
                    if rows.len() > 32768 {
                        return Err(Error::Malformed("oversized corpus starting history".into()));
                    }
                    for row in rows {
                        store
                            .submit(row)
                            .map_err(|e| Error::Malformed(e.to_string()))?;
                    }
                }
                let app = Server::bundled(
                    Box::new(store),
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
        if let Some(path) = args.str("response_path") {
            if std::fs::metadata(path)
                .map_err(|e| Error::Malformed(e.to_string()))?
                .len()
                > 81924
            {
                return Err(Error::Malformed("oversized corpus response".into()));
            }
            if std::fs::read(path).map_err(|e| Error::Malformed(e.to_string()))? != reply {
                return Err(Error::Malformed(
                    "full corpus reply differs from oracle".into(),
                ));
            }
        }
        Ok(
            json!({"sha256":hash_bytes(bytes),"response_sha256":hash_bytes(&reply),"response_length":reply.len(),"error":error}),
        )
    }
}
#[test]
fn corpus_title_http_flows() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen HTTP flow expectations"
    );
    let count = run_dir("ws-wondergate-server", &ServicesDigest);
    if count != 0 {
        // Compare the complete bytes as well as the frozen digests. The
        // enrollment sidecar points back to the independent native capture.
        let directory = corpus_root("captured HTTP reply bytes")
            .unwrap()
            .join("ws-wondergate-server");
        let spec = Spec::load(&directory.join("spec.json")).unwrap();
        let enrollment_path = directory.join("enrollment.json");
        assert!(std::fs::metadata(&enrollment_path).unwrap().len() <= 8 * 1024 * 1024);
        let enrollment: Value =
            serde_json::from_slice(&std::fs::read(enrollment_path).unwrap()).unwrap();
        let records = enrollment["samples"].as_array().unwrap();
        assert_eq!(records.len(), spec.samples.len());
        for (name, sample) in &spec.samples {
            let record = records
                .iter()
                .find(|r| {
                    r["input_sha256"] == sample.expect["sha256"]
                        && r["response_sha256"] == sample.expect["response_sha256"]
                })
                .unwrap();
            let response_path = record["response_path"].as_str().unwrap();
            assert!(std::fs::metadata(response_path).unwrap().len() <= 81924);
            let expected = std::fs::read(response_path).unwrap();
            assert_eq!(hash_bytes(&expected), sample.expect["response_sha256"]);
            assert_eq!(sample.args["store"], json!([]));
            let app = Server::bundled(
                Box::new(MemoryStore::new()),
                Box::new(FixedClock(
                    Timestamp::parse_fixed(sample.args["clock"].as_str().unwrap()).unwrap(),
                )),
                Box::new(SeededRng::new(sample.args["seed"].as_u64().unwrap())),
            )
            .unwrap();
            let reply = app.dispatch_http(&std::fs::read(directory.join(name)).unwrap());
            assert_eq!(reply, expected, "{name}: full captured HTTP response bytes");
            eprintln!(
                "{name}: {} response bytes identical, SHA-256 {}",
                reply.len(),
                hash_bytes(&reply)
            );
        }
    }
    eprintln!("ws-wondergate-server: {count} captured HTTP flows passed byte for byte");
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

#[test]
fn corpus_ranking_histories() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen ranking expectations"
    );
    let count = run_dir("ws-wondergate-rankings", &ServicesDigest);
    eprintln!("ws-wondergate-rankings: {count} independent history/request/reply vectors passed");
}
