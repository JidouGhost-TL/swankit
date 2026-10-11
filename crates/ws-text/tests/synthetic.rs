//! Small artificial records exercise grammar, round trips and rejection paths.
use formatkit_core::Error;
use serde_json::{json, Value};
use ws_text::*;

fn map(mut value: Value) -> CharacterMap {
    value["schema"] = json!("swankit-charmap/1");
    CharacterMap::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
}

fn simple() -> CharacterMap {
    map(
        json!({"entries":[{"bytes":"01","text":"A"},{"bytes":"02","text":"B"}],
        "controls":[{"code":255,"ends":true}]}),
    )
}

#[test]
fn tsv_preserves_empty_last_column_and_json_text_escapes() {
    let m = CharacterMap::from_tsv(
        "bytes\ttext\tpage\n01\t\"A\"\t\n02\t\"\\n\"\t0\n",
        CodeUnit::Byte,
    )
    .unwrap();
    assert_eq!(
        m.decode(&[1, 2], TextLimits::default()).unwrap().text,
        "A\n"
    );
    assert!(CharacterMap::from_tsv("01\tA", CodeUnit::Byte).is_err());
    assert!(CharacterMap::from_tsv("01\t\"A\"\tbad", CodeUnit::Byte).is_err());
    for bad in ["", "0", "gg", "é0"] {
        assert!(hex_bytes(bad).is_err());
    }
}

#[test]
fn pages_and_longest_prefix_keep_invisible_source_tokens() {
    let m = map(json!({"entries":[{"bytes":"01","text":"A","page":0},
        {"bytes":"01","text":"B","page":1},{"bytes":"0203","text":"Z"},
        {"bytes":"02","text":"C"}],"controls":[{"code":240,"set_page":0},
        {"code":241,"set_page":1},{"code":255,"ends":true}]}));
    let source = [1, 241, 1, 2, 3, 255, 99];
    let d = m.decode(&source, TextLimits::default()).unwrap();
    assert_eq!(d.text, "ABZ");
    assert_eq!(d.consumed, 6);
    assert_eq!(d.final_page, 1);
    assert!(d.terminated);
    assert_eq!(
        m.encode_tokens(&d.tokens, TextLimits::default()).unwrap(),
        source[..6]
    );
    assert_eq!(m.encode(&d.text, 6).unwrap(), source[..6]);
    let mut forged = d.tokens;
    forged[0].text = "B".into();
    assert!(m.encode_tokens(&forged, TextLimits::default()).is_err());
}

#[test]
fn little_endian_controls_have_conditional_operand_lengths() {
    let m = map(
        json!({"unit":"word-le","entries":[{"bytes":"0100","text":"A"}],
        "controls":[{"code":61952,"mask":65280,"display":"{c{lo:d}{args}}",
        "argument_unit":"word-le","argument_separator":":",
        "argument_lengths":[{"low_min":0,"low_max":1,"bytes":4}]},
        {"code":65280,"mask":65280,"ends":true}]}),
    );
    let bytes = [1, 0, 1, 242, 0x34, 0x12, 0xcd, 0xab, 2, 242, 7, 255];
    let d = m.decode(&bytes, TextLimits::default()).unwrap();
    assert_eq!(d.text, "A{c1:1234:abcd}{c2}");
    assert_eq!(
        m.encode_tokens(&d.tokens, TextLimits::default()).unwrap(),
        bytes
    );
    assert_eq!(
        m.decode(&m.encode(&d.text, 32).unwrap(), TextLimits::default())
            .unwrap()
            .text,
        d.text
    );
    assert!(m.decode(&[1, 242, 0], TextLimits::default()).is_err());
    assert!(m.decode(&[1], TextLimits::default()).is_err());
    let be = map(json!({"unit":"word-be","entries":[{"bytes":"1234","text":"X"}]}));
    assert_eq!(
        be.decode(&[0x12, 0x34], TextLimits::default())
            .unwrap()
            .text,
        "X"
    );
}

#[test]
fn partially_masked_low_byte_controls_encode_the_requested_value() {
    let m = map(json!({"controls":[{"code":224,"mask":240,"display":"[{lo:02x}]"}]}));
    assert_eq!(m.encode("[e3]", 1).unwrap(), [0xe3]);
}

