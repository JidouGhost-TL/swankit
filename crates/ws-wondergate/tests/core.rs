// SPDX-License-Identifier: MIT OR Apache-2.0
use ws_wondergate::{
    backend::{Scripted, ScriptedPeer},
    frame::{Decoder, Frame, HeaderMode},
    profile::{BplReply, CloseReply, Eof, Readline},
    Adapter, Config, Connection, Event, Limits, NetError, Network, Profile, Recv,
};
fn frame(kind: u8, cmd: u8, params: &[u8]) -> Frame {
    Frame {
        kind,
        cmd,
        params: params.to_vec(),
    }
}
fn config(profile: Profile) -> Config {
    Config {
        profile,
        ..Config::default()
    }
}
fn call<N: Network>(a: &mut Adapter<N>, kind: u8, cmd: u8, params: &[u8]) -> Frame {
    a.try_handle(&frame(kind, cmd, params)).unwrap().unwrap()
}
fn address(id: u8) -> [u8; 17] {
    [id, 0, 2, 80, 0, 127, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]
}
fn setup(
    profile: Profile,
    receives: Vec<Recv>,
) -> (Adapter<Scripted>, ws_wondergate::backend::PeerLog, u8) {
    let mut net = Scripted::default();
    let peer = ScriptedPeer::new([127, 0, 0, 1], 80, receives);
    let log = net.queue(peer);
    let mut a = Adapter::new(config(profile), net);
    let id = call(&mut a, 0x11, 1, &[0]).params[0];
    assert_eq!(call(&mut a, 0x11, 3, &address(id)).params, [0, 1]);
    (a, log, id)
}
#[test]
fn codec_roundtrips_at_both_mode_boundaries() {
    for mode in [HeaderMode::ZeroThenLen8, HeaderMode::Len16Be] {
        for n in [0, 1, 252, mode.max_length() - 1] {
            let original = frame(0x11, 8, &vec![0x81; n]);
            let bytes = original.encode(mode).unwrap();
            assert_eq!(bytes.len(), n + 4);
            let mut decoder = Decoder::new(
                mode,
                true,
                Limits {
                    max_frame: mode.max_length(),
                    max_sockets: 255,
                    ..Limits::default()
                },
            );
            for byte in &bytes[..bytes.len() - 1] {
                assert!(decoder.push(*byte).is_none());
            }
            assert_eq!(
                decoder.push(bytes[bytes.len() - 1]).unwrap().unwrap(),
                original
            );
            assert!(decoder.finish().is_none());
        }
        assert!(frame(1, 0, &vec![0; mode.max_length()])
            .encode(mode)
            .is_err());
    }
}
#[test]
fn decoder_drains_over_limit_and_recovers_without_interpreting_payload() {
    let mut d = Decoder::new(
        HeaderMode::Len16Be,
        true,
        Limits {
            max_frame: 3,
            max_sockets: 255,
            ..Limits::default()
        },
    );
    for b in [0x11, 0] {
        assert!(d.push(b).is_none());
    }
    assert!(d.push(5).unwrap().is_err());
    for b in [1, 0, 1, 2, 0] {
        assert!(d.push(b).is_none());
    }
    for b in [0, 1, 0] {
        assert!(d.push(b).is_none());
    }
    assert!(d.push(1).is_none());
    assert_eq!(d.push(2).unwrap().unwrap(), frame(1, 2, &[]));
}
#[test]
fn zero_length_and_incomplete_streams_report_errors() {
    let mut d = Decoder::new(HeaderMode::Len16Be, false, Limits::default());
    assert!(d.push(1).is_none());
    assert!(d.push(0).is_none());
    assert!(d.push(0).unwrap().is_err());
    for prefix in [vec![1], vec![1, 0], vec![1, 0, 3], vec![1, 0, 3, 2]] {
        for b in prefix {
            assert!(d.push(b).is_none());
        }
        assert!(d.finish().is_some());
        assert!(d.finish().is_none());
    }
    let mut d = Decoder::new(
        HeaderMode::Len16Be,
        false,
        Limits {
            max_frame: 2,
            max_sockets: 255,
            ..Limits::default()
        },
    );
    for b in [1, 0] {
        d.push(b);
    }
    assert!(d.push(3).unwrap().is_err());
    d.push(1);
    assert!(d.finish().is_some());
}
#[test]
fn legacy_uses_low_byte_and_padding_is_only_skipped_between_frames() {
    let mut d = Decoder::new(HeaderMode::ZeroThenLen8, true, Limits::default());
    let mut frames = Vec::new();
    for b in [0, 0, 0x11, 0x80, 3, 0xe, 0, 0, 0, 1, 0, 1, 2] {
        if let Some(value) = d.push(b) {
            frames.push(value.unwrap());
        }
    }
    assert_eq!(frames, [frame(0x11, 0xe, &[0, 0]), frame(1, 2, &[])]);
}
#[test]
fn phase0_f_hostname_path_uses_nonzero_high_byte_and_be16_reply() {
    // F hostname stores CH/CL at header+1/+2; strlen 254 => length 0100.
    let name = vec![b'x'; 254];
    let mut params = name.clone();
    params.push(0);
    let request = frame(0x11, 8, &params);
    assert_eq!(
        &request.encode(HeaderMode::Len16Be).unwrap()[..4],
        &[0x11, 1, 0, 8]
    );
    assert!(request.encode(HeaderMode::ZeroThenLen8).is_err());
    let mut net = Scripted::default();
    net.resolutions.insert(name, [192, 0, 2, 1]);
    let mut a = Adapter::new(Config::default(), net);
    let mut out = Vec::new();
    a.push(
        &request.encode(HeaderMode::Len16Be).unwrap(),
        &mut out,
        &mut |_| {},
    );
    assert_eq!(&out[..5], &[0x11, 1, 5, 8, 1]);
    assert_eq!(&out[out.len() - 4..], &[192, 0, 2, 1]);
    let mut d = Decoder::new(HeaderMode::Len16Be, true, Limits::default());
    let decoded: Vec<_> = out.into_iter().filter_map(|b| d.push(b)).collect();
    assert_eq!(decoded.len(), 1);
    assert!(decoded[0].is_ok());
    assert_eq!(Profile::REVIVAL.header, HeaderMode::Len16Be);
}
#[test]
fn phase0_bpl_24_byte_allow_is_delivered_opaquely_and_in_full() {
    assert_eq!(Profile::REVIVAL.bpl_reply, BplReply::FieldPreserving24);
    // Synthetic metadata, deliberately different from any captured request.
    let mut request = vec![0x42; 24];
    request[..8].copy_from_slice(&[1, 24, 0, 0, 29, 0, 0, 0]);
    request.extend_from_slice(&[5, 0, 1, 0, b'x']);
    let mut response = request[..24].to_vec();
    response[..8].copy_from_slice(&[1, 24, 16, 0, 24, 0, 0, 0]);
    let mut net = Scripted::default();
    let log = net.queue(ScriptedPeer::new(
        [127, 0, 0, 1],
        5555,
        [
            Recv::Data(response[..8].to_vec()),
            Recv::Data(response[8..].to_vec()),
            Recv::Eof,
        ],
    ));
    let cfg = Config {
        builtin_bpl: false,
        ..Config::default()
    };
    let mut a = Adapter::new(cfg.clone(), net);
    call(&mut a, 0x11, 1, &[0]);
    let mut addr = address(0);
    addr[3..5].copy_from_slice(&5555u16.to_le_bytes());
    assert_eq!(call(&mut a, 0x11, 3, &addr).params, [0, 1]);
    let mut send = vec![0];
    send.extend_from_slice(&request);
    assert_eq!(call(&mut a, 0x11, 0xe, &send).params, [0, 29]);
    assert_eq!(log.sent(), request);
    let first = call(&mut a, 0x11, 0xf, &[0, 24, 0]);
    let second = call(&mut a, 0x11, 0xf, &[0, 16, 0]);
    let delivered = [&first.params[2..], &second.params[2..]].concat();
    assert_eq!(delivered, response);
    assert_eq!(delivered.len(), 24);
    assert_eq!(
        u16::from_le_bytes([delivered[2], delivered[3]]) & 0x3f,
        0x10
    );
    assert_eq!(&delivered[8..], &request[8..24]);
    assert_eq!(call(&mut a, 0x11, 0xf, &[0, 24, 0]).params, [0, 255]);
    // Eight-byte responder: the client's full-read cannot complete before EOF.
    let mut net = Scripted::default();
    net.queue(ScriptedPeer::new(
        [127, 0, 0, 1],
        5555,
        [Recv::Data(vec![1, 8, 16, 0, 8, 0, 0, 0]), Recv::Eof],
    ));
    let mut a = Adapter::new(cfg, net);
    call(&mut a, 0x11, 1, &[0]);
    call(&mut a, 0x11, 3, &addr);
    let first = call(&mut a, 0x11, 0xf, &[0, 24, 0]);
    assert_eq!(first.params[1], 8);
    let next = call(&mut a, 0x11, 0xf, &[0, 16, 0]);
    assert_eq!(next.params, [0, 255]);
    assert!(first.params[2..].len() < 24);
}
#[test]
fn all_commands_under_both_profiles_and_state_transitions() {
    for profile in [Profile::UPSTREAM, Profile::REVIVAL] {
        let mut net = Scripted::default();
        net.resolutions
            .insert(b"unit.invalid".to_vec(), [192, 0, 2, 7]);
        net.queue(ScriptedPeer::new(
            [127, 0, 0, 1],
            80,
            [Recv::Data(b"line\nrest".to_vec()), Recv::Eof],
        ));
        let mut a = Adapter::new(config(profile), net);
        assert_eq!(call(&mut a, 1, 0, &[]).params, [3, 15]);
        assert_eq!(call(&mut a, 1, 2, &[]).params, b"10");
        assert_eq!(call(&mut a, 2, 1, &[3]).params, [2]);
        assert_eq!(call(&mut a, 1, 0, &[]).params, [0, 15]);
        assert_eq!(
            call(&mut a, 1, 0x10, &[7, 0, 0, 0, 0, 1, 1, b'u', 1, b'p']).params,
            [1]
        );
        assert!(a.snapshot().ppp_configured);
        let dns = [4, 0, 0, 0, 0, 0, 0, 0, 0, 192, 0, 2, 1, 192, 0, 2, 2];
        assert_eq!(call(&mut a, 1, 0x11, &dns).params, [1]);
        assert_eq!(a.snapshot().dns, [[192, 0, 2, 1], [192, 0, 2, 2]]);
        assert_eq!(call(&mut a, 1, 8, &[0, 1, 3, 1, b'#']).cmd, 0xb);
        assert_eq!(call(&mut a, 2, 1, &[3]).params, [3]);
        let hostname = call(&mut a, 0x11, 8, b"unit.invalid\0");
        assert_eq!(
            hostname.params,
            [vec![1], b"unit.invalid\0".to_vec(), vec![192, 0, 2, 7]].concat()
        );
        let id = call(&mut a, 0x11, 1, &[0]).params[0];
        assert_eq!(id, profile.first_socket);
        assert_eq!(call(&mut a, 0x11, 3, &address(id)).params, [0, 1]);
        assert_eq!(
            call(&mut a, 0x11, 6, &[id, 0]).params,
            if profile.poll_reports_allocation {
                vec![0, 1]
            } else {
                vec![1]
            }
        );
        assert_eq!(call(&mut a, 0x11, 0xe, &[id, b'q']).params, [0, 1]);
        let line = call(&mut a, 0x11, 0xd, &[id, 20, 0]);
        assert_eq!(
            &line.params[2..],
            if profile.readline == Readline::ThroughLf {
                &b"line\n"[..]
            } else {
                &b"line\nrest"[..]
            }
        );
        if profile.readline == Readline::ThroughLf {
            assert_eq!(call(&mut a, 0x11, 0xf, &[id, 20, 0]).params, b"\0\x04rest");
        }
        assert_eq!(
            call(&mut a, 0x11, 0xf, &[id, 20, 0]).params,
            [0, if profile.eof == Eof::Ff { 255 } else { 0 }]
        );
        assert_eq!(
            call(&mut a, 0x11, 7, &[id]).params,
            if profile.close_reply == CloseReply::SixByte {
                vec![0, 1]
            } else {
                vec![1]
            }
        );
        for cmd in [1, 3, 4, 9] {
            let response = a.try_handle(&frame(1, cmd, &[0])).unwrap();
            if profile.f_stack_metadata {
                let response = response.unwrap();
                assert_eq!(response.params, [0]);
                assert_eq!(response.cmd, if cmd == 9 { 0xb } else { cmd });
            } else {
                assert!(response.is_none());
            }
        }
        assert_eq!(call(&mut a, 1, 0xa, &[1]).params, [0]);
        assert_eq!(a.snapshot().status, 2);
        assert!(a.snapshot().phone.is_empty());
        let off = call(&mut a, 0xf, 0xff, &[0x55, 0xaa, 0x55, 0xaa]);
        assert_eq!(
            off.params,
            if profile.poweroff_parameter {
                vec![0]
            } else {
                vec![]
            }
        );
        assert_eq!(a.snapshot().status, 0);
    }
}
#[test]
fn malformed_prefixes_return_errors_without_side_effects() {
    let cases = [
        (2, 1, vec![3]),
        (
            1,
            0x10,
            vec![7, 0, 0, 0, 0, 1, 2, b'u', b'u', 2, b'p', b'p'],
        ),
        (
            1,
            0x11,
            vec![4, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8],
        ),
        (1, 8, vec![0, 1, 3, 2, b'#', b'#']),
        (1, 0xa, vec![1]),
        (0xf, 0xff, vec![0x55, 0xaa, 0x55, 0xaa]),
        (0x11, 1, vec![0]),
        (0x11, 3, address(0).to_vec()),
        (0x11, 6, vec![0, 2]),
        (0x11, 7, vec![0]),
        (0x11, 8, b"unit.invalid\0".to_vec()),
        (0x11, 0xe, vec![0]),
        (0x11, 0xd, vec![0, 1]),
        (0x11, 0xf, vec![0, 1]),
    ];
    for profile in [Profile::UPSTREAM, Profile::REVIVAL] {
        for (kind, cmd, valid) in &cases {
            if (*kind, *cmd) == (0x11, 6) && !profile.poll_reports_allocation {
                // Legacy poll never inspects parameters.
                continue;
            }
            if (*kind, *cmd) == (2, 1) && profile.status_without_parameter {
                // Parameterless status is a valid client form in this profile.
                continue;
            }
            for n in 0..valid.len() {
                let mut a = Adapter::new(config(profile), Scripted::default());
                let before = a.snapshot();
                assert!(
                    a.try_handle(&frame(*kind, *cmd, &valid[..n])).is_err(),
                    "{kind:02x}:{cmd:02x}, {n}"
                );
                assert_eq!(a.snapshot(), before);
            }
        }
    }
}
#[test]
fn invalid_fields_and_unterminated_names_fail() {
    let mut a = Adapter::new(Config::default(), Scripted::default());
    for f in [
        frame(2, 1, &[99]),
        frame(1, 8, &[0, 1, 7, 0]),
        frame(1, 0x10, &[0; 8]),
        frame(1, 0x11, &[0; 17]),
        frame(1, 0xa, &[0]),
        frame(0x11, 1, &[1]),
        frame(0x11, 8, b"unit.invalid"),
        frame(0xf, 0xff, &[0; 4]),
    ] {
        assert!(a.try_handle(&f).is_err());
    }
    assert!(a.handle(&frame(0xf, 0xff, &[0; 4])).is_none());
    assert!(a.try_handle(&frame(0x99, 0, &[])).unwrap().is_none());
    assert!(a.try_handle(&frame(1, 0x99, &[])).unwrap().is_none());
}
#[test]
fn credentials_redacted_from_events_config_debug_and_snapshot() {
    let mut a = Adapter::new(Config::default(), Scripted::default());
    let f = frame(
        1,
        0x10,
        &[
            7, 0, 0, 0, 0, 1, 1, b'u', 6, b's', b'e', b'c', b'r', b'e', b't',
        ],
    );
    let mut events = Vec::new();
    let mut out = Vec::new();
    a.push(
        &f.encode(HeaderMode::Len16Be).unwrap(),
        &mut out,
        &mut |e| events.push(e),
    );
    let Event::FrameIn(input) = &events[0] else {
        panic!("input event missing");
    };
    assert!(input.params.iter().all(|&b| b == 0));
    assert!(a.snapshot().ppp_configured);
    let mut cfg = Config::default();
    cfg.ppp.password = b"secret".to_vec();
    assert!(!format!("{cfg:?}").contains("secret"));
    assert!(!format!("{:?}", a.snapshot()).contains("secret"));
}
#[test]
fn failure_replies_emit_diagnostics_and_preserve_reply_shapes() {
    let mut a = Adapter::new(Config::default(), Scripted::default());
    let mut out = Vec::new();
    let mut drops = 0;
    a.push(&[0x11, 0, 1, 0xd], &mut out, &mut |e| {
        if matches!(e, Event::Dropped(_)) {
            drops += 1;
        }
    });
    assert_eq!(out, [0x11, 0, 3, 0xd, 0, 255]);
    assert_eq!(drops, 1);
    assert!(a.handle(&frame(0x11, 0xe, &[])).is_some());
    assert_eq!(a.handle(&frame(1, 0x10, &[])).unwrap().params, [0]);
}
#[test]
fn sockets_exhaust_safely_and_reserved_ff_is_never_allocated() {
    for profile in [Profile::UPSTREAM, Profile::REVIVAL] {
        let mut a = Adapter::new(config(profile), Scripted::default());
        for id in profile.first_socket..255 {
            assert_eq!(call(&mut a, 0x11, 1, &[0]).params, [id]);
        }
        assert_eq!(call(&mut a, 0x11, 1, &[0]).params, [255]);
        assert_eq!(call(&mut a, 0x11, 7, &[255]).params.last(), Some(&0));
        call(&mut a, 0x11, 7, &[profile.first_socket]);
        assert_eq!(call(&mut a, 0x11, 1, &[0]).params, [profile.first_socket]);
    }
    let mut cfg = Config::default();
    cfg.limits.max_sockets = 1;
    let mut a = Adapter::new(cfg, Scripted::default());
    assert_eq!(call(&mut a, 0x11, 1, &[0]).params, [0]);
    assert_eq!(call(&mut a, 0x11, 1, &[0]).params, [255]);
}
#[test]
fn partial_write_error_failed_connect_and_exactly_once_close() {
    let mut net = Scripted::default();
    net.peers.push_back(Err(NetError::Connect));
    let mut peer = ScriptedPeer::new([127, 0, 0, 1], 80, [Recv::Error]);
    peer.send_limit = 2;
    let log = net.queue(peer);
    let mut a = Adapter::new(Config::default(), net);
    call(&mut a, 0x11, 1, &[0]);
    assert_eq!(call(&mut a, 0x11, 3, &address(0)).params, [0, 0]);
    assert_eq!(call(&mut a, 0x11, 3, &address(0)).params, [0, 1]);
    assert_eq!(call(&mut a, 0x11, 0xe, b"\0abcd").params, [0, 2]);
    assert_eq!(log.sent(), b"ab");
    assert_eq!(call(&mut a, 0x11, 0xf, &[0, 10]).params, [0, 255]);
    drop(a);
    assert_eq!(log.close_calls(), 1);
    let (mut a, log, id) = setup(Profile::REVIVAL, vec![Recv::Eof]);
    call(&mut a, 0x11, 7, &[id]);
    drop(a);
    assert_eq!(log.close_calls(), 1);
}
#[test]
fn zero_size_read_does_not_call_backend_even_after_eof() {
    let (mut a, log, id) = setup(Profile::REVIVAL, vec![Recv::Eof]);
    assert_eq!(call(&mut a, 0x11, 0xd, &[id, 0]).params, [0, 0]);
    assert_eq!(log.recv_calls(), 0);
    assert_eq!(call(&mut a, 0x11, 0xf, &[id, 1]).params, [0, 255]);
    assert_eq!(log.recv_calls(), 1);
    assert_eq!(call(&mut a, 0x11, 0xf, &[id, 0]).params, [0, 0]);
    assert_eq!(log.recv_calls(), 1);
}
#[test]
fn legacy_read_reply_cannot_overflow_length_byte_or_reserved_count() {
    for profile in [Profile::UPSTREAM, Profile::REVIVAL] {
        let (mut a, _, id) = setup(profile, vec![Recv::Data(vec![0x81; 255])]);
        let result = call(&mut a, 0x11, 0xf, &[id, 255]);
        assert_eq!(
            result.params[1],
            if profile.header == HeaderMode::ZeroThenLen8 {
                252
            } else {
                254
            }
        );
        assert!(result.encode(profile.header).is_ok());
    }
}
#[test]
fn dns_zero_addresses_are_failure_and_large_reply_is_rejected_before_lookup() {
    let mut net = Scripted::default();
    net.resolutions.insert(b"zero.invalid".to_vec(), [0; 4]);
    let mut a = Adapter::new(Config::default(), net);
    assert_eq!(call(&mut a, 0x11, 8, b"zero.invalid\0").params, [0]);
    assert_eq!(call(&mut a, 0x11, 8, b"other.invalid\0").params, [0]);
    let mut cfg = Config::default();
    cfg.limits.max_frame = 8;
    let mut a = Adapter::new(cfg, Scripted::default());
    assert!(a.try_handle(&frame(0x11, 8, b"xx\0")).is_err());
}
struct Broken;
impl Connection for Broken {
    fn send(&mut self, data: &[u8]) -> Result<usize, NetError> {
        Ok(data.len() + 1)
    }
    fn recv(&mut self, max: usize) -> Recv {
        Recv::Data(vec![0x81; max + 1])
    }
    fn close(&mut self) {}
}
impl Network for Broken {
    fn resolve(&mut self, _: &[u8]) -> Option<[u8; 4]> {
        None
    }
    fn connect(&mut self, _: [u8; 4], _: u16) -> Result<Box<dyn Connection>, NetError> {
        Ok(Box::new(Broken))
    }
}
#[test]
fn invalid_backend_counts_never_leak_excess_data() {
    let mut a = Adapter::new(Config::default(), Broken);
    call(&mut a, 0x11, 1, &[0]);
    call(&mut a, 0x11, 3, &address(0));
    assert_eq!(call(&mut a, 0x11, 0xe, &[0, 5]).params, [0, 255]);
    assert_eq!(call(&mut a, 0x11, 0xf, &[0, 10]).params, [0, 255]);
}
#[test]
fn deterministic_malformed_streams_and_command_parameters_never_panic() {
    let mut seed = 0x8ae23d17u32;
    let commands = [
        (1, 0),
        (1, 1),
        (1, 2),
        (1, 3),
        (1, 4),
        (1, 8),
        (1, 9),
        (1, 0xa),
        (1, 0x10),
        (1, 0x11),
        (2, 1),
        (0xf, 0xff),
        (0x11, 1),
        (0x11, 3),
        (0x11, 6),
        (0x11, 7),
        (0x11, 8),
        (0x11, 0xd),
        (0x11, 0xe),
        (0x11, 0xf),
    ];
    for profile in [Profile::UPSTREAM, Profile::REVIVAL] {
        let mut a = Adapter::new(config(profile), Scripted::default());
        for i in 0..4096 {
            let n = i % 32;
            let bytes: Vec<u8> = (0..n)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    seed as u8
                })
                .collect();
            let (kind, cmd) = commands[i % commands.len()];
            let _ = a.try_handle(&frame(kind, cmd, &bytes));
            let mut out = Vec::new();
            a.push(&bytes, &mut out, &mut |_| {});
            if i % 8 == 0 {
                a.finish();
            }
        }
        a.finish();
    }
}

