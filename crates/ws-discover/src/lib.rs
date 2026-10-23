//! Evidence-driven draft maps. Runtime associations are candidates, never proof.
#![forbid(unsafe_code)]
pub mod search;
use formatkit_core::{bytes_at, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use ws_assets::{map::Reference, AssetMap, Charmaps};

pub const SCHEMA: &str = "swankit-discovery-evidence/1";
pub const MAX_OBSERVATIONS: usize = 32_768;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub source: usize,
    pub routine: String,
    #[serde(default)]
    pub destination: Option<usize>,
    /// Actual decoder offset register when its segmented address matches the capture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec_destination: Option<u16>,
    /// Known transfer length, not a codec's claimed length.
    #[serde(default)]
    pub length: Option<usize>,
    #[serde(default)]
    pub decoded_length: Option<usize>,
    /// Independent routine-exit witness, if the final snapshot overwrote it.
    #[serde(default)]
    pub output: Option<Vec<u8>>,
    #[serde(default)]
    pub prehistory: Option<String>,
    #[serde(default)]
    pub references: Vec<Reference>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextEntry {
    pub offset: usize,
    pub length: usize,
    pub charmap: Value,
    pub routine: String,
    #[serde(default)]
    pub references: Vec<Reference>,
    #[serde(default)]
    pub font: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub schema: String,
    /// Optional previous draft or adapter-produced raw snapshot map.
    pub seed: Value,
    pub ram: Vec<u8>,
    pub ports: Vec<u8>,
    #[serde(default)]
    pub observations: Vec<Observation>,
    #[serde(default)]
    pub texts: Vec<TextEntry>,
    /// Tilemaps whose tile slots refer to this snapshot's VRAM placement.
    #[serde(default)]
    pub snapshot_maps: Vec<String>,
    /// An external data profile, for human provenance only.
    #[serde(default)]
    pub family: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub codec_proofs: Vec<CodecProof>,
    pub observed_sources: usize,
    pub accepted_raw: usize,
    pub accepted_compressed: usize,
    pub text_entries: usize,
    pub references: usize,
    pub rejected: Vec<String>,
    pub matched_codecs: Vec<String>,
    pub mapped_bytes: usize,
    pub rom_bytes: usize,
    pub explained_tile_bytes: usize,
    pub visible_tile_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodecProof {
    pub codec: String,
    pub source: usize,
    pub destination: usize,
    pub stored_length: usize,
    pub decoded_length: usize,
    pub decoded_sha256: String,
    pub routine: String,
    pub representable_in_map: bool,
    pub prehistory: Option<String>,
}

pub struct Discovery {
    pub map: AssetMap,
    pub report: Report,
}

fn bad(s: impl Into<String>) -> Error {
    Error::Malformed(s.into())
}
fn number(v: &Value) -> Option<usize> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .or_else(|| {
            v.as_str().and_then(|s| {
                s.strip_prefix("0x")
                    .map_or_else(|| s.parse().ok(), |h| usize::from_str_radix(h, 16).ok())
            })
        })
}
fn useful(bytes: &[u8]) -> bool {
    bytes.len() >= 32 && bytes.iter().copied().collect::<BTreeSet<_>>().len() >= 4
}
fn locate(haystack: &[u8], bytes: &[u8], destination: Option<usize>) -> Option<usize> {
    if bytes.is_empty() || bytes.len() > haystack.len() {
        return None;
    }
    if let Some(d) = destination {
        if haystack.get(d..d.checked_add(bytes.len())?) == Some(bytes) {
            return Some(d);
        }
    }
    haystack.windows(bytes.len()).position(|w| w == bytes)
}
fn locate_witness(evidence: &Evidence, o: &Observation, bytes: &[u8]) -> Option<usize> {
    if let Some(output) = &o.output {
        if output != bytes {
            return None;
        }
        let dest = o.destination?;
        (dest.checked_add(bytes.len())? <= 65536).then_some(dest)
    } else {
        locate(&evidence.ram, bytes, o.destination)
    }
}

/// Codec acceptance requires complete decoder output equality to captured RAM.
/// Seeds are identity checked, references validated, and relocation stays disabled.
fn add_asset(document: &mut Value, asset: Value) -> Result<()> {
    let assets = document["assets"]
        .as_array_mut()
        .ok_or_else(|| bad("assets is not array"))?;
    if !assets.iter().any(|a| a["id"] == asset["id"]) {
        assets.push(asset);
    }
    Ok(())
}

pub fn discover(rom: &[u8], evidence: &Evidence) -> Result<Discovery> {
    if evidence.schema != SCHEMA {
        return Err(bad("unsupported discovery evidence schema"));
    }
    if evidence.ram.len() != 65536 || evidence.ports.len() != 256 {
        return Err(bad("snapshot requires 65536 RAM bytes and 256 ports"));
    }
    if evidence.observations.len() > MAX_OBSERVATIONS || evidence.texts.len() > MAX_OBSERVATIONS {
        return Err(bad("observation population limit"));
    }
    let seed =
        AssetMap::from_json(&serde_json::to_vec(&evidence.seed).map_err(|e| bad(e.to_string()))?)?;
    ws_assets::check_identity(&seed, rom)?;
    let mut document = evidence.seed.clone();
    let mut report = Report {
        observed_sources: evidence.observations.len(),
        rom_bytes: rom.len(),
        ..Report::default()
    };
    let mode = evidence.ports[0x60];
    let tb = if mode & 0x40 != 0 { 32 } else { 16 };
    let base = if tb == 32 { 0x4000 } else { 0x2000 };
    let mut signatures = BTreeSet::new();
    let mut new_sources = BTreeSet::new();
    for o in &evidence.observations {
        if o.source >= rom.len() {
            report
                .rejected
                .push(format!("source {:#x} outside ROM", o.source));
            continue;
        }
        if o.output.as_ref().is_some_and(|b| b.len() > 65536) {
            return Err(bad("routine witness exceeds RAM"));
        }
        let mut candidates = Vec::new();
        if let Some(n) = o.length.filter(|n| *n <= 65536 && *n >= 32) {
            if let Ok(raw) = bytes_at(rom, o.source, n) {
                candidates.push(("raw", raw.to_vec(), n));
            }
        }
        let window = &rom[o.source..rom.len().min(o.source.saturating_add(65536))];
        if let Ok(d) = ws_compress::lz_rle_7f::decode(window, 65536) {
            candidates.push(("ws-lz-rle-7f", d.bytes, d.consumed));
        }
        if let Ok(d) = ws_compress::lz_bits_16::decode(window, 65536) {
            candidates.push(("ws-lz-bits-16", d.bytes, d.consumed));
        }
        if let Some(size) = o.decoded_length.filter(|s| *s > 0 && *s <= 65536) {
            if let Ok(d) = ws_compress::okumura::decode(
                window,
                size,
                ws_compress::BitOrder::LsbFirst,
                0x20,
                65536,
            ) {
                candidates.push(("ws-okumura-lzss", d.bytes, d.consumed));
            }
            // The final snapshot cannot prove the history at decoder entry.
            // History-dependent streams need a separate entry-state witness.
            if let Ok(d) = ws_compress::lzss_12::decode(window, size, 65536) {
                if useful(&d.bytes) {
                    if let Some(dest) = locate_witness(evidence, o, &d.bytes) {
                        if new_sources.insert(o.source) {
                            report.codec_proofs.push(CodecProof {
                                codec: "ws-lzss-12".into(),
                                source: o.source,
                                destination: dest,
                                stored_length: d.consumed,
                                decoded_length: d.bytes.len(),
                                decoded_sha256: ws_assets::sha256_hex(&d.bytes),
                                routine: o.routine.clone(),
                                representable_in_map: true,
                                prehistory: None,
                            });
                            add_asset(
                                &mut document,
                                json!({"id":format!("codec-source.{:x}",o.source),"kind":"other","offset":o.source,"length":d.consumed,"codec":"raw","confidence":"high","references_complete":false,"references":o.references,"evidence":{"loader":o.routine,"trace":format!("ws-lzss-12 output equals RAM {dest:#x}; see discovery report"),"notes":"Stored compressed bytes only; codec enum addition requested."}}),
                            )?;
                            candidates.push(("ws-lzss-12", d.bytes, d.consumed));
                        }
                    }
                }
            }
        }
        let policy = match o.prehistory.as_deref().unwrap_or("low5-prefix") {
            "low5-prefix" => ws_compress::lzss_u16_zero::Prehistory::Low5Prefix,
            "low8-prefix" => ws_compress::lzss_u16_zero::Prehistory::Low8Prefix,
            "zero-until-start" => ws_compress::lzss_u16_zero::Prehistory::ZeroUntilStart,
            _ => return Err(bad("unknown LZSS prehistory policy")),
        };
        let initial_destination = o
            .codec_destination
            .or_else(|| o.destination.and_then(|d| u16::try_from(d).ok()))
            .unwrap_or(0x2000);
        // A temporal RAM-write hint is not the decoder's original DI. Retry
        // the canonical aligned destination if the hint produces no witness;
        // the actual located destination must still reproduce every byte.
        let mut destinations = vec![initial_destination];
        if o.codec_destination.is_none() && initial_destination != 0x2000 {
            destinations.push(0x2000);
        }
        for initial_destination in destinations {
            let Ok(d) = ws_compress::lzss_u16_zero::decode_with_policy(
                window,
                65536,
                initial_destination,
                policy,
            ) else {
                continue;
            };
            if useful(&d.bytes) {
                if let Some(dest) = locate_witness(evidence, o, &d.bytes) {
                    let Ok(checked) = ws_compress::lzss_u16_zero::decode_with_policy(
                        window,
                        65536,
                        o.codec_destination.unwrap_or(dest as u16),
                        policy,
                    ) else {
                        continue;
                    };
                    if checked.bytes != d.bytes || !new_sources.insert(o.source) {
                        continue;
                    }
                    report.codec_proofs.push(CodecProof {
                        codec: "ws-lzss-u16-zero".into(),
                        source: o.source,
                        destination: dest,
                        stored_length: d.consumed,
                        decoded_length: d.bytes.len(),
                        decoded_sha256: ws_assets::sha256_hex(&d.bytes),
                        routine: o.routine.clone(),
                        representable_in_map: true,
                        prehistory: o.prehistory.clone(),
                    });
                    // Retain the immutable stored-byte baseline alongside the decoded asset.
                    add_asset(
                        &mut document,
                        json!({"id":format!("codec-source.{:x}",o.source),"kind":"other","offset":o.source,"length":d.consumed,"codec":"raw","confidence":"high","references_complete":false,"references":o.references,"evidence":{"loader":o.routine,"trace":format!("ws-lzss-u16-zero output equals RAM {dest:#x}; see discovery report"),"notes":"Stored compressed bytes only; codec enum addition requested."}}),
                    )?;
                    candidates.push(("ws-lzss-u16-zero", d.bytes, d.consumed));
                    break;
                }
            }
        }
        let mut accepted = new_sources.contains(&o.source);
        for (codec, bytes, stored) in candidates {
            if !useful(&bytes) {
                continue;
            }
            let Some(dest) = locate_witness(evidence, o, &bytes) else {
                continue;
            };
            if !signatures.insert((o.source, stored, dest)) {
                accepted = true;
                continue;
            }
            let end = dest + bytes.len();
            let tiles = dest >= base
                && end <= if tb == 32 { 0xc000 } else { 0x6000 }
                && (dest - base) % tb == 0
                && bytes.len() % tb == 0;
            let id = format!("trace.{codec}.{:x}.{dest:x}.{stored:x}", o.source);
            if document["assets"]
                .as_array()
                .is_some_and(|a| a.iter().any(|a| a["id"] == id))
            {
                accepted = true;
                continue;
            }
            let format = if tiles {
                json!({"bpp":if tb==32 {4}else{2},"layout":if tb==32 && mode&0x20!=0 {"packed"}else{"planar"},"width":16})
            } else {
                json!({})
            };
            let codec_params = match codec {
                "ws-okumura-lzss" => json!({"decoded_length": bytes.len()}),
                "ws-lzss-12" => json!({"decoded_length": bytes.len(), "history": []}),
                "ws-lzss-u16-zero" => {
                    json!({"destination": o.codec_destination.map_or(dest, usize::from), "prehistory": o.prehistory.as_deref().unwrap_or("low5-prefix")})
                }
                _ => json!({}),
            };
            let asset = json!({"id":id,"kind":if tiles {"tiles"}else{"other"},"offset":o.source,"length":stored,"codec":codec,"codec_params":codec_params,"format":format,"confidence":"high","references_complete":false,"references":o.references,"evidence":{"loader":o.routine,"trace":format!("complete {codec} decode equals captured RAM {dest:#x}..{end:#x}"),"notes":"A complete byte match establishes this payload, not a general taint relationship or exhaustive references."}});
            document["assets"]
                .as_array_mut()
                .ok_or_else(|| bad("assets is not array"))?
                .push(asset);
            if tiles && o.output.is_none() {
                if let Some(assets) = document["assets"].as_array_mut() {
                    for a in assets.iter_mut().filter(|a| {
                        a["kind"] == "tilemap"
                            && evidence.snapshot_maps.iter().any(|id| a["id"] == *id)
                            && a["format"]["entry"].as_str().unwrap_or("ws-word") == "ws-word"
                    }) {
                        let rel = a
                            .as_object_mut()
                            .ok_or_else(|| bad("asset not object"))?
                            .entry("relations")
                            .or_insert(json!({}));
                        let ts = rel
                            .as_object_mut()
                            .ok_or_else(|| bad("relations not object"))?
                            .entry("tiles")
                            .or_insert(json!([]));
                        ts.as_array_mut()
                            .ok_or_else(|| bad("tiles not array"))?
                            .push(json!({"id":id,"at":(dest-base)/tb}));
                    }
                }
            }
            report.references += o.references.len();
            if codec == "raw" {
                report.accepted_raw += 1;
            } else {
                report.accepted_compressed += 1;
                report.matched_codecs.push(codec.into());
            }
            accepted = true;
            break;
        }
        if !accepted {
            report.rejected.push(format!(
                "{:#x}: no complete useful decode equals snapshot",
                o.source
            ));
        }
    }
    for (i, t) in evidence.texts.iter().enumerate() {
        bytes_at(rom, t.offset, t.length)?;
        let id = format!("text.{:x}.{i}", t.offset);
        let charmap = format!("charmap.{i}");
        if document.get("charmaps").is_none() {
            document["charmaps"] = json!({});
        }
        document["charmaps"][&charmap] = if t.charmap.get("schema").is_some() {
            json!({"inline":t.charmap})
        } else {
            t.charmap.clone()
        };
        let mut relations = json!({"charmap":charmap});
        if let Some(f) = &t.font {
            relations["font"] = json!(f);
        }
        add_asset(
            &mut document,
            json!({"id":id,"kind":"text","offset":t.offset,"length":t.length,"relations":relations,"references":t.references,"references_complete":false,"confidence":"low","evidence":{"loader":t.routine,"notes":"External charmap/profile and observed source; reading a string does not prove dialogue semantics."}}),
        )?;
        report.text_entries += 1;
        report.references += t.references.len();
    }
    // Declare overlaps explicitly. Neither a duplicate nor an overlap proves free space.
    let assets = document["assets"]
        .as_array_mut()
        .ok_or_else(|| bad("assets not array"))?;
    let spans: Vec<_> = assets
        .iter()
        .map(|a| {
            (
                number(&a["offset"]),
                number(&a["length"]),
                a["id"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    for i in 0..assets.len() {
        for j in i + 1..assets.len() {
            if let (Some(a), Some(n), Some(b), Some(m)) =
                (spans[i].0, spans[i].1, spans[j].0, spans[j].1)
            {
                if a < b.saturating_add(m) && b < a.saturating_add(n) {
                    for (x, y) in [(i, j), (j, i)] {
                        if assets[x].get("overlaps").is_none() {
                            assets[x]["overlaps"] = json!([]);
                        }
                        let list = assets[x]["overlaps"]
                            .as_array_mut()
                            .ok_or_else(|| bad("overlaps not array"))?;
                        if !list.iter().any(|v| v.as_str() == Some(&spans[y].2)) {
                            list.push(json!(spans[y].2));
                        }
                    }
                }
            }
        }
    }
    let map = AssetMap::from_json(&serde_json::to_vec(&document).map_err(|e| bad(e.to_string()))?)?;
    let cm = Charmaps::load(&map, |_| {
        Err(bad("discovery uses inline or builtin charmaps"))
    })?;
    let verify = ws_assets::verify(&map, &cm, rom)?;
    if !verify.ok() {
        return Err(bad(format!(
            "draft verification failed: {:?}",
            verify.errors
        )));
    }
    report.mapped_bytes = verify.mapped_bytes;
    report.references = verify.references;
    report.text_entries = map
        .assets
        .iter()
        .filter(|a| a.kind == ws_assets::Kind::Text)
        .count();
    report.matched_codecs.sort();
    report.matched_codecs.dedup();
    let mut slots = BTreeSet::new();
    for (enable, shift, sx, sy) in [(1, 0, 0x10, 0x11), (2, 4, 0x12, 0x13)] {
        if evidence.ports[0] & enable == 0 {
            continue;
        }
        let mb = ((evidence.ports[7] >> shift) & 15) as usize * 2048;
        for y in 0..18 {
            for x in 0..28 {
                let row = (evidence.ports[sy] as usize / 8 + y) % 32;
                let col = (evidence.ports[sx] as usize / 8 + x) % 32;
                let p = mb + 2 * (row * 32 + col);
                let w = u16::from_le_bytes([evidence.ram[p], evidence.ram[p + 1]]);
                let slot = (w & 511) as usize
                    | if mode & 128 != 0 {
                        (w as usize & 0x2000) >> 4
                    } else {
                        0
                    };
                slots.insert(slot);
            }
        }
    }
    report.visible_tile_bytes = slots.len() * tb;
    let mut payloads = Vec::new();
    for a in &map.assets {
        if a.kind == ws_assets::Kind::Tiles {
            if let Ok(d) = ws_assets::decode_asset(&map, &cm, rom, &a.id) {
                payloads.push(d.decoded);
            }
        }
    }
    // Include legacy proofs as well as decoded assets; byte membership is a union.
    for p in &report.codec_proofs {
        let stored = bytes_at(rom, p.source, p.stored_length)?;
        let d = if p.codec == "ws-lzss-u16-zero" {
            let policy = match p.prehistory.as_deref() {
                Some("low8-prefix") => ws_compress::lzss_u16_zero::Prehistory::Low8Prefix,
                Some("zero-until-start") => ws_compress::lzss_u16_zero::Prehistory::ZeroUntilStart,
                _ => ws_compress::lzss_u16_zero::Prehistory::Low5Prefix,
            };
            ws_compress::lzss_u16_zero::decode_with_policy(
                stored,
                65536,
                p.destination as u16,
                policy,
            )?
        } else {
            let history = &[];
            ws_compress::lzss_12::decode_with_history(stored, history, p.decoded_length, 65536)?
        };
        payloads.push(d.bytes);
    }
    for s in slots {
        if let Some(t) = evidence.ram.get(base + s * tb..base + (s + 1) * tb) {
            if payloads.iter().any(|p| p.chunks_exact(tb).any(|q| q == t)) {
                report.explained_tile_bytes += tb;
            }
        }
    }
    Ok(Discovery { map, report })
}
