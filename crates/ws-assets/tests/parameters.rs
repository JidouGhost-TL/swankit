use ws_assets::{
    map::{Num, RomIdentity},
    parameters::{self, Context, Registers, Route, TableDefinition},
};
fn rom(size: usize) -> Vec<u8> {
    let mut b: Vec<_> = (0..size).map(|i| (i % 251) as u8).collect();
    b[size - 16] = 0xea;
    b[size - 3] = 0;
    b
}
fn definition(b: &[u8]) -> TableDefinition {
    TableDefinition {
        schema: parameters::SCHEMA.into(),
        rom: RomIdentity {
            sha256: ws_assets::sha256_hex(b),
            size: Num(b.len() as u64),
            label: None,
        },
        source_segment: 0x5123,
        source_base: 0x2345,
        route: Route::AlignedFallthrough,
        context: Context::Explicit {
            banks: vec![Registers {
                c0: 1,
                c2: 0,
                c3: 0,
            }],
        },
    }
}
#[test]
fn paragraph_and_routes_use_direct_entry_al() {
    let b = rom(0x400000);
    let mut d = definition(&b);
    let r = parameters::resolve(&b, &d).unwrap();
    assert_eq!(r.first_cpu_address, 0x53575);
    assert_eq!(r.slices[0].file_offset, Num(0x153575));
    assert_ne!(r.slices[0].file_offset, Num(0x152345));
    assert_eq!(r.slices[0].length, 256);
    d.route = Route::CoreUnion;
    assert_eq!(parameters::resolve(&b, &d).unwrap().slices[0].length, 271);
    d.route = Route::Record { entry_al: 255 };
    let r = parameters::resolve(&b, &d).unwrap();
    assert_eq!(r.first_si, 0x2444);
    assert_eq!(r.slices[0].file_offset, Num(0x153674));
    assert_eq!(r.slices[0].length, 16);
}
#[test]
fn retained_coordinate_exposes_all_normalized_aliases() {
    let b = rom(0x400000);
    let mut d = definition(&b);
    d.context = Context::RetainedCode {
        segment: 0x4000,
        offset: 0x1000,
        file_offset: Num(0x141000),
    };
    let r = parameters::resolve(&b, &d).unwrap();
    assert_eq!(
        r.compatible_contexts
            .iter()
            .map(|b| b.c0)
            .collect::<Vec<_>>(),
        [1, 5, 9, 13]
    );
    assert!(r
        .compatible_contexts
        .iter()
        .all(|b| b.c2.is_none() && b.c3.is_none()));
    assert_eq!(r.slices[0].context_indices, [0, 1, 2, 3]);
    assert_eq!(r.decision, "unique");
    d.context = Context::RetainedCode {
        segment: 0x4000,
        offset: 0x1000,
        file_offset: Num(0x1234),
    };
    assert!(parameters::resolve(&b, &d).is_err());
}
#[test]
fn padded_chip_offsets_are_not_file_offsets() {
    let b = rom(0xc0000);
    let mut d = definition(&b);
    d.source_segment = 0x5000;
    d.source_base = 0;
    d.context = Context::Explicit {
        banks: vec![Registers {
            c0: 0,
            c2: 0,
            c3: 0,
        }],
    };
    let r = parameters::resolve(&b, &d).unwrap();
    assert_eq!(r.padding_size, 0x40000);
    assert_eq!(r.slices[0].effective_offset, Num(0x50000));
    assert_eq!(r.slices[0].file_offset, Num(0x10000));
    d.source_segment = 0x2000;
    assert!(parameters::resolve(&b, &d).is_err());
}
#[test]
fn refuses_si_mapping_window_and_twenty_bit_ram_wraps() {
    let b = rom(0x400000);
    let mut d = definition(&b);
    d.source_base = 0xffff;
    assert!(parameters::resolve(&b, &d).is_err());
    d.source_base = 0;
    d.source_segment = 0x4fff;
    assert!(parameters::resolve(&b, &d).is_err());
    d.source_segment = 0xffff;
    d.source_base = 16;
    assert!(parameters::resolve(&b, &d).is_err());
}
#[test]
fn separately_banked_sources_require_explicit_context() {
    let b = rom(0x400000);
    let mut d = definition(&b);
    d.source_segment = 0x2000;
    d.source_base = 4;
    d.context = Context::RetainedCode {
        segment: 0x4000,
        offset: 0x1000,
        file_offset: Num(0x141000),
    };
    assert!(parameters::resolve(&b, &d).is_err());
    d.context = Context::Explicit {
        banks: vec![Registers {
            c0: 0,
            c2: 3,
            c3: 5,
        }],
    };
    assert_eq!(
        parameters::resolve(&b, &d).unwrap().slices[0].file_offset,
        Num(0x30004)
    );
}
#[test]
fn ambiguous_offsets_do_not_make_a_raw_map() {
    let b = rom(0x400000);
    let mut d = definition(&b);
    d.context = Context::Explicit {
        banks: vec![
            Registers {
                c0: 0,
                c2: 0,
                c3: 0,
            },
            Registers {
                c0: 1,
                c2: 0,
                c3: 0,
            },
        ],
    };
    let r = parameters::resolve(&b, &d).unwrap();
    assert_eq!(r.decision, "ambiguous");
    assert_eq!(r.slices.len(), 2);
    assert!(r.asset_map().is_err());
    d.context = Context::Explicit { banks: vec![] };
    assert!(parameters::resolve(&b, &d).is_err());
}
#[test]
fn identity_unknown_mapper_and_unknown_definition_fields_fail() {
    let mut b = rom(0x400000);
    let d = definition(&b);
    b[0] ^= 1;
    assert!(parameters::resolve(&b, &d).is_err());
    let mapper_byte = b.len() - 3;
    b[mapper_byte] = 0xfe;
    let d = definition(&b);
    assert!(parameters::resolve(&b, &d).is_err());
    let mut value = serde_json::to_value(d).unwrap();
    value["extra"] = true.into();
    assert!(TableDefinition::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    value.as_object_mut().unwrap().remove("extra");
    value.as_object_mut().unwrap().remove("source_segment");
    assert!(TableDefinition::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
}
