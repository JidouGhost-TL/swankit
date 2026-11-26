use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path, process::Command};

fn run_swankit(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn version_json_matches_build_identity_schema() {
    for args in [
        &["--version", "--json"][..],
        &["--json", "--version"][..],
        &["-V", "--json"][..],
        &["--json", "-V"][..],
    ] {
        let output = run_swankit(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());

        let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(parsed["schema"], "formatkit.build-identity.v1");
        assert_eq!(parsed["tool"], "swankit");
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(parsed["git_commit"].is_string() || parsed["git_commit"].is_null());
        assert!(parsed["git_dirty"].is_boolean());
        assert!(parsed["formatkit_version"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert!(parsed["target"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(matches!(
            parsed["profile"].as_str(),
            Some("debug" | "release")
        ));
    }
}

#[test]
fn capabilities_json_matches_schema_and_committed_manifest() {
    let output = run_swankit(&["capabilities", "--json"]);
    assert!(
        output.status.success(),
        "capabilities --json: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["schema"], "formatkit.capabilities.v1");
    assert_eq!(parsed["tool"], "swankit");
    assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
    assert!(parsed["formatkit_version"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));

    let allowed_caps: BTreeSet<&str> = ["parse", "decode", "reproduce", "edit", "author"]
        .into_iter()
        .collect();
    let formats = parsed["formats"].as_array().unwrap();
    assert!(!formats.is_empty());

    let mut seen_ids = BTreeSet::new();
    for entry in formats {
        let id = entry["id"].as_str().unwrap();
        assert!(!id.is_empty());
        assert!(seen_ids.insert(id), "duplicate format id: {id}");
        assert!(entry["category"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(entry["notes"].as_str().is_some_and(|s| !s.is_empty()));
        let tier = entry["tier"].as_u64().unwrap();
        assert!(
            (1..=3).contains(&tier),
            "tier out of range for {id}: {tier}"
        );
        let caps = entry["capabilities"].as_array().unwrap();
        assert!(!caps.is_empty(), "empty capabilities for {id}");
        for cap in caps {
            let cap_str = cap.as_str().unwrap();
            assert!(
                allowed_caps.contains(cap_str),
                "invalid capability {cap_str:?} for {id}"
            );
        }
    }

    assert_eq!(
        parsed["save_engines"],
        serde_json::json!(["swankit-save-def/1", "swankit-save-def/2"])
    );
    assert_eq!(parsed["patch_formats"], serde_json::json!(["ips"]));

    // Plain output succeeds and is non-empty.
    let plain = run_swankit(&["capabilities"]);
    assert!(plain.status.success());
    assert!(plain.stderr.is_empty());
    let plain_text = String::from_utf8(plain.stdout).unwrap();
    assert!(plain_text.contains("ws-rom"));
    assert!(plain_text.contains("ws-patch-ips"));

    // Verify zero drift against committed capabilities.json and CAPABILITIES.md.
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let committed_json_path = repo_root.join("capabilities.json");
    let committed_json: Value =
        serde_json::from_str(&fs::read_to_string(&committed_json_path).unwrap()).unwrap();
    assert_eq!(
        parsed, committed_json,
        "capabilities.json is out of sync with `swankit capabilities --json`"
    );

    let committed_md = fs::read_to_string(repo_root.join("CAPABILITIES.md")).unwrap();
    for id in &seen_ids {
        assert!(
            committed_md.contains(id),
            "CAPABILITIES.md is missing format id {id}"
        );
    }
}
