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
fn stored_edits_require_the_selected_mediums_full_image() {
    let mut d = definition();
    d["media"].as_array_mut().unwrap().push(json!({
        "kind":"internal-eeprom", "sizes":[1024]
    }));
    let mut companion = d["fields"][0].clone();
    companion["medium"] = json!("console IEEP");
    d["fields"].as_array_mut().unwrap().push(companion);
    let format = Format::from_json(&d.to_string()).unwrap();
    for length in [1, 127, 129, 1024] {
        let mut bytes = vec![0xa5; length];
        let before = bytes.clone();
        assert!(format.set_field(&mut bytes, 0, &json!(7)).is_err());
        assert_eq!(bytes, before);
    }
    let mut cartridge = vec![0xa5; 128];
    let before = cartridge.clone();
    assert!(format.set_field(&mut cartridge, 1, &json!(7)).is_err());
    assert_eq!(cartridge, before);
    let mut console = vec![0xa5; 1024];
    format.set_field(&mut console, 1, &json!(7)).unwrap();
    assert_eq!(console[0], 7);
    assert!(console[1..].iter().all(|v| *v == 0xa5));
}

#[test]
fn malformed_field_media_and_array_geometry_are_rejected() {
    for medium in ["unknown", "cart SRAM"] {
        let mut d = definition();
        d["fields"][0]["medium"] = json!(medium);
        assert!(Format::from_json(&d.to_string()).is_err());
    }
    let mut d = definition();
    d["fields"][0]["offset"] = json!(128);
    assert!(Format::from_json(&d.to_string()).is_err());
    for array in [
        json!({"count":0,"stride":1}),
        json!({"count":1,"stride":0}),
        json!({"count":2,"stride":1}),
        json!({"count":u64::MAX,"stride":2}),
        json!(null),
    ] {
        let mut d = definition();
        d["fields"][0]["array"] = array;
        assert!(Format::from_json(&d.to_string()).is_err());
    }
    let mut d = definition();
    d["fields"][0]["size"] = json!(4);
    d["fields"][0]["encoding"] = json!("u8[4]");
    d["fields"][0]["type"] = json!("bytes");
    d["fields"][0]["array"] = json!({"count":4,"stride":1});
    let format = Format::from_json(&d.to_string()).unwrap();
    let mut bytes = [0xa5; 128];
    let before = bytes;
    assert!(format
        .set_field(&mut bytes, 0, &json!([1, 2, 3, 256]))
        .is_err());
    assert_eq!(bytes, before);
    format
        .set_field(&mut bytes, 0, &json!([1, 2, 3, 4]))
        .unwrap();
    assert_eq!(&bytes[..4], &[1, 2, 3, 4]);
    assert_eq!(&bytes[4..], &before[4..]);
}

#[test]
fn corrected_packed_record_widths_preserve_bounds() {
    let bytes: Vec<_> = (0..32768).map(|n| (n ^ (n >> 8)) as u8).collect();
    for width in [109, 1714] {
        let mut d = definition();
        d["fields"] = json!([]);
        d["media"] = json!([{"kind":"cartridge-sram","sizes":[32768]}]);
        d["records"] = json!([{"name":"packed","width":width,"fields":[]}]);
        let format = Format::from_json(&d.to_string()).unwrap();
        assert_eq!(
            format.record(&bytes, "packed", 5, 1).unwrap(),
            &bytes[5 + width..5 + 2 * width]
        );
        assert!(format.record(&bytes, "packed", usize::MAX, 0).is_err());
        assert!(format.record(&bytes, "packed", 5, usize::MAX).is_err());
        assert!(format
            .record(&bytes, "packed", bytes.len() - width + 1, 0)
            .is_err());
    }
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
