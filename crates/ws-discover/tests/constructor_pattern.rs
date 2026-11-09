//! Invented instruction operands; these checks establish code-pattern leads only.
use ws_discover::search::{search, Family, Options};

fn options() -> Options {
    serde_json::from_str(r#"{"families":["banked-queue-constructor"]}"#).unwrap()
}
fn core(segment: u16, table: u16, queue: u16) -> Vec<u8> {
    let mut out = vec![
        0xbf, 0, 0, 0xb8, 0, 0, 0x8e, 0xc0, 0x33, 0xc0, 0x8e, 0xd8, 0x8a, 0xc6, 0xd1, 0xe0, 0xd1,
        0xe0, 0x03, 0xf8, 0x26, 0x8b, 0x45, 0x02, 0xe6, 0xc3, 0x26, 0x8b, 0x3d, 0x50, 0xb8, 0,
        0x30, 0x8e, 0xc0, 0x32, 0xf6, 0xd1, 0xe2, 0x03, 0xfa, 0x26, 0x8b, 0x3d, 0x8b, 0x36, 0, 0,
        0x58, 0x89, 0x04, 0x8b, 0xc7, 0x89, 0x44, 0x02, 0x89, 0x4c, 0x04, 0x89, 0x5c, 0x06, 0x83,
        0xc6, 0x08, 0xc7, 0x04, 0, 0, 0x89, 0x36, 0, 0,
    ];
    out[1..3].copy_from_slice(&table.to_le_bytes());
    out[4..6].copy_from_slice(&segment.to_le_bytes());
    out[46..48].copy_from_slice(&queue.to_le_bytes());
    out[71..73].copy_from_slice(&queue.to_le_bytes());
    out
}
#[test]
fn explicit_opt_in_preserves_literals_without_claiming_callers() {
    let bytes = core(0xe000, 0x4567, 0x08a4);
    let report = search(&bytes, &options()).unwrap();
    assert_eq!(report.leads.len(), 1);
    let lead = &report.leads[0];
    assert_eq!((lead.offset, lead.stored_length), (0, 73));
    assert_eq!(lead.decoded_length, None);
    assert_eq!(lead.stored_sha256, ws_assets::sha256_hex(&bytes));
    assert!(lead.evidence.contains("ES=0xe000"));
    assert!(lead.evidence.contains("table=0x4567"));
    assert!(lead.evidence.contains("queue=0x08a4"));
    assert!(lead.evidence.contains("OUT 0xc3"));
    assert!(lead.requires.iter().any(|s| s.contains("incoming")));
    assert!(lead.requires.iter().any(|s| s.contains("clobber")));
    assert!(!lead.requires.is_empty());
    assert_eq!(report.counts[&lead.family].candidates, 1);
}
#[test]
fn empty_reader_configuration_remains_empty() {
    let options = Options {
        families: vec![Family::ReaderSignature],
        ..Options::default()
    };
    assert!(search(&core(0xe000, 0x4567, 0x08a4), &options)
        .unwrap()
        .leads
        .is_empty());
    assert!(!Options::default().families.contains(&options_from_family()));
}
fn options_from_family() -> Family {
    options().families[0]
}
#[test]
fn repeated_queue_operand_is_required() {
    let mut bytes = core(0xe000, 0x4567, 0x08a4);
    bytes[71..73].copy_from_slice(&0x08a6u16.to_le_bytes());
    assert!(search(&bytes, &options()).unwrap().leads.is_empty());
}
#[test]
fn instruction_roles_and_segment_overrides_are_not_masked() {
    for (offset, replacement) in [
        (7, 0xd8),
        (20, 0x3e),
        (25, 0xc2),
        (32, 0x20),
        (35, 0x33),
        (57, 0x48),
        (64, 0x10),
    ] {
        let mut bytes = core(0xe000, 0x4567, 0x08a4);
        bytes[offset] = replacement;
        assert!(
            search(&bytes, &options()).unwrap().leads.is_empty(),
            "offset{offset}"
        );
    }
}
#[test]
fn changed_literal_segment_and_table_remain_distinct() {
    let one = search(&core(0xe000, 0x4567, 0x08a4), &options()).unwrap();
    let two = search(&core(0x9000, 0x2345, 0x07b2), &options()).unwrap();
    assert_eq!(two.leads.len(), 1);
    assert!(two.leads[0].evidence.contains("ES=0x9000"));
    assert!(two.leads[0].evidence.contains("table=0x2345"));
    assert_ne!(one.leads[0].stored_sha256, two.leads[0].stored_sha256);
    assert_ne!(one.leads[0].evidence, two.leads[0].evidence);
}
#[test]
fn truncation_insertion_and_every_fixed_byte_mutation_refuse() {
    let bytes = core(0xe000, 0x4567, 0x08a4);
    for end in 0..bytes.len() {
        assert!(search(&bytes[..end], &options()).unwrap().leads.is_empty());
    }
    for at in 0..bytes.len() {
        if [1, 2, 4, 5, 46, 47, 71, 72].contains(&at) {
            continue;
        }
        let mut altered = bytes.clone();
        altered[at] ^= 1;
        assert!(
            search(&altered, &options()).unwrap().leads.is_empty(),
            "fixed byte{at}"
        );
    }
    let mut inserted = bytes.clone();
    inserted.insert(44, 0x90);
    assert!(search(&inserted, &options()).unwrap().leads.is_empty());
}
#[test]
fn bounds_stride_and_budget_are_honest() {
    let mut bytes = vec![0x90; 3];
    bytes.extend(core(0xe000, 0x4567, 0x08a4));
    let mut opts = options();
    opts.stride = 2;
    assert!(search(&bytes, &opts).unwrap().leads.is_empty());
    opts.stride = 1;
    opts.max_probes = 3;
    let limited = search(&bytes, &opts).unwrap();
    assert!(!limited.complete_for_configured_scan);
    assert!(limited.leads.is_empty());
    opts.max_probes = 4;
    let exact = search(&bytes, &opts).unwrap();
    assert_eq!(exact.leads[0].offset, 3);
}
#[test]
fn retention_counts_candidates_without_changing_code() {
    let mut bytes = core(0xe000, 0x4567, 0x08a4);
    bytes.extend(core(0x9000, 0x2345, 0x07b2));
    let original = bytes.clone();
    let mut opts = options();
    opts.leads_per_family = 1;
    let report = search(&bytes, &opts).unwrap();
    assert_eq!(bytes, original);
    assert_eq!(report.leads.len(), 1);
    assert_eq!(report.counts[&options_from_family()].candidates, 2);
}
