//! Invented listing and instruction operands; no cartridge fixtures.
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    dir: PathBuf,
    rom: Vec<u8>,
    lines: Vec<String>,
    functions: Vec<String>,
}
fn core() -> Vec<Vec<u8>> {
    [
        "bf6745", "b80010", "8ec0", "33c0", "8ed8", "8ac6", "d1e0", "d1e0", "03f8", "268b4502",
        "e6c3", "268b3d", "50", "b80030", "8ec0", "32f6", "d1e2", "03fa", "268b3d", "8b36a408",
        "58", "8904", "8bc7", "894402", "894c04", "895c06", "83c608", "c7040000", "8936a408",
    ]
    .iter()
    .map(|s| {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    })
    .collect()
}
impl Fixture {
    fn new(ops: Vec<(Vec<u8>, &str, &str)>) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "retained-source-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let mut f = Self {
            dir,
            rom: vec![0xcc; 65536],
            lines: Vec::new(),
            functions: Vec::new(),
        };
        let mut helper = vec![(vec![0x50], "SP", "FALL_THROUGH")];
        helper.extend(
            core()
                .into_iter()
                .map(|v| (v, "AX,BX,CX,DX,SI,DI,DS,ES", "FALL_THROUGH")),
        );
        helper.push((vec![0xcb], "CS,SP", "TERMINATOR"));
        f.function(0x200, helper);
        let mut caller = ops;
        caller.push((vec![0x9a, 0, 0, 0x20, 0], "CS,SP", "UNCONDITIONAL_CALL"));
        f.function(0x100, caller);
        f
    }
    fn function(&mut self, start: usize, ops: Vec<(Vec<u8>, &str, &str)>) {
        let n = ops.len();
        let mut at = start;
        for (b, w, flow) in ops {
            let target = if b.len() == 5 && b[0] == 0x9a {
                format!(
                    "{:x}",
                    usize::from(u16::from_le_bytes([b[3], b[4]])) * 16
                        + usize::from(u16::from_le_bytes([b[1], b[2]]))
                )
            } else {
                "-".into()
            };
            self.row(at, Some(start), &b, w, flow, &target);
            at += b.len();
        }
        self.functions.push(format!(
            "F\t{start:x}\t{:04x}:{:04x}\thelper\t{n}\t0\t{start:x}\t{:x}",
            start >> 4,
            start & 15,
            at - 1
        ));
    }
    fn row(
        &mut self,
        at: usize,
        function: Option<usize>,
        b: &[u8],
        writes: &str,
        flow: &str,
        target: &str,
    ) {
        self.rom[at..at + b.len()].copy_from_slice(b);
        let hex: String = b.iter().map(|v| format!("{v:02x}")).collect();
        let owner = function
            .map(|v| format!("{v:x}"))
            .unwrap_or_else(|| "-".into());
        self.lines.push(format!(
            "I\t{at:x}\t{owner}\t{flow}\t{hex}\t{}\t.\t{writes}\toperation\t{target}",
            "ff".repeat(b.len())
        ));
    }
    fn path(&self, n: &str) -> String {
        self.dir.join(n).to_str().unwrap().into()
    }
    fn put(&self, n: &str, b: &[u8]) -> String {
        let p = self.path(n);
        fs::write(&p, b).unwrap();
        p
    }
    fn data(&self, n: &str, v: Value) -> String {
        self.put(n, &serde_json::to_vec(&v).unwrap())
    }
    fn inputs(&self) -> (String, String, String) {
        let rom = self.put("input.bin", &self.rom);
        let text = self
            .functions
            .iter()
            .chain(&self.lines)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        self.put("listing.tsv", text.as_bytes());
        let source=self.data("source.json",json!({"schema":"swankit-retained-source/1","rom_sha256":ws_assets::sha256_hex(&self.rom),"rom_bytes":self.rom.len(),"listing":{"path":"listing.tsv","sha256":ws_assets::sha256_hex(text.as_bytes()),"bytes":text.len()},"mapping":[{"logical_start":0,"physical_start":0,"length":65536}]}));
        let config = self.data(
            "config.json",
            json!({"families":["banked-queue-constructor"]}),
        );
        (rom, source, config)
    }
    fn command(&self) -> Output {
        let (r, s, c) = self.inputs();
        command(&[
            "assets",
            "search",
            &r,
            "--config",
            &c,
            "--retained-source",
            &s,
            "--output",
            &self.path("report.json"),
            "--json",
        ])
    }
    fn sidecar(&self) -> Value {
        let r = self.command();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        serde_json::from_slice(&fs::read(self.path("report.callers.json")).unwrap()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
fn command(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_swankit"))
        .args(args)
        .output()
        .unwrap()
}
fn fixed() -> Vec<(Vec<u8>, &'static str, &'static str)> {
    vec![
        (vec![0xba, 1, 0], "DX", "FALL_THROUGH"),
        (vec![0xbb, 0x56, 0x34], "BX", "FALL_THROUGH"),
        (vec![0xb9, 0, 0], "CX", "FALL_THROUGH"),
    ]
}
fn args(s: &Value) -> &Value {
    &s["callers"][0]["arguments"]
}
#[test]
fn original_bound_word_and_byte_definitions_create_linked_provenance() {
    for ops in [
        fixed(),
        vec![
            (vec![0xb6, 0], "DX", "FALL_THROUGH"),
            (vec![0xb2, 1], "DX", "FALL_THROUGH"),
            (vec![0xbb, 0x56, 0x34], "BX", "FALL_THROUGH"),
            (vec![0x33, 0xc9], "CX", "FALL_THROUGH"),
        ],
    ] {
        let f = Fixture::new(ops);
        let s = f.sidecar();
        assert_eq!(args(&s)["DX"]["value"], 1);
        assert_eq!(args(&s)["BX"]["value"], 0x3456);
        assert_eq!(args(&s)["CX"]["value"], 0);
        assert_eq!(s["constructors"][0]["function_entry"], 0x200);
        assert_eq!(s["constructors"][0]["core_logical"], 0x201);
        let r: Value = serde_json::from_slice(&fs::read(f.path("report.json")).unwrap()).unwrap();
        assert_eq!(
            r["report"]["leads"][0]["source_provenance"]["sha256"],
            ws_assets::sha256_hex(&fs::read(f.path("report.callers.json")).unwrap())
        );
        assert!(s["assurance"].as_str().unwrap().contains("incomplete"));
        assert_eq!(s["callers"][0]["pointer"]["status"], "HOLD");
    }
}
#[test]
fn missing_high_lane_is_hold_not_zero() {
    let f = Fixture::new(vec![(vec![0xb2, 1], "DX", "FALL_THROUGH")]);
    let s = f.sidecar();
    assert!(args(&s)["DX"]["value"].is_null());
    assert_eq!(args(&s)["DX"]["low"]["value"], 1);
    assert!(args(&s)["DX"]["high"].is_null());
}
#[test]
fn implicit_mul_and_computed_or_memory_writes_kill_values() {
    for (code, writes) in [
        (vec![0xf7, 0xe3], "AX,DX"),
        (vec![0x80, 0xc2, 8], "DX"),
        (vec![0x8b, 0x16, 0, 9], "DX"),
    ] {
        let mut ops = fixed();
        ops.push((code, writes, "FALL_THROUGH"));
        let f = Fixture::new(ops);
        let s = f.sidecar();
        assert!(args(&s)["DX"]["value"].is_null());
        assert_eq!(args(&s)["BX"]["value"], 0x3456);
        assert_eq!(args(&s)["CX"]["value"], 0);
    }
}
#[test]
fn branch_and_intervening_call_clear_all_reaching_values() {
    for (code, writes, flow) in [
        (vec![0x75, 0], ".", "CONDITIONAL_JUMP"),
        (vec![0xe8, 0, 0], "SP", "UNCONDITIONAL_CALL"),
        (vec![0xc3], "SP", "TERMINATOR"),
    ] {
        let mut ops = fixed();
        ops.push((code, writes, flow));
        let f = Fixture::new(ops);
        let s = f.sidecar();
        for r in ["DX", "BX", "CX"] {
            assert!(args(&s)[r]["value"].is_null());
        }
    }
}
#[test]
fn incoming_targets_from_other_functions_and_unowned_code_reset_state() {
    for owner in [true, false] {
        let mut f = Fixture::new(fixed());
        let displacement = (0x103i32 - 0x803) as u16;
        let code = vec![0xe9, displacement as u8, (displacement >> 8) as u8];
        if owner {
            f.functions
                .push("F\t800\t0080:0000\tother\t1\t0\t800\t802".into());
        }
        f.row(
            0x800,
            owner.then_some(0x800),
            &code,
            ".",
            "UNCONDITIONAL_JUMP",
            "103",
        );
        let s = f.sidecar();
        assert!(args(&s)["DX"]["value"].is_null());
        assert_eq!(args(&s)["BX"]["value"], 0x3456);
        assert_eq!(args(&s)["CX"]["value"], 0);
    }
}
#[test]
fn interior_instruction_entry_is_not_a_valid_canonical_transfer() {
    let mut f = Fixture::new(fixed());
    let displacement = (0x104i32 - 0x803) as u16;
    f.functions
        .push("F\t800\t0080:0000\tother\t1\t0\t800\t802".into());
    f.row(
        0x800,
        Some(0x800),
        &[0xe9, displacement as u8, (displacement >> 8) as u8],
        ".",
        "UNCONDITIONAL_JUMP",
        "104",
    );
    let s = f.sidecar();
    assert_eq!(s["callers"][0]["source_status"], "HOLD");
    for r in ["DX", "BX", "CX"] {
        assert!(args(&s)[r]["value"].is_null());
    }
}

#[test]
fn unsupported_effect_and_export_gap_are_not_silent_nonclobbers() {
    let mut ops = fixed();
    ops.push((vec![0xd1, 0xc0], "AX", "FALL_THROUGH"));
    let f = Fixture::new(ops);
    let s = f.sidecar();
    assert!(args(&s)["DX"]["value"].is_null());
    let mut f = Fixture::new(fixed());
    let original = f.rom.clone();
    for line in &mut f.lines {
        let mut columns: Vec<String> = line.split('\t').map(str::to_owned).collect();
        let at = usize::from_str_radix(&columns[1], 16).unwrap();
        if columns[2] == "100" && at >= 0x103 {
            let count = columns[4].len() / 2;
            f.rom[at + 1..at + 1 + count].copy_from_slice(&original[at..at + count]);
            columns[1] = format!("{:x}", at + 1);
            *line = columns.join("\t");
        }
    }
    for function in &mut f.functions {
        if function.starts_with("F\t100\t") {
            let mut c: Vec<_> = function.split('\t').map(str::to_owned).collect();
            c[7] = format!("{:x}", usize::from_str_radix(&c[7], 16).unwrap() + 1);
            *function = c.join("\t");
        }
    }
    let s = f.sidecar();
    assert!(args(&s)["DX"]["value"].is_null());
    assert_eq!(args(&s)["BX"]["value"], 0x3456);
}
#[test]
fn missing_partial_and_original_mismatch_sources_refuse() {
    for mode in [0, 1, 2, 3] {
        let mut f = Fixture::new(fixed());
        if mode == 1 {
            f.lines.remove(2);
        }
        if mode == 2 {
            f.rom[0x100] ^= 1;
        }
        if mode == 3 {
            f.functions[0] = f.functions[0].replace("0020:0000", "0030:0000");
        }
        let (r, s, c) = f.inputs();
        if mode == 0 {
            fs::remove_file(f.path("listing.tsv")).unwrap();
        }
        let got = command(&[
            "assets",
            "search",
            &r,
            "--config",
            &c,
            "--retained-source",
            &s,
            "--output",
            &f.path("bad.json"),
            "--json",
        ]);
        assert!(!got.status.success());
        assert!(!PathBuf::from(f.path("bad.json")).exists());
        assert!(!PathBuf::from(f.path("bad.callers.json")).exists());
    }
}
#[test]
fn absent_attachment_keeps_normal_report_shape() {
    let f = Fixture::new(fixed());
    let (r, _, c) = f.inputs();
    let got = command(&["assets", "search", &r, "--config", &c, "--json"]);
    assert!(got.status.success());
    let d: Value = serde_json::from_slice(&got.stdout).unwrap();
    assert!(d.get("retained_source").is_none());
    assert!(d["report"]["leads"][0].get("source_provenance").is_none());
    let c = f.data("empty.json", json!({"families":["reader-signature"]}));
    let got = command(&["assets", "search", &r, "--config", &c, "--json"]);
    let d: Value = serde_json::from_slice(&got.stdout).unwrap();
    assert_eq!(d["report"]["leads"].as_array().unwrap().len(), 0);
}
#[test]
fn provenance_output_cannot_overwrite_any_input() {
    let f = Fixture::new(fixed());
    let (r, s, c) = f.inputs();
    let old = fs::read(f.path("listing.tsv")).unwrap();
    assert!(!command(&[
        "assets",
        "search",
        &r,
        "--config",
        &c,
        "--retained-source",
        &s,
        "--provenance-output",
        &f.path("listing.tsv"),
        "--json"
    ])
    .status
    .success());
    assert_eq!(fs::read(f.path("listing.tsv")).unwrap(), old);
}
#[test]
fn batch_resume_rebinds_sources_and_recomputes_sidecars() {
    let f = Fixture::new(fixed());
    let (r, s, c) = f.inputs();
    let manifest=f.data("batch.json",json!({"schema":"swankit-format-search-inputs/1","inputs":[{"id":"sample","path":r,"retained_source":s}]}));
    let output = f.path("out");
    let base = [
        "assets",
        "search-corpus",
        &manifest,
        "--config",
        &c,
        "--output",
        &output,
        "--json",
    ];
    let first = command(&base);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let mut resume = base.to_vec();
    resume.push("--resume");
    assert!(command(&resume).status.success());
    let side = fs::read_dir(PathBuf::from(&output).join("provenance"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let original = fs::read(&side).unwrap();
    let mut v: Value = serde_json::from_slice(&original).unwrap();
    v["callers"][0]["arguments"]["DX"]["value"] = json!(7);
    fs::write(&side, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    assert!(!command(&resume).status.success());
    fs::write(&side, &original).unwrap();
    assert!(command(&resume).status.success());
    fs::remove_file(&side).unwrap();
    assert!(!command(&resume).status.success());
}

#[test]
fn core_bytes_without_compatible_instruction_boundaries_remain_pattern_only() {
    let mut f = Fixture::new(fixed());
    // Preserve original bytes and full span but move a retained boundary by one.
    let a = f
        .lines
        .iter()
        .position(|l| l.starts_with("I\t201\t"))
        .unwrap();
    let b = f
        .lines
        .iter()
        .position(|l| l.starts_with("I\t204\t"))
        .unwrap();
    let mut left: Vec<_> = f.lines[a].split('\t').map(str::to_owned).collect();
    let mut right: Vec<_> = f.lines[b].split('\t').map(str::to_owned).collect();
    left[4].push_str(&right[4][..2]);
    left[5].push_str("ff");
    right[4] = right[4][2..].into();
    right[5] = right[5][2..].into();
    right[1] = "205".into();
    f.lines[a] = left.join("\t");
    f.lines[b] = right.join("\t");
    let s = f.sidecar();
    assert_eq!(s["constructors"][0]["status"], "HOLD");
    assert!(s["callers"].as_array().unwrap().is_empty());
}
#[test]
fn changed_source_identity_and_missing_listing_fail_resume_actionably() {
    let f = Fixture::new(fixed());
    let (r, s, c) = f.inputs();
    let m=f.data("batch.json",json!({"schema":"swankit-format-search-inputs/1","inputs":[{"id":"sample","path":r,"retained_source":s}]}));
    let out = f.path("out");
    let base = [
        "assets",
        "search-corpus",
        &m,
        "--config",
        &c,
        "--output",
        &out,
        "--json",
    ];
    assert!(command(&base).status.success());
    let mut resume = base.to_vec();
    resume.push("--resume");
    let original = fs::read(&s).unwrap();
    let mut changed = original.clone();
    changed.push(b'\n');
    fs::write(&s, changed).unwrap();
    assert!(!command(&resume).status.success());
    fs::write(&s, original).unwrap();
    assert!(command(&resume).status.success());
    let original_config = fs::read(&c).unwrap();
    fs::write(
        &c,
        br#"{"families":["banked-queue-constructor"],"stride":2}"#,
    )
    .unwrap();
    assert!(!command(&resume).status.success());
    fs::write(&c, original_config).unwrap();
    assert!(command(&resume).status.success());
    fs::remove_file(f.path("listing.tsv")).unwrap();
    assert!(!command(&resume).status.success());
    let summary: Value =
        serde_json::from_slice(&fs::read(PathBuf::from(out).join("summary.json")).unwrap())
            .unwrap();
    assert_eq!(summary["failed"], 1);
}
#[test]
fn exact_reader_profiles_keep_their_existing_semantics_with_attachment() {
    let f = Fixture::new(fixed());
    let (r, s, _) = f.inputs();
    let config=f.data("mixed.json",json!({"families":["reader-signature","banked-queue-constructor"],"signatures":[{"id":"same-code","family":"reader-signature","bytes":core().concat()}]}));
    let got = command(&[
        "assets",
        "search",
        &r,
        "--config",
        &config,
        "--retained-source",
        &s,
        "--output",
        &f.path("mixed-report.json"),
        "--json",
    ]);
    assert!(
        got.status.success(),
        "{}",
        String::from_utf8_lossy(&got.stderr)
    );
    let r: Value = serde_json::from_slice(&got.stdout).unwrap();
    let leads = r["report"]["leads"].as_array().unwrap();
    assert_eq!(leads.len(), 2);
    for lead in leads {
        assert_eq!(
            lead.get("source_provenance").is_some(),
            lead["family"] == "banked-queue-constructor"
        );
    }
}
#[test]
fn forged_sidecar_hashes_and_report_references_do_not_replace_recomputation() {
    let f = Fixture::new(fixed());
    let (r, s, c) = f.inputs();
    let m=f.data("batch.json",json!({"schema":"swankit-format-search-inputs/1","inputs":[{"id":"sample","path":r,"retained_source":s}]}));
    let out = f.path("out");
    let base = [
        "assets",
        "search-corpus",
        &m,
        "--config",
        &c,
        "--output",
        &out,
        "--json",
    ];
    assert!(command(&base).status.success());
    let side = fs::read_dir(PathBuf::from(&out).join("provenance"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let report = fs::read_dir(PathBuf::from(&out).join("leads"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut v: Value = serde_json::from_slice(&fs::read(&side).unwrap()).unwrap();
    v["callers"][0]["arguments"]["DX"]["value"] = json!(7);
    let typed: ws_discover::retained::Sidecar = serde_json::from_value(v).unwrap();
    let bytes = serde_json::to_vec_pretty(&typed).unwrap();
    let hash = ws_assets::sha256_hex(&bytes);
    fs::write(side, bytes).unwrap();
    let mut v: Value = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
    v["retained_source"]["sidecar_sha256"] = json!(hash);
    v["report"]["leads"][0]["source_provenance"]["sha256"] = json!(hash);
    let typed: ws_discover::search::SearchReport =
        serde_json::from_value(v["report"].clone()).unwrap();
    v["report_sha256"] = json!(ws_assets::sha256_hex(&serde_json::to_vec(&typed).unwrap()));
    fs::write(report, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let mut resume = base.to_vec();
    resume.push("--resume");
    assert!(!command(&resume).status.success());
}

#[test]
fn mapped_producers_do_not_prove_an_unmapped_function_entry() {
    let mut ops = vec![(vec![0x90], ".", "FALL_THROUGH")];
    ops.extend(fixed());
    let f = Fixture::new(ops);
    let (r, s, c) = f.inputs();
    let mut attachment: Value = serde_json::from_slice(&fs::read(&s).unwrap()).unwrap();
    attachment["mapping"] =
        json!([{"logical_start":0x101,"physical_start":0x101,"length":65536-0x101}]);
    fs::write(&s, serde_json::to_vec(&attachment).unwrap()).unwrap();
    let got = command(&[
        "assets",
        "search",
        &r,
        "--config",
        &c,
        "--retained-source",
        &s,
        "--output",
        &f.path("unmapped.json"),
        "--json",
    ]);
    assert!(
        got.status.success(),
        "{}",
        String::from_utf8_lossy(&got.stderr)
    );
    let side: Value =
        serde_json::from_slice(&fs::read(f.path("unmapped.callers.json")).unwrap()).unwrap();
    assert_eq!(side["callers"][0]["source_status"], "HOLD");
    assert_eq!(side["unmapped_instructions"], 1);
    for r in ["DX", "BX", "CX"] {
        assert!(args(&side)[r]["value"].is_null());
    }
}

#[test]
fn incompatible_caller_instruction_extent_does_not_hide_a_register_write() {
    let mut ops = fixed();
    // This is two real instructions incorrectly exported as one long MOV row:
    // MOV SI,AX; MOV DX,9. Original-byte identity alone cannot validate its extent.
    ops.push((vec![0x8b, 0xf0, 0xba, 9, 0], "SI", "FALL_THROUGH"));
    let f = Fixture::new(ops);
    let side = f.sidecar();
    assert!(args(&side)["DX"]["value"].is_null());
    assert!(side["callers"][0]["pointer"].is_null());
}

#[test]
fn immediate_byte_compare_is_not_a_register_write() {
    let mut ops = fixed();
    ops.push((vec![0x80, 0xfa, 8], ".", "FALL_THROUGH"));
    let f = Fixture::new(ops);
    let side = f.sidecar();
    assert_eq!(args(&side)["DX"]["value"], 1);
}
