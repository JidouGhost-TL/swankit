use std::process::Command;

#[test]
fn supported_command_help_and_invalid_requests() {
    for path in [
        vec!["info"],
        vec!["detect"],
        vec!["capabilities"],
        vec!["tiles"],
        vec!["extract"],
        vec!["extract", "text"],
        vec!["extract", "font"],
        vec!["extract", "sound"],
        vec!["extract", "codec"],
        vec!["patch"],
        vec!["patch", "apply"],
        vec!["patch", "create"],
        vec!["assets"],
        vec!["assets", "extract"],
        vec!["assets", "replace"],
        vec!["assets", "verify"],
        vec!["assets", "discover"],
        vec!["link"],
        vec!["wg-adapter"],
        vec!["wg-decode"],
        vec!["wg-server"],
        vec!["interop"],
    ]
    .into_iter()
    .chain(if cfg!(feature = "saves") {
        vec![
            vec!["save"],
            vec!["save", "decode"],
            vec!["save", "set"],
            vec!["save", "fix"],
        ]
    } else {
        vec![]
    }) {
        for flag in ["--help", "-h"] {
            let output = Command::new(env!("CARGO_BIN_EXE_swankit"))
                .args(&path)
                .arg(flag)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{path:?} {flag}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!output.stdout.is_empty());
            assert!(output.stderr.is_empty());
        }
    }
    for args in [
        vec!["unknown", "--help"],
        vec!["save", "unknown", "--help"],
        vec!["extract", "unknown", "--help"],
        vec!["patch", "unknown", "--help"],
        vec!["assets", "unknown", "--help"],
        vec!["assets", "--help", "extra"],
        vec!["extract", "sound", "--bogus", "--help"],
        vec!["tiles", "--help", "--help"],
        vec!["patch", "apply", "--output"],
        vec!["unknown"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_swankit"))
            .args(&args)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "malformed request accepted: {args:?}"
        );
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn output_error_hints_apply_only_to_existing_files() {
    let dir = std::env::temp_dir().join(format!(
        "swankit-output-hints-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let input = dir.join("tile.bin");
    std::fs::write(&input, [0u8; 16]).unwrap();
    let existing = dir.join("existing.png");
    std::fs::write(&existing, b"sentinel").unwrap();
    for (path, force, hint) in [
        (dir.join("absent/out.png"), false, false),
        (dir.join("absent/out.png"), true, false),
        (existing.clone(), false, true),
        (dir.clone(), true, false),
        (dir.clone(), false, false),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_swankit"));
        command.args([
            "tiles",
            input.to_str().unwrap(),
            "--format",
            "2bpp",
            "--length",
            "16",
            "--output",
            path.to_str().unwrap(),
        ]);
        if force {
            command.arg("--force");
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        let message = String::from_utf8_lossy(&output.stderr);
        assert_eq!(message.contains("use --force"), hint, "{message}");
    }
    assert_eq!(std::fs::read(&existing).unwrap(), b"sentinel");
    assert!(!dir.join("absent").exists());
    let success = Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args([
            "tiles",
            input.to_str().unwrap(),
            "--format",
            "2bpp",
            "--length",
            "16",
            "--output",
            existing.to_str().unwrap(),
            "--force",
        ])
        .output()
        .unwrap();
    assert!(success.status.success());
    assert!(std::fs::read(&existing).unwrap().starts_with(b"\x89PNG"));
    std::fs::remove_dir_all(dir).unwrap();
}
