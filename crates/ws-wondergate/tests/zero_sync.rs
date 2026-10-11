// SPDX-License-Identifier: MIT OR Apache-2.0
use ws_wondergate::{backend::Scripted, frame::Frame, Adapter, Config, Event, Profile};

#[test]
fn alternate_init_validates_before_powering_on_and_preserves_upstream() {
    let mut adapter = Adapter::new(Config::default(), Scripted::default());
    for params in [vec![], vec![0], vec![3, 0]] {
        let before = adapter.snapshot();
        assert!(adapter
            .try_handle(&Frame {
                kind: 2,
                cmd: 0,
                params
            })
            .is_err());
        assert_eq!(adapter.snapshot(), before);
    }
    let request = Frame {
        kind: 2,
        cmd: 0,
        params: vec![3],
    };
    assert_eq!(
        adapter.try_handle(&request).unwrap(),
        Some(Frame {
            kind: 2,
            cmd: 2,
            params: vec![]
        })
    );
    assert_eq!(adapter.snapshot().status, 2);
    assert_eq!(adapter.snapshot().pdc, 0);
    let cfg = Config {
        profile: Profile::UPSTREAM,
        ..Config::default()
    };
    let mut original = Adapter::new(cfg, Scripted::default());
    assert!(original.try_handle(&request).unwrap().is_none());
    assert_eq!(original.snapshot().status, 0);
}

#[test]
fn optional_dial_padding_is_bounded_and_default_reply_is_unchanged() {
    let request = Frame {
        kind: 1,
        cmd: 8,
        params: vec![0, 1, 1, 2, b'#', b'#'],
    };
    for enabled in [false, true] {
        let cfg = Config {
            dial_padding: enabled,
            ..Config::default()
        };
        let mut adapter = Adapter::new(cfg, Scripted::default());
        let response = adapter.try_handle(&request).unwrap().unwrap();
        assert_eq!(response.params, if enabled { vec![0, 0] } else { vec![0] });
        assert_eq!(adapter.snapshot().status, 3);
    }
    let mut cfg = Config::from_ini("[swankit]\ndial_padding=true\n").unwrap();
    cfg.limits.max_frame = 2;
    let mut adapter = Adapter::new(cfg, Scripted::default());
    assert!(adapter.try_handle(&request).is_err());
    assert_eq!(adapter.snapshot().status, 0);
    assert!(Config::from_ini("[swankit]\ndial_padding=maybe\n").is_err());
}

#[test]
fn warmup_is_opt_in_chunk_independent_and_preserves_frames() {
    for chunk in [1, 3, 64] {
        let cfg = Config::from_ini("[swankit]\nzero_sync=true\n").unwrap();
        let mut adapter = Adapter::new(cfg, Scripted::default());
        let mut input = vec![0; 10];
        input.extend_from_slice(&[1, 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        input.extend_from_slice(&[1, 0, 1, 2]);
        let mut out = Vec::new();
        let mut syncs = 0;
        for part in input.chunks(chunk) {
            adapter.push(part, &mut out, &mut |event| {
                if matches!(event, Event::ZeroSync) {
                    syncs += 1;
                }
            });
        }
        assert_eq!(syncs, 1);
        assert_eq!(&out[..5], &[0; 5]);
        assert!(out.ends_with(&[1, 0, 3, 2, b'1', b'0']));
        assert_eq!(adapter.snapshot().status, 2);
        assert!(adapter.finish().is_none());
    }
    assert!(!Config::default().zero_sync);
    assert!(Config::from_ini("[swankit]\nzero_sync=maybe\n").is_err());
    let mut adapter = Adapter::new(Config::default(), Scripted::default());
    let mut out = Vec::new();
    adapter.push(&[0; 10], &mut out, &mut |_| {});
    assert!(out.is_empty());
}

#[test]
fn interrupted_padding_and_rejected_body_do_not_trigger_sync() {
    let cfg = Config::from_ini("[swankit]\nzero_sync=1\nprobe=ignore\nmax_frame=3\n").unwrap();
    let mut adapter = Adapter::new(cfg, Scripted::default());
    let mut out = Vec::new();
    adapter.push(&[0; 9], &mut out, &mut |_| {});
    adapter.push(&[0x8f], &mut out, &mut |_| {});
    adapter.push(&[0; 9], &mut out, &mut |_| {});
    // Oversized frame declares ten zero payload bytes. Drain them as a frame.
    adapter.push(&[1, 0, 10], &mut out, &mut |_| {});
    adapter.push(&[0; 10], &mut out, &mut |_| {});
    assert!(out.is_empty());
    adapter.push(&[0; 10], &mut out, &mut |_| {});
    assert_eq!(out, [0; 5]);
    assert!(adapter.finish().is_none());
}
