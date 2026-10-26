use ws_discover::search::{search, Family, Options, Signature};

fn counted() -> Vec<u8> {
    let mut bytes = vec![1, 0, 32, 0];
    bytes.extend(0u8..64);
    bytes
}

fn tile_map_record16(map_words: &[u16]) -> Vec<u8> {
    assert_eq!(map_words.len(), 4);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0x5a3cu16.to_le_bytes()); // Unknown header word.
    bytes.extend_from_slice(&0xc7e1u16.to_le_bytes()); // Unknown header word.
    for word in [16u16, 16, 2, 4, 16, 48] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(&[0x44; 32]);
    for &word in map_words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn tile_map_options() -> Options {
    Options {
        families: vec![Family::TileMapRecord16],
        ..Options::default()
    }
}

#[test]
fn all_supported_families_are_selectable_with_empty_and_duplicate_lists_rejected() {
    let options = Options {
        families: vec![
            Family::LzU8Zero,
            Family::LzRle7f,
            Family::LzBits16,
            Family::LzWord16Msb,
            Family::LzWordCounted,
            Family::Rgb444,
            Family::ScreenMap,
            Family::TileMapRecord16,
            Family::ReaderSignature,
            Family::ReferenceBytes,
        ],
        ..Options::default()
    };
    let report = search(&[], &options).unwrap();
    assert_eq!(report.options.families.len(), 10);
    assert!(search(
        &[],
        &Options {
            families: Vec::new(),
            ..options.clone()
        }
    )
    .is_err());
    assert!(search(
        &[],
        &Options {
            families: vec![Family::TileMapRecord16, Family::TileMapRecord16],
            ..options
        }
    )
    .is_err());
}

#[test]
fn tile_map_record16_accepts_odd_start_unknown_words_flips_and_exact_end() {
    let record = tile_map_record16(&[0x0000, 0x4001, 0x8000, 0xc001]);
    let mut rom = vec![0xa5; 7];
    rom.extend_from_slice(&record);
    let report = search(&rom, &tile_map_options()).unwrap();
    let lead = report
        .leads
        .iter()
        .find(|lead| lead.family == Family::TileMapRecord16)
        .unwrap();
    assert_eq!(lead.offset, 7);
    assert_eq!(lead.stored_length, record.len());
    assert_eq!(record.len(), 56);
    assert!(lead.decoded_length.is_none());
    assert!(lead.evidence.contains("descriptor relations"));
    assert!(report.complete_for_configured_scan);
    assert_eq!(report.counts[&Family::TileMapRecord16].candidates, 1);
    assert_eq!(report.counts[&Family::TileMapRecord16].probes, 52);
}

#[test]
fn tile_map_record16_rejects_truncation_relations_indices_and_attributes() {
    let valid = tile_map_record16(&[0, 1, 0, 1]);
    for cut in [0, 15, 16, 47, 48, 54, 55] {
        let report = search(&valid[..cut], &tile_map_options()).unwrap();
        assert!(report.leads.is_empty(), "cut={cut}");
    }

    let malformed_words: [(usize, u16); 7] = [
        (4, 15),  // Width is not a multiple of eight.
        (6, 24),  // Height changes the required map-entry count.
        (8, 0),   // Empty tile table.
        (8, 513), // Tile table exceeds the family bound.
        (10, 3),  // Wrong map entry count.
        (12, 15), // Wrong tile offset.
        (14, 47), // Wrong map offset.
    ];
    for (byte, value) in malformed_words {
        let mut malformed = valid.clone();
        malformed[byte..byte + 2].copy_from_slice(&value.to_le_bytes());
        assert!(search(&malformed, &tile_map_options())
            .unwrap()
            .leads
            .is_empty());
    }
    let mut out_of_range = valid.clone();
    out_of_range[48..50].copy_from_slice(&2u16.to_le_bytes());
    assert!(search(&out_of_range, &tile_map_options())
        .unwrap()
        .leads
        .is_empty());
    let mut unsupported = valid;
    unsupported[48..50].copy_from_slice(&0x0200u16.to_le_bytes());
    assert!(search(&unsupported, &tile_map_options())
        .unwrap()
        .leads
        .is_empty());
}

