mod review;
use review::decode;
use serde_json::json;

fn converted(
    expression: &str,
    width: usize,
    endian: &str,
    signed: bool,
) -> formatkit_core::Result<serde_json::Value> {
    decode(&format!(
        "import json\ndef main():\n print(json.dumps({{'bytes':list(({expression}).to_bytes({width},'{endian}',signed={}))}}))\n",
        if signed { "True" } else { "False" }
    ))
}

#[test]
fn requested_width_preserves_unsigned_and_signed_extrema() {
    for width in [1, 2, 4, 8, 15] {
        let bits = width * 8;
        for (expression, value, signed) in [
            ("0".to_owned(), 0, false),
            (format!("(1<<{bits})-1"), (1i128 << bits) - 1, false),
            (format!("(-1<<{})", bits - 1), -(1i128 << (bits - 1)), true),
            (
                format!("(1<<{})-1", bits - 1),
                (1i128 << (bits - 1)) - 1,
                true,
            ),
        ] {
            for endian in ["big", "little"] {
                let raw = if endian == "little" {
                    value.to_le_bytes()
                } else {
                    value.to_be_bytes()
                };
                let expected = if endian == "little" {
                    &raw[..width]
                } else {
                    &raw[16 - width..]
                };
                assert_eq!(
                    converted(&expression, width, endian, signed).unwrap(),
                    json!({"bytes":expected}),
                    "{expression}, {width}, {endian}, {signed}"
                );
            }
        }
    }
    for (expression, value, signed) in [
        ("(-1<<127)", i128::MIN, true),
        ("((1<<126)-1)+(1<<126)", i128::MAX, true),
        ("((1<<126)-1)+(1<<126)", i128::MAX, false),
    ] {
        for endian in ["big", "little"] {
            let expected = if endian == "little" {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            };
            assert_eq!(
                converted(expression, 16, endian, signed).unwrap(),
                json!({"bytes":expected})
            );
        }
    }
}

#[test]
fn requested_width_rejects_overflow_and_underflow() {
    for width in [1, 2, 4, 8, 15] {
        let bits = width * 8;
        for (expression, signed) in [
            (format!("1<<{bits}"), false),
            ("-1".to_owned(), false),
            (format!("1<<{}", bits - 1), true),
            (format!("(-1<<{})-1", bits - 1), true),
        ] {
            for endian in ["big", "little"] {
                assert!(
                    converted(&expression, width, endian, signed).is_err(),
                    "{expression}, {width}, {endian}, {signed}"
                );
            }
        }
    }
    assert!(converted("-1", 16, "big", false).is_err());
}

#[test]
fn zero_width_matches_empty_integer_conversion_domain() {
    for endian in ["big", "little"] {
        for signed in [false, true] {
            assert_eq!(
                converted("0", 0, endian, signed).unwrap(),
                json!({"bytes":[]})
            );
            assert!(converted("1", 0, endian, signed).is_err());
            assert!(converted("-2", 0, endian, signed).is_err());
        }
        assert_eq!(
            converted("-1", 0, endian, true).unwrap(),
            json!({"bytes":[]})
        );
        assert!(converted("-1", 0, endian, false).is_err());
    }
}
