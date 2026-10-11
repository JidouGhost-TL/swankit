use serde_json::json;
use ws_discover::{discover, Evidence, Observation, SCHEMA};
fn fixture() -> (Vec<u8>, Evidence) {
    let rom = vec![0u8; 65536];
    let seed = json!({"schema":"swankit-asset-map/1","rom":{"sha256":ws_assets::sha256_hex(&rom),"size":rom.len()},"assets":[]});
    (
        rom,
        Evidence {
            schema: SCHEMA.into(),
            seed,
            ram: vec![0; 65536],
            ports: vec![0; 256],
            observations: vec![],
            texts: vec![],
            snapshot_maps: vec![],
            family: None,
        },
    )
}
#[test]
fn codec_requires_full_output_and_correct_identity() {
    let (mut rom, mut e) = fixture();
    let payload: Vec<_> = (0..256).map(|n| (n % 17) as u8).collect();
    let stored = ws_compress::lz_rle_7f::encode(&payload).unwrap();
    rom[100..100 + stored.len()].copy_from_slice(&stored);
    e.seed["rom"]["sha256"] = json!(ws_assets::sha256_hex(&rom));
    e.ram[0x2000..0x2100].copy_from_slice(&payload);
    e.observations.push(Observation {
        source: 100,
        routine: "f000:0100".into(),
        destination: Some(0x2000),
        length: None,
        decoded_length: None,
        output: None,
        prehistory: None,
        references: vec![],
    });
    let d = discover(&rom, &e).unwrap();
    assert_eq!(d.report.accepted_compressed, 1);
    assert_eq!(d.map.assets.len(), 1);
    assert!(!d.map.assets[0].references_complete);
    e.ram[0x2080] ^= 1;
    assert_eq!(discover(&rom, &e).unwrap().report.accepted_compressed, 0);
    rom[0] = 1;
    assert!(discover(&rom, &e).is_err());
}
#[test]
fn snapshot_sizes_and_source_bounds() {
    let (rom, mut e) = fixture();
    e.ram.pop();
    assert!(discover(&rom, &e).is_err());
    e.ram.push(0);
    e.observations.push(Observation {
        source: usize::MAX,
        routine: "unknown".into(),
        destination: None,
        length: None,
        decoded_length: None,
        output: None,
        prehistory: None,
        references: vec![],
    });
    assert_eq!(discover(&rom, &e).unwrap().report.rejected.len(), 1);
}
#[test]
fn uniform_false_positives_are_rejected() {
    let (rom, mut e) = fixture();
    e.observations.push(Observation {
        source: 0,
        routine: "fill".into(),
        destination: Some(0x2000),
        length: Some(256),
        decoded_length: None,
        output: None,
        prehistory: None,
        references: vec![],
    });
    assert_eq!(discover(&rom, &e).unwrap().report.accepted_raw, 0);
}
#[test]
fn invalid_reference_is_not_promoted_to_verified_map() {
    let (mut rom, mut e) = fixture();
    let raw: Vec<_> = (0..64).collect();
    rom[100..164].copy_from_slice(&raw);
    e.ram[0x2000..0x2040].copy_from_slice(&raw);
    e.seed["rom"]["sha256"] = json!(ws_assets::sha256_hex(&rom));
    let reference =
        serde_json::from_value(json!({"at":300,"width":2,"kind":"code-immediate"})).unwrap();
    e.observations.push(Observation {
        source: 100,
        routine: "copy".into(),
        destination: Some(0x2000),
        length: Some(64),
        decoded_length: None,
        output: None,
        prehistory: None,
        references: vec![reference],
    });
    assert!(discover(&rom, &e).is_err());
}

#[test]
fn exit_witness_requires_exact_output_and_bounded_destination() {
    let (mut rom, mut e) = fixture();
    let payload: Vec<_> = (0..128).map(|n| (n % 17) as u8).collect();
    let stored = ws_compress::lzss_12::encode(&payload).unwrap();
    rom[100..100 + stored.len()].copy_from_slice(&stored);
    e.seed["rom"]["sha256"] = json!(ws_assets::sha256_hex(&rom));
    e.observations.push(Observation {
        source: 100,
        routine: "decoder-exit".into(),
        destination: Some(0x4000),
        length: None,
        decoded_length: Some(payload.len()),
        output: Some(payload.clone()),
        prehistory: None,
        references: vec![],
    });
    assert_eq!(discover(&rom, &e).unwrap().report.codec_proofs.len(), 1);
    e.observations[0].output.as_mut().unwrap().push(0);
    assert!(discover(&rom, &e).unwrap().report.codec_proofs.is_empty());
    e.observations[0].output = Some(payload);
    e.observations[0].destination = Some(usize::MAX);
    assert!(discover(&rom, &e).unwrap().report.codec_proofs.is_empty());
}

#[test]
fn same_source_and_destination_different_lengths_have_distinct_ids() {
    let (mut rom, mut e) = fixture();
    let payload: Vec<_> = (0..128).collect();
    rom[100..228].copy_from_slice(&payload);
    e.ram[0x2000..0x2080].copy_from_slice(&payload);
    e.seed["rom"]["sha256"] = json!(ws_assets::sha256_hex(&rom));
    for n in [64, 128] {
        e.observations.push(Observation {
            source: 100,
            routine: "copy".into(),
            destination: Some(0x2000),
            length: Some(n),
            decoded_length: None,
            output: None,
            prehistory: None,
            references: vec![],
        });
    }
    assert_eq!(discover(&rom, &e).unwrap().map.assets.len(), 2);
}
