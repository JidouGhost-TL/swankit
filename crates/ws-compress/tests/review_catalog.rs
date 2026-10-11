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