#[test]
fn tile_map_record16_bounds_work_and_retention() {
    let mut rom = tile_map_record16(&[0, 1, 0, 1]);
    rom.extend_from_slice(&[0xee; 8]);
    rom.extend_from_slice(&tile_map_record16(&[1, 0, 1, 0]));
    let report = search(
        &rom,
        &Options {
            leads_per_family: 1,
            ..tile_map_options()
        },
    )
    .unwrap();
    assert_eq!(report.counts[&Family::TileMapRecord16].candidates, 2);
    assert_eq!(report.counts[&Family::TileMapRecord16].retained, 1);
    assert_eq!(report.leads.len(), 1);

    let limited = search(
        &tile_map_record16(&[0, 1, 0, 1]),
        &Options {
            max_probes: 2,
            ..tile_map_options()
        },
    )
    .unwrap();
    assert!(!limited.complete_for_configured_scan);
    assert!(limited.leads.is_empty());
    assert_eq!(limited.counts[&Family::TileMapRecord16].probes, 2);
    assert!(limited.counts[&Family::TileMapRecord16].probes_skipped_by_budget > 0);

    let mut huge = tile_map_record16(&[0, 1, 0, 1]);
    huge[4..6].copy_from_slice(&65528u16.to_le_bytes());
    huge[6..8].copy_from_slice(&65528u16.to_le_bytes());
    huge[10..12].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(search(&huge, &tile_map_options()).unwrap().leads.is_empty());
}

#[test]
fn exact_odd_start_complete_extent_and_no_payload_export() {
    let mut rom = vec![255; 17];
    rom.extend(counted());
    rom.extend([255; 19]);
    let options = Options {
        families: vec![Family::LzWordCounted],
        min_savings: 0,
        ..Options::default()
    };
    let got = search(&rom, &options).unwrap();
    let lead = got.leads.iter().find(|l| l.offset == 17).unwrap();
    assert_eq!(lead.stored_length, 68);
    assert_eq!(lead.decoded_length, Some(64));
    assert_eq!(
        lead.decoded_sha256.as_deref(),
        Some(ws_assets::sha256_hex(&(0u8..64).collect::<Vec<_>>()).as_str())
    );
    assert!(lead.evidence.contains("no original-output comparison"));
    assert!(got.complete_for_configured_scan);
    assert_eq!(got.positions_scanned, rom.len());
    let json = serde_json::to_value(&got).unwrap();
    assert!(json["leads"][0].get("decoded_bytes").is_none());
    let stepped = search(
        &rom,
        &Options {
            stride: 2,
            ..options
        },
    )
    .unwrap();
    assert!(!stepped.leads.iter().any(|l| l.offset == 17));
}