#[test]
fn configuration_bounds_fail_before_device_mutation() {
    let mut cfg = Config::default();
    cfg.limits.max_frame = 2;
    let mut a = Adapter::new(cfg, Scripted::default());
    let before = a.snapshot();
    assert!(a.try_handle(&frame(1, 2, &[])).is_err());
    assert_eq!(a.snapshot(), before);
    let mut cfg = Config::default();
    cfg.limits.max_sockets = 0;
    let mut a = Adapter::new(cfg, Scripted::default());
    assert_eq!(call(&mut a, 0x11, 1, &[0]).params, [255]);
    let mut cfg = Config::default();
    cfg.limits.max_frame = 3;
    let mut a = Adapter::new(cfg, Scripted::default());
    let before = a.snapshot();
    assert!(a.try_handle(&frame(0x11, 0xf, &[0, 1])).is_err());
    assert_eq!(a.snapshot(), before);
    assert_eq!(
        a.handle(&frame(0x11, 0xf, &[0, 1])).unwrap().params,
        [0, 255]
    );
}

#[test]
fn power_cycle_closes_sockets_and_clears_credentials_dns_and_phone() {
    let (mut a, log, id) = setup(Profile::REVIVAL, vec![Recv::Eof]);
    call(&mut a, 1, 0x10, &[7, 0, 0, 0, 0, 1, 1, b'u', 1, b'p']);
    call(&mut a, 1, 8, &[0, 1, 1, 1, b'#']);
    call(&mut a, 0xf, 0xff, &[0x55, 0xaa, 0x55, 0xaa]);
    assert_eq!(a.snapshot().sockets[0].id, id);
    call(&mut a, 1, 2, &[]);
    let state = a.snapshot();
    assert!(state.sockets.is_empty());
    assert!(state.phone.is_empty());
    assert!(!state.ppp_configured);
    assert_eq!(state.dns, [[0; 4]; 2]);
    assert_eq!(log.close_calls(), 1);
    assert_eq!(state.status, 2);
}