#[test]
fn controls_can_display_operands_without_a_literal_prefix() {
    let m = map(json!({"controls":[{"code":240,"argument_bytes":1,"display":"{args}!"}]}));
    let d = m.decode(&[240, 0xab], TextLimits::default()).unwrap();
    assert_eq!(d.text, "ab!");
    assert_eq!(m.encode(&d.text, 2).unwrap(), [240, 0xab]);
}

#[test]
fn expanded_control_spellings_have_a_retention_budget() {
    let controls: Vec<_> = (0x8000..0xa100)
        .step_by(256)
        .map(|code| json!({"code":code,"mask":65280,"display":"X".repeat(1024)}))
        .collect();
    let definition = json!({"schema":"swankit-charmap/1","unit":"word-le","controls":controls});
    assert!(matches!(
        CharacterMap::from_json(&serde_json::to_vec(&definition).unwrap()),
        Err(Error::ResourceLimit {
            resource: "compiled control spelling bytes",
            ..
        })
    ));
}

#[test]
fn masked_attributes_and_dash_context_round_trip() {
    let m = map(
        json!({"unit":"word-le","glyph_mask":{"below":32768,"mask":2047},
        "entries":[{"bytes":"0100","text":"カ"},{"bytes":"0200","text":"\u{e00b}","aliases":["ー","一","－"]},
        {"bytes":"0300","text":"人"}],
        "normalization":{"contextual_dash":{"placeholder":"\u{e00b}","counters":"人"}}}),
    );
    let input = [1, 0x18, 2, 0, 2, 0, 3, 0];
    let d = m.decode(&input, TextLimits::default()).unwrap();
    assert_eq!(d.text, "カー一人");
    assert_eq!(
        m.encode_tokens(&d.tokens, TextLimits::default()).unwrap(),
        input
    );
    assert_eq!(m.encode(&d.text, 8).unwrap(), [1, 0, 2, 0, 2, 0, 3, 0]);
}

#[test]
fn kana_normalization_does_not_change_raw_cell_tokens() {
    let m = map(
        json!({"entries":[{"bytes":"01","text":"ホ"},{"bytes":"02","text":"゛"},
        {"bytes":"03","text":"－"}],"normalization":{"compose_kana":true,"long_dash_after_kana":true}}),
    );
    let d = m.decode(&[1, 2, 3], TextLimits::default()).unwrap();
    assert_eq!(d.text, "ホ゛－");
    assert_eq!(d.normalized, "ボー");
    assert_eq!(
        m.encode_tokens(&d.tokens, TextLimits::default()).unwrap(),
        [1, 2, 3]
    );
}

#[test]
fn standard_encodings_are_strict_and_have_explicit_roman_policy() {
    let sjis = [0x82, 0xa0, 0x82, 0xa2, 0xb6, 0xde];
    assert_eq!(
        decode_standard(TextEncoding::ShiftJis, &sjis).unwrap(),
        "あいｶﾞ"
    );
    assert_eq!(
        encode_standard(TextEncoding::ShiftJis, "あいｶﾞ").unwrap(),
        sjis
    );
    for bad in [&[0x82][..], &[0x82, 0x22][..], &[0xff][..]] {
        assert!(decode_standard(TextEncoding::ShiftJis, bad).is_err());
    }
    assert!(encode_standard(TextEncoding::ShiftJis, "🦢").is_err());
    assert_eq!(
        decode_standard(TextEncoding::JisX0201, &[0x5c, 0x7e, 0xa1, 0xdf]).unwrap(),
        "¥‾｡ﾟ"
    );
    assert_eq!(
        encode_standard(TextEncoding::JisX0201, "¥‾｡ﾟ").unwrap(),
        [0x5c, 0x7e, 0xa1, 0xdf]
    );
    assert!(encode_standard(TextEncoding::JisX0201, "\\~").is_err());
    assert!(decode_standard(TextEncoding::JisX0201, &[0x80]).is_err());
    let m = map(json!({"encoding":"shift-jis","controls":[{"code":0,"ends":true}]}));
    assert_eq!(
        m.decode(&[0x82, 0xa0, 0], TextLimits::default())
            .unwrap()
            .text,
        "あ"
    );
    assert_eq!(m.encode("あ", 3).unwrap(), [0x82, 0xa0, 0]);
}

