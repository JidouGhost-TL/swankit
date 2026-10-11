use serde_json::json;
mod review;
use review::{decode, format};
#[test]
fn normalize_keeps_nested_semantic_file_numbers() {
    let mut value = json!({"file":"transport", "slots":[{"file":2, "nested":{"file":3}}]});
    ws_saves::normalize(&mut value);
    assert_eq!(value, json!({"slots":[{"file":2,"nested":{"file":3}}]}));
}
#[test]
fn python_shift_jis_preserves_jis_wave_dash_and_minus() {
    assert_eq!(decode("import json\ndef main():\n print(json.dumps({'text': bytes([129,96,129,124]).decode('shift_jis')}))\n").unwrap()["text"], "〜−");
}
#[test]
fn python_shift_jis_rejects_vendor_extensions() {
    assert!(decode("import json\ndef main():\n print(json.dumps({'text': bytes([135,64]).decode('shift_jis')}))\n").is_err());
}
#[test]
fn repair_nonzero_return_is_transactional_failure() {
    let f = format(
        "def main():\n with open('repaired','wb') as f:\n  f.write(bytes([9])*128)\n return 1\n",
    );
    let mut image = [0; 128];
    assert!(f.fix(&mut image, "sample.eeprom").is_err());
    assert_eq!(image, [0; 128]);
}

#[test]
fn recognition_does_not_decode_companion_medium_as_primary() {
    let original = review::format("import json\ndef main():\n print(json.dumps({'valid':True}))\n");
    let mut definition = original.definition().clone();
    definition["media"] = json!([
        {"kind":"cartridge-eeprom","sizes":[128]},
        {"kind":"internal-eeprom","sizes":[32]}
    ]);
    let format = ws_saves::Format::from_json(&definition.to_string()).unwrap();
    assert!(
        format.accepts_size(32),
        "companion remains a declared physical medium"
    );
    assert!(
        format
            .recognize_with_context(&[0; 32], "console.eeprom", &Default::default())
            .unwrap()
            .is_none(),
        "companion must not enter primary semantic recognition"
    );
    assert!(format
        .recognize_with_context(&[0; 128], "primary.eeprom", &Default::default())
        .unwrap()
        .is_some());
}

#[test]
fn python_cp932_preserves_private_single_bytes_and_invalid_pair_recovery() {
    assert_eq!(decode("import json\ndef main():\n print(json.dumps({'text':bytes([160,253,254,255]).decode('cp932')}))\n").unwrap()["text"], "\u{f8f0}\u{f8f1}\u{f8f2}\u{f8f3}");
    assert_eq!(decode("import json\ndef main():\n print(json.dumps({'text':bytes([129,160]).decode('cp932')}))\n").unwrap()["text"], "□");
    assert_eq!(decode("import json\ndef main():\n print(json.dumps({'text':bytes([129,253]).decode('cp932','replace')}))\n").unwrap()["text"], "�\u{f8f1}");
    assert_eq!(decode("import json\ndef main():\n print(json.dumps({'text':bytes([129,173]).decode('cp932','ignore')}))\n").unwrap()["text"], "ｭ");
}
