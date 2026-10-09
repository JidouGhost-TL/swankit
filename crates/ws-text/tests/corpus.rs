//! All real bytes, populations and oracle expectations are external JSON.
use formatkit_core::{bytes_at, Error, Result};
use formatkit_corpus_test::{hash_bytes, run_dir, Args, Digest};
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::Cell;
use ws_text::{builtin_definition, decode_standard, encode_standard, TextEncoding, TextLimits};

#[derive(Deserialize)]
struct StringOracle {
    offset: usize,
    length: usize,
    text: String,
    normalized: String,
    #[serde(default = "yes")]
    terminated: bool,
}
fn yes() -> bool {
    true
}
#[derive(Deserialize)]
struct FontOracle {
    font: String,
    count: usize,
    palette: Vec<[u8; 4]>,
}

#[derive(Deserialize)]
struct TableOracle {
    table: String,
    offsets: Vec<usize>,
    text_sha256: String,
}

#[derive(Deserialize)]
struct RejectedOracle {
    offset: usize,
    length: usize,
    error: String,
}

#[derive(Default)]
struct TextDigest {
    strings: Cell<usize>,
    glyphs: Cell<usize>,
    exact: Cell<usize>,
    canonical: Cell<usize>,
    rejected: Cell<usize>,
    table_entries: Cell<usize>,
}

