// SPDX-License-Identifier: MIT OR Apache-2.0
//! Replay independent adapter captures against their recorded TCP peers.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{hash_bytes, run_dir, Args, Digest};
use serde_json::{json, Value};
use ws_wondergate::{
    backend::{Transcript, TranscriptPeer},
    frame::HeaderMode,
    transcript::{self, TranscriptLimits},
    Adapter, Config, Limits, Profile,
};

const MAX_TAP: usize = 8 * 1024 * 1024;
const MAX_PEER: usize = 1024 * 1024;
fn malformed(reason: &str) -> Error {
    Error::Malformed(format!("adapter corpus: {reason}"))
}
fn array(value: Option<&Value>, maximum: usize) -> Result<&[Value]> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("missing array"))?;
    if values.len() > maximum {
        return Err(malformed("oversized array"));
    }
    Ok(values)
}
fn hex(value: &Value, maximum: usize) -> Result<Vec<u8>> {
    let text = value
        .as_str()
        .ok_or_else(|| malformed("missing hex bytes"))?;
    if text.len() > maximum * 2 || text.len() % 2 != 0 {
        return Err(malformed("invalid hex length"));
    }
    fn nibble(b: u8) -> Result<u8> {
        match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            _ => Err(malformed("invalid hex digit")),
        }
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok(nibble(pair[0])? * 16 + nibble(pair[1])?))
        .collect()
}
fn ip(value: &Value) -> Result<[u8; 4]> {
    let values = array(Some(value), 4)?;
    if values.len() != 4 {
        return Err(malformed("invalid IPv4 address"));
    }
    let mut result = [0; 4];
    for (out, value) in result.iter_mut().zip(values) {
        *out = value
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .ok_or_else(|| malformed("invalid IPv4 byte"))?;
    }
    Ok(result)
}
fn network(args: &Args) -> Result<Transcript> {
    let mut network = Transcript::default();
    for resolution in array(args.get("resolutions"), 4096)? {
        let name = hex(&resolution["hostname_hex"], 255)?;
        if name.is_empty()
            || name.contains(&0)
            || network
                .resolutions
                .insert(name, ip(&resolution["ip"])?)
                .is_some()
        {
            return Err(malformed("duplicate or invalid hostname"));
        }
    }
    for peer in array(args.get("peers"), 4096)? {
        let port = peer["port"]
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .filter(|v| *v != 0)
            .ok_or_else(|| malformed("invalid port"))?;
        let peer = TranscriptPeer::new(
            ip(&peer["ip"])?,
            port,
            hex(&peer["request_hex"], MAX_PEER)?,
            hex(&peer["response_hex"], MAX_PEER)?,
        )
        .map_err(|_| malformed("oversized peer"))?;
        network
            .queue(peer)
            .map_err(|_| malformed("peer retention limit"))?;
    }
    Ok(network)
}
fn frame_digest(bytes: &[u8], profile: Profile) -> Result<Value> {
    let frames = transcript::frames(
        bytes,
        profile.header,
        profile.skip_zero_padding,
        Limits::default(),
    )
    .map_err(|e| malformed(&e.to_string()))?;
    Ok(Value::Array(
        frames
            .iter()
            .map(|frame| {
                json!({"offset": frame.offset, "kind": frame.frame.kind, "cmd": frame.frame.cmd,
                    "params_length": frame.frame.params.len(), "params_sha256": hash_bytes(&frame.frame.params)})
            })
            .collect(),
    ))
}
struct AdapterDigest;
impl Digest for AdapterDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let mut profile = match args.str("profile") {
            Some("upstream") => Profile::UPSTREAM,
            Some("revival") => Profile::REVIVAL,
            _ => return Err(malformed("unknown profile")),
        };
        profile.header = match args.str("header") {
            Some("len16be") => HeaderMode::Len16Be,
            Some("zero-then-len8") => HeaderMode::ZeroThenLen8,
            _ => return Err(malformed("unknown header")),
        };
        let text = std::str::from_utf8(bytes).map_err(|_| malformed("tap must be UTF-8"))?;
        let streams = transcript::parse_with_limits(
            text,
            TranscriptLimits {
                max_text: MAX_TAP,
                max_stream: MAX_TAP / 2,
                max_records: 1_000_000,
            },
        )
        .map_err(|e| malformed(&e.to_string()))?;
        if streams.input.is_empty() || streams.output.is_empty() {
            return Err(malformed("both directions required"));
        }
        let input_frames = frame_digest(&streams.input, profile)?;
        let output_frames = frame_digest(&streams.output, profile)?;
        let config = Config {
            profile,
            // Replay the independently captured BPL connection too.
            builtin_bpl: false,
            ..Config::default()
        };
        let mut state = Value::Null;
        // Tap read boundaries are incidental. Require the same complete bytes
        // for byte-at-a-time, small-chunk and raw-pump-sized input.
        for size in [1, 7, 4096] {
            let network = network(args)?;
            let log = network.log();
            let mut adapter = Adapter::new(config.clone(), network);
            let mut output = Vec::new();
            for chunk in streams.input.chunks(size) {
                adapter.push(chunk, &mut output, &mut |_| {});
                if output.len() > MAX_TAP / 2 {
                    return Err(malformed("replay output exceeds bound"));
                }
            }
            if adapter.finish().is_some() {
                return Err(malformed("incomplete replay input"));
            }
            drop(adapter);
            log.verify()
                .map_err(|_| malformed("recorded TCP peers did not match"))?;
            if output != streams.output {
                let at = output
                    .iter()
                    .zip(&streams.output)
                    .position(|(a, b)| a != b)
                    .unwrap_or(output.len().min(streams.output.len()));
                return Err(malformed(&format!(
                    "response differs at byte {at}: {} generated, {} recorded",
                    output.len(),
                    streams.output.len()
                )));
            }
            let snapshot = log.snapshot();
            state = json!({"expected": snapshot.expected, "connected": snapshot.connected,
                "closed": snapshot.closed, "matched_requests": snapshot.matched_requests,
                "mismatches": snapshot.mismatches});
        }
        eprintln!(
            "{}: {} input / {} output bytes identical at 3 chunk sizes; {} TCP peers verified",
            args.str("id").unwrap_or("sample"),
            streams.input.len(),
            streams.output.len(),
            state["expected"]
        );
        Ok(json!({"sha256": hash_bytes(bytes),
            "input_sha256": hash_bytes(&streams.input), "input_length": streams.input.len(), "input_frames": input_frames,
            "output_sha256": hash_bytes(&streams.output), "output_length": streams.output.len(), "output_frames": output_frames,
            "network": state}))
    }
}

