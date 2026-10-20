//! Frozen external pairs and Java expectations; no title names or counts here.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{hash_bytes, load_pairs, run_dir, Args, Digest, Pair};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use ws_compress::{lz_bits_16, lz_rle_7f, DecodeOutcome};

fn decode(codec: &str, stored: &[u8], limit: usize) -> Result<DecodeOutcome> {
    match codec {
        "lz-rle-7f" => lz_rle_7f::decode(stored, limit),
        "lz-bits-16" => lz_bits_16::decode(stored, limit),
        _ => Err(Error::Unsupported(format!("codec {codec}"))),
    }
}
fn encode(codec: &str, raw: &[u8]) -> Result<Vec<u8>> {
    match codec {
        "lz-rle-7f" => lz_rle_7f::encode(raw),
        "lz-bits-16" => lz_bits_16::encode(raw),
        _ => Err(Error::Unsupported(format!("codec {codec}"))),
    }
}

#[derive(Default)]
struct Counts {
    samples: usize,
    captured: usize,
    raw: usize,
    stored: usize,
    encoded: usize,
    smaller: usize,
    equal: usize,
    larger: usize,
}
#[derive(Default)]
struct CodecDigest(RefCell<BTreeMap<String, Counts>>);

impl Digest for CodecDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let pair =
            Pair::from_bytes(bytes).ok_or_else(|| Error::Malformed("invalid codec pair".into()))?;
        if args.get("codec").and_then(Value::as_str) != Some(&pair.codec) {
            return Err(Error::Malformed(
                "pair codec differs from frozen selection".into(),
            ));
        }
        let got = decode(&pair.codec, &pair.stored, pair.raw.len())?;
        if got.bytes != pair.raw || got.consumed != pair.stored.len() {
            return Err(Error::Malformed(format!(
                "decode/extent parity: {}",
                pair.provenance
            )));
        }
        let packed = encode(&pair.codec, &pair.raw)?;
        let back = decode(&pair.codec, &packed, pair.raw.len())?;
        if back.bytes != pair.raw || back.consumed != packed.len() {
            return Err(Error::Malformed(format!(
                "encoder round trip: {}",
                pair.provenance
            )));
        }
        let fits = packed.len() <= pair.stored.len();
        if args.get("require_fit").and_then(Value::as_bool) == Some(true) && !fits {
            return Err(Error::Malformed(format!(
                "encoded stream exceeds required slot: {}",
                pair.provenance
            )));
        }
        let mut totals = self.0.borrow_mut();
        let count = totals.entry(pair.codec.clone()).or_default();
        count.samples += 1;
        count.captured +=
            usize::from(args.get("evidence").and_then(Value::as_str) == Some("captured-ram"));
        count.raw += pair.raw.len();
        count.stored += pair.stored.len();
        count.encoded += packed.len();
        count.smaller += usize::from(packed.len() < pair.stored.len());
        count.equal += usize::from(packed.len() == pair.stored.len());
        count.larger += usize::from(packed.len() > pair.stored.len());
        Ok(json!({
            "codec": pair.codec, "raw_len": pair.raw.len(), "stored_len": pair.stored.len(),
            "raw_sha256": hash_bytes(&got.bytes), "stored_sha256": hash_bytes(&pair.stored),
            "consumed": got.consumed, "encoded_len": packed.len(),
            "encoded_sha256": hash_bytes(&packed), "fits_original": fits,
        }))
    }
}

#[test]
fn corpus_codec_parity() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen specs"
    );
    let handler = CodecDigest::default();
    let samples = run_dir("ws-compress", &handler);
    if samples == 0 {
        return;
    }
    let pair_root =
        formatkit_corpus_test::compress_oracle::pair_root().expect("enrolled pair root");
    let mut counted = 0;
    for (codec, totals) in handler.0.borrow().iter() {
        let pairs = load_pairs(&pair_root, codec);
        assert!(
            !pairs.is_empty(),
            "golden samples must have Formatkit oracle pairs"
        );
        for pair in &pairs {
            let got = decode(codec, &pair.stored, pair.raw.len()).unwrap();
            assert_eq!(got.bytes, pair.raw, "{}", pair.provenance);
            assert_eq!(got.consumed, pair.stored.len(), "{}", pair.provenance);
        }
        counted += totals.samples;
        eprintln!("ws-compress {codec}: {} records, {} unique Formatkit pairs, {} captured RAM records; all Java decode bytes/extents, encoder bytes/sizes, and round trips identical; raw={} stored={} encoded={}; smaller={} equal={} larger={}",
            totals.samples, pairs.len(), totals.captured, totals.raw, totals.stored, totals.encoded, totals.smaller, totals.equal, totals.larger);
    }
    assert_eq!(counted, samples);
}

