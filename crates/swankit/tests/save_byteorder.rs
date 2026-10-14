use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use swankit::{
    saves::{DecodeContext, Format},
    Toolkit,
};

fn definition(argument: &str) -> Format {
    let source = format!("def main():\n pass\ndef decode_image(data,name):\n return {{'valid':True}}\ndef fix_image(data):\n data=bytearray(data)\n value=int.from_bytes(data[0:2],'little')\n data[32:34]=value.to_bytes(2,{argument})\n return bytes(data)\n");
    let mut child = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../ws-saves/tests/review/python_ast.py"
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
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let program: Value = serde_json::from_slice(&output.stdout).unwrap();
    Format::from_json(&json!({"schema":"swankit-save-def/1","id":"synthetic-byteorder","implementation":"generated",
        "media":[{"kind":"cartridge-eeprom","sizes":[128]}],"fields":[],"records":[],"blocks":[],
        "checksums":{},"primitives":{},"validity":{"policy":"program","exclude":[]},
        "entry":{"mode":"direct","decode":"decode_image","fix":"fix_image"},"program":program}).to_string()).unwrap()
}

#[test]
fn save_fix_keyword_byteorder_writes_exact_bytes_and_is_idempotent() {
    let toolkit = Toolkit::new().unwrap();
    let context = DecodeContext::default();
    let mut input = [0xa5; 128];
    input[..2].copy_from_slice(&[52, 18]);
    for (argument, converted) in [
        ("'little'", [52, 18]),
        ("byteorder='little'", [52, 18]),
        ("'big'", [18, 52]),
        ("byteorder='big'", [18, 52]),
    ] {
        let format = definition(argument);
        let mut expected = input;
        expected[32..34].copy_from_slice(&converted);
        let output = toolkit
            .fix_save(&format, &input, "sample.eeprom", &context)
            .unwrap();
        assert_eq!(output, expected, "{argument}");
        assert_eq!(
            toolkit
                .fix_save(&format, &output, "sample.eeprom", &context)
                .unwrap(),
            output
        );
        assert_eq!(input[32..34], [0xa5, 0xa5]);
    }
}

#[test]
fn save_fix_rejects_misspelled_byteorder_before_returning_output() {
    let toolkit = Toolkit::new().unwrap();
    let input = [0xa5; 128];
    for argument in ["'littel'", "byteorder='littel'"] {
        let error = toolkit
            .fix_save(
                &definition(argument),
                &input,
                "sample.eeprom",
                &DecodeContext::default(),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("byteorder"), "{error}");
        assert_eq!(input, [0xa5; 128]);
    }
}
