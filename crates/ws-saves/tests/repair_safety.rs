use serde_json::json;
use ws_saves::Format;

mod review;

fn direct(source: &str) -> Format {
    let mut definition = review::format(source).definition().clone();
    definition["entry"] = json!({"mode":"direct","decode":"decode_image","fix":"repair_image"});
    Format::from_json(&definition.to_string()).unwrap()
}

#[test]
fn direct_decode_verify_and_repair_do_not_require_main() {
    let format = direct(
        "def decode_image(data, name):\n return {'valid': data[0] == 7, 'checksum_ok': data[-1] == sum(data[:-1]) % 256}\ndef repair_image(data):\n return data[:-1] + bytes([sum(data[:-1]) % 256])\n",
    );
    let mut image = [0; 128];
    image[0] = 7;
    assert_eq!(
        format.decode(&image, "sample.eeprom").unwrap()["valid"],
        true
    );
    assert_eq!(
        format.verify(&image, "sample.eeprom").unwrap()["/checksum_ok"],
        false
    );
    format.fix(&mut image, "sample.eeprom").unwrap();
    let mut expected = [0; 128];
    expected[0] = 7;
    expected[127] = 7;
    assert_eq!(image, expected);
    assert_eq!(
        format.verify(&image, "sample.eeprom").unwrap()["/checksum_ok"],
        true
    );
}

#[test]
fn nested_invalid_records_are_not_rescued_by_positive_checks() {
    for signature in ["", ", 'signature_valid': True"] {
        let source = format!(
            "import json\ndef main():\n print(json.dumps({{'records': [{{'valid': False, 'checksum_ok': True{signature}}}, {{'valid': False, 'checksum_ok': True{signature}}}]}}))\n"
        );
        let format = review::format(&source);
        let actual = format
            .recognize_with_context(&[0; 128], "sample.eeprom", &Default::default())
            .unwrap();
        assert!(
            actual.is_none(),
            "all invalid records must refuse: {actual:?}; {signature}"
        );
    }
}

#[test]
fn signature_cannot_rescue_an_explicitly_invalid_record() {
    let format = review::format(
        "import json\ndef main():\n print(json.dumps({'groups': [[{'valid': False, 'signature_valid': True, 'checksum_ok': True}]]}))\n",
    );
    assert!(format
        .recognize_with_context(&[0; 128], "sample.eeprom", &Default::default())
        .unwrap()
        .is_none());
}

#[test]
fn recursive_record_validity_and_exclusions_preserve_any_record_semantics() {
    let cases = [
        ("{'valid': False, 'records': [{'valid': True}]}", false),
        ("{'global_valid': False, 'records': [{'valid': True}]}", false),
        ("{'records': [[{'valid': False, 'checksum_ok': True}]]}", false),
        ("{'records': [{'global_valid': False, 'nested': {'valid': True, 'signature_valid': True}}]}", false),
        ("{'records': [{'valid': False, 'nested': {'valid': True}}]}", false),
        ("{'records': [{'valid': False, 'checksum_ok': True}, {'valid': True, 'checksum_ok': True}]}", true),
        ("{'records': [[{'valid': False}], [{'valid': True}]]}", true),
        ("{'records': [{'valid': True}, {'valid': True}]}", true),
        ("{'records': [{'valid': False}], 'working_copy': {'valid': True, 'checksum_ok': True}}", false),
        ("{'groups': [{'records': [{'valid': False}], 'working_copy': [{'valid': True}]}]}", false),
        ("{'records': [{'valid': True}], 'working_copy': {'valid': False}}", true),
        ("{'working_copy': [{'valid': False}], 'checksum_ok': True}", true),
        ("{'records': [{'checksum_ok': True}]}", true),
        ("{'records': [{'signature_valid': True, 'checksum_ok': True}]}", true),
        ("{'records': [{'signature_valid': False, 'checksum_ok': True}]}", false),
        ("{'records': [{'checksum_ok': False}]}", false),
        ("{'records': [{'valid': False}], 'checksum_ok': True}", false),
        ("{'records': []}", false),
    ];
    for (tree, expected) in cases {
        let source = format!("import json\ndef main():\n print(json.dumps({tree}))\n");
        let mut definition = review::format(&source).definition().clone();
        definition["validity"]["exclude"] = json!(["working_copy"]);
        let format = Format::from_json(&definition.to_string()).unwrap();
        let actual = format
            .recognize_with_context(&[0; 128], "sample.eeprom", &Default::default())
            .unwrap();
        assert_eq!(actual.is_some(), expected, "{tree}: {actual:?}");
    }
}