impl Digest for TextDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let malformed = |message: &str| Error::Malformed(message.into());
        if args.get("operation").and_then(Value::as_str) == Some("standard") {
            let encoding: TextEncoding = serde_json::from_value(
                args.get("encoding")
                    .cloned()
                    .ok_or_else(|| malformed("spec lacks encoding"))?,
            )
            .map_err(|e| Error::Malformed(e.to_string()))?;
            let records: Vec<StringOracle> = serde_json::from_value(
                args.get("strings")
                    .cloned()
                    .ok_or_else(|| malformed("spec lacks strings"))?,
            )
            .map_err(|e| Error::Malformed(e.to_string()))?;
            let mut text_bytes = Vec::new();
            for r in &records {
                let input = bytes_at(bytes, r.offset, r.length)?;
                let text = decode_standard(encoding, input)?;
                if text != r.text {
                    return Err(Error::Malformed(format!(
                        "standard text differs at {:#x}",
                        r.offset
                    )));
                }
                if decode_standard(encoding, &encode_standard(encoding, &text)?)? != text {
                    return Err(malformed("standard round trip differs"));
                }
                text_bytes.extend_from_slice(text.as_bytes());
                text_bytes.push(0);
            }
            self.strings.set(self.strings.get() + records.len());
            return Ok(
                json!({"sha256":hash_bytes(bytes),"strings":records.len(),"text_sha256":hash_bytes(&text_bytes)}),
            );
        }
        let id = args
            .get("definition")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("spec lacks definition"))?;
        let definition = builtin_definition(id)?;
        let map = definition.character_map()?;
        if args.get("operation").and_then(Value::as_str) == Some("rejected") {
            let records: Vec<RejectedOracle> = serde_json::from_value(
                args.get("rejected")
                    .cloned()
                    .ok_or_else(|| malformed("spec lacks rejected records"))?,
            )
            .map_err(|e| Error::Malformed(e.to_string()))?;
            for record in &records {
                let input = bytes_at(bytes, record.offset, record.length)?;
                match map.decode(input, TextLimits::default()) {
                    Err(Error::Malformed(message)) if message.contains(&record.error) => (),
                    got => {
                        return Err(Error::Malformed(format!(
                            "expected diagnostic {:?}, got {got:?}",
                            record.error
                        )))
                    }
                }
            }
            self.rejected.set(self.rejected.get() + records.len());
            eprintln!(
                "ws-text {id}: {} independent unknown-control rejections passed",
                records.len()
            );
            return Ok(json!({"sha256":hash_bytes(bytes),"rejected":records.len()}));
        }
        if args.get("operation").and_then(Value::as_str) == Some("tables") {
            let records: Vec<TableOracle> = serde_json::from_value(
                args.get("tables")
                    .cloned()
                    .ok_or_else(|| malformed("spec lacks tables"))?,
            )
            .map_err(|e| Error::Malformed(e.to_string()))?;
            let mut entry_count = 0;
            let mut text_bytes = Vec::new();
            for record in &records {
                let table = definition
                    .tables
                    .get(&record.table)
                    .ok_or_else(|| malformed("unknown oracle table"))?;
                let dumped = table.dump(
                    bytes,
                    &map,
                    TextLimits {
                        output_bytes: 4 * 1024 * 1024,
                        tokens: 1024 * 1024,
                        ..TextLimits::default()
                    },
                    65536,
                )?;
                if dumped.iter().map(|s| s.offset).collect::<Vec<_>>() != record.offsets {
                    return Err(Error::Malformed(format!(
                        "pointer targets differ for {}",
                        record.table
                    )));
                }
                let mut table_text = Vec::new();
                for string in dumped {
                    table_text.extend_from_slice(string.decoded.text.as_bytes());
                    table_text.push(0);
                }
                if hash_bytes(&table_text) != record.text_sha256 {
                    return Err(Error::Malformed(format!(
                        "pointer table text differs for {}",
                        record.table
                    )));
                }
                entry_count += record.offsets.len();
                text_bytes.extend_from_slice(&table_text);
            }
            self.table_entries
                .set(self.table_entries.get() + entry_count);
            eprintln!(
                "ws-text {id}: {} tables / {entry_count} independently referenced entries passed",
                records.len()
            );
            return Ok(
                json!({"sha256":hash_bytes(bytes),"tables":records.len(),"entries":entry_count,"text_sha256":hash_bytes(&text_bytes)}),
            );
        }
        let records: Vec<StringOracle> = serde_json::from_value(
            args.get("strings")
                .cloned()
                .ok_or_else(|| malformed("spec lacks strings"))?,
        )
        .map_err(|e| Error::Malformed(e.to_string()))?;
        let fonts: Vec<FontOracle> = serde_json::from_value(
            args.get("fonts")
                .cloned()
                .ok_or_else(|| malformed("spec lacks fonts"))?,
        )
        .map_err(|e| Error::Malformed(e.to_string()))?;
        let mut text_bytes = Vec::new();
        let mut normalized_bytes = Vec::new();
        let mut total_bytes = 0;
        for r in &records {
            let input = bytes_at(bytes, r.offset, r.length)?;
            let decoded = map.decode(input, TextLimits::default())?;
            if decoded.text != r.text
                || decoded.normalized != r.normalized
                || decoded.consumed != r.length
                || decoded.terminated != r.terminated
            {
                return Err(Error::Malformed(format!("text oracle differs for {id} at {:#x}: got {:?}, expected {:?}; consumed {}/{}; normalized {:?}/{:?}",r.offset,decoded.text,r.text,decoded.consumed,r.length,decoded.normalized,r.normalized)));
            }
            let exact = map.encode_tokens(&decoded.tokens, TextLimits::default())?;
            if exact != input {
                return Err(malformed("source token encoding differs"));
            }
            let canonical = map.encode(&decoded.text, 65536)?;
            let decoded_again = map.decode(&canonical, TextLimits::default())?;
            if decoded_again.text != decoded.text {
                return Err(malformed("readable encoding round trip differs"));
            }
            text_bytes.extend_from_slice(decoded.text.as_bytes());
            text_bytes.push(0);
            normalized_bytes.extend_from_slice(decoded.normalized.as_bytes());
            normalized_bytes.push(0);
            total_bytes += r.length;
        }
        let mut rendered = Vec::new();
        let mut total_glyphs = 0;
        for f in fonts {
            let font = definition
                .fonts
                .get(&f.font)
                .ok_or_else(|| malformed("unknown oracle font"))?;
            if font.count != f.count {
                return Err(malformed("font oracle count differs"));
            }
            let mut rgba = Vec::new();
            let mut advances = Vec::new();
            for index in 0..f.count {
                let glyph = font.glyph(bytes, index, &f.palette, 65536)?;
                rgba.extend_from_slice(&glyph.image.rgba);
                advances.push(glyph.advance);
            }
            total_glyphs += f.count;
            rendered.push(json!({"font":f.font,"count":f.count,"rgba_sha256":hash_bytes(&rgba),"advances":advances}));
        }
        self.strings.set(self.strings.get() + records.len());
        self.exact.set(self.exact.get() + records.len());
        self.canonical.set(self.canonical.get() + records.len());
        self.glyphs.set(self.glyphs.get() + total_glyphs);
        eprintln!(
            "ws-text {id}: {} strings and {total_glyphs} glyphs passed",
            records.len()
        );
        Ok(
            json!({"sha256":hash_bytes(bytes),"strings":records.len(),"string_bytes":total_bytes,
            "text_sha256":hash_bytes(&text_bytes),"normalized_sha256":hash_bytes(&normalized_bytes),
            "token_round_trips":records.len(),"canonical_round_trips":records.len(),"fonts":rendered}),
        )
    }
}

#[test]
fn corpus_text_and_fonts() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen text specs"
    );
    let handler = TextDigest::default();
    let inspected = run_dir("ws-text", &handler) + run_dir("ws-text-extra", &handler);
    if inspected > 0 {
        eprintln!("ws-text corpus: {inspected} sources; {} decoded strings; {} byte-exact token round trips; {} readable encode/decode round trips; {} glyph images; {} pointer-table entries; {} unknown-control rejections; all independent golden values passed",
            handler.strings.get(), handler.exact.get(), handler.canonical.get(), handler.glyphs.get(), handler.table_entries.get(), handler.rejected.get());
    }
}
