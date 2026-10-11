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
