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
            "lz-word16-msb",
            swankit::compress::lz_word16_msb::encode(b"abcabc").unwrap(),
            None,
        ),
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
fn word_flag_definition_starts_at_mode_and_refuses_external_history() {
    let value = json!({"schema":"swankit-codec-def/1","codec":"lz-word16-msb",
        "offset":2,"length":8,"max_output":4});
    let mut def: extract::CodecDefinition =
        extract::definition(&serde_json::to_vec(&value).unwrap()).unwrap();
    let src = [0x34, 0x12, 1, 0, 0x80, 42, 0xf0, 0xff, 0, 0, 0xaa];
    let t = Toolkit::new().unwrap();
    let got = t.extract_codec(&src, &def).unwrap();
    assert_eq!(got.bytes, [42; 4]);
    assert_eq!(got.consumed, 8);
    def.history = vec![42];
    assert!(t.extract_codec(&src, &def).is_err());
    def.history.clear();
    def.length = 7;
    assert!(t.extract_codec(&src, &def).is_err());
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

#[test]
fn v2_constraint_rejects_by_default_and_explicit_override_keeps_storage_checks() {
    let toolkit = Toolkit::new().unwrap();
    let mut definition = save_definition();
    definition["schema"] = json!("swankit-save-def/2");
    definition["fields"][0]["size"] = json!(2);
    definition["fields"][0]["encoding"] = json!("u16le");
    definition["fields"][0]["constraint"] =
        json!({"kind":"unsigned-range","minimum":0,"maximum":1079});
    let format = swankit::saves::Format::from_json(&definition.to_string()).unwrap();
    let context = swankit::saves::DecodeContext::default();
    let bytes = [165, 1].repeat(64);
    assert!(toolkit
        .set_save(&format, &bytes, "save.eeprom", &context, 0, &json!(1080))
        .is_err());
    let valid = toolkit
        .set_save(&format, &bytes, "save.eeprom", &context, 0, &json!(720))
        .unwrap();
    assert_eq!(&valid[1..3], &[0xd0, 0x02]);
    let updated = toolkit
        .set_save_with_options(
            &format,
            &bytes,
            "save.eeprom",
            &context,
            0,
            &json!(1080),
            swankit::SaveEditOptions {
                skip_validation: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(&updated[1..3], &[0x38, 0x04]);
    assert!(toolkit
        .set_save_with_options(
            &format,
            &bytes,
            "save.eeprom",
            &context,
            0,
            &json!(65536),
            swankit::SaveEditOptions {
                skip_validation: true,
                ..Default::default()
            }
        )
        .is_err());
}

#[test]
fn v2_enum_bcd_and_named_copy_group_have_explicit_semantics() {
    let toolkit = Toolkit::new().unwrap();
    let context = swankit::saves::DecodeContext::default();
    let bytes = [165, 1].repeat(64);
    let mut enum_def = save_definition();
    enum_def["schema"] = json!("swankit-save-def/2");
    enum_def["fields"][0]["constraint"] = json!({"kind":"enum","values":[0,1,2]});
    let enum_format = swankit::saves::Format::from_json(&enum_def.to_string()).unwrap();
    assert!(toolkit
        .set_save(&enum_format, &bytes, "save.eeprom", &context, 0, &json!(3))
        .is_err());
    assert!(toolkit
        .set_save(&enum_format, &bytes, "save.eeprom", &context, 0, &json!(2))
        .is_ok());

    let mut bcd_def = save_definition();
    bcd_def["schema"] = json!("swankit-save-def/2");
    bcd_def["fields"][0]["size"] = json!(4);
    bcd_def["fields"][0]["encoding"] = json!("packed BCD, low byte first");
    bcd_def["fields"][0]["type"] = json!("bytes");
    bcd_def["fields"][0]["constraint"] = json!({"kind":"packed-bcd","digits":8,"byte_order":"little","minimum":0,"maximum":99999999});
    let bcd_format = swankit::saves::Format::from_json(&bcd_def.to_string()).unwrap();
    let valid_bcd = json!([0x20, 0x07, 0, 0]);
    assert!(toolkit
        .set_save(&bcd_format, &bytes, "save.eeprom", &context, 0, &valid_bcd)
        .is_ok());
    let invalid_bcd = json!([0xa0, 0, 0, 0]);
    assert!(toolkit
        .set_save(
            &bcd_format,
            &bytes,
            "save.eeprom",
            &context,
            0,
            &invalid_bcd
        )
        .is_err());
    assert!(toolkit
        .set_save_with_options(
            &bcd_format,
            &bytes,
            "save.eeprom",
            &context,
            0,
            &invalid_bcd,
            swankit::SaveEditOptions {
                skip_validation: true,
                ..Default::default()
            }
        )
        .is_ok());

    let mut group_def = save_definition();
    group_def["schema"] = json!("swankit-save-def/2");
    for (name, offset) in [("setting.copy_b", 10), ("setting.copy_c", 40)] {
        group_def["fields"].as_array_mut().unwrap().push(
            json!({"name":name,"medium":"cart EEPROM","offset":offset,"size":1,"encoding":"u8"}),
        );
    }
    group_def["copy_groups"] =
        json!([{"name":"setting.copies","fields":["setting","setting.copy_b","setting.copy_c"]}]);
    let group_format = swankit::saves::Format::from_json(&group_def.to_string()).unwrap();
    let scalar = toolkit
        .set_save(&group_format, &bytes, "save.eeprom", &context, 0, &json!(7))
        .unwrap();
    assert_eq!(
        (scalar[1], scalar[10], scalar[40]),
        (7, bytes[10], bytes[40])
    );
    let copied = toolkit
        .set_save_with_options(
            &group_format,
            &bytes,
            "save.eeprom",
            &context,
            0,
            &json!(7),
            swankit::SaveEditOptions {
                all_copies: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!((copied[1], copied[10], copied[40]), (7, 7, 7));
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

#[test]
fn counted_word_definition_anchors_source_window_and_refuses_history() {
    let value = json!({"schema":"swankit-codec-def/1","codec":"lz-word-counted", "offset":0xfffa,"length":6,"max_output":2});
    let mut def: extract::CodecDefinition =
        extract::definition(&serde_json::to_vec(&value).unwrap()).unwrap();
    let mut src = vec![0; 0x10001];
    src[0xfffa..0x10000].copy_from_slice(&[1, 0, 1, 0, 7, 9]);
    assert_eq!(
        Toolkit::new()
            .unwrap()
            .extract_codec(&src, &def)
            .unwrap()
            .bytes,
        [7, 9]
    );
    def.history = vec![0, 0];
    assert!(def.decode(&src).is_err());
    def.history.clear();
    def.offset = 0xfffb;
    src[0xfffb..0x10001].copy_from_slice(&[1, 0, 1, 0, 7, 9]);
    assert!(def.decode(&src).is_err());
}
