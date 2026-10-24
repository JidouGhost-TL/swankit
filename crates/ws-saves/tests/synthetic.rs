use serde_json::{json, Value};
use ws_saves::{Field, Format};

fn definition() -> Value {
    json!({"schema":"swankit-save-def/1","id":"synthetic","implementation":"generated",
        "media":[{"kind":"cartridge-eeprom","sizes":[128]}],
        "fields":[{"name":"setting","medium":"cart EEPROM","offset":0,"size":1,
            "type":"unsigned","encoding":"u8","meaning":"setting","confidence":"synthetic","evidence":"synthetic"}],
        "records":[],"blocks":[],"checksums":{},"primitives":{},
        "validity":{"policy":"program","exclude":[]}, "entry":{"mode":"program"},
        "program":{"t":"Module","body":[],"type_ignores":[]}})
}

#[test]
fn stored_edits_preserve_unrelated_bytes_and_reject_overflow() {
    let format = Format::from_json(&definition().to_string()).unwrap();
    let mut bytes = [0xa5; 128];
    format.set_field(&mut bytes, 0, &json!(7)).unwrap();
    assert_eq!(bytes[0], 7);
    assert!(bytes[1..].iter().all(|v| *v == 0xa5));
    let before = bytes;
    assert!(format.set_field(&mut bytes, 0, &json!(256)).is_err());
    assert!(format.set_field(&mut bytes, usize::MAX, &json!(0)).is_err());
    assert_eq!(bytes, before);
    let field = Field {
        name: "raw".into(),
        medium: "cart EEPROM".into(),
        offset: usize::MAX,
        size: 2,
        encoding: "bytes".into(),
    };
    assert!(field.read(&bytes).is_err());
    assert!(field.write(&mut bytes, &json!([1, 2])).is_err());
    assert_eq!(bytes, before);
}

#[test]
fn malformed_definitions_and_failed_repairs_return_errors() {
    assert!(Format::from_json("[]").is_err());
    let mut d = definition();
    d["schema"] = "unsupported".into();
    assert!(Format::from_json(&d.to_string()).is_err());
    d = definition();
    d["fields"][0]["offset"] = json!(u64::MAX);
    assert!(Format::from_json(&d.to_string()).is_err());
    d = definition();
    d["records"] = json!([{"name":"empty","width":0}]);
    assert!(Format::from_json(&d.to_string()).is_err());
    let format = Format::from_json(&definition().to_string()).unwrap();
    let mut bytes = [0xa5; 128];
    let before = bytes;
    assert!(format.decode(&bytes, "save.eeprom").is_err());
    assert!(format.fix(&mut bytes, "save.eeprom").is_err());
    assert_eq!(bytes, before);
}

#[test]
fn bundled_definitions_have_schema_and_resolvable_block_indexes() {
    for format in ws_saves::formats().unwrap() {
        assert!(matches!(
            format.definition()["schema"].as_str(),
            Some("swankit-save-def/1" | "swankit-save-def/2")
        ));
        for block in format.definition()["blocks"].as_array().unwrap() {
            assert!(block["field"].as_u64().unwrap() < format.fields.len() as u64);
        }
    }
}

#[test]
fn v2_policy_definitions_reject_unknown_malformed_and_ambiguous_rules() {
    let mut v1 = definition();
    v1["fields"][0]["constraint"] = json!({"kind":"unsigned-range","minimum":0,"maximum":9});
    assert!(Format::from_json(&v1.to_string())
        .unwrap_err()
        .to_string()
        .contains("require swankit-save-def/2"));

    let mut unknown = definition();
    unknown["schema"] = json!("swankit-save-def/2");
    unknown["fields"][0]["constraint"] = json!({"kind":"clamp","maximum":9});
    assert!(Format::from_json(&unknown.to_string()).is_err());

    let mut enum_values = definition();
    enum_values["schema"] = json!("swankit-save-def/2");
    enum_values["fields"][0]["constraint"] = json!({"kind":"enum","values":[1,1]});
    assert!(Format::from_json(&enum_values.to_string()).is_err());

    let mut dangling = definition();
    dangling["schema"] = json!("swankit-save-def/2");
    dangling["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"other","medium":"cart EEPROM","offset":4,"size":1,"encoding":"u8"}));
    dangling["copy_groups"] = json!([{"name":"g","fields":["setting","absent"]}]);
    assert!(Format::from_json(&dangling.to_string())
        .unwrap_err()
        .to_string()
        .contains("dangling"));

    let mut ambiguous = dangling.clone();
    ambiguous["fields"].as_array_mut().unwrap()[1]["name"] = json!("setting");
    ambiguous["copy_groups"] = json!([{"name":"g","fields":["setting","other"]}]);
    assert!(Format::from_json(&ambiguous.to_string())
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));

    let mut overlap = definition();
    overlap["schema"] = json!("swankit-save-def/2");
    overlap["fields"][0]["size"] = json!(2);
    overlap["fields"][0]["encoding"] = json!("u16le");
    overlap["fields"].as_array_mut().unwrap().extend([
        json!({"name":"copy_b","medium":"cart EEPROM","offset":3,"size":2,"type":"unsigned","encoding":"u16le"}),
        json!({"name":"copy_c","medium":"cart EEPROM","offset":4,"size":2,"type":"unsigned","encoding":"u16le"}),
    ]);
    overlap["copy_groups"] = json!([{"name":"g","fields":["setting","copy_b","copy_c"]}]);
    assert!(Format::from_json(&overlap.to_string())
        .unwrap_err()
        .to_string()
        .contains("overlapping"));
}

