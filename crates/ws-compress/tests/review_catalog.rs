#[test]
fn catalog_capabilities_agree_with_embedded_support() {
    for descriptor in ws_compress::catalog::FORMATS {
        let embedded = ws_compress::catalog::EMBEDDED
            .iter()
            .find(|row| row.id == descriptor.id)
            .unwrap();
        assert_eq!(
            descriptor.capabilities.round_trip, embedded.capabilities.round_trip,
            "round-trip capability for {}",
            descriptor.id
        );
        assert_eq!(
            descriptor.capabilities.corpus, embedded.capabilities.corpus,
            "corpus capability for {}",
            descriptor.id
        );
        assert_eq!(
            descriptor.capabilities.corpus,
            embedded.corpus_evidence.is_some()
        );
    }
}

#[test]
fn decode_only_identities_are_appended_without_changing_existing_capabilities() {
    use ws_compress::catalog::*;
    let old_ids = [
        LZ_RLE_7F_ID,
        LZ_BITS_16_ID,
        OKUMURA_ID,
        LZSS_U16_ZERO_ID,
        LZSS_12_ID,
        LZ_U8_ZERO_ID,
        LZ_WORD16_MSB_ID,
        LZ_WORD_COUNTED_ID,
    ];
    for (index, id) in old_ids.iter().enumerate() {
        assert_eq!(EMBEDDED[index].id, *id);
        assert!(EMBEDDED[index].capabilities.write);
        assert!(!EMBEDDED[index].capabilities.edit);
    }
    let new = FORMATS.iter().find(|f| f.id == LZSS_12_EXT_ID).unwrap();
    assert!(new.probes.is_empty() && new.extension_hints.is_empty());
    assert!(new.capabilities.decode && new.capabilities.parse && new.capabilities.corpus);
    assert!(!new.capabilities.write && !new.capabilities.edit && !new.capabilities.round_trip);
    assert!(!new.capabilities.bindings);
    assert_eq!(EMBEDDED[8].id, LZSS_12_EXT_ID);
    assert_eq!(EMBEDDED[8].capabilities.write, new.capabilities.write);
    let flag = FORMATS
        .iter()
        .find(|descriptor| descriptor.id == FLAG16_DISTANCE11_ID)
        .unwrap();
    assert!(flag.probes.is_empty() && flag.extension_hints.is_empty());
    assert!(flag.capabilities.decode && flag.capabilities.parse && flag.capabilities.corpus);
    assert!(!flag.capabilities.write && !flag.capabilities.edit && !flag.capabilities.round_trip);
    assert!(!flag.capabilities.bindings);
    assert_eq!(EMBEDDED[9].id, FLAG16_DISTANCE11_ID);
}