#[test]
fn explicit_send_error_and_line_error_tail_are_bounded() {
    let mut net = Scripted::default();
    let mut peer = ScriptedPeer::new(
        [127, 0, 0, 1],
        80,
        [Recv::Data(b"tail".to_vec()), Recv::Error],
    );
    peer.send_error = true;
    net.queue(peer);
    let mut a = Adapter::new(Config::default(), net);
    call(&mut a, 0x11, 1, &[0]);
    call(&mut a, 0x11, 3, &address(0));
    assert_eq!(call(&mut a, 0x11, 0xe, &[0, 5]).params, [0, 255]);
    assert_eq!(call(&mut a, 0x11, 0xd, &[0, 30]).params, b"\0\x04tail");
    assert_eq!(call(&mut a, 0x11, 0xd, &[0, 30]).params, [0, 255]);
}

#[test]
fn upstream_poll_does_not_require_parameters_and_profiles_switch_independently() {
    let mut a = Adapter::new(config(Profile::UPSTREAM), Scripted::default());
    assert_eq!(call(&mut a, 0x11, 6, &[]).params, [1]);
    let mut profile = Profile::REVIVAL;
    profile.header = HeaderMode::ZeroThenLen8;
    profile.close_reply = CloseReply::FiveByte;
    profile.eof = Eof::Zero;
    profile.readline = Readline::Block;
    profile.first_socket = 0x30;
    profile.poweroff_parameter = true;
    profile.skip_zero_padding = false;
    profile.f_stack_metadata = false;
    profile.poll_reports_allocation = false;
    profile.dial_modes = &[3];
    profile.status_params = &[3];
    profile.status_without_parameter = false;
    assert_eq!(profile, Profile::UPSTREAM);
}

#[test]
fn parameterless_status_is_an_independent_client_policy() {
    let mut a = Adapter::new(config(Profile::REVIVAL), Scripted::default());
    call(&mut a, 1, 2, &[]);
    let reply = call(&mut a, 2, 1, &[]);
    assert_eq!(reply.cmd, 2);
    assert_eq!(reply.params, [2]);
    let mut profile = Profile::REVIVAL;
    profile.status_without_parameter = false;
    let mut strict = Adapter::new(config(profile), Scripted::default());
    assert!(strict
        .try_handle(&Frame {
            kind: 2,
            cmd: 1,
            params: vec![]
        })
        .is_err());
    assert_eq!(call(&mut strict, 2, 1, &[0]).params, [0]);
}