#[test]
fn bank_boundary_is_not_silently_crossed() {
    let mut rom = vec![255; 65534];
    rom.extend(counted());
    let got = search(
        &rom,
        &Options {
            families: vec![Family::LzWordCounted],
            min_savings: 0,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(!got.leads.iter().any(|l| l.offset == 65534));
}

#[test]
fn budget_and_retention_are_reported_separately() {
    let mut rom = Vec::new();
    for _ in 0..8 {
        rom.extend(counted());
    }
    let got = search(
        &rom,
        &Options {
            families: vec![Family::LzWordCounted],
            min_savings: 0,
            max_probes: 1,
            leads_per_family: 1,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(got.decoder_probes, 1);
    assert_eq!(got.positions_scanned, rom.len());
    assert!(!got.complete_for_configured_scan);
    assert!(got.counts[&Family::LzWordCounted].probes_skipped_by_budget > 0);
    let complete = search(
        &rom,
        &Options {
            families: vec![Family::LzWordCounted],
            min_savings: 0,
            leads_per_family: 1,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(complete.complete_for_configured_scan);
    assert!(
        complete.counts[&Family::LzWordCounted].candidates
            > complete.counts[&Family::LzWordCounted].retained
    );
    assert_eq!(complete.leads.len(), 1);
}

#[test]
fn reader_patterns_and_capture_references_are_different_leads() {
    let rom = (0u8..96).collect::<Vec<_>>();
    let options = Options {
        families: vec![Family::ReaderSignature, Family::ReferenceBytes],
        signatures: vec![
            Signature {
                id: "reader".into(),
                bytes: vec![16, 17, 0, 0, 20, 21],
                mask: Some(vec![255, 255, 0, 0, 255, 255]),
                family: Family::ReaderSignature,
            },
            Signature {
                id: "captured-tile".into(),
                bytes: (48u8..64).collect(),
                mask: None,
                family: Family::ReferenceBytes,
            },
        ],
        ..Options::default()
    };
    let got = search(&rom, &options).unwrap();
    assert!(got
        .leads
        .iter()
        .any(|l| l.family == Family::ReaderSignature && l.offset == 16));
    assert!(got
        .leads
        .iter()
        .any(|l| l.family == Family::ReferenceBytes && l.offset == 48));
    let limited = search(
        &rom,
        &Options {
            max_probes: 1,
            ..options
        },
    )
    .unwrap();
    assert!(!limited.complete_for_configured_scan);
    assert_eq!(limited.signature_probes, 1);
}

#[test]
fn malformed_configuration_and_uniform_data_do_not_become_assets() {
    assert!(search(
        &[],
        &Options {
            stride: 0,
            ..Options::default()
        }
    )
    .is_err());
    assert!(search(
        &[],
        &Options {
            leads_per_family: 0,
            ..Options::default()
        }
    )
    .is_err());
    assert!(search(
        &[],
        &Options {
            signatures: vec![Signature {
                id: "wild".into(),
                bytes: vec![0; 8],
                mask: Some(vec![0; 8]),
                family: Family::ReaderSignature
            }],
            ..Options::default()
        }
    )
    .is_err());
    assert!(search(
        &[],
        &Options {
            signatures: vec![Signature {
                id: "blank-capture".into(),
                bytes: vec![0; 16],
                mask: None,
                family: Family::ReferenceBytes
            }],
            ..Options::default()
        }
    )
    .is_err());
    for byte in [0, 255] {
        let report = search(&vec![byte; 8192], &Options::default()).unwrap();
        assert!(report.leads.is_empty());
        assert!(report.complete_for_configured_scan);
    }
}

#[test]
fn structural_windows_are_labelled_as_heuristics() {
    let mut rom = Vec::new();
    for i in 0..64u16 {
        rom.extend_from_slice(&(i * 0x11).to_le_bytes());
    }
    let options = Options {
        families: vec![Family::Rgb444, Family::ScreenMap],
        ..Options::default()
    };
    let got = search(&rom, &options).unwrap();
    assert!(got.leads.iter().any(|l| l.family == Family::Rgb444));
    assert!(got
        .leads
        .iter()
        .all(|l| l.decoded_length.is_none() && l.evidence.contains("heuristic")));
    assert_eq!(
        serde_json::to_vec(&got).unwrap(),
        serde_json::to_vec(&search(&rom, &options).unwrap()).unwrap()
    );
}

#[test]
fn decoded_capture_chunk_is_corroboration_not_a_complete_asset_witness() {
    let mut stream = vec![64];
    stream.extend(0u8..64);
    stream.push(0);
    let options = Options {
        families: vec![Family::LzU8Zero, Family::ReferenceBytes],
        min_savings: 0,
        signatures: vec![Signature {
            id: "visible-tile".into(),
            bytes: (24u8..40).collect(),
            mask: None,
            family: Family::ReferenceBytes,
        }],
        ..Options::default()
    };
    let report = search(&stream, &options).unwrap();
    let lead = report
        .leads
        .iter()
        .find(|l| l.family == Family::LzU8Zero && l.offset == 0)
        .unwrap();
    assert_eq!(lead.reference_matches.len(), 1);
    assert_eq!(lead.reference_matches[0].decoded_offset, 24);
    assert_eq!(lead.reference_matches[0].length, 16);
    assert_eq!(lead.stored_length, 66);
    assert!(lead.evidence.contains("no original-output comparison"));
    assert!(report.decoded_reference_probes > 0);
    assert!(search(
        &stream,
        &Options {
            families: vec![Family::LzU8Zero],
            ..options
        }
    )
    .is_err());
}

#[test]
fn all_supported_codec_searches_retain_an_embedded_positive() {
    let bytes = (0u8..32).cycle().take(256).collect::<Vec<_>>();
    for (family, encoded) in [
        (
            Family::LzU8Zero,
            ws_compress::lz_u8_zero::encode(&bytes).unwrap(),
        ),
        (
            Family::LzRle7f,
            ws_compress::lz_rle_7f::encode(&bytes).unwrap(),
        ),
        (
            Family::LzBits16,
            ws_compress::lz_bits_16::encode(&bytes).unwrap(),
        ),
        (
            Family::LzWord16Msb,
            ws_compress::lz_word16_msb::encode(&bytes).unwrap(),
        ),
        (
            Family::LzWordCounted,
            ws_compress::lz_word_counted::encode(&bytes).unwrap(),
        ),
    ] {
        let mut rom = vec![255; 7];
        rom.extend(&encoded);
        rom.extend([255; 19]);
        let got = search(
            &rom,
            &Options {
                families: vec![family],
                min_savings: 0,
                ..Options::default()
            },
        )
        .unwrap();
        let lead = got.leads.iter().find(|l| l.offset == 7).unwrap();
        assert_eq!(lead.stored_length, encoded.len());
        assert_eq!(lead.decoded_length, Some(bytes.len()));
        assert_eq!(
            lead.decoded_sha256.as_deref(),
            Some(ws_assets::sha256_hex(&bytes).as_str())
        );
    }
}

#[test]
fn savings_filter_suppresses_literal_parse_flood_without_claiming_corruption() {
    let stream = counted();
    let filtered = search(
        &stream,
        &Options {
            families: vec![Family::LzWordCounted],
            ..Options::default()
        },
    )
    .unwrap();
    assert!(filtered.leads.is_empty());
    assert!(filtered.counts[&Family::LzWordCounted].decoded_parses > 0);
    assert!(filtered.counts[&Family::LzWordCounted].filtered_parses > 0);
    let broad = search(
        &stream,
        &Options {
            families: vec![Family::LzWordCounted],
            min_savings: 0,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(broad.leads.iter().any(|l| l.offset == 0));
}

#[test]
fn report_redacts_patterns_and_distinguishes_literal_from_decoded_only_chunks() {
    let literal = (0u8..16).collect::<Vec<_>>();
    let mut decoded = literal.clone();
    decoded.extend(&literal);
    let mut stream = vec![16];
    stream.extend(&literal);
    stream.extend([0x8d, 15, 0]);
    let options = Options {
        families: vec![Family::LzU8Zero, Family::ReferenceBytes],
        min_decoded: 32,
        signatures: vec![
            Signature {
                id: "literal".into(),
                bytes: literal.clone(),
                mask: None,
                family: Family::ReferenceBytes,
            },
            Signature {
                id: "assembled".into(),
                bytes: decoded.clone(),
                mask: None,
                family: Family::ReferenceBytes,
            },
        ],
        ..Options::default()
    };
    let got = search(&stream, &options).unwrap();
    let lead = got
        .leads
        .iter()
        .find(|l| l.family == Family::LzU8Zero && l.offset == 0)
        .unwrap();
    assert!(lead
        .reference_matches
        .iter()
        .any(|m| m.id == "literal" && m.already_present_in_stored_input));
    assert!(lead
        .reference_matches
        .iter()
        .any(|m| m.id == "assembled" && !m.already_present_in_stored_input));
    let json = serde_json::to_value(&got).unwrap();
    assert!(json["options"]["signatures"][0].get("bytes").is_none());
    assert!(json["options"]["signatures"][0].get("mask").is_none());
    assert_eq!(json["options"]["signatures"][0]["length"], 16);
    let mut duplicate = options.clone();
    duplicate.signatures.push(Signature {
        id: "same-pattern-other-context".into(),
        bytes: literal,
        mask: None,
        family: Family::ReferenceBytes,
    });
    let repeated = search(&stream, &duplicate).unwrap();
    assert_eq!(
        lead.rank,
        repeated
            .leads
            .iter()
            .find(|l| l.family == Family::LzU8Zero && l.offset == 0)
            .unwrap()
            .rank
    );
    assert_ne!(got.configuration_sha256, repeated.configuration_sha256);
}

#[test]
fn overlapping_parses_are_exposed_without_discarding_alternative_starts() {
    // The outer literal block contains a second complete literal stream.
    let mut stream = vec![40, 32];
    stream.extend(0u8..32);
    stream.push(0);
    stream.extend([7; 6]);
    stream.push(0);
    let got = search(
        &stream,
        &Options {
            families: vec![Family::LzU8Zero],
            min_decoded: 32,
            min_savings: 0,
            ..Options::default()
        },
    )
    .unwrap();
    let first = got.leads.iter().position(|l| l.offset == 0).unwrap();
    let nested = got.leads.iter().position(|l| l.offset == 1).unwrap();
    assert!(got
        .overlapping_codec_leads
        .iter()
        .any(|g| g.lead_indices.contains(&first) && g.lead_indices.contains(&nested)));
    for group in &got.overlapping_codec_leads {
        assert!(group.lead_indices.len() > 1);
        assert!(group.lead_indices.contains(&group.preferred_lead));
        for &i in &group.lead_indices {
            assert!(
                got.leads[i].offset >= group.start
                    && got.leads[i].offset + got.leads[i].stored_length <= group.end
            );
        }
    }
    assert!(got.leads.iter().any(|l| l.offset == 0));
}

#[test]
fn exact_end_match_does_not_invent_budget_incompleteness() {
    let bytes = (0u8..16).collect::<Vec<_>>();
    let got = search(
        &bytes,
        &Options {
            families: vec![Family::ReferenceBytes],
            max_probes: 1,
            signatures: vec![Signature {
                id: "whole-reference".into(),
                bytes: bytes.clone(),
                mask: None,
                family: Family::ReferenceBytes,
            }],
            ..Options::default()
        },
    )
    .unwrap();
    assert!(got.complete_for_configured_scan);
    assert_eq!(got.signature_probes, 1);
    assert_eq!(got.leads.len(), 1);
}
