use serde_json::{json, Value};
use ws_saves::Format;

#[allow(dead_code)]
mod review;

fn recognize_text(tree: &str, exclude: &[&str]) -> Option<ws_saves::ValidityEvidence> {
    let source = format!("import json\ndef main():\n print(json.dumps({tree}))\n");
    let mut definition = review::format(&source).definition().clone();
    definition["validity"]["exclude"] = json!(exclude);
    Format::from_json(&definition.to_string())
        .unwrap()
        .recognize_with_context(&[0; 128], "sample.eeprom", &Default::default())
        .unwrap()
}

fn recognize(tree: Value, exclude: &[&str]) -> Option<ws_saves::ValidityEvidence> {
    fn literal(value: &Value) -> String {
        match value {
            Value::Null => "None".into(),
            Value::Bool(true) => "True".into(),
            Value::Bool(false) => "False".into(),
            Value::Array(values) => format!(
                "[{}]",
                values.iter().map(literal).collect::<Vec<_>>().join(",")
            ),
            Value::Object(map) => format!(
                "{{{}}}",
                map.iter()
                    .map(|(key, value)| format!("{}:{}", json!(key), literal(value)))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            _ => value.to_string(),
        }
    }
    recognize_text(&literal(&tree), exclude)
}

#[test]
fn invalid_explicit_decisions_override_signatures_at_every_position() {
    for flag in ["valid", "global_valid"] {
        for position in ["ancestor", "sibling", "within"] {
            for arrays in [false, true] {
                for depth in 0..4 {
                    let mut record = json!({flag: false, "checksum_ok": true});
                    if position == "within" {
                        record["signature_valid"] = json!(true);
                    }
                    let mut tree = match position {
                        "ancestor" => json!({"signature_valid": true, "records": [record]}),
                        "sibling" => {
                            json!({"header": {"signature_valid": true}, "records": [record]})
                        }
                        _ => json!({"records": [record]}),
                    };
                    for _ in 0..depth {
                        tree = if arrays {
                            json!([tree])
                        } else {
                            json!({"group": tree})
                        };
                    }
                    assert!(
                        recognize(tree.clone(), &[]).is_none(),
                        "{flag}/{position}/arrays={arrays}/depth={depth}: {tree}"
                    );
                }
            }
        }
    }
}

#[test]
fn explicit_precedence_is_independent_of_input_key_order() {
    for (tree, expected) in [
        (
            r#"{"signature_valid":True,"records":[{"valid":False}]}"#,
            false,
        ),
        (
            r#"{"records":[{"valid":False}],"signature_valid":True}"#,
            false,
        ),
        (
            r#"{"a":{"signature_valid":True},"z":{"global_valid":False}}"#,
            false,
        ),
        (
            r#"{"z":{"global_valid":False},"a":{"signature_valid":True}}"#,
            false,
        ),
        (
            r#"{"valid":True,"records":[{"valid":False}],"signature_valid":False}"#,
            true,
        ),
        (
            r#"{"signature_valid":False,"records":[{"valid":False}],"valid":True}"#,
            true,
        ),
    ] {
        assert_eq!(recognize_text(tree, &[]).is_some(), expected, "{tree}");
    }
}

#[test]
fn explicit_decisions_and_fallback_preserve_accepted_diagnostics() {
    // Each tuple pins the existing public diagnostic counters, independently
    // of which kind of evidence establishes admission.
    let cases = [
        (
            json!({"signature_valid": true, "records": [{"valid": false, "checksum_ok": true}, {"valid": true, "signature_valid": true, "checksum_ok": false}]}),
            (3, 4, 2),
        ),
        (
            json!({"records": [[{"global_valid": false}], [{"global_valid": true}]]}),
            (1, 1, 1),
        ),
        (
            json!({"valid": true, "signature_valid": false, "records": [{"valid": false}, {"valid": false}]}),
            (1, 1, 3),
        ),
        (
            json!({"global_valid": true, "signature_valid": true, "records": [{"valid": false}]}),
            (2, 2, 1),
        ),
        (
            json!({"group": {"global_valid": true, "records": [{"valid": false}]}}),
            (1, 1, 1),
        ),
        (
            json!({"records": [{"valid": true, "signature_valid": false}]}),
            (1, 1, 1),
        ),
        (
            json!({"records": [{"signature_valid": true, "checksum_ok": false}]}),
            (1, 1, 1),
        ),
        (
            json!({"signature_valid": true, "checks": [{"checksum_valid": true}, {"crc_ok": false}]}),
            (1, 2, 1),
        ),
        (
            json!({"checksum_ok": true, "checksum_valid": false, "crc_ok": true, "ok": false}),
            (0, 2, 2),
        ),
        (
            json!({"valid": "false", "signature_valid": true}),
            (1, 1, 0),
        ),
    ];
    for (tree, expected) in cases {
        let actual = recognize(tree.clone(), &[]).unwrap_or_else(|| panic!("rejected {tree}"));
        assert_eq!(
            (
                actual.usable_flags,
                actual.checks_passed,
                actual.checks_failed
            ),
            expected,
            "{tree}"
        );
    }
    for tree in [
        json!({"records": [{"valid": false}], "checksum_ok": true}),
        json!({"records": [{"signature_valid": false, "checksum_ok": true}]}),
        json!({"checksum_ok": false}),
        json!({"records": []}),
        json!({"payload": 1}),
        json!({"signature_valid": true, "group": {"valid": false, "child": {"valid": true}}}),
        json!({"signature_valid": true, "group": {"global_valid": false, "child": {"global_valid": true}}}),
    ] {
        assert!(recognize(tree.clone(), &[]).is_none(), "admitted {tree}");
    }
}

#[test]
fn exclusions_neither_supply_nor_suppress_explicit_decisions() {
    let cases = [
        (
            json!({"signature_valid": true, "working_copy": {"valid": false}}),
            vec!["working_copy"],
            Some((1, 1, 0)),
        ),
        (
            json!({"checksum_ok": true, "working_copy": [{"global_valid": false}]}),
            vec!["working_copy"],
            Some((0, 1, 0)),
        ),
        (
            json!({"records": [{"valid": true}], "working_copy": {"global_valid": false}}),
            vec!["working_copy"],
            Some((1, 1, 0)),
        ),
        (
            json!({"signature_valid": true, "working_copy": {"valid": true}, "records": [{"valid": false}]}),
            vec!["working_copy"],
            None,
        ),
        (
            json!({"checksum_ok": true, "working_copy": {"valid": true}}),
            vec!["working_copy"],
            Some((0, 1, 0)),
        ),
        (
            json!({"working_copy": {"valid": true, "signature_valid": true, "checksum_ok": true}}),
            vec!["working_copy"],
            None,
        ),
        (
            json!({"group": [{"working_copy": {"valid": false}, "signature_valid": true}]}),
            vec!["working_copy"],
            Some((1, 1, 0)),
        ),
        (
            json!({"group": {"valid": false, "signature_valid": true}}),
            vec!["valid"],
            Some((1, 1, 0)),
        ),
        (
            json!({"group": {"global_valid": false, "signature_valid": true}}),
            vec!["global_valid"],
            Some((1, 1, 0)),
        ),
        (
            json!({"valid": true, "signature_valid": true, "records": [{"valid": false}]}),
            vec!["valid"],
            Some((1, 1, 0)),
        ),
        (
            json!({"signature_valid": false, "checksum_ok": true}),
            vec!["signature_valid"],
            Some((0, 1, 0)),
        ),
        (
            json!({"global_valid": true, "working_copy": {"valid": false}}),
            vec!["working_copy"],
            Some((1, 1, 0)),
        ),
        (
            json!({"valid": true, "records": [{"global_valid": false}]}),
            vec!["valid"],
            None,
        ),
    ];
    for (tree, exclude, expected) in cases {
        let actual = recognize(tree.clone(), &exclude)
            .map(|e| (e.usable_flags, e.checks_passed, e.checks_failed));
        assert_eq!(actual, expected, "{tree}; exclude={exclude:?}");
    }
}

#[test]
fn root_false_veto_survives_exclusions_and_positive_flags() {
    for flag in ["valid", "global_valid"] {
        for exclude in [vec![], vec![flag], vec!["valid", "global_valid"]] {
            let tree = json!({flag: false, "signature_valid": true, "records": [{"valid": true, "global_valid": true}]});
            assert!(
                recognize(tree.clone(), &exclude).is_none(),
                "{tree}; {exclude:?}"
            );
        }
    }
    assert!(recognize(json!({"valid": true, "global_valid": false}), &[]).is_none());
    assert!(recognize(json!({"valid": false, "global_valid": true}), &[]).is_none());
}
