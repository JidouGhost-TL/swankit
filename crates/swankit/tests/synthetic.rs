use serde_json::json;
#[cfg(feature = "saves")]
use serde_json::Value;
use swankit::{extract, Toolkit};

fn rom() -> Vec<u8> {
    let mut b = vec![0; 65536];
    b[65520..].copy_from_slice(&[0xea, 0, 0, 0, 0xf0, 0, 1, 0, 2, 1, 0, 0, 0, 1, 0, 0]);
    let sum = b[..65534]
        .iter()
        .fold(0u16, |s, &v| s.wrapping_add(u16::from(v)));
    b[65534..].copy_from_slice(&sum.to_le_bytes());
    b
}
#[test]
fn composed_detection_and_typed_info_preserve_rom_policy() {
    assert!(extract::grayscale(1).is_err());
    assert!(extract::grayscale(usize::MAX).is_err());
    let t = Toolkit::new().unwrap();
    let mut b = rom();
    assert!(t
        .catalog()
        .module(swankit::text::catalog::FONT_STRIP)
        .is_some());
    assert!(t
        .catalog()
        .module(swankit::sound::catalog::SDK_SEQUENCE)
        .is_some());
    assert!(t
        .catalog()
        .module(swankit::compress::catalog::OKUMURA_ID)
        .is_some());
    assert_eq!(
        t.detect(&b, Some("ips")).selected.as_deref(),
        Some("ws-rom")
    );
    let info = t.info(&b).unwrap();
    assert!(info.checksum_ok);
    assert!(!info.declared_size_matches);
    b[0] = 1;
    assert!(!t.info(&b).unwrap().checksum_ok);
    assert_eq!(
        t.detect(b"PATCHEOF", None).selected.as_deref(),
        Some("ws-patch-ips")
    );
    assert!(t.detect(b"PATCH", Some("ips")).selected.is_none());
    assert!(t.detect(&[0; 16], Some("wsc")).selected.is_none());
    assert!(t.detect(&[0x7f, 0xff], None).selected.is_none());
    assert!(t.info(&[0; 16]).is_err());
}
#[test]
fn codec_definitions_dispatch_all_grammars_and_bound_outputs() {
    for (codec, stored, size) in [
        (
            "lz-rle-7f",
            swankit::compress::lz_rle_7f::encode(b"abcabc").unwrap(),
            None,
        ),
        (
            "lz-bits-16",
            swankit::compress::lz_bits_16::encode(b"abcabc").unwrap(),
            None,
        ),
        (
            "lzss-u16-zero",
            swankit::compress::lzss_u16_zero::encode(b"abcabc").unwrap(),
            Some(6),
        ),
        (
            "lzss-12",
            swankit::compress::lzss_12::encode(b"abcabc").unwrap(),
            Some(6),
        ),
        (
            "okumura-lzss",
            swankit::compress::okumura::encode(b"abcabc", swankit::compress::BitOrder::LsbFirst, 0),
            Some(6),
        ),
    ] {
        let value = json!({"schema":"swankit-codec-def/1","codec":codec,"length":stored.len(),"max_output":6,"decoded_size":size});
        let def: extract::CodecDefinition =
            extract::definition(&serde_json::to_vec(&value).unwrap()).unwrap();
        let out = def.decode(&stored).unwrap();
        assert_eq!(out.bytes, b"abcabc");
        assert_eq!(out.consumed, stored.len());
        let mut def = def;
        def.max_output = 5;
        assert!(def.decode(&stored).is_err());
        def.max_output = 6;
        def.offset = usize::MAX;
        assert!(def.decode(&stored).is_err());
    }
}
#[test]
fn zero_terminated_byte_codec_dispatch() {
    let value = json!({"schema":"swankit-codec-def/1","codec":"lz-u8-zero",
        "offset":1,"length":7,"max_output":8});
    let mut def: extract::CodecDefinition =
        extract::definition(&serde_json::to_vec(&value).unwrap()).unwrap();
    let source = [0xee, 2, b'A', b'B', 0x83, 1, 0, 0xaa];
    let out = Toolkit::new()
        .unwrap()
        .extract_codec(&source, &def)
        .unwrap();
    assert_eq!(out.bytes, b"ABABABAB");
    assert_eq!(out.consumed, 6);
    def.max_output = 7;
    assert!(def.decode(&source).is_err());
    def.max_output = 8;
    def.length = 5;
    assert!(def.decode(&source).is_err());
    def.length = 8;
    assert!(def.decode(&source).is_err());
}