#[test]
fn mixed_records_keep_all_included_check_evidence() {
    let source = "import json\ndef main():\n print(json.dumps({'records': [{'valid': False, 'signature_valid': True, 'checksum_ok': True}, {'valid': True, 'checksum_ok': True}], 'working_copy': {'valid': True}}))\n";
    let mut definition = review::format(source).definition().clone();
    definition["validity"]["exclude"] = json!(["working_copy"]);
    let format = Format::from_json(&definition.to_string()).unwrap();
    let evidence = format
        .recognize_with_context(&[0; 128], "sample.eeprom", &Default::default())
        .unwrap()
        .unwrap();
    assert_eq!(evidence.usable_flags, 1);
    assert_eq!(evidence.checks_passed, 4);
    assert_eq!(evidence.checks_failed, 1);
    // Verification continues to expose excluded regions for caller inspection.
    assert_eq!(
        format.verify(&[0; 128], "sample.eeprom").unwrap()["/working_copy/valid"],
        true
    );
}

#[test]
fn low_level_repair_retains_its_no_usability_requirement() {
    let format = direct("def decode_image(data, name):\n return {'valid': False}\ndef repair_image(data):\n return bytes([9]) + data[1:]\n");
    let mut image = [3; 128];
    format.fix(&mut image, "sample.eeprom").unwrap();
    let mut expected = [3; 128];
    expected[0] = 9;
    assert_eq!(image, expected);
    assert!(format
        .recognize_with_context(&image, "sample.eeprom", &Default::default())
        .unwrap()
        .is_none());
}

#[test]
fn non_direct_entry_modes_keep_their_argument_conventions() {
    for (mode, source) in [
        ("sys-argv", "import json\ndef main():\n args = sys_argv\n if '--fix' in args:\n  with open(args[3], 'wb') as output:\n   output.write(bytes([8]) * 128)\n else:\n  print(json.dumps({'args': args}))\n"),
        ("program", "import json\ndef main(args):\n if '--fix' in args:\n  with open(args[3], 'wb') as output:\n   output.write(bytes([8]) * 128)\n else:\n  print(json.dumps({'args': args}))\n"),
        ("arguments", "import json\ndef main(args):\n if '--fix' in args:\n  with open(args[2], 'wb') as output:\n   output.write(bytes([8]) * 128)\n else:\n  print(json.dumps({'args': args}))\n"),
    ] {
        let mut definition = review::format(source).definition().clone();
        definition["entry"] = json!({"mode":mode});
        let format = Format::from_json(&definition.to_string()).unwrap();
        let expected = if mode == "arguments" { json!(["sample.eeprom"]) } else { json!(["save", "sample.eeprom"]) };
        assert_eq!(format.decode(&[0;128], "sample.eeprom").unwrap()["args"], expected, "{mode}");
        let mut image = [0;128];
        format.fix(&mut image, "sample.eeprom").unwrap();
        assert_eq!(image, [8;128], "{mode}");
    }
    assert_eq!(
        review::decode("import json\ndef main():\n print(json.dumps({'valid': True}))\n").unwrap(),
        json!({"valid":true})
    );
}

#[test]
fn dispatch_and_repair_failures_remain_explicit_and_transactional() {
    let original = direct("def decode_image(data, name):\n return {'valid': True}\ndef repair_image(data):\n return data + bytes([1])\n");
    let mut definition = original.definition().clone();
    let mut image = [5; 128];
    assert!(original
        .fix(&mut image, "sample.eeprom")
        .unwrap_err()
        .to_string()
        .contains("repair changes image size"));
    assert_eq!(image, [5; 128]);
    definition["entry"]["decode"] = json!("absent");
    let missing = Format::from_json(&definition.to_string()).unwrap();
    assert!(missing
        .decode(&image, "sample.eeprom")
        .unwrap_err()
        .to_string()
        .contains("direct decode function"));
    definition["entry"]["fix"] = json!("absent");
    let missing = Format::from_json(&definition.to_string()).unwrap();
    assert!(missing
        .fix(&mut image, "sample.eeprom")
        .unwrap_err()
        .to_string()
        .contains("direct repair function"));
    assert_eq!(image, [5; 128]);
    for mode in ["program", "sys-argv", "arguments"] {
        definition["entry"] = json!({"mode":mode});
        let missing = Format::from_json(&definition.to_string()).unwrap();
        assert!(missing
            .decode(&image, "sample.eeprom")
            .unwrap_err()
            .to_string()
            .contains("save program lacks entry"));
        assert!(missing
            .fix(&mut image, "sample.eeprom")
            .unwrap_err()
            .to_string()
            .contains("save program lacks entry"));
        assert_eq!(image, [5; 128]);
    }
}
