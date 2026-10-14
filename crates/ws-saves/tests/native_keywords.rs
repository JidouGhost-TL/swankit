mod review;
use review::decode;
use serde_json::json;

fn converted(call: &str) -> formatkit_core::Result<serde_json::Value> {
    decode(&format!(
        "import json\ndef main():\n print(json.dumps({{'bytes':list({call})}}))\n"
    ))
}

#[test]
fn repeated_byteorder_keywords_reject_before_last_value_wins() {
    for call in [
        "(4660).to_bytes(2,byteorder='big',byteorder='little')",
        "(4660).to_bytes(2,byteorder='little',byteorder='little')",
        "(4660).to_bytes(2,byteorder='big',byteorder=(1//0))",
    ] {
        let error = converted(call).unwrap_err().to_string();
        assert!(
            error.contains("duplicate") && error.contains("byteorder"),
            "{call}: {error}"
        );
    }
}

#[test]
fn repeated_keyword_names_reject_for_other_parameters_and_functions() {
    for (call, name) in [
        (
            "(4660).to_bytes(length=1,length=2,byteorder='big')",
            "length",
        ),
        (
            "(4660).to_bytes(2,'big',signed=False,signed=True)",
            "signed",
        ),
    ] {
        let error = converted(call).unwrap_err().to_string();
        assert!(
            error.contains("duplicate") && error.contains(name),
            "{call}: {error}"
        );
    }
    let error = decode(
        "import json\ndef value(x):\n return x\ndef main():\n print(json.dumps(value(x=1,x=2)))\n",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("duplicate") && error.contains("x"),
        "{error}"
    );
}

#[test]
fn single_keyword_and_positional_calls_remain_valid() {
    for call in [
        "(4660).to_bytes(2,'little')",
        "(4660).to_bytes(2,byteorder='little')",
        "(4660).to_bytes(length=2,byteorder='little')",
        "(4660).to_bytes(byteorder='little',length=2)",
    ] {
        assert_eq!(converted(call).unwrap(), json!({"bytes":[52,18]}), "{call}");
    }
}

#[test]
fn dictionary_literal_duplicates_and_assignment_keep_last_value() {
    let output = decode("import json\ndef main():\n d={'byteorder':'big','byteorder':'little','count':1,'count':2}\n d['count']=3\n print(json.dumps(d))\n").unwrap();
    assert_eq!(output, json!({"byteorder":"little","count":3}));
}
