use ws_discover::search::{
    search, DestinationSegment, Family, Options, ReaderCaptures, ReaderSourceStatus,
    SecondByteRegister,
};

#[test]
fn per_family_budget_does_not_starve_the_other_relational_core() {
    let mut bytes = vec![0xbf, 0, 0];
    bytes.extend(token(false));
    bytes.extend(table(false));
    let mut o = Options {
        families: vec![
            Family::IndexedEightWordCopy,
            Family::TokenDistance12Length4Plus3,
        ],
        max_probes: 1,
        ..Options::default()
    };
    let shared = search(&bytes, &o).unwrap();
    assert!(shared.leads.is_empty());
    o.budget_scope = ws_discover::search::BudgetScope::PerFamily;
    let separate = search(&bytes, &o).unwrap();
    assert_eq!(separate.leads.len(), 1);
    assert_eq!(
        separate.leads[0].family,
        Family::TokenDistance12Length4Plus3
    );
    assert!(!separate.complete_by_family[&Family::IndexedEightWordCopy]);
    assert!(separate.complete_by_family[&Family::TokenDistance12Length4Plus3]);
    assert!(!separate.complete_for_configured_scan);
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn table(ss: bool) -> Vec<u8> {
    let mut b = unhex(
        "bf34128a0e785632ed03f9880556beffff32e403f0bf0040a07856c0e00403f8b823818ec0b90800268b04",
    );
    if ss {
        b.push(0x36);
    }
    b.extend(unhex(if ss {
        "890583c60283c702e2f25e"
    } else {
        "890583c60283c702e2f35e"
    }));
    b
}
fn token(al: bool) -> Vec<u8> {
    let mut b =
        unhex("4b268a0fff46fcc45efc4b268a1732edd1e1d1e1d1e1d1e18ada32ffd1ebd1ebd1ebd1eb03cb");
    if al {
        b[13] = 7;
        b[25] = 0xd8;
    }
    b.extend(unhex(if al {
        "8bf1240f32e4050300"
    } else {
        "80e20f32f683c203"
    }));
    b
}
fn options(family: Family) -> Options {
    Options {
        families: vec![family],
        ..Options::default()
    }
}

#[test]
fn captures_are_bounded_versioned_and_opt_in() {
    let b = table(true);
    assert!(search(&b, &Options::default())
        .unwrap()
        .leads
        .iter()
        .all(|l| l.reader_core.is_none()));
    let r = search(&b, &options(Family::IndexedEightWordCopy)).unwrap();
    assert_eq!(r.leads.len(), 1);
    assert_eq!(r.leads[0].stored_length, 55);
    let core = r.leads[0].reader_core.as_ref().unwrap();
    assert_eq!(core.version, 1);
    assert_eq!(core.source_status, ReaderSourceStatus::RawByteMatch);
    assert_eq!(
        core.captures,
        ReaderCaptures::IndexedEightWordCopy {
            slot_base: 0x1234,
            index_address: 0x5678,
            source_base: 0xffff,
            source_segment_literal: 0x8123,
            destination_base: 0x4000,
            destination_segment: DestinationSegment::SS,
            copy_words: 8,
            index_shift_width_bits: 8,
            preceding_shift4_store: false,
        }
    );
    for al in [false, true] {
        let r = search(&token(al), &options(Family::TokenDistance12Length4Plus3)).unwrap();
        assert_eq!(r.leads.len(), 1);
        assert_eq!(
            r.leads[0].reader_core.as_ref().unwrap().captures,
            ReaderCaptures::TokenDistance12Length4Plus3 {
                pointer_bp_displacement: -4,
                second_byte_register: if al {
                    SecondByteRegister::AL
                } else {
                    SecondByteRegister::DL
                },
                pointer_increment_bits: 16,
            }
        );
    }
}

#[test]
fn operand_equality_control_flow_and_width_are_discriminating() {
    let original = table(false);
    for (at, replacement) in [(25, 0x79), (52, 0xf4), (27, 0xc1), (38, 9), (43, 0x88)] {
        let mut b = original.clone();
        b[at] = replacement;
        assert!(search(&b, &options(Family::IndexedEightWordCopy))
            .unwrap()
            .leads
            .is_empty());
    }
    let mut relocated = original;
    relocated[5] ^= 1;
    relocated[25] ^= 1;
    assert_eq!(
        search(&relocated, &options(Family::IndexedEightWordCopy))
            .unwrap()
            .leads
            .len(),
        1
    );
    for al in [false, true] {
        for at in [9, 13, 25] {
            let mut b = token(al);
            b[at] ^= 1;
            assert!(search(&b, &options(Family::TokenDistance12Length4Plus3))
                .unwrap()
                .leads
                .is_empty());
        }
        let mut b = token(al);
        b[6] ^= 1;
        b[9] ^= 1;
        assert_eq!(
            search(&b, &options(Family::TokenDistance12Length4Plus3))
                .unwrap()
                .leads
                .len(),
            1
        );
    }
}

#[test]
fn optional_prelude_is_a_fact_not_an_entry_or_header_claim() {
    let mut b = unhex("268a4701c0e004884425");
    b.extend(table(false));
    let before = search(&b, &options(Family::IndexedEightWordCopy)).unwrap();
    assert!(matches!(
        before.leads[0].reader_core.as_ref().unwrap().captures,
        ReaderCaptures::IndexedEightWordCopy {
            preceding_shift4_store: true,
            ..
        }
    ));
    b[9] ^= 1;
    let after = search(&b, &options(Family::IndexedEightWordCopy)).unwrap();
    assert_eq!(after.leads.len(), 1);
    assert!(matches!(
        after.leads[0].reader_core.as_ref().unwrap().captures,
        ReaderCaptures::IndexedEightWordCopy {
            preceding_shift4_store: false,
            ..
        }
    ));
}

#[test]
fn truncation_budget_stride_and_retention_remain_visible() {
    let b = table(false);
    for end in 0..b.len() {
        assert!(search(&b[..end], &options(Family::IndexedEightWordCopy))
            .unwrap()
            .leads
            .is_empty());
    }
    let mut input = vec![0xbf, 0, 0];
    input.extend(&b);
    let mut o = options(Family::IndexedEightWordCopy);
    o.max_probes = 1;
    let r = search(&input, &o).unwrap();
    assert!(!r.complete_for_configured_scan);
    assert!(r.leads.is_empty());
    o = options(Family::IndexedEightWordCopy);
    o.stride = 2;
    assert!(search(&input, &o).unwrap().leads.is_empty());
    o.stride = 1;
    o.leads_per_family = 1;
    input.extend(&b);
    let r = search(&input, &o).unwrap();
    assert!(r.complete_for_configured_scan);
    assert_eq!(r.leads.len(), 1);
    assert_eq!(r.counts[&Family::IndexedEightWordCopy].candidates, 2);
}
