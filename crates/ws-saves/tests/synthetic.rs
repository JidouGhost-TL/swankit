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
        assert_eq!(format.definition()["schema"], "swankit-save-def/1");
        for block in format.definition()["blocks"].as_array().unwrap() {
            assert!(block["field"].as_u64().unwrap() < format.fields.len() as u64);
        }
    }
}