#[test]
fn sound_definitions_produce_structured_records_and_reject_truncation() {
    for (kind, bytes) in [
        ("sdk-sequence", vec![0xff, 0]),
        ("cell-sequence", vec![0xff, 0, 0xff, 0xff]),
        ("byte-sequence", vec![0x9b]),
    ] {
        let def: extract::SoundDefinition = extract::definition(
            &serde_json::to_vec(
                &json!({"schema":"swankit-sound-def/1","format":kind,"length":bytes.len()}),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(def.extract(&bytes).unwrap()["terminated"], true);
        assert!(def.extract(&bytes[..bytes.len() - 1]).is_err());
    }
    let def: extract::SoundDefinition =
        extract::definition(br#"{"schema":"swankit-sound-def/1","format":"wave","length":16}"#)
            .unwrap();
    assert_eq!(def.extract(&[0x21; 16]).unwrap()["waves"][0][0], 1);
    assert!(extract::definition::<extract::SoundDefinition>(br#"{"unknown":1}"#).is_err());
}

#[test]
fn unsigned_pcm_selection_has_its_own_module_and_checked_definition() {
    let toolkit = Toolkit::new().unwrap();
    assert!(toolkit
        .catalog()
        .module(swankit::sound::catalog::UNSIGNED_PCM)
        .is_some());
    let def: extract::SoundDefinition = extract::definition(
        br#"{"schema":"swankit-sound-def/1","format":"unsigned-pcm","length":2,"sample_rate":12000}"#,
    )
    .unwrap();
    let result = toolkit.extract_sound(&[0, 255], &def).unwrap();
    assert_eq!(result["samples"], json!([-32768, 32512]));
    assert_eq!(result["sample_rate"], 12000);
}

#[cfg(feature = "saves")]
#[test]
fn save_media_validity_exclusions_and_transactional_edits() {
    let toolkit = Toolkit::new().unwrap();
    let mut definition = save_definition();
    let format = swankit::saves::Format::from_json(&definition.to_string()).unwrap();
    let context = swankit::saves::DecodeContext::default();
    let mut bytes = vec![0; 128];
    bytes[0] = 165;
    assert_eq!(
        toolkit.detect(&bytes, None).selected.as_deref(),
        Some("ws-save-image")
    );
    assert!(toolkit.detect(&bytes, None).save_media.len() > 1);
    assert!(format
        .recognize_with_context(&bytes, "save.eeprom", &context)
        .unwrap()
        .is_some());
    let updated = toolkit
        .set_save(&format, &bytes, "save.eeprom", &context, 0, &json!(7))
        .unwrap();
    assert_eq!(updated[1], 7);
    assert_eq!(bytes[1], 0);
    assert_eq!(updated[0], 165);
    assert!(toolkit
        .set_save(&format, &bytes, "save.eeprom", &context, 0, &json!(256))
        .is_err());
    assert!(toolkit
        .set_save(&format, &[0; 7], "save.eeprom", &context, 0, &json!(7))
        .is_err());
    bytes[0] = 0;
    assert!(format
        .recognize_with_context(&bytes, "save.eeprom", &context)
        .unwrap()
        .is_none());
    let fixed = toolkit
        .fix_save(&format, &bytes, "save.eeprom", &context)
        .unwrap();
    assert_eq!(fixed[0], 165);
    assert!(toolkit.auto_save(&[0; 7], "save.eeprom", &context).is_err());
    definition["validity"]["exclude"] = json!(["valid"]);
    let excluded = swankit::saves::Format::from_json(&definition.to_string()).unwrap();
    assert!(excluded
        .recognize_with_context(&fixed, "save.eeprom", &context)
        .unwrap()
        .is_none());
    definition["validity"]["policy"] = json!("unknown");
    let unsupported = swankit::saves::Format::from_json(&definition.to_string()).unwrap();
    assert!(unsupported
        .recognize_with_context(&fixed, "save.eeprom", &context)
        .is_err());
    definition["validity"]["policy"] = json!("program");
    definition["validity"]["exclude"] = json!([7]);
    let malformed = swankit::saves::Format::from_json(&definition.to_string()).unwrap();
    assert!(malformed
        .recognize_with_context(&fixed, "save.eeprom", &context)
        .is_err());
    assert!(swankit::save::context_from_json(
        br#"{"schema":"swankit-save-context/1","tables":{"bad":[256]}}"#
    )
    .is_err());
}

#[cfg(feature = "saves")]
fn save_definition() -> Value {
    let constant = |value: Value| json!({"t":"Constant","value":value});
    let data = json!({"t":"Name","id":"data"});
    let magic = json!({"t":"Subscript","value":data,"slice":constant(json!(0))});
    let setting = json!({"t":"Subscript","value":data,"slice":constant(json!(1))});
    let function = |name: &str, arguments: &[&str], body: Value| {
        json!({"t":"FunctionDef","name":name,"args":{
        "args":arguments.iter().map(|n|json!({"arg":n})).collect::<Vec<_>>(),"defaults":[]},"body":body})
    };
    json!({"schema":"swankit-save-def/1","id":"synthetic","implementation":"generated",
        "media":[{"kind":"cartridge-eeprom","sizes":[128]}],
        "fields":[{"name":"setting","medium":"cart EEPROM","offset":1,"size":1,"type":"unsigned","encoding":"u8","meaning":"setting","confidence":"synthetic","evidence":"synthetic"}],
        "records":[],"blocks":[],"checksums":{},"primitives":{},"validity":{"policy":"program","exclude":["working_copy"]},
        "entry":{"mode":"direct","decode":"decode_image","fix":"fix_image"},
        "program":{"t":"Module","body":[
            function("main",&[],json!([{"t":"Return","value":constant(Value::Null)}])),
            function("decode_image",&["data","name"],json!([{"t":"Return","value":{"t":"Dict",
                "keys":[constant(json!("valid")),constant(json!("setting"))],
                "values":[{"t":"Compare","left":magic,"ops":[{"t":"Eq"}],"comparators":[constant(json!(165))]},setting]}}])),
            function("fix_image",&["data"],json!([
                {"t":"Assign","targets":[magic],"value":constant(json!(165))},
                {"t":"Return","value":data}
            ]))
        ],"type_ignores":[]}})
}
