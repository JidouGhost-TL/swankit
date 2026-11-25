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
fn gather_beat_supply_points_enforce_the_documented_maximum() {
    let format = ws_saves::formats()
        .unwrap()
        .iter()
        .find(|format| format.definition()["id"] == "SD_Gundam_G_Generation_-_Gather_Beat_Japan")
        .unwrap();
    let mut bytes = vec![0; 32768];
    format.set_field(&mut bytes, 5, &json!(999_999)).unwrap();
    let before = bytes.clone();
    assert!(format.set_field(&mut bytes, 5, &json!(1_000_000)).is_err());
    assert_eq!(bytes, before);
    format
        .set_field_with_options(&mut bytes, 5, &json!(1_000_000), true)
        .unwrap();
    assert_eq!(&bytes[0x5006..0x500a], &1_000_000u32.to_le_bytes());
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

#[test]
fn enrolled_group_a_fixes_exclusions_and_preserved_constraints_hold() {
    let beatmania = ws_saves::format("beatmania_for_WonderSwan_Japan").unwrap();
    let mut sram = vec![0; 32768];
    beatmania.set_field(&mut sram, 34, &json!(0)).unwrap();
    beatmania.set_field(&mut sram, 34, &json!(3)).unwrap();
    assert!(beatmania.set_field(&mut sram, 34, &json!(4)).is_err());
    beatmania.set_field(&mut sram, 51, &json!(0)).unwrap();
    beatmania.set_field(&mut sram, 51, &json!(4)).unwrap();
    assert!(beatmania.set_field(&mut sram, 51, &json!(5)).is_err());

    let kiss = ws_saves::format("Kiss_Yori.._-_Seaside_Serenade_Japan_Rev_2").unwrap();
    kiss.set_field(&mut sram, 17, &json!(0)).unwrap();
    kiss.set_field(&mut sram, 17, &json!(4)).unwrap();
    assert!(kiss.set_field(&mut sram, 17, &json!(5)).is_err());

    let kishi = ws_saves::format("SD_Gundam_Eiyuu_Den_-_Kishi_Densetsu_Japan").unwrap();
    assert_eq!(kishi.fields[24].name, "script_result");
    kishi.set_field(&mut sram, 24, &json!(0)).unwrap();
    kishi.set_field(&mut sram, 24, &json!(4)).unwrap();
    kishi.set_field(&mut sram, 24, &json!(255)).unwrap();

    let sennou = ws_saves::format("Sennou_Millennium_Japan").unwrap();
    let mut eeprom = vec![0; 128];
    sennou
        .set_field(&mut eeprom, 4, &json!(1_476_242_430u32))
        .unwrap();
    sennou.set_field(&mut eeprom, 19, &json!(150)).unwrap();

    let frontier = ws_saves::format("Battle_Spirit_-_Digimon_Frontier_Japan_Rev_1").unwrap();
    assert_eq!(frontier.fields[16].name, "match_time");
    frontier.set_field(&mut sram, 16, &json!(0)).unwrap();
    frontier.set_field(&mut sram, 16, &json!(2)).unwrap();
    assert!(frontier.set_field(&mut sram, 16, &json!(3)).is_err());
}

#[test]
fn pro_mahjong_kiwame_streak_counters_split_into_independent_bcd_bytes() {
    let format = ws_saves::format("Pro_Mahjong_Kiwame_for_WonderSwan_Japan_Rev_1").unwrap();
    assert_eq!(format.fields[24].name, "streak counter, type A");
    assert_eq!(format.fields[24].offset, 0x122);
    assert_eq!(format.fields[24].size, 1);
    assert_eq!(format.fields[24].encoding, "BCD u8");
    assert_eq!(format.fields[25].name, "streak counter, type B");
    assert_eq!(format.fields[25].offset, 0x123);
    assert_eq!(format.fields[25].size, 1);
    assert_eq!(format.fields[25].encoding, "BCD u8");

    let block_names: Vec<&str> = format.definition()["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            format.fields[b["field"].as_u64().unwrap() as usize]
                .name
                .as_str()
        })
        .collect();
    assert_eq!(
        block_names,
        vec!["turn log", "kept hands 1-4", "replay slot"]
    );

    let mut sram = vec![0; 32768];
    format.set_field(&mut sram, 24, &json!([0x99])).unwrap();
    assert_eq!(sram[0x122], 0x99);
    assert_eq!(sram[0x123], 0x00);
    format.set_field(&mut sram, 25, &json!([0x42])).unwrap();
    assert_eq!(sram[0x122], 0x99);
    assert_eq!(sram[0x123], 0x42);

    assert!(format.set_field(&mut sram, 24, &json!([0x9a])).is_err());
    assert!(format.set_field(&mut sram, 25, &json!([0xff])).is_err());
    assert_eq!(sram[0x122], 0x99);
    assert_eq!(sram[0x123], 0x42);
}