#[test]
fn invalid_definitions_are_rejected_before_parsing() {
    for v in [
        json!({"entries":[{"bytes":"01","text":"A"},{"bytes":"01","text":"B"}]}),
        json!({"entries":[{"bytes":"01","text":"A"},{"bytes":"01","text":"B","page":1}]}),
        json!({"unit":"word-le","entries":[{"bytes":"01","text":"A"}]}),
        json!({"entries":[{"bytes":"01","text":""}]}),
        json!({"controls":[{"code":1,"argument_bytes":1,"argument_unit":"word-le"}]}),
        json!({"controls":[{"code":224,"mask":240},{"code":225}]}),
        json!({"controls":[{"code":256}]}),
        json!({"controls":[{"code":1,"display":"{args}{args}"}]}),
        json!({"entries":[{"bytes":"0102","text":"A"}],"controls":[{"code":1}]}),
        json!({"controls":[{"code":1,"display_overrides":[{"low_min":2,"low_max":1,"display":"X"}]}]}),
        json!({"controls":[{"code":1,"argument_lengths":[{"low_min":0,"low_max":2,"bytes":1},
            {"low_min":2,"low_max":3,"bytes":1}]}]}),
    ] {
        let mut v = v;
        v["schema"] = json!("swankit-charmap/1");
        assert!(
            CharacterMap::from_json(&serde_json::to_vec(&v).unwrap()).is_err(),
            "{v}"
        );
    }
    assert!(CharacterMap::from_json(br#"{"schema":"unknown"}"#).is_err());
    assert!(CharacterMap::from_json(br#"{"schema":"swankit-charmap/1","typo":1}"#).is_err());
    for id in [
        "byte-pages-1",
        "word-dialog-1",
        "word-dialog-2",
        "word-mask-1",
        "byte-prefix-1",
    ] {
        builtin_definition(id).unwrap();
    }
}

#[test]
fn text_limits_and_unknown_input_return_errors() {
    let m = simple();
    for limits in [
        TextLimits {
            input_bytes: 1,
            ..TextLimits::default()
        },
        TextLimits {
            tokens: 1,
            ..TextLimits::default()
        },
        TextLimits {
            output_bytes: 1,
            ..TextLimits::default()
        },
    ] {
        assert!(matches!(
            m.decode(&[1, 2], limits),
            Err(Error::ResourceLimit { .. })
        ));
    }
    assert!(m.decode(&[3], TextLimits::default()).is_err());
    assert!(m.encode("AB", 2).is_err()); // Includes end byte.
    assert!(m.encode("C", 10).is_err());
    assert_eq!(m.decode(&[], TextLimits::default()).unwrap().consumed, 0);
}

#[test]
fn resident_insert_is_atomic_on_fit_and_encoding_errors() {
    let m = simple();
    let mut bytes = [9; 8];
    let original = bytes;
    assert!(insert_text(&mut bytes, 2, 2, &m, "AB", 0).is_err());
    assert_eq!(bytes, original);
    assert!(insert_text(&mut bytes, 2, 4, &m, "C", 0).is_err());
    assert_eq!(bytes, original);
    assert!(insert_text(&mut bytes, usize::MAX, 4, &m, "A", 0).is_err());
    assert_eq!(bytes, original);
    assert_eq!(insert_text(&mut bytes, 2, 4, &m, "A", 0).unwrap(), 2);
    assert_eq!(bytes, [9, 9, 1, 255, 0, 0, 9, 9]);
}

#[test]
fn fixed_and_offset_tables_enforce_extent_and_termination() {
    let m = simple();
    let bytes = [1, 255, 9, 2, 255, 9];
    let fixed = StringTableDefinition::Fixed {
        offset: 0,
        count: 2,
        stride: 3,
        length: 2,
        require_end: true,
    };
    let out = fixed.dump(&bytes, &m, TextLimits::default(), 2).unwrap();
    assert_eq!((&out[0].decoded.text, out[1].offset), (&"A".to_string(), 3));
    assert!(fixed
        .dump(&bytes[..4], &m, TextLimits::default(), 2)
        .is_err());
    assert!(fixed.dump(&bytes, &m, TextLimits::default(), 1).is_err());
    let offsets = StringTableDefinition::Offsets {
        offsets: vec![3, 0],
        maximum_bytes: 2,
        require_end: true,
    };
    assert_eq!(
        offsets.dump(&bytes, &m, TextLimits::default(), 2).unwrap()[0]
            .decoded
            .text,
        "B"
    );
    let unterminated = StringTableDefinition::Offsets {
        offsets: vec![0],
        maximum_bytes: 1,
        require_end: true,
    };
    assert!(unterminated
        .dump(&bytes, &m, TextLimits::default(), 2)
        .is_err());
    let overflow = StringTableDefinition::Fixed {
        offset: usize::MAX,
        count: 2,
        stride: usize::MAX,
        length: 1,
        require_end: false,
    };
    assert!(overflow.dump(&bytes, &m, TextLimits::default(), 2).is_err());
}

#[test]
fn integer_and_far_pointers_follow_explicit_windows_and_bounds() {
    let mut bytes = [0; 32];
    bytes[..4].copy_from_slice(&[0, 2, 0, 4]);
    bytes[12..14].copy_from_slice(&[1, 255]);
    bytes[16..18].copy_from_slice(&[2, 255]);
    let pointer = PointerDefinition::Integer {
        width: 2,
        endian: PointerEndian::Big,
        base: 8,
        scale: 2,
    };
    let table = StringTableDefinition::Pointers {
        offset: 0,
        count: 2,
        stride: 2,
        pointer_offset: 0,
        pointer,
        maximum_bytes: 2,
        require_end: true,
        skip_marker: None,
        skip_marker_below: None,
    };
    let out = table
        .dump(&bytes, &simple(), TextLimits::default(), 2)
        .unwrap();
    assert_eq!(out[0].decoded.text, "A");
    assert_eq!(out[1].offset, 16);
    let far = PointerDefinition::Far {
        endian: PointerEndian::Little,
        maximum_offset: Some(15),
        windows: vec![SegmentWindow {
            minimum: 0xf000,
            maximum: 0xffff,
            base: 100,
            record_bank: false,
        }],
    };
    assert_eq!(far.target(&[3, 0, 1, 240], 0).unwrap(), 119);
    assert!(far.target(&[16, 0, 1, 240], 0).is_err());
    assert!(far.target(&[3, 0, 0, 32], 0).is_err());
    assert!(far.target(&[3, 0, 1], 0).is_err());
    let bank = PointerDefinition::Far {
        endian: PointerEndian::Little,
        maximum_offset: None,
        windows: vec![SegmentWindow {
            minimum: 0x3000,
            maximum: 0x3fff,
            base: 0,
            record_bank: true,
        }],
    };
    let mut large = vec![0; 65540];
    large[65536..].copy_from_slice(&[5, 0, 0, 48]);
    assert_eq!(bank.target(&large, 65536).unwrap(), 65541);
    let overflow = PointerDefinition::Integer {
        width: 4,
        endian: PointerEndian::Little,
        base: usize::MAX,
        scale: 1,
    };
    assert!(overflow.target(&[1, 0, 0, 0], 0).is_err());
}

#[test]
fn marker_preludes_are_bounded_and_preserve_a_final_empty_string() {
    let table = StringTableDefinition::Pointers {
        offset: 0,
        count: 1,
        stride: 2,
        pointer_offset: 0,
        pointer: PointerDefinition::Integer {
            width: 2,
            endian: PointerEndian::Little,
            base: 0,
            scale: 1,
        },
        maximum_bytes: 4,
        require_end: true,
        skip_marker: Some(255),
        skip_marker_below: Some(10),
    };
    let result = table
        .dump(&[2, 0, 255, 1, 255], &simple(), TextLimits::default(), 1)
        .unwrap();
    assert_eq!(result[0].offset, 3);
    assert_eq!(result[0].decoded.text, "A");
    let result = table
        .dump(&[2, 0, 255], &simple(), TextLimits::default(), 1)
        .unwrap();
    assert_eq!(result[0].offset, 2);
    assert_eq!(result[0].decoded.text, "");
    let mut bounded = table;
    if let StringTableDefinition::Pointers {
        skip_marker_below, ..
    } = &mut bounded
    {
        *skip_marker_below = Some(2);
    }
    let result = bounded
        .dump(&[2, 0, 255, 1, 255], &simple(), TextLimits::default(), 1)
        .unwrap();
    assert_eq!(result[0].offset, 2);
    assert_eq!(result[0].decoded.text, "");
}

#[test]
fn ambiguous_pointer_windows_fail_instead_of_picking_an_order() {
    let pointer = PointerDefinition::Far {
        endian: PointerEndian::Little,
        maximum_offset: None,
        windows: vec![
            SegmentWindow {
                minimum: 0x3000,
                maximum: 0x3fff,
                base: 0,
                record_bank: false,
            },
            SegmentWindow {
                minimum: 0x3800,
                maximum: 0x4000,
                base: 100,
                record_bank: false,
            },
        ],
    };
    assert!(pointer.target(&[0, 0, 0, 48], 0).is_err());
}

fn font(format: FontFormat, stride: usize, wide: u32, high: u32) -> FontDefinition {
    FontDefinition {
        offset: 0,
        count: 1,
        stride,
        format,
        tiles_wide: wide,
        tiles_high: high,
        order: GlyphOrder::Row,
        tile_offsets: vec![],
        widths: vec![],
        width_table: None,
    }
}

#[test]
fn one_bit_font_is_msb_first_and_supports_sparse_half_width_storage() {
    let mut f = font(FontFormat::Bpp1, 24, 1, 2);
    f.tile_offsets = vec![0, 16];
    f.widths = vec![6];
    let mut source = [0; 25];
    source[0] = 0x80;
    source[16] = 0x01;
    source[24] = 7;
    let palette = [[255; 4], [0, 0, 0, 255], [20; 4], [30; 4]];
    let g = f.glyph(&source, 0, &palette, 512).unwrap();
    assert_eq!((g.image.width, g.image.height, g.advance), (8, 16, 6));
    assert_eq!(&g.image.rgba[..4], &palette[1]);
    assert_eq!(&g.image.rgba[4..8], &palette[0]);
    assert_eq!(&g.image.rgba[(8 * 8 + 7) * 4..(8 * 8 + 8) * 4], &palette[1]);
    f.width_table = Some(24);
    assert_eq!(f.glyph(&source, 0, &palette, 512).unwrap().advance, 7);
    f.width_table = Some(25);
    assert!(f.glyph(&source, 0, &palette, 512).is_err());
}

#[test]
fn glyph_tile_order_uses_graphics_renderer() {
    let palette = [[255; 4], [1; 4], [2; 4], [3; 4]];
    let mut source = [0; 64];
    for (tile, color) in [0u8, 1, 2, 3].into_iter().enumerate() {
        for row in 0..8 {
            source[tile * 16 + row * 2] = if color & 1 != 0 { 255 } else { 0 };
            source[tile * 16 + row * 2 + 1] = if color & 2 != 0 { 255 } else { 0 };
        }
    }
    let mut f = font(FontFormat::Bpp2, 64, 2, 2);
    f.order = GlyphOrder::Column;
    let g = f.glyph(&source, 0, &palette, 1024).unwrap();
    assert_eq!(&g.image.rgba[8 * 4..9 * 4], &palette[2]);
    assert_eq!(&g.image.rgba[8 * 16 * 4..(8 * 16 + 1) * 4], &palette[1]);
}

#[test]
fn malformed_fonts_return_errors_before_indexing_or_allocation() {
    let palette = [[255; 4], [0; 4], [1; 4], [2; 4]];
    let f = font(FontFormat::Bpp2, 16, 1, 1);
    assert!(f.glyph(&[0; 15], 0, &palette, 256).is_err());
    assert!(f.glyph(&[0; 16], 1, &palette, 256).is_err());
    assert!(f.glyph(&[0; 16], 0, &palette, 255).is_err());
    let mut bad = f.clone();
    bad.tiles_wide = u32::MAX;
    assert!(bad.glyph(&[], 0, &palette, usize::MAX).is_err());
    bad = f.clone();
    bad.offset = usize::MAX;
    assert!(bad.glyph(&[], 0, &palette, usize::MAX).is_err());
    bad = f.clone();
    bad.tile_offsets = vec![usize::MAX];
    assert!(bad.glyph(&[0; 16], 0, &palette, 256).is_err());
    bad = f.clone();
    bad.widths = vec![9];
    assert!(bad.glyph(&[0; 16], 0, &palette, 256).is_err());
}

#[test]
fn documented_definition_dumps_edits_and_renders_a_synthetic_source() {
    let definition = TextDefinition::from_json(br#"{
        "schema":"swankit-text-def/1", "id":"example",
        "map":{"schema":"swankit-charmap/1",
            "entries":[{"bytes":"01","text":"A"},{"bytes":"02","text":"B"}],
            "controls":[{"code":255,"ends":true}]},
        "tables":{"labels":{"kind":"fixed","offset":0,"count":2,"stride":4,"length":4,"require_end":true}},
        "fonts":{"main":{"offset":8,"count":1,"stride":8,"format":"bpp1","tiles_wide":1,"tiles_high":1}}
    }"#).unwrap();
    let mut source = [1, 255, 0, 0, 2, 255, 0, 0, 128, 0, 0, 0, 0, 0, 0, 0];
    let m = definition.character_map().unwrap();
    let dumped = definition.tables["labels"]
        .dump(&source, &m, TextLimits::default(), 16)
        .unwrap();
    assert_eq!(dumped[0].decoded.text, "A");
    assert_eq!(dumped[1].decoded.text, "B");
    insert_text(&mut source, dumped[0].offset, 4, &m, "AB", 0).unwrap();
    assert_eq!(&source[..8], &[1, 2, 255, 0, 2, 255, 0, 0]);
    let glyph = definition.fonts["main"]
        .glyph(&source, 0, &[[255; 4], [0, 0, 0, 255]], 256)
        .unwrap();
    assert_eq!((glyph.image.width, glyph.image.height), (8, 8));
    assert_eq!(&glyph.image.rgba[..4], &[0, 0, 0, 255]);
}

fn masked() -> CharacterMap {
    map(
        json!({"unit":"word-le","glyph_mask":{"below":32768,"mask":2047},
        "entries":[{"bytes":"0100","text":"カ"},{"bytes":"0200","text":"キ"},
        {"bytes":"0300","text":"人"},{"bytes":"0400","text":"A","aliases":["Ａ"]}],
        "controls":[{"code":65535,"ends":true},
        {"code":65024,"mask":65280,"display":"|"}]}),
    )
}

#[test]
fn edits_keep_unchanged_tokens_exactly() {
    let m = masked();
    // カ, hidden-operand control fe07, キ, end.
    let original = [1, 0, 7, 0xfe, 2, 0, 0xff, 0xff];
    let out = m
        .encode_edit(&original, "カ|人", TextLimits::default())
        .unwrap();
    assert_eq!(out, [1, 0, 7, 0xfe, 3, 0, 0xff, 0xff]);
}

#[test]
fn edits_keep_hidden_attribute_bits_inside_the_edited_span() {
    let m = masked();
    // カ キ 人 with attribute bits 0x1800 on キ and 人 (colour), then end.
    let original = [1, 0, 2, 0x18, 3, 0x18, 0xff, 0xff];
    // Replace the attributed middle: each new glyph inherits the attributes
    // of the source glyph at the same span position.
    let out = m
        .encode_edit(&original, "カ人キ", TextLimits::default())
        .unwrap();
    assert_eq!(out, [1, 0, 3, 0x18, 2, 0x18, 0xff, 0xff]);
    // A longer span continues with the last source glyph's attributes.
    let out = m
        .encode_edit(&original, "カ人キAA", TextLimits::default())
        .unwrap();
    assert_eq!(out, [1, 0, 3, 0x18, 2, 0x18, 4, 0x18, 4, 0x18, 0xff, 0xff]);
    // An unchanged token at its span position keeps its exact bytes (alias).
    let aliased = [4, 0x20, 2, 0x18, 0xff, 0xff];
    let out = m
        .encode_edit(&aliased, "Aカ", TextLimits::default())
        .unwrap();
    assert_eq!(out, [4, 0x20, 1, 0x18, 0xff, 0xff]);
    assert_eq!(m.decode(&out, TextLimits::default()).unwrap().text, "Aカ");
}
