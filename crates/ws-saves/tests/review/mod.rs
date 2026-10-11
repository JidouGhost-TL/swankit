use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use ws_saves::Format;
pub fn format(source: &str) -> Format {
    let mut child = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/review/python_ast.py"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let program: Value = serde_json::from_slice(&out.stdout).unwrap();
    Format::from_json(&json!({"schema":"swankit-save-def/1","id":"synthetic", "implementation":"generated",
        "media":[{"kind":"cartridge-eeprom","sizes":[128]}], "fields":[], "records":[], "blocks":[], "checksums":{}, "primitives":{},
        "validity":{"policy":"program","exclude":[]}, "entry":{"mode":"sys-argv"}, "program":program}).to_string()).unwrap()
}
pub fn decode(source: &str) -> formatkit_core::Result<Value> {
    format(source).decode(&[0; 128], "sample.eeprom")
}
