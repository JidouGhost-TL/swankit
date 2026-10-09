//! External JSON-owned facts and golden digests; no corpus paths or populations here.
use formatkit_core::{Error, Result};
use formatkit_corpus_test::{hash_bytes, run_dir, verify_versioned_samples, Args, Digest};
use serde_json::{json, Value};
use std::cell::Cell;
use ws_rom::{catalog::probe, publisher_name, Mapper, Orientation, RomFacts, SaveDeclaration};

fn save_label(save: SaveDeclaration) -> String {
    let size = |kbit| {
        if kbit >= 1024 {
            format!("{}Mbit", kbit / 1024)
        } else {
            format!("{kbit}Kbit")
        }
    };
    match save {
        SaveDeclaration::None => "none".into(),
        SaveDeclaration::Sram { kbit } => format!("SRAM {}", size(kbit)),
        SaveDeclaration::Eeprom { kbit } => format!("EEPROM {}", size(kbit)),
        SaveDeclaration::Unknown(code) => format!("unknown 0x{code:02x}"),
    }
}

fn facts_value(bytes: &[u8], facts: RomFacts) -> Value {
    let h = facts.header;
    let declared = h
        .declared_rom_size()
        .map_or("?".into(), |size| format!("{}Mbit", size / 0x20000));
    let speed = h.rom_speed_cycles();
    let footer_hex: String = bytes[bytes.len() - ws_rom::FOOTER_SIZE..]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    json!({
        "publisher_id": format!("{:02x}", h.publisher_id),
        "publisher": publisher_name(h.publisher_id).unwrap_or("?"),
        "color": h.color(), "game_id": format!("{:02x}", h.game_id),
        "version": format!("{:02x}", h.version),
        "rom_size_code": format!("{:02x}", h.rom_size_code), "rom_size_decl": declared,
        "save_code": format!("{:02x}", h.save_code), "save": save_label(h.save()),
        "flags": format!("{:02x}", h.flags),
        "orientation": match h.orientation() { Orientation::Horizontal => "horizontal", Orientation::Vertical => "vertical" },
        "bus_width": h.bus_width().to_string(),
        "rom_speed": if speed == 1 { "1 cycle" } else { "2 cycles" },
        "mapper": match h.mapper() { Mapper::Bandai2001 => "2001", Mapper::Bandai2003 => "2003", Mapper::Unknown(_) => "?" },
        "checksum_footer": format!("{:04x}", h.checksum),
        "checksum_computed": format!("{:04x}", facts.checksum_computed),
        "checksum_ok": facts.checksum_ok(),
        "reset_seg_off": format!("{:04x}:{:04x}", h.reset_segment, h.reset_offset),
        "reset_linear": format!("{:05x}", h.reset_linear()), "footer_jmp_ok": true,
        "size_bytes": facts.geometry.file_size(), "size_pow2": facts.geometry.is_power_of_two(),
        "mapper_code": format!("{:02x}", h.mapper_code),
        "maintenance": format!("{:02x}", h.maintenance), "footer_hex": footer_hex,
        "sha256": hash_bytes(bytes),
    })
}

#[derive(Default)]
struct FactsDigest {
    raw_disagreements: Cell<usize>,
    reference_fields: Cell<usize>,
    checksum_mismatches: Cell<usize>,
    size_mismatches: Cell<usize>,
}

impl Digest for FactsDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let facts = RomFacts::parse(bytes)?;
        if !probe(bytes) {
            return Err(Error::Malformed(
                "valid corpus image did not match catalog probe".into(),
            ));
        }
        let got = facts_value(bytes, facts);
        let reference = args
            .get("reference_facts")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::Malformed("spec lacks independent reference facts".into()))?;
        let corrections = args
            .get("corrections")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::Malformed("spec lacks correction ledger".into()))?;
        if reference.len() + 1 != got.as_object().map_or(0, |object| object.len()) {
            return Err(Error::Malformed(
                "reference does not cover every fact field".into(),
            ));
        }
        for (field, expected) in reference {
            let actual = got
                .get(field)
                .ok_or_else(|| Error::Malformed(format!("unknown reference field {field}")))?;
            self.reference_fields.set(self.reference_fields.get() + 1);
            if actual != expected {
                let fix = corrections.get(field).ok_or_else(|| {
                    Error::Malformed(format!(
                        "unreviewed reference disagreement for {field}: {expected} -> {actual}"
                    ))
                })?;
                if fix.get("oracle") != Some(expected)
                    || fix.get("corrected") != Some(actual)
                    || fix
                        .get("reason")
                        .and_then(Value::as_str)
                        .is_none_or(str::is_empty)
                {
                    return Err(Error::Malformed(format!(
                        "incorrect correction ledger for {field}"
                    )));
                }
                self.raw_disagreements.set(self.raw_disagreements.get() + 1);
            } else if corrections.contains_key(field) {
                return Err(Error::Malformed(format!(
                    "stale correction ledger for {field}"
                )));
            }
        }
        self.checksum_mismatches
            .set(self.checksum_mismatches.get() + usize::from(!facts.checksum_ok()));
        self.size_mismatches
            .set(self.size_mismatches.get() + usize::from(!facts.declared_size_matches()));
        Ok(got)
    }
}

#[test]
fn corpus_facts() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen specs"
    );
    let handler = FactsDigest::default();
    let inspected = run_dir("ws-rom", &handler);
    let versioned = verify_versioned_samples("ws-rom", "sha256", "ws-rom", &["reference_sha256"]);
    assert_eq!(
        inspected, versioned,
        "every inspected sample must have provenance"
    );
    if inspected > 0 {
        eprintln!("ws-rom corpus: {inspected} ROMs; {} reference field comparisons; {} reviewed raw disagreements; {} checksum mismatches; {} declared/actual size mismatches; all golden digests and catalog probes passed",
            handler.reference_fields.get(), handler.raw_disagreements.get(), handler.checksum_mismatches.get(), handler.size_mismatches.get());
    }
}
