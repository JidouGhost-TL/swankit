//! Read-only diagnostic survey; final parity gates are in corpus.rs.
use formatkit_corpus_test::{corpus_root, required_mode, Spec};
use serde_json::{json, Value};
fn differences(expected: &Value, actual: &Value, path: &str, out: &mut Vec<String>) {
    if expected == actual {
        return;
    }
    if out.len() >= 8 {
        return;
    }
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, v) in e {
                if let Some(got) = a.get(k) {
                    differences(v, got, &format!("{path}/{k}"), out);
                } else {
                    out.push(format!("{path}/{k}: missing"));
                }
            }
            for k in a.keys() {
                if !e.contains_key(k) {
                    out.push(format!("{path}/{k}: extra"));
                }
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                out.push(format!("{path}: array lengths {} != {}", e.len(), a.len()));
            }
            for (i, (e, a)) in e.iter().zip(a).enumerate() {
                differences(e, a, &format!("{path}/{i}"), out);
            }
        }
        _ => out.push(format!("{path}: {expected} != {actual}")),
    }
}
#[test]
fn diagnostic_survey() {
    let namespace =
        std::env::var("SWANKIT_SAVE_SURVEY_NAMESPACE").unwrap_or_else(|_| "ws-saves".into());
    let Some(root) = corpus_root(&namespace) else {
        assert!(!required_mode());
        return;
    };
    assert_eq!(
        std::env::var("FORMATKIT_CORPUS_RUNNER_LOCKED").as_deref(),
        Ok("1")
    );
    let spec = Spec::load(&root.join(&namespace).join("spec.json")).unwrap();
    let enrollment: Value = serde_json::from_slice(
        &std::fs::read(root.join(format!("{namespace}-enrollment.json"))).unwrap(),
    )
    .unwrap();
    let origins = enrollment["records"].as_array().unwrap();
    let mut passed = 0;
    let mut failed = 0;
    for (name, sample) in spec.samples {
        let id = sample.args["id"].as_str().unwrap();
        let record = origins
            .iter()
            .find(|r| {
                r["id"] == id
                    && r["sha256"] == sample.expect["sha256"]
                    && r["name"] == sample.args["name"]
            })
            .unwrap();
        let medium = record["path"].as_str().unwrap();
        if let Ok(selection) = std::env::var("SWANKIT_SAVE_SURVEY_IDS") {
            if !selection.split(',').any(|selected| selected == id) {
                continue;
            }
        }
        let bytes = std::fs::read(root.join(&namespace).join(&name)).unwrap();
        let mut context = ws_saves::DecodeContext::default();
        if let Some(tables) = sample.args.get("context").and_then(Value::as_object) {
            for (key, v) in tables {
                context.tables.insert(
                    key.clone(),
                    v.as_array()
                        .unwrap()
                        .iter()
                        .map(|n| n.as_u64().unwrap() as u8)
                        .collect(),
                );
            }
        }
        let companion = std::path::Path::new(medium).with_extension("ieep");
        if companion.is_file() {
            context.images.insert(
                companion.to_string_lossy().into_owned(),
                std::fs::read(&companion).unwrap(),
            );
        }
        let result =
            ws_saves::format(id).and_then(|f| f.decode_with_context(&bytes, medium, &context));
        let mut diffs = vec![];
        let status = match result {
            Ok(got) => {
                differences(&sample.expect["decoded"], &got, "", &mut diffs);
                if !diffs.is_empty() {
                    if let Some(error) = got.get("error") {
                        diffs.insert(0, format!("implementation detail: {error}"));
                    } else if let Some(map) = got.as_object() {
                        for (_, entry) in map {
                            if let Some(error) = entry.get("error") {
                                diffs.insert(0, format!("implementation detail: {error}"));
                                break;
                            }
                        }
                    }
                }
                if diffs.is_empty() {
                    passed += 1;
                    "pass"
                } else {
                    failed += 1;
                    "mismatch"
                }
            }
            Err(e) => {
                failed += 1;
                diffs.push(e.to_string());
                "error"
            }
        };
        eprintln!(
            "AUDIT {}",
            json!({"id":id,"sample":name,"status":status,"differences":diffs})
        );
    }
    eprintln!("SURVEY {passed} pass, {failed} need implementation or review; diagnostic only");
}
