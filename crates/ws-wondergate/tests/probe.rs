// SPDX-License-Identifier: MIT OR Apache-2.0
use ws_wondergate::{backend::Scripted, frame::HeaderMode, Adapter, Config, Event, Probe};

#[test]
fn standalone_probe_echo_is_opt_in_and_preserves_framed_payloads() {
    for header in [HeaderMode::Len16Be, HeaderMode::ZeroThenLen8] {
        for chunk in [1, 2, 64] {
            let mut cfg = Config::from_ini("[swankit]\nprobe=echo\nprofile=revival\n").unwrap();
            cfg.profile.header = header;
            let mut adapter = Adapter::new(cfg, Scripted::default());
            // Probes surrounding power-on and opaque metadata containing 8F.
            let input = [0x8f, 0x8f, 1, 0, 1, 2, 1, 0, 2, 3, 0x8f, 0x8f];
            let mut out = Vec::new();
            let mut probes = 0;
            for part in input.chunks(chunk) {
                adapter.push(part, &mut out, &mut |event| {
                    if matches!(event, Event::Probe { echoed: true }) {
                        probes += 1;
                    }
                });
            }
            assert_eq!(
                out,
                [0x8f, 0x8f, 1, 0, 3, 2, b'1', b'0', 1, 0, 2, 3, 0, 0x8f]
            );
            assert_eq!(probes, 3);
            assert_eq!(adapter.snapshot().status, 2);
            assert!(adapter.finish().is_none());
        }
    }
    let cfg = Config::default();
    assert_eq!(cfg.probe, Probe::Framed);
    let mut adapter = Adapter::new(cfg, Scripted::default());
    let mut out = Vec::new();
    adapter.push(&[0x8f], &mut out, &mut |_| {});
    assert!(out.is_empty());
    assert!(adapter.finish().is_some());
}

#[test]
fn rejected_frame_drain_never_echoes_probe_bytes() {
    let mut cfg = Config::from_ini("[swankit]\nprobe=echo\nmax_frame=3\n").unwrap();
    cfg.profile.header = HeaderMode::Len16Be;
    let mut adapter = Adapter::new(cfg, Scripted::default());
    let mut out = Vec::new();
    adapter.push(
        &[1, 0, 4, 0x8f, 0x8f, 0x8f, 0x8f, 0x8f],
        &mut out,
        &mut |_| {},
    );
    assert_eq!(out, [0x8f]);
    assert_eq!(adapter.snapshot().status, 0);
    assert!(adapter.finish().is_none());
    assert!(Config::from_ini("[swankit]\nprobe=maybe\n").is_err());
}

#[test]
fn ignored_detector_does_not_poison_later_init_or_metadata() {
    for header in [HeaderMode::Len16Be, HeaderMode::ZeroThenLen8] {
        for chunk in [1, 3, 128] {
            let mut cfg = Config::from_ini("[swankit]\nprobe=ignore\n").unwrap();
            cfg.profile.header = header;
            let mut adapter = Adapter::new(cfg, Scripted::default());
            let input = [0x8f, 0x8f, 1, 0, 1, 2, 1, 0, 2, 3, 0x8f, 0x8f];
            let mut out = Vec::new();
            let mut ignored = 0;
            for part in input.chunks(chunk) {
                adapter.push(part, &mut out, &mut |event| {
                    if matches!(event, Event::Probe { echoed: false }) {
                        ignored += 1;
                    }
                });
            }
            assert_eq!(out, [1, 0, 3, 2, b'1', b'0', 1, 0, 2, 3, 0]);
            assert_eq!(ignored, 3);
            assert!(adapter.finish().is_none());
        }
    }
}
