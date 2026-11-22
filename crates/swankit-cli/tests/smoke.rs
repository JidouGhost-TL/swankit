use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "swankit-smoke-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self, n: &str) -> String {
        self.0.join(n).to_str().unwrap().to_owned()
    }
    fn put(&self, n: &str, b: &[u8]) -> String {
        let p = self.path(n);
        fs::write(&p, b).unwrap();
        p
    }
    fn json(&self, n: &str, v: Value) -> String {
        self.put(n, &serde_json::to_vec(&v).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn command(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args(args)
        .env_remove("FORMATKIT_CORPUS_DIR")
        .env_remove("FORMATKIT_CORPUS_REQUIRED")
        .env_remove("FORMATKIT_CORPUS_WRITE")
        .env_remove("FORMATKIT_CORPUS_RUNNER_LOCKED")
        .output()
        .unwrap()
}
fn ok(args: &[&str]) -> Output {
    let o = command(args);
    assert!(
        o.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    o
}
fn value(args: &[&str]) -> Value {
    serde_json::from_slice(&ok(args).stdout).unwrap()
}

#[test]
fn version_flags_report_package_version() {
    let expected = format!("swankit {}\n", env!("CARGO_PKG_VERSION"));
    for flag in ["--version", "-V"] {
        let output = ok(&[flag]);
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        assert!(output.stderr.is_empty());
    }
    let help = String::from_utf8(ok(&["--help"]).stdout).unwrap();
    assert!(help.contains("--version"));
    assert!(help.contains("-V"));
}

#[test]
fn interop_server_keeps_running_after_a_bad_request() {
    use std::io::Write;
    use std::process::Stdio;
    let fixture = Fixture::new();
    let tiles = fixture.put("tiles.payload", &[0; 544]);
    let mut child = Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args(["interop", "--json", "--server"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(input, "invalid JSON").unwrap();
    writeln!(input, "{}", json!({"version":1,"operation":2,"path":tiles})).unwrap();
    drop(input);
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    let replies: Vec<Value> = String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 2);
    assert!(replies[0]["error"].is_string());
    assert_eq!(replies[1]["version"], 1);
    let output = replies[1]["output"].as_array().unwrap();
    assert_eq!(output.len(), 256);
    for pixel in output.chunks_exact(4) {
        assert_eq!(pixel, &[json!(0), json!(0), json!(0), json!(255)]);
    }
}
fn png(path: &str, w: u32, h: u32) -> Vec<u8> {
    let bytes = fs::read(path).unwrap();
    let mut r = formatkit_core::Reader::new(&bytes);
    assert_eq!(r.bytes(8).unwrap(), b"\x89PNG\r\n\x1a\n");
    let mut data = Vec::new();
    loop {
        let len = u32::from_be_bytes(r.bytes(4).unwrap().try_into().unwrap()) as usize;
        let kind = r.bytes(4).unwrap();
        let raw = r.bytes(len).unwrap();
        let got = u32::from_be_bytes(r.bytes(4).unwrap().try_into().unwrap());
        let algorithm = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
        let mut crc = algorithm.digest();
        crc.update(kind);
        crc.update(raw);
        assert_eq!(got, crc.finalize());
        if kind == b"IHDR" {
            assert_eq!(&raw[..4], &w.to_be_bytes());
            assert_eq!(&raw[4..8], &h.to_be_bytes());
            assert_eq!(&raw[8..], &[8, 6, 0, 0, 0]);
        }
        if kind == b"IDAT" {
            data.extend_from_slice(raw);
        }
        if kind == b"IEND" {
            break;
        }
    }
    assert_eq!(r.remaining(), 0);
    miniz_oxide::inflate::decompress_to_vec_zlib(&data).unwrap()
}
#[test]
fn info_detect_tiles_and_snapshot() {
    let f = Fixture::new();
    let mut b = vec![0; 65536];
    b[65520..].copy_from_slice(&[0xea, 0, 0, 0, 0xf0, 0, 1, 0, 2, 1, 0, 0, 0, 1, 0, 0]);
    let sum = b[..65534]
        .iter()
        .fold(0u16, |s, &v| s.wrapping_add(u16::from(v)));
    b[65534..].copy_from_slice(&sum.to_le_bytes());
    let rom = f.put("rom", &b);
    let unknown = f.put("unknown", b"unrecognized");
    assert_eq!(value(&["info", &rom, "--json"])["checksum_ok"], true);
    assert!(String::from_utf8(ok(&["info", &rom]).stdout)
        .unwrap()
        .contains("Publisher: Bandai"));
    let d = value(&["detect", &rom, &unknown, "--json"]);
    assert_eq!(d[0]["detection"]["selected"], "ws-rom");
    assert!(d[1]["detection"]["selected"].is_null());
    let out = f.path("tiles.png");
    ok(&[
        "tiles",
        &rom,
        "--offset",
        "0x0",
        "--length",
        "16",
        "--format",
        "2bpp",
        "--columns",
        "1",
        "--output",
        &out,
    ]);
    let raw = png(&out, 8, 8);
    assert_eq!(raw.len(), 8 * (1 + 8 * 4));
    assert_eq!(&raw[1..5], &[255, 255, 255, 255]);
    assert!(
        !command(&["tiles", &rom, "--length", "16", "--format", "2bpp", "--output", &out])
            .status
            .success()
    );
    ok(&[
        "tiles",
        &rom,
        "--length",
        "16",
        "--format",
        "2bpp",
        "--columns",
        "1",
        "--output",
        &out,
        "--force",
    ]);
    let snap = f.put("snapshot", &vec![0; 65536 + 256]);
    let screen = f.path("screen.png");
    ok(&[
        "tiles",
        &snap,
        "--snapshot",
        "--color",
        "--output",
        &screen,
        "--json",
    ]);
    assert_eq!(png(&screen, 224, 144).len(), 144 * (1 + 224 * 4));
    let error = command(&["info", &unknown, "--json"]);
    assert!(!error.status.success());
    assert!(error.stdout.is_empty());
    assert!(serde_json::from_slice::<Value>(&error.stderr).unwrap()["error"].is_string());
    assert!(!command(&["info", &rom, "--unknown", "1"]).status.success());
    assert!(!command(&[
        "tiles",
        &rom,
        "--length",
        "16",
        "--format",
        "2bpp",
        "--columns",
        "0",
        "--output",
        &out,
        "--force"
    ])
    .status
    .success());
}
#[test]
fn extract_every_kind_and_patch_roundtrip() {
    let f = Fixture::new();
    let bytes = f.put("input", &[65, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let def=f.json("text.json",json!({"schema":"swankit-text-def/1","id":"synthetic","map":{"schema":"swankit-charmap/1","entries":[{"bytes":"41","text":"A"}],"controls":[{"code":0,"ends":true}]},"tables":{"demo":{"kind":"fixed","offset":0,"count":1,"stride":2,"length":2}},"fonts":{"demo":{"offset":0,"count":1,"stride":16,"format":"bpp2","tiles_wide":1,"tiles_high":1}}}));
    assert_eq!(
        value(&["extract", "text", &bytes, "--def", &def, "--table", "demo", "--json"])["strings"]
            [0]["text"],
        "A"
    );
    let font = f.path("font.png");
    ok(&[
        "extract",
        "font",
        &bytes,
        "--def",
        &def,
        "--font",
        "demo",
        "--columns",
        "1",
        "--output",
        &font,
    ]);
    png(&font, 8, 8);
    let wave = f.json(
        "wave.json",
        json!({"schema":"swankit-sound-def/1","format":"wave","length":16}),
    );
    assert_eq!(
        value(&["extract", "sound", &bytes, "--def", &wave, "--json"])["waves"][0]
            .as_array()
            .unwrap()
            .len(),
        32
    );
    let codec = f.json(
        "codec.json",
        json!({"schema":"swankit-codec-def/1","codec":"lz-rle-7f","length":3,"max_output":3}),
    );
    let compressed = f.put("compressed", &[0xe2, 0x7f, 0xff]);
    let raw = f.path("raw");
    ok(&[
        "extract",
        "codec",
        &compressed,
        "--def",
        &codec,
        "--output",
        &raw,
    ]);
    assert_eq!(fs::read(&raw).unwrap(), [0, 0, 0]);
    let base = f.put("base", b"abcdef");
    let target = f.put("target", b"abXYef");
    let patch = f.path("patch");
    let applied = f.path("applied");
    ok(&[
        "patch", "create", &base, &target, "--output", &patch, "--json",
    ]);
    assert_eq!(
        value(&["detect", &patch, "--json"])[0]["detection"]["selected"],
        "ws-patch-ips"
    );
    ok(&["patch", "apply", &patch, &base, "--output", &applied]);
    assert_eq!(fs::read(&applied).unwrap(), b"abXYef");
    let truncated = f.put("truncated", b"PATCH");
    assert!(!command(&[
        "patch",
        "apply",
        &truncated,
        &base,
        "--output",
        &f.path("bad")
    ])
    .status
    .success());
    assert!(
        !command(&["extract", "codec", &compressed, "--def", &codec])
            .status
            .success()
    );
}

#[cfg(feature = "saves")]
#[test]
fn save_decode_set_fix_and_auto_errors() {
    let f = Fixture::new();
    let def = f.json("save.json", save_definition());
    let mut bytes = vec![0; 128];
    bytes[0] = 165;
    let image = f.put("save", &bytes);
    assert_eq!(
        value(&["save", "decode", &def, &image, "--json"])["setting"],
        0
    );
    let edited = f.path("edited");
    ok(&[
        "save", "set", &def, &image, "--field", "0", "--value", "7", "--output", &edited, "--json",
    ]);
    let mut expected = bytes.clone();
    expected[1] = 7;
    assert_eq!(fs::read(&edited).unwrap(), expected);
    bytes[0] = 0;
    let corrupt = f.put("corrupt", &bytes);
    let fixed = f.path("fixed");
    ok(&["save", "fix", &def, &corrupt, "--output", &fixed]);
    let fixed = fs::read(fixed).unwrap();
    assert_eq!(fixed[0], 165);
    assert!(fixed[1..].iter().all(|&v| v == 0));
    assert!(!command(&[
        "save",
        "set",
        &def,
        &image,
        "--field",
        "0",
        "--value",
        "256",
        "--output",
        &f.path("bad")
    ])
    .status
    .success());
    let unknown = f.put("unknown", b"unknown");
    assert!(!command(&["save", "decode", "auto", &unknown, "--json"])
        .status
        .success());
    assert!(!command(&["save", "decode", "unknown-definition", &image])
        .status
        .success());
}

#[cfg(feature = "saves")]
#[test]
fn save_v2_flags_are_independent_atomic_and_protect_input_aliases() {
    let f = Fixture::new();
    let mut definition = save_definition();
    definition["schema"] = json!("swankit-save-def/2");
    definition["fields"][0]["constraint"] =
        json!({"kind":"unsigned-range","minimum":0,"maximum":9});
    definition["fields"].as_array_mut().unwrap().push(json!({"name":"setting.copy_b","medium":"cart EEPROM","offset":10,"size":1,"encoding":"u8"}));
    definition["fields"].as_array_mut().unwrap().push(json!({"name":"setting.copy_c","medium":"cart EEPROM","offset":40,"size":1,"encoding":"u8"}));
    definition["copy_groups"] =
        json!([{"name":"setting.copies","fields":["setting","setting.copy_b","setting.copy_c"]}]);
    let def = f.json("save-v2.json", definition);
    let mut bytes = vec![0; 128];
    bytes[0] = 165;
    bytes[10] = 11;
    bytes[40] = 13;
    let input = f.put("input.save", &bytes);

    // Default refusal must not create or overwrite an output, including --force.
    let absent = f.path("absent.save");
    assert!(!command(&[
        "save", "set", &def, &input, "--field", "0", "--value", "10", "--output", &absent
    ])
    .status
    .success());
    assert!(!std::path::Path::new(&absent).exists());
    let sentinel = f.put("sentinel.save", b"keep");
    assert!(!command(&[
        "save", "set", &def, &input, "--field", "0", "--value", "10", "--output", &sentinel,
        "--force"
    ])
    .status
    .success());
    assert_eq!(fs::read(&sentinel).unwrap(), b"keep");

    let scalar = f.path("scalar.save");
    ok(&[
        "save", "set", &def, &input, "--field", "0", "--value", "7", "--output", &scalar,
    ]);
    let scalar = fs::read(scalar).unwrap();
    assert_eq!((scalar[1], scalar[10], scalar[40]), (7, 11, 13));
    let copies = f.path("copies.save");
    ok(&[
        "save",
        "set",
        &def,
        &input,
        "--field",
        "0",
        "--value",
        "7",
        "--all-copies",
        "--no-repair",
        "--output",
        &copies,
        "--json",
    ]);
    let report = fs::read(&copies).unwrap();
    assert_eq!((report[1], report[10], report[40]), (7, 7, 7));
    let skipped = f.path("skipped.save");
    let out = ok(&[
        "save",
        "set",
        &def,
        &input,
        "--field",
        "0",
        "--value",
        "10",
        "--skip-validation",
        "--no-repair",
        "--output",
        &skipped,
        "--json",
    ]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"validation\":\"skipped\""));
    assert_eq!(fs::read(&skipped).unwrap()[1], 10);
    let skipped_plain = f.path("skipped-plain.save");
    let out = ok(&[
        "save",
        "set",
        &def,
        &input,
        "--field",
        "0",
        "--value",
        "10",
        "--skip-validation",
        "--no-repair",
        "--output",
        &skipped_plain,
    ]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("validation skipped"));

    // --skip-validation keeps byte representation and input identity protections.
    assert!(!command(&[
        "save",
        "set",
        &def,
        &input,
        "--field",
        "0",
        "--value",
        "256",
        "--skip-validation",
        "--output",
        &f.path("overflow")
    ])
    .status
    .success());
    assert!(!command(&[
        "save",
        "set",
        &def,
        &input,
        "--field",
        "0",
        "--value",
        "7",
        "--skip-validation",
        "--output",
        &input,
        "--force"
    ])
    .status
    .success());
    assert_eq!(fs::read(&input).unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let alias = f.path("symlink.save");
        symlink(&input, &alias).unwrap();
        assert!(!command(&[
            "save",
            "set",
            &def,
            &input,
            "--field",
            "0",
            "--value",
            "7",
            "--skip-validation",
            "--output",
            &alias,
            "--force"
        ])
        .status
        .success());
    }
    #[cfg(any(unix, windows))]
    {
        let hard = f.path("hardlink.save");
        fs::hard_link(&input, &hard).unwrap();
        assert!(!command(&[
            "save",
            "set",
            &def,
            &input,
            "--field",
            "0",
            "--value",
            "7",
            "--skip-validation",
            "--output",
            &hard,
            "--force"
        ])
        .status
        .success());
        assert_eq!(fs::read(&input).unwrap(), bytes);
    }
}
#[cfg(not(feature = "saves"))]
#[test]
fn disabled_saves_returns_a_clear_error() {
    let result = command(&["save", "decode", "auto", "input", "--json"]);
    assert!(!result.status.success());
    assert!(
        serde_json::from_slice::<Value>(&result.stderr).unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("saves feature")
    );
}

#[cfg(feature = "saves")]
fn save_definition() -> Value {
    let constant = |value: Value| json!({"t":"Constant","value":value});
    let data = json!({"t":"Name","id":"data"});
    let magic = json!({"t":"Subscript","value":data,"slice":constant(json!(0))});
    let setting = json!({"t":"Subscript","value":data,"slice":constant(json!(1))});
    let function = |name: &str, arguments: &[&str], body: Value| {
        json!({"t":"FunctionDef","name":name,"args":{
        "args":arguments.iter().map(|n|json!({"arg":n})).collect::<Vec<_>>(),"defaults":[]},"body":body})
    };
    json!({"schema":"swankit-save-def/1","id":"synthetic","implementation":"generated",
        "media":[{"kind":"cartridge-eeprom","sizes":[128]}],
        "fields":[{"name":"setting","medium":"cart EEPROM","offset":1,"size":1,"type":"unsigned","encoding":"u8","meaning":"setting","confidence":"synthetic","evidence":"synthetic"}],
        "records":[],"blocks":[],"checksums":{},"primitives":{},"validity":{"policy":"program","exclude":["working_copy"]},
        "entry":{"mode":"direct","decode":"decode_image","fix":"fix_image"},
        "program":{"t":"Module","body":[
            function("main",&[],json!([{"t":"Return","value":constant(Value::Null)}])),
            function("decode_image",&["data","name"],json!([{"t":"Return","value":{"t":"Dict",
                "keys":[constant(json!("valid")),constant(json!("setting"))],
                "values":[{"t":"Compare","left":magic,"ops":[{"t":"Eq"}],"comparators":[constant(json!(165))]},setting]}}])),
            function("fix_image",&["data"],json!([
                {"t":"Assign","targets":[magic],"value":constant(json!(165))},
                {"t":"Return","value":data}
            ]))
        ],"type_ignores":[]}})
}

#[test]
fn discover_draft_is_verified_and_wrong_identity_writes_nothing() {
    let help = ok(&["assets", "discover", "--help"]);
    assert!(String::from_utf8_lossy(&help.stdout).contains("swankit-discovery-evidence/1"));
    let f = Fixture::new();
    let rom = vec![0; 65536];
    let image = f.put("image.fixture", &rom);
    let mut evidence = json!({"schema":"swankit-discovery-evidence/1",
        "seed":{"schema":"swankit-asset-map/1","rom":{"sha256":ws_assets::sha256_hex(&rom),"size":rom.len()},"assets":[]},
        "ram":vec![0;65536],"ports":vec![0;256]});
    let input = f.json("evidence.json", evidence.clone());
    let output = f.path("draft.json");
    let report = f.path("report.json");
    ok(&[
        "assets", "discover", &image, &input, "-o", &output, "--report", &report, "--json",
    ]);
    ok(&["assets", "verify", &image, &output, "--json"]);
    evidence["seed"]["rom"]["sha256"] = json!("0".repeat(64));
    let bad = f.json("wrong.json", evidence);
    let absent = f.path("absent.json");
    assert!(
        !command(&["assets", "discover", &image, &bad, "-o", &absent])
            .status
            .success()
    );
    assert!(!std::path::Path::new(&absent).exists());
}

#[test]
fn snapshot_scanline_ports_change_only_the_selected_rows() {
    let f = Fixture::new();
    let mut snapshot = vec![0; 65536 + 256];
    snapshot[65536 + 0x1c] = 0xf0;
    let snap = f.put("line-snapshot", &snapshot);
    let mut rows = vec![0; 144 * 256];
    for y in 0..144 {
        rows[y * 256 + 0x1c] = 0xf0;
        if y >= 72 {
            rows[y * 256 + 1] = 1;
        }
    }
    let ports = f.put("line-ports", &rows);
    let output = f.path("lines.png");
    ok(&[
        "tiles",
        &snap,
        "--snapshot",
        "--line-ports",
        &ports,
        "--output",
        &output,
        "--json",
    ]);
    let pixels = png(&output, 224, 144);
    let stride = 1 + 224 * 4;
    assert_ne!(&pixels[1..5], &pixels[100 * stride + 1..100 * stride + 5]);
    assert_eq!(&pixels[1..5], &pixels[71 * stride + 1..71 * stride + 5]);
    let short = f.put("short-ports", &rows[..rows.len() - 1]);
    assert!(!command(&[
        "tiles",
        &snap,
        "--snapshot",
        "--line-ports",
        &short,
        "--output",
        &f.path("bad.png"),
        "--json"
    ])
    .status
    .success());
}

#[test]
fn extended_lzss_standalone_extraction_and_refusals() {
    let fixture = Fixture::new();
    let src = fixture.put("source.bin", &[0xff, 2, b'A', 0, 0x10, 0xee]);
    let base = json!({"schema":"swankit-codec-def/1","codec":"lzss-12-ext",
        "offset":1,"length":4,"decoded_size":4,"max_output":4});
    let def = fixture.json("codec.json", base.clone());
    let out = fixture.path("decoded.bin");
    let meta = value(&[
        "extract", "codec", &src, "--def", &def, "--output", &out, "--json",
    ]);
    assert_eq!(fs::read(&out).unwrap(), b"AAAA");
    assert_eq!(meta["details"]["consumed"], 4);
    for (n, mut bad) in [base.clone(), base.clone(), base.clone(), base.clone()]
        .into_iter()
        .enumerate()
    {
        match n {
            0 => {
                bad.as_object_mut().unwrap().remove("decoded_size");
            }
            1 => bad["length"] = json!(3),
            2 => bad["max_output"] = json!(3),
            _ => {
                bad["offset"] = json!(3);
                bad["length"] = json!(3);
            }
        }
        let d = fixture.json(&format!("bad{n}.json"), bad);
        let output = fixture.path(&format!("bad{n}.bin"));
        assert!(
            !command(&["extract", "codec", &src, "--def", &d, "--output", &output])
                .status
                .success()
        );
        assert!(!std::path::Path::new(&output).exists());
    }
    let src = fixture.put("legacy.bin", &[1, b'A', 0, 0]);
    let mut legacy = base;
    legacy["codec"] = json!("lzss-12");
    legacy["offset"] = json!(0);
    let d = fixture.json("legacy.json", legacy);
    let out = fixture.path("legacy-out.bin");
    ok(&["extract", "codec", &src, "--def", &d, "--output", &out]);
    assert_eq!(fs::read(out).unwrap(), b"AAAA");
}

#[test]
fn mask16_fill_standalone_extraction_is_decode_only_and_bounded() {
    let fixture = Fixture::new();
    let mut stored = vec![1, 0xee, 0x01, 0x80];
    stored.extend(0..7);
    stored.extend(7..14);
    let source = fixture.put("source.bin", &stored);
    let definition = fixture.json(
        "codec.json",
        json!({"schema":"swankit-codec-def/1","codec":"mask16-fill",
            "length":18,"decoded_size":16,"max_output":16}),
    );
    let output = fixture.path("decoded.bin");
    let result = value(&[
        "extract",
        "codec",
        &source,
        "--def",
        &definition,
        "--output",
        &output,
        "--json",
    ]);
    let mut expected: Vec<u8> = (0..7).collect();
    expected.extend_from_slice(&[0xee, 0xee]);
    expected.extend(7..14);
    assert_eq!(fs::read(&output).unwrap(), expected);
    assert_eq!(result["details"]["consumed"], 18);

    for (n, def) in [
        json!({"schema":"swankit-codec-def/1","codec":"mask16-fill",
            "length":18,"decoded_size":15,"max_output":16}),
        json!({"schema":"swankit-codec-def/1","codec":"mask16-fill",
            "length":18,"max_output":16,"history":[0]}),
        json!({"schema":"swankit-codec-def/1","codec":"mask16-fill",
            "length":1,"max_output":15}),
        json!({"schema":"swankit-codec-def/1","codec":"mask16-fill",
            "length":1,"max_output":16}),
    ]
    .into_iter()
    .enumerate()
    {
        let d = fixture.json(&format!("invalid{n}.json"), def);
        let input = if n == 3 {
            fixture.put(&format!("zero{n}.bin"), &[0])
        } else if n == 2 {
            fixture.put(&format!("bounded{n}.bin"), &[1])
        } else {
            source.clone()
        };
        let output = fixture.path(&format!("invalid{n}.bin"));
        assert!(
            !command(&["extract", "codec", &input, "--def", &d, "--output", &output,])
                .status
                .success()
        );
        assert!(!std::path::Path::new(&output).exists());
    }
}

#[test]
fn lz_command16_lsb_standalone_extraction_is_explicit_and_bounded() {
    let fixture = Fixture::new();
    let source = fixture.put("source.bin", &[0, 2, 0x06, b'A', 0x00, 0x10]);
    let definition = fixture.json(
        "codec.json",
        json!({"schema":"swankit-codec-def/1","codec":"lz-command16-lsb",
            "length":6,"decoded_size":4,"max_output":4}),
    );
    let output = fixture.path("decoded.bin");
    let result = value(&[
        "extract",
        "codec",
        &source,
        "--def",
        &definition,
        "--output",
        &output,
        "--json",
    ]);
    assert_eq!(fs::read(&output).unwrap(), b"AAAA");
    assert_eq!(result["details"]["consumed"], 6);

    let with_history = fixture.put("history.bin", &[0, 1, 0x03, 0x00, 0x10]);
    let history_definition = fixture.json(
        "history.json",
        json!({"schema":"swankit-codec-def/1","codec":"lz-command16-lsb",
            "length":5,"max_output":3,"history":[90]}),
    );
    let history_output = fixture.path("history-out.bin");
    ok(&[
        "extract",
        "codec",
        &with_history,
        "--def",
        &history_definition,
        "--output",
        &history_output,
    ]);
    assert_eq!(fs::read(&history_output).unwrap(), b"ZZZ");

    let unknown_history = fixture.put("unknown-history.bin", &[0, 1, 0x03, 0x00, 0x10]);
    for (n, def) in [
        json!({"schema":"swankit-codec-def/1","codec":"lz-command16-lsb",
            "length":6,"max_output":3}),
        json!({"schema":"swankit-codec-def/1","codec":"lz-command16-lsb",
            "length":6,"max_output":3}),
        json!({"schema":"swankit-codec-def/1","codec":"lz-command16-lsb",
            "length":6,"decoded_size":3,"max_output":4}),
        json!({"schema":"swankit-codec-def/1","codec":"lz-command16-lsb",
            "length":5,"max_output":3}),
    ]
    .into_iter()
    .enumerate()
    {
        let d = fixture.json(&format!("invalid{n}.json"), def);
        let input = if n == 1 {
            fixture.put("truncated.bin", &[0, 2, 0x06, b'A', 0x00])
        } else if n == 3 {
            unknown_history.clone()
        } else {
            source.clone()
        };
        let output = fixture.path(&format!("invalid{n}.bin"));
        assert!(
            !command(&["extract", "codec", &input, "--def", &d, "--output", &output])
                .status
                .success()
        );
        assert!(!std::path::Path::new(&output).exists());
    }
}

#[test]
fn lzss_msb_distance12_standalone_extraction_requires_explicit_size() {
    let fixture = Fixture::new();
    let source = fixture.put("source.bin", &[0xff, 0x40, b'A', 3, 0, 0xde]);
    let mut definition = json!({
        "schema": "swankit-codec-def/1",
        "codec": "lzss-msb-distance12",
        "offset": 1,
        "length": 5,
        "decoded_size": 5,
        "max_output": 5,
    });
    let path = fixture.json("codec.json", definition.clone());
    let output = fixture.path("decoded.bin");
    let result = value(&[
        "extract", "codec", &source, "--def", &path, "--output", &output, "--json",
    ]);
    assert_eq!(fs::read(&output).unwrap(), b"AAAAA");
    assert_eq!(result["details"]["consumed"], 4);

    let mut missing_size = definition.clone();
    missing_size.as_object_mut().unwrap().remove("decoded_size");
    let mut short_slice = definition.clone();
    short_slice["length"] = json!(3);
    let mut beyond_source = definition.clone();
    beyond_source["length"] = json!(6);
    definition["max_output"] = json!(4);
    for (n, invalid) in [missing_size, short_slice, beyond_source, definition]
        .into_iter()
        .enumerate()
    {
        let path = fixture.json(&format!("invalid{n}.json"), invalid);
        let output = fixture.path(&format!("invalid{n}.bin"));
        let result = command(&[
            "extract", "codec", &source, "--def", &path, "--output", &output,
        ]);
        assert!(!result.status.success());
        assert!(!std::path::Path::new(&output).exists());
    }
}

#[test]
fn unit_carrier_codec_receipts_and_prewrite_refusals() {
    let f = Fixture::new();
    let bytes = [9, 0, 1, 1, b'A', 0, 28, 0xde, 0xad];
    let input = f.put("unit-input.bin", &bytes);
    let mut def = json!({"schema":"swankit-codec-def/1","codec":"lzss-be-units","offset":1,"length":8,"max_output":16,"unit_size":16,"source_width":16,"original_sha256":ws_assets::sha256_hex(&bytes)});
    let path = f.json("unit-def.json", def.clone());
    let output = f.path("unit-output.bin");
    let v = value(&[
        "extract", "codec", &input, "--def", &path, "--output", &output, "--json",
    ]);
    assert_eq!(fs::read(&output).unwrap(), vec![b'A'; 16]);
    assert_eq!(v["details"]["consumed"], 6);
    let p = &v["details"]["provenance"];
    assert_eq!(p["schema"], "swankit-codec-extraction/1");
    assert_eq!(p["stored"]["consumed"], 6);
    assert_eq!(p["normalized"]["consumed"], 4);
    assert_eq!(p["stored"]["sha256"], ws_assets::sha256_hex(&bytes[1..7]));
    assert_eq!(p["decoded"]["sha256"], ws_assets::sha256_hex(&[b'A'; 16]));
    def["original_sha256"] = json!("0".repeat(64));
    let bad = f.json("unit-bad.json", def.clone());
    let absent = f.path("unit-absent.bin");
    let refused = command(&[
        "extract", "codec", &input, "--def", &bad, "--output", &absent, "--json",
    ]);
    assert!(!refused.status.success());
    assert!(!std::path::Path::new(&absent).exists());
    let r: Value = serde_json::from_slice(&refused.stderr).unwrap();
    assert_eq!(r["schema"], "swankit-codec-extraction/1");
    assert_eq!(r["reason"], "identity-mismatch");
    def["original_sha256"] = json!(ws_assets::sha256_hex(&bytes));
    for (field, bad_value) in [
        ("unit_size", json!(8)),
        ("source_width", json!(8)),
        ("history", json!([0])),
        ("max_output", json!(15)),
    ] {
        let mut d = def.clone();
        d[field] = bad_value;
        let dp = f.json("unit-refusal.json", d);
        assert!(!command(&[
            "extract", "codec", &input, "--def", &dp, "--output", &absent, "--json"
        ])
        .status
        .success());
        assert!(!std::path::Path::new(&absent).exists());
    }
    let legacy=f.json("unit-legacy.json",json!({"schema":"swankit-codec-def/1","codec":"lzss-12","offset":0,"length":2,"max_output":1,"decoded_size":1}));
    let oldinput = f.put("unit-legacy.bin", &[1, b'Q']);
    let old = value(&[
        "extract",
        "codec",
        &oldinput,
        "--def",
        &legacy,
        "--output",
        &f.path("unit-old-output.bin"),
        "--json",
    ]);
    assert_eq!(old["details"], json!({"consumed":2}));
}

#[test]
fn unit_carrier_cli_width_boundary() {
    let f = Fixture::new();
    for unit in [16usize, 32] {
        for (width, size, accept) in [
            (16, 65536, false),
            (32, 65536, true),
            (16, 65536 - unit, true),
            (32, 65536 + unit, false),
        ] {
            let mut bytes = ((size / unit) as u16).to_be_bytes().to_vec();
            for _ in 0..size / 8 {
                bytes.push(255);
                bytes.extend_from_slice(b"abcdefgh");
            }
            let label = format!("u{unit}-w{width}-n{size}");
            let input = f.put(&format!("{label}.bin"), &bytes);
            let definition=f.json(&format!("{label}.json"),json!({"schema":"swankit-codec-def/1","codec":"lzss-be-units","offset":0,"length":bytes.len(),"max_output":65536,"unit_size":unit,"source_width":width,"original_sha256":ws_assets::sha256_hex(&bytes)}));
            let output = f.path(&format!("{label}.out"));
            let result = command(&[
                "extract",
                "codec",
                &input,
                "--def",
                &definition,
                "--output",
                &output,
                "--json",
            ]);
            assert_eq!(
                result.status.success(),
                accept,
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            if accept {
                assert_eq!(fs::read(&output).unwrap().len(), size);
            } else {
                assert!(!std::path::Path::new(&output).exists());
            }
        }
    }
}

#[test]
fn fixed_stride_table_resolves_ranges_and_reports_uniform_gaps() {
    let fixture = Fixture::new();
    let mut rom = vec![0; 64];
    for (i, word) in [4u32, 4, 16, 2].into_iter().enumerate() {
        rom[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    rom[20..24].copy_from_slice(b"data");
    rom[24..32].copy_from_slice(&[1, 2, 3, 4, 1, 2, 3, 4]);
    rom[32..34].copy_from_slice(b"ok");
    rom[34..40].fill(255);
    let input = fixture.put("input.wsc", &rom);
    let mut definition = json!({
        "schema": "swankit-fixed-stride-table/1",
        "rom": {"size": rom.len(), "sha256": ws_assets::sha256_hex(&rom)},
        "table_offset": 0, "entry_count": 2, "entry_stride": 8,
        "offset_field": 0, "offset_unit": 1, "data_offset": 16, "data_length": 24,
        "size": {"kind": "explicit-u32", "field_offset": 4, "unit": 1},
        "padding_min_bytes": 4
    });
    let definition_path = fixture.json("table.json", definition.clone());
    let receipt = fixture.path("receipt.json");
    let map = fixture.path("map.json");
    let result = value(&[
        "assets",
        "resolve-table",
        &input,
        &definition_path,
        "--output",
        &receipt,
        "--map-out",
        &map,
        "--json",
    ]);
    assert_eq!(result["entries"][1]["offset"], 32);
    assert_eq!(result["padding"][0]["kind"], "all-zero");
    assert_eq!(result["padding"][1]["kind"], "repeated-word32");
    assert_eq!(result["padding"][1]["fill"], 0x0403_0201);
    assert_eq!(result["padding"][2]["fill"], 255);
    let generated: Value = serde_json::from_slice(&fs::read(&map).unwrap()).unwrap();
    assert!(generated["free_space"].as_array().is_none_or(Vec::is_empty));
    let out = fixture.path("out");
    let extraction = value(&["assets", "extract", &input, &map, &out, "--json"]);
    assert_eq!(extraction["failures"], 0);
    assert_eq!(
        fs::read(std::path::Path::new(&out).join("entry.0000.bin")).unwrap(),
        b"data"
    );

    definition["size"] = json!({"kind":"sequential-until","sentinel_end":20});
    let sequence = fixture.json("sequence.json", definition.clone());
    let seq_receipt = fixture.path("sequence-receipt.json");
    let result = value(&[
        "assets",
        "resolve-table",
        &input,
        &sequence,
        "--output",
        &seq_receipt,
        "--json",
    ]);
    assert_eq!(result["entries"][0]["length"], 12);
    assert_eq!(result["entries"][1]["length"], 4);

    definition["rom"]["sha256"] = json!("00".repeat(32));
    let wrong = fixture.json("wrong.json", definition);
    let refused = fixture.path("refused.json");
    assert!(!command(&[
        "assets",
        "resolve-table",
        &input,
        &wrong,
        "--output",
        &refused
    ])
    .status
    .success());
    assert!(!std::path::Path::new(&refused).exists());
}

#[test]
fn fixed_stride_table_protects_its_metadata() {
    let fixture = Fixture::new();
    let mut rom = vec![0; 65536];
    let mut definition = json!({
        "schema": "swankit-fixed-stride-table/1",
        "rom": {"size": rom.len(), "sha256": ws_assets::sha256_hex(&rom)},
        "table_offset": 32, "entry_count": 1, "entry_stride": 8,
        "offset_field": 0, "offset_unit": 1, "data_offset": 24, "data_length": 24,
        "size": {"kind": "explicit-u32", "field_offset": 4, "unit": 1},
        "padding_min_bytes": 4
    });
    let input = fixture.put("zero.ws", &rom);
    let spec = fixture.json("padding.json", definition.clone());
    let receipt = fixture.path("padding-receipt.json");
    let result = value(&[
        "assets",
        "resolve-table",
        &input,
        &spec,
        "--output",
        &receipt,
        "--json",
    ]);
    let gaps = result["padding"].as_array().unwrap();
    assert_eq!(gaps.len(), 2);
    assert_eq!(
        (gaps[0]["offset"].as_u64(), gaps[0]["length"].as_u64()),
        (Some(24), Some(8))
    );
    assert_eq!(
        (gaps[1]["offset"].as_u64(), gaps[1]["length"].as_u64()),
        (Some(40), Some(8))
    );

    definition["data_offset"] = json!(36);
    definition["data_length"] = json!(12);
    let spec = fixture.json("clipped.json", definition.clone());
    let receipt = fixture.path("clipped-receipt.json");
    let result = value(&[
        "assets",
        "resolve-table",
        &input,
        &spec,
        "--output",
        &receipt,
        "--json",
    ]);
    assert_eq!(
        result["padding"],
        json!([{"offset":40,"length":8,"kind":"all-zero","fill":0}])
    );

    rom[36..40].copy_from_slice(&4u32.to_le_bytes());
    definition["rom"]["sha256"] = json!(ws_assets::sha256_hex(&rom));
    let input = fixture.put("overlap.ws", &rom);
    let spec = fixture.json("overlap.json", definition);
    let receipt = fixture.path("overlap-receipt.json");
    let result = value(&[
        "assets",
        "resolve-table",
        &input,
        &spec,
        "--output",
        &receipt,
        "--json",
    ]);
    assert_eq!(result["entries"][0]["offset"], 36);
    let map = fixture.path("overlap-map.json");
    let refused_receipt = fixture.path("refused-receipt.json");
    let attempt = command(&[
        "assets",
        "resolve-table",
        &input,
        &spec,
        "--output",
        &refused_receipt,
        "--map-out",
        &map,
        "--force",
    ]);
    assert!(!attempt.status.success());
    assert!(String::from_utf8_lossy(&attempt.stderr).contains("overlaps source table metadata"));
    assert!(!std::path::Path::new(&map).exists());
    assert!(!std::path::Path::new(&refused_receipt).exists());
}

#[test]
fn fixed_stride_footer_is_extractable_and_protected_from_replacement() {
    let fixture = Fixture::new();
    let mut rom = vec![0; 65536];
    rom[4..8].copy_from_slice(&4u32.to_le_bytes());
    let input = fixture.put("input.ws", &rom);
    let spec = fixture.json(
        "table.json",
        json!({
            "schema": "swankit-fixed-stride-table/1",
            "rom": {"size": rom.len(), "sha256": ws_assets::sha256_hex(&rom)},
            "table_offset": 0, "entry_count": 1, "entry_stride": 8,
            "offset_field": 0, "offset_unit": 1, "data_offset": 65532, "data_length": 4,
            "size": {"kind": "explicit-u32", "field_offset": 4, "unit": 1}
        }),
    );
    let receipt = fixture.path("receipt.json");
    let map = fixture.path("map.json");
    value(&[
        "assets",
        "resolve-table",
        &input,
        &spec,
        "--output",
        &receipt,
        "--map-out",
        &map,
        "--json",
    ]);
    let out = fixture.path("out");
    let extraction = value(&["assets", "extract", &input, &map, &out, "--json"]);
    assert_eq!(extraction["failures"], 0);
    assert_eq!(
        fs::read(std::path::Path::new(&out).join("entry.0000.bin")).unwrap(),
        [0; 4]
    );
    let payload = fixture.put("replacement.bin", b"ABCD");
    let patched = fixture.path("patched.ws");
    let attempt = command(&[
        "assets",
        "replace",
        &input,
        &map,
        "--id",
        "entry.0000",
        "--from",
        &payload,
        "-o",
        &patched,
        "--force",
    ]);
    assert!(!attempt.status.success());
    assert!(String::from_utf8_lossy(&attempt.stderr).contains("overlaps the ROM footer"));
    assert!(!std::path::Path::new(&patched).exists());
}

#[test]
fn lzs2_asset_map_is_extractable_and_refuses_replacement() {
    let fixture = Fixture::new();
    let mut rom = vec![0; 65536];
    rom[16..20].copy_from_slice(b"LZS2");
    rom[20] = 5;
    rom[24] = 4;
    rom[32..36].copy_from_slice(&[0x40, b'A', 3, 0]);
    let input = fixture.put("input.wsc", &rom);
    let map = fixture.json(
        "map.json",
        json!({
            "schema":"swankit-asset-map/1",
            "rom":{"size":rom.len(),"sha256":ws_assets::sha256_hex(&rom)},
            "assets":[{"id":"block","kind":"other","offset":16,"codec":"ws-lzs2"}]
        }),
    );
    let out = fixture.path("out");
    let result = value(&["assets", "extract", &input, &map, &out, "--json"]);
    assert_eq!(result["failures"], 0);
    let output = std::path::Path::new(&out);
    assert_eq!(fs::read(output.join("block.bin")).unwrap(), b"AAAAA");
    let index: Value =
        serde_json::from_slice(&fs::read(output.join("index.json")).unwrap()).unwrap();
    assert_eq!(index["assets"][0]["stored_length"], 20);
    let edited = fixture.put("edited.bin", b"AAAAA");
    let rejected = fixture.path("rejected.wsc");
    assert!(!command(&[
        "assets", "replace", &input, &map, "--id", "block", "--from", &edited, "-o", &rejected,
        "--force"
    ])
    .status
    .success());
    assert!(!std::path::Path::new(&rejected).exists());
}
