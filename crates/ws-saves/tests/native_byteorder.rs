mod review;
use review::decode;
use serde_json::json;

fn converted(call: &str) -> formatkit_core::Result<serde_json::Value> {
    decode(&format!(
        "import json\ndef main():\n print(json.dumps({{'bytes':list({call})}}))\n"
    ))
}

#[test]
fn real_ast_binds_positional_and_keyword_byteorder() {
    for call in [
        "(0x1234).to_bytes(2,'little')",
        "(0x1234).to_bytes(2,byteorder='little')",
        "(0x1234).to_bytes(length=2,byteorder='little')",
        "(0x1234).to_bytes(byteorder='little',length=2)",
    ] {
        assert_eq!(converted(call).unwrap(), json!({"bytes":[52,18]}), "{call}");
    }
    for call in [
        "(0x1234).to_bytes(2,'big')",
        "(0x1234).to_bytes(2,byteorder='big')",
        "(0x1234).to_bytes(length=2,byteorder='big')",
        "(0x1234).to_bytes(2)",
    ] {
        assert_eq!(converted(call).unwrap(), json!({"bytes":[18,52]}), "{call}");
    }
    assert_eq!(
        converted("(-0x1234).to_bytes(2,byteorder='little',signed=True)").unwrap(),
        json!({"bytes":[204,237]})
    );
}

#[test]
fn real_ast_rejects_invalid_and_duplicate_byteorder_with_diagnostic() {
    for order in ["'littel'", "''", "'Little'", "None", "True", "7"] {
        for argument in [order.to_owned(), format!("byteorder={order}")] {
            let call = format!("(0x1234).to_bytes(2,{argument})");
            let error = converted(&call).unwrap_err().to_string();
            assert!(error.contains("byteorder"), "{call}: {error}");
        }
    }
    let error = converted("(0x1234).to_bytes(2,'big',byteorder='little')")
        .unwrap_err()
        .to_string();
    assert!(error.contains("byteorder"), "{error}");
    assert!(converted("(0x1234).to_bytes(2,'little',True)").is_err());
    assert!(converted("(0x1234).to_bytes(2,other='little')").is_err());
    assert!(converted("(0x1234).to_bytes(2,length=2)").is_err());
}

#[test]
fn keyword_binding_retains_signed_and_unsigned_width_guards() {
    for (call, bytes) in [
        ("(255).to_bytes(1,byteorder='little')", vec![255]),
        ("(-128).to_bytes(1,byteorder='big',signed=True)", vec![128]),
        (
            "(127).to_bytes(length=1,byteorder='little',signed=True)",
            vec![127],
        ),
        ("(0).to_bytes(length=0,byteorder='big')", vec![]),
        ("(-1).to_bytes(0,byteorder='little',signed=True)", vec![]),
    ] {
        assert_eq!(converted(call).unwrap(), json!({"bytes":bytes}));
    }
    for call in [
        "(256).to_bytes(1,byteorder='little')",
        "(-1).to_bytes(length=1,byteorder='big')",
        "(128).to_bytes(1,byteorder='little',signed=True)",
        "(-129).to_bytes(length=1,byteorder='big',signed=True)",
        "(1).to_bytes(0,byteorder='little')",
    ] {
        assert!(converted(call).is_err(), "{call}");
    }
}