struct AdditionalCodecDigest;
impl Digest for AdditionalCodecDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let pair =
            Pair::from_bytes(bytes).ok_or_else(|| Error::Malformed("invalid codec pair".into()))?;
        let destination = args
            .get("destination")
            .and_then(Value::as_u64)
            .unwrap_or(0x2000);
        let destination = u16::try_from(destination)
            .map_err(|_| Error::Malformed("destination outside segment".into()))?;
        let policy = match args
            .get("prehistory")
            .and_then(Value::as_str)
            .unwrap_or("low5-prefix")
        {
            "low5-prefix" => ws_compress::lzss_u16_zero::Prehistory::Low5Prefix,
            "low8-prefix" => ws_compress::lzss_u16_zero::Prehistory::Low8Prefix,
            "zero-until-start" => ws_compress::lzss_u16_zero::Prehistory::ZeroUntilStart,
            _ => return Err(Error::Malformed("unknown prehistory policy".into())),
        };
        let (got, encoded) = match pair.codec.as_str() {
            "lzss-u16-zero" => (
                ws_compress::lzss_u16_zero::decode_with_policy(
                    &pair.stored,
                    pair.raw.len(),
                    destination,
                    policy,
                )?,
                ws_compress::lzss_u16_zero::encode(&pair.raw)?,
            ),
            "lzss-12" => (
                ws_compress::lzss_12::decode(&pair.stored, pair.raw.len(), pair.raw.len())?,
                ws_compress::lzss_12::encode(&pair.raw)?,
            ),
            _ => return Err(Error::Unsupported(pair.codec)),
        };
        if args.get("codec").and_then(Value::as_str) != Some(&pair.codec)
            || got.bytes != pair.raw
            || got.consumed != pair.stored.len()
        {
            return Err(Error::Malformed(
                "captured decompressor output/extent mismatch".into(),
            ));
        }
        let back = match pair.codec.as_str() {
            "lzss-u16-zero" => ws_compress::lzss_u16_zero::decode(&encoded, pair.raw.len())?,
            _ => ws_compress::lzss_12::decode(&encoded, pair.raw.len(), pair.raw.len())?,
        };
        if back.bytes != pair.raw || back.consumed != encoded.len() {
            return Err(Error::Malformed("new encoder round trip".into()));
        }
        Ok(
            json!({"codec":pair.codec,"raw_sha256":hash_bytes(&got.bytes),"stored_sha256":hash_bytes(&pair.stored),"consumed":got.consumed,"round_trip":true}),
        )
    }
}
#[test]
fn corpus_additional_codecs() {
    assert_ne!(std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(), Ok("1"));
    let n = run_dir("ws-compress-extra", &AdditionalCodecDigest);
    eprintln!("ws-compress-extra: {n} game-decompressor witnesses equal; encoders round-trip");
}

struct U8ZeroDigest;
impl Digest for U8ZeroDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let pair =
            Pair::from_bytes(bytes).ok_or_else(|| Error::Malformed("invalid codec pair".into()))?;
        if pair.codec != "lz-u8-zero"
            || args.get("codec").and_then(Value::as_str) != Some(&pair.codec)
        {
            return Err(Error::Malformed("unexpected byte-command codec".into()));
        }
        let decoded = ws_compress::lz_u8_zero::decode(&pair.stored, pair.raw.len())?;
        if decoded.bytes != pair.raw || decoded.consumed != pair.stored.len() {
            return Err(Error::Malformed(
                "byte-command output/extent mismatch".into(),
            ));
        }
        let encoded = ws_compress::lz_u8_zero::encode(&pair.raw)?;
        let back = ws_compress::lz_u8_zero::decode(&encoded, pair.raw.len())?;
        if back.bytes != pair.raw || back.consumed != encoded.len() {
            return Err(Error::Malformed("byte-command encoder round trip".into()));
        }
        Ok(
            json!({"codec":pair.codec,"raw_sha256":hash_bytes(&decoded.bytes),
            "stored_sha256":hash_bytes(&pair.stored),"consumed":decoded.consumed,"round_trip":true}),
        )
    }
}

#[test]
fn corpus_zero_terminated_byte_codec() {
    assert_ne!(std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(), Ok("1"));
    let n = run_dir("ws-compress-u8-zero", &U8ZeroDigest);
    eprintln!(
        "ws-compress-u8-zero: {n} source streams equal independent complete output and extent"
    );
}
