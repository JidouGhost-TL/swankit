mod review;
use review::decode;
use serde_json::json;
#[test]
fn native_missing_arguments_return_errors_without_panicking() {
    for expr in ["struct.pack_into('B')", "dict().get()"] {
        let source = format!("import json, struct\ndef main():\n print(json.dumps({expr}))\n");
        let result = std::panic::catch_unwind(|| decode(&source));
        assert!(result.is_ok(), "malformed primitive must not panic: {expr}");
        assert!(result.unwrap().is_err());
    }
}
#[test]
fn map_equality_compares_non_json_values() {
    let out = decode("import json\ndef main():\n a={'x':bytes([1])}\n b={'x':bytes([2])}\n print(json.dumps({'equal':a==b,'same':a=={'x':bytes([1])}}))\n").unwrap();
    assert_eq!(out, json!({"equal":false,"same":true}));
}
#[test]
fn dictionary_spread_and_duplicate_literal_keys_follow_assignment() {
    let out = decode("import json\ndef main():\n a={'x':1,**{'x':2}}\n a['x']=3\n b={'x':1,'x':2}\n b['x']=4\n print(json.dumps({'a':a,'b':b}))\n").unwrap();
    assert_eq!(out, json!({"a":{"x":3},"b":{"x":4}}));
}
#[test]
fn signed_integer_shift_overflow_is_an_error() {
    assert!(
        decode("import json\ndef main():\n print(json.dumps({'bad': (1<<126)<<2}))\n").is_err()
    );
}
#[test]
fn unsigned_sixteen_byte_integer_never_wraps_negative() {
    assert!(decode("import json\ndef main():\n x=int.from_bytes(bytes([128]+[0]*15),'big')\n print(json.dumps({'negative':x<0}))\n").is_err());
    assert_eq!(decode("import json\ndef main():\n x=int.from_bytes(bytes([128]+[0]*15),'big',signed=True)\n print(json.dumps({'negative':x<0}))\n").unwrap(), json!({"negative":true}));
}
#[test]
fn list_and_bytearray_augmented_add_preserve_aliases() {
    let out = decode("import json\ndef main():\n a=[1]\n b=a\n a+=[2]\n x=bytearray([3])\n y=x\n x+=bytes([4])\n print(json.dumps({'list':b,'bytes':list(y)}))\n").unwrap();
    assert_eq!(out, json!({"list":[1,2],"bytes":[3,4]}));
}
#[test]
fn recursive_definition_returns_error_in_a_debug_subprocess() {
    if std::env::var_os("SWANKIT_DEPTH_CHILD").is_some() {
        assert!(decode("import json\ndef r(n):\n return 0 if n==0 else 1+r(n-1)\ndef main():\n print(json.dumps({'x':r(127)}))\n").is_err());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "recursive_definition_returns_error_in_a_debug_subprocess",
            "--nocapture",
        ])
        .env("SWANKIT_DEPTH_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "recursive parser must return Err without aborting: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn mixed_sort_comparisons_return_error_without_panicking() {
    let mut state = 113u64;
    for _ in 0..64 {
        let values: Vec<_> = (0..64)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                if state.is_multiple_of(3) {
                    format!("'{}'", state % 100)
                } else {
                    format!("{}", state % 100)
                }
            })
            .collect();
        let source = format!(
            "import json\ndef main():\n print(json.dumps(sorted([{}])))\n",
            values.join(",")
        );
        let result = std::panic::catch_unwind(|| decode(&source));
        assert!(
            result.is_ok(),
            "incomparable sort keys must return an error without panicking"
        );
        assert!(result.unwrap().is_err());
    }
}

#[test]
fn cyclic_values_return_errors_without_aborting_a_subprocess() {
    if std::env::var_os("SWANKIT_CYCLE_CHILD").is_some() {
        for source in [
            "import json\ndef main():\n a={}\n a['self']=a\n b={}\n b['self']=b\n print(json.dumps({'equal':a==b}))\n",
            "import json\ndef main():\n a=[]\n a.append(a)\n print(json.dumps(a))\n",
        ] { assert!(decode(source).is_err()); }
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cyclic_values_return_errors_without_aborting_a_subprocess",
            "--nocapture",
        ])
        .env("SWANKIT_CYCLE_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn aliased_json_expansion_is_bounded() {
    assert!(
        decode(
            "import json\ndef main():\n a=list(range(10000))\n b=[a]*200\n print(json.dumps(b))\n"
        )
        .is_err(),
        "shared small containers can still expand beyond JSON output budget"
    );
}
