use serde_json::json;
use ws_text::{CharacterMap, TextLimits};
#[test]
fn visible_ending_requires_an_explicit_terminator() {
    let map = CharacterMap::from_json(&serde_json::to_vec(&json!({"schema":"swankit-charmap/1","entries":[{"bytes":"01","text":"A"}],"controls":[{"code":255,"display":"[end]","ends":true}]})).unwrap()).unwrap();
    let mut slot = [0xaa; 8];
    assert!(ws_text::insert_text(&mut slot, 0, 8, &map, "A", 0).is_err());
    assert_eq!(slot, [0xaa; 8], "failed insertion is transactional");
    assert_eq!(
        map.encode("A", 8).unwrap(),
        [1],
        "fragment encoding stays available"
    );
    assert_eq!(
        ws_text::insert_text(&mut slot, 0, 8, &map, "A[end]", 0).unwrap(),
        2
    );
    assert_eq!(slot, [1, 255, 0, 0, 0, 0, 0, 0]);
    let bytes = map.encode("A[end]", 8).unwrap();
    assert_eq!(bytes, [1, 255]);
    assert!(
        map.decode(&bytes, TextLimits::default())
            .unwrap()
            .terminated
    );
}
#[test]
fn maximum_map_does_not_spend_quadratic_time_validating_codes() {
    let entries: Vec<_> = (0..65536u32)
        .map(|n| json!({"bytes":format!("{:02x}{:02x}",n&255,n>>8),"text":"x"}))
        .collect();
    let definition = serde_json::to_vec(
        &json!({"schema":"swankit-charmap/1","unit":"word-le","entries":entries}),
    )
    .unwrap();
    let start = std::time::Instant::now();
    let map = CharacterMap::from_json(&definition).unwrap();
    assert_eq!(
        map.decode(&[0, 0], TextLimits::default()).unwrap().text,
        "x"
    );
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "maximum map validation took {:?}",
        start.elapsed()
    );
}

#[test]
fn contextual_dash_uses_java_letter_or_decimal_digit_categories() {
    let resolved = |prev: char| {
        let definition = json!({"schema":"swankit-charmap/1","entries":[{"bytes":"01","text":prev.to_string()},{"bytes":"02","text":"\u{e00b}"}],"normalization":{"contextual_dash":{"placeholder":"\u{e00b}","counters":""}}});
        let map = CharacterMap::from_json(&serde_json::to_vec(&definition).unwrap()).unwrap();
        map.decode(&[1, 2], TextLimits::default()).unwrap().text
    };
    for prev in [
        '\u{b2}', '\u{2161}', '\u{2461}', '\u{345}', '\u{903}', '\u{24b6}', '\u{1c89}',
    ] {
        assert_eq!(resolved(prev), format!("{prev}ー"), "U+{:04X}", prev as u32);
    }
    for prev in ['A', '\u{669}'] {
        assert_eq!(resolved(prev), format!("{prev}－"));
    }
}