#[test]
fn corpus_adapter_goldens() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen adapter expectations"
    );
    let count = run_dir("ws-wondergate-adapter", &AdapterDigest);
    eprintln!("ws-wondergate-adapter: {count} independent streams matched byte for byte");
}

#[test]
fn replay_rejects_changed_response_and_unused_peer() {
    let args = Args(
        json!({"profile": "revival", "header": "len16be",
        "resolutions": [], "peers": []})
        .as_object()
        .unwrap()
        .clone(),
    );
    let tap = b"T 0 > 02000101\nT 1 < 0200020200\n";
    assert!(AdapterDigest.digest(tap, &args).is_ok());
    assert!(AdapterDigest
        .digest(b"T 0 > 02000101\nT 1 < 0200020201\n", &args)
        .is_err());
    let mut unused = args;
    unused.0.insert(
        "peers".into(),
        json!([{"ip": [127,0,0,1], "port": 80,
        "request_hex": "", "response_hex": ""}]),
    );
    assert!(AdapterDigest.digest(tap, &unused).is_err());
}

#[test]
fn replay_rejects_malformed_metadata_and_truncated_tap() {
    let tap = b"T 0 > 02000101\nT 1 < 0200020200\n";
    for values in [
        json!({}),
        json!({"profile": "revival", "header": "unknown"}),
        json!({"profile": "revival", "header": "len16be", "resolutions": [null], "peers": []}),
        json!({"profile": "revival", "header": "len16be", "resolutions": [], "peers": [null]}),
    ] {
        let args = Args(values.as_object().unwrap().clone());
        assert!(AdapterDigest.digest(tap, &args).is_err());
    }
    let args = Args(
        json!({"profile": "revival", "header": "len16be", "resolutions": [], "peers": []})
            .as_object()
            .unwrap()
            .clone(),
    );
    assert!(AdapterDigest
        .digest(b"T 0 > 020001\nT 1 < 0200020200\n", &args)
        .is_err());
}