#[test]
fn v2_packed_bcd_partial_digits_check_high_padding_in_both_byte_orders() {
    for (order, valid, invalid) in [
        ("little", json!([0x23, 0x01]), json!([0x30, 0x12])),
        ("big", json!([0x01, 0x23]), json!([0x12, 0x30])),
    ] {
        let mut d = definition();
        d["schema"] = json!("swankit-save-def/2");
        d["fields"][0]["size"] = json!(2);
        d["fields"][0]["type"] = json!("bytes");
        d["fields"][0]["encoding"] = json!("packed BCD");
        d["fields"][0]["constraint"] =
            json!({"kind":"packed-bcd","digits":3,"byte_order":order,"minimum":0,"maximum":999});
        let format = Format::from_json(&d.to_string()).unwrap();
        let mut bytes = [0; 128];
        format.set_field(&mut bytes, 0, &valid).unwrap();
        let expected = valid
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap() as u8)
            .collect::<Vec<_>>();
        assert_eq!(&bytes[..2], expected.as_slice());
        let before = bytes;
        assert!(format.set_field(&mut bytes, 0, &invalid).is_err());
        assert_eq!(bytes, before);
        format
            .set_field_with_options(&mut bytes, 0, &invalid, true)
            .unwrap();
    }
}

#[test]
fn format_set_field_enforces_constraints_unless_explicitly_skipped() {
    let mut d = definition();
    d["schema"] = json!("swankit-save-def/2");
    d["fields"][0]["constraint"] = json!({"kind":"unsigned-range","minimum":0,"maximum":9});
    let format = Format::from_json(&d.to_string()).unwrap();
    let mut bytes = [0; 128];
    assert!(format.set_field(&mut bytes, 0, &json!(10)).is_err());
    assert_eq!(bytes[0], 0);
    format
        .set_field_with_options(&mut bytes, 0, &json!(10), true)
        .unwrap();
    assert_eq!(bytes[0], 10);
    assert!(format
        .set_field_with_options(&mut bytes, 0, &json!(256), true)
        .is_err());
    assert_eq!(bytes[0], 10);
}

#[test]
fn v2_packed_bcd_digit_limit_matches_schema() {
    let mut d = definition();
    d["schema"] = json!("swankit-save-def/2");
    d["fields"][0]["size"] = json!(9);
    d["fields"][0]["type"] = json!("bytes");
    d["fields"][0]["encoding"] = json!("packed BCD");
    d["fields"][0]["constraint"] = json!({"kind":"packed-bcd","digits":16,"byte_order":"little","minimum":0,"maximum":9999999999999999u64});
    assert!(Format::from_json(&d.to_string()).is_ok());
    d["fields"][0]["constraint"]["digits"] = json!(17);
    assert!(Format::from_json(&d.to_string()).is_err());
}

#[test]
fn v2_unknown_properties_are_rejected_while_v1_remains_permissive() {
    let mut d = definition();
    d["schema"] = json!("swankit-save-def/2");
    d["fields"][0]["constraints"] = json!({"kind":"unsigned-range","minimum":0,"maximum":9});
    assert!(Format::from_json(&d.to_string()).is_err());
    let mut d = definition();
    d["schema"] = json!("swankit-save-def/2");
    d["fields"][0]["constraint_typo"] = json!({"kind":"unsigned-range","minimum":0,"maximum":9});
    assert!(Format::from_json(&d.to_string()).is_err());
    let mut d = definition();
    d["schema"] = json!("swankit-save-def/2");
    d["copy_group"] = json!([]);
    assert!(Format::from_json(&d.to_string()).is_err());
    let mut legacy = definition();
    legacy["fields"][0]["constraint_typo"] = json!(true);
    assert!(Format::from_json(&legacy.to_string()).is_ok());
}

#[test]
fn v2_packed_bcd_requires_byte_storage_even_when_validation_is_skipped() {
    let mut d = definition();
    d["schema"] = json!("swankit-save-def/2");
    d["fields"][0]["size"] = json!(1);
    d["fields"][0]["type"] = json!("bytes");
    d["fields"][0]["encoding"] = json!("u8");
    d["fields"][0]["constraint"] =
        json!({"kind":"packed-bcd","digits":2,"byte_order":"little","minimum":0,"maximum":99});
    assert!(Format::from_json(&d.to_string()).is_err());
}
