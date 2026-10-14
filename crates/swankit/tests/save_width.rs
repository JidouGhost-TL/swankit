use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use swankit::{
    saves::{DecodeContext, Format},
    Toolkit,
};

fn format(width: usize, signed: bool, endian: &str) -> Format {
    let input_width = width + 1;
    let signed = if signed { "True" } else { "False" };
    let source = format!("import struct\ndef main():\n pass\ndef decode_image(data,name):\n return {{'valid':True}}\ndef fix_image(data):\n data=bytearray(data)\n value=int.from_bytes(data[0:{input_width}],'little',signed={signed})\n data[32:{}]=value.to_bytes({width},'{endian}',signed={signed})\n return bytes(data)\n",32+width);
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
    Format::from_json(&json!({"schema":"swankit-save-def/1","id":"synthetic-width","implementation":"generated",
        "media":[{"kind":"cartridge-eeprom","sizes":[128]}],
        "fields":[{"name":"source-counter","medium":"cart EEPROM","offset":0,"size":input_width,"type":"bytes","encoding":"-","meaning":"synthetic width control","confidence":"synthetic","evidence":"synthetic"}],
        "records":[],"blocks":[],"checksums":{},"primitives":{},"validity":{"policy":"program","exclude":[]},
        "entry":{"mode":"direct","decode":"decode_image","fix":"fix_image"},"program":program}).to_string()).unwrap()
}

#[test]
fn save_set_extrema_preserve_bytes_and_repeat_exactly() {
    let toolkit = Toolkit::new().unwrap();
    let context = DecodeContext::default();
    for width in [1, 2, 4, 8, 15] {
        for signed in [false, true] {
            let bits = width * 8;
            let low = if signed { -(1i128 << (bits - 1)) } else { 0 };
            let high = (1i128 << (bits - if signed { 1 } else { 0 })) - 1;
            for endian in ["big", "little"] {
                let definition = format(width, signed, endian);
                for value in [low, high] {
                    let input = [0xa5; 128];
                    let stored = value.to_le_bytes()[..width + 1].to_vec();
                    let mut expected = input;
                    expected[..width + 1].copy_from_slice(&stored);
                    let raw = if endian == "little" {
                        value.to_le_bytes()
                    } else {
                        value.to_be_bytes()
                    };
                    let narrow = if endian == "little" {
                        &raw[..width]
                    } else {
                        &raw[16 - width..]
                    };
                    expected[32..32 + width].copy_from_slice(narrow);
                    let updated = toolkit
                        .set_save(
                            &definition,
                            &input,
                            "sample.eeprom",
                            &context,
                            0,
                            &json!(stored),
                        )
                        .unwrap();
                    assert_eq!(updated, expected);
                    assert_eq!(
                        toolkit
                            .set_save(
                                &definition,
                                &updated,
                                "sample.eeprom",
                                &context,
                                0,
                                &json!(stored)
                            )
                            .unwrap(),
                        updated
                    );
                    assert_eq!(
                        toolkit
                            .fix_save(&definition, &updated, "sample.eeprom", &context)
                            .unwrap(),
                        updated
                    );
                }
            }
        }
    }
}

#[test]
fn save_set_rejects_repair_overflow_and_underflow() {
    let toolkit = Toolkit::new().unwrap();
    let context = DecodeContext::default();
    for width in [1, 2, 4, 8, 15] {
        for signed in [false, true] {
            let bits = width * 8;
            let low = if signed { -(1i128 << (bits - 1)) } else { 0 };
            let high = (1i128 << (bits - if signed { 1 } else { 0 })) - 1;
            for endian in ["big", "little"] {
                let definition = format(width, signed, endian);
                for value in [low - 1, high + 1] {
                    if !signed && value < 0 {
                        continue;
                    }
                    let input = [0xa5; 128];
                    let stored = value.to_le_bytes()[..width + 1].to_vec();
                    assert!(
                        toolkit
                            .set_save(
                                &definition,
                                &input,
                                "sample.eeprom",
                                &context,
                                0,
                                &json!(stored)
                            )
                            .is_err(),
                        "width={width}, signed={signed}, endian={endian}, value={value}"
                    );
                    assert_eq!(input, [0xa5; 128]);
                }
            }
        }
    }
}
