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
    let source = format!("def main():\n pass\ndef decode_image(data,name):\n return {{'valid':True}}\ndef fix_image(data):\n data=bytearray(data)\n data[24]=85\n data[32:34]=(4660).to_bytes({argument})\n return bytes(data)\n");
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
    Format::from_json(&json!({"schema":"swankit-save-def/1","id":"synthetic-keywords","implementation":"generated",
        "media":[{"kind":"cartridge-eeprom","sizes":[128]}],"fields":[],"records":[],"blocks":[],
        "checksums":{},"primitives":{},"validity":{"policy":"program","exclude":[]},
        "entry":{"mode":"direct","decode":"decode_image","fix":"fix_image"},"program":program}).to_string()).unwrap()
}

#[test]
fn late_repeated_keyword_rejects_without_returning_mutated_save() {
    let toolkit = Toolkit::new().unwrap();
    let input = [0xa5; 128];
    for argument in [
        "2,byteorder='big',byteorder='little'",
        "length=1,length=2,byteorder='little'",
        "2,'little',signed=False,signed=True",
    ] {
        let error = toolkit
            .fix_save(
                &definition(argument),
                &input,
                "sample.eeprom",
                &DecodeContext::default(),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("duplicate"), "{argument}: {error}");
        assert_eq!(input, [0xa5; 128]);
    }
}

#[test]
fn neighboring_valid_calls_preserve_bytes_and_repeat_exactly() {
    let toolkit = Toolkit::new().unwrap();
    let input = [0xa5; 128];
    let mut expected = input;
    expected[24] = 85;
    expected[32..34].copy_from_slice(&[52, 18]);
    for argument in [
        "2,'little'",
        "2,byteorder='little'",
        "length=2,byteorder='little'",
    ] {
        let format = definition(argument);
        let output = toolkit
            .fix_save(&format, &input, "sample.eeprom", &DecodeContext::default())
            .unwrap();
        assert_eq!(output, expected, "{argument}");
        assert_eq!(
            toolkit
                .fix_save(&format, &output, "sample.eeprom", &DecodeContext::default())
                .unwrap(),
            output
        );
    }
    assert_eq!(input, [0xa5; 128]);
}
