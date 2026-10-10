// SPDX-License-Identifier: MIT OR Apache-2.0
use ws_wondergate::{
    backend::{Scripted, ScriptedPeer, Tcp},
    bpl::{BplAllow, HEADER_SIZE, IP, PORT},
    config::{integer, Backend},
    frame::{Frame, HeaderMode},
    redirect::{self, Endpoint, HostRedirect, SocketRedirect},
    Adapter, Config, Connection, Limits, Profile, Recv,
};
fn call<N: ws_wondergate::Network>(a: &mut Adapter<N>, cmd: u8, params: &[u8]) -> Vec<u8> {
    a.try_handle(&Frame {
        kind: 0x11,
        cmd,
        params: params.to_vec(),
    })
    .unwrap()
    .unwrap()
    .params
}
fn connect<N: ws_wondergate::Network>(a: &mut Adapter<N>, ip: [u8; 4], port: u16) {
    let id = call(a, 1, &[0])[0];
    let mut bytes = vec![id, 0, 2];
    bytes.extend_from_slice(&port.to_le_bytes());
    bytes.extend_from_slice(&ip);
    bytes.extend_from_slice(&[0; 8]);
    assert_eq!(call(a, 3, &bytes), [0, 1]);
}
fn packet(body: &[u8], metadata: u8) -> Vec<u8> {
    let mut header = vec![metadata; HEADER_SIZE];
    header[..4].copy_from_slice(&[1, 24, 0x80, 0x42]);
    header[4..8].copy_from_slice(&((HEADER_SIZE + body.len()) as u32).to_le_bytes());
    header.extend_from_slice(body);
    header
}
fn data(recv: Recv) -> Vec<u8> {
    match recv {
        Recv::Data(v) => v,
        _ => panic!("expected data"),
    }
}
fn allow(request: &[u8]) -> Vec<u8> {
    let mut reply = request[..24].to_vec();
    reply[..8].copy_from_slice(&[1, 24, 16, 0, 24, 0, 0, 0]);
    reply
}
#[test]
fn capture_shaped_bpl_vector_rebuilt_with_synthetic_url_and_metadata() {
    // The browser capture has total 45, a 24-byte header and a 21-byte URL
    // record. URL and metadata here are newly constructed synthetic bytes.
    // Client exchange at 1E8B37/1E8B55 requires all 24 bytes; flag mask at
    // 1E8B6C/1E8B70 accepts 0010 and branches to HTTP via B024.
    let request = packet(
        &[b"\x15\0\x01\0".as_slice(), b"unit.example/abc/"].concat(),
        0x5a,
    );
    assert_eq!(request.len(), 45);
    let mut bpl = BplAllow::default();
    for byte in &request {
        assert_eq!(bpl.send(&[*byte]).unwrap(), 1);
    }
    let mut reply = data(bpl.recv(8));
    reply.extend(data(bpl.recv(16)));
    assert_eq!(reply, allow(&request));
    assert_eq!(reply.len(), 24);
    assert_eq!(u16::from_le_bytes([reply[2], reply[3]]) & 0x3f, 0x10);
    assert_eq!(&reply[8..], &request[8..24]);
    assert!(bpl.finish().is_ok());
}
#[test]
fn bpl_waits_for_complete_body_and_queues_consecutive_messages() {
    let first = packet(b"opaque\0body", 0x11);
    let second = packet(b"next", 0x22);
    let mut bpl = BplAllow::default();
    bpl.feed(&first[..first.len() - 1]).unwrap();
    assert!(matches!(bpl.recv(24), Recv::Eof));
    assert!(bpl.finish().is_err());
    bpl.feed(&[&first[first.len() - 1..], &second, &packet(&[], 0x33)].concat())
        .unwrap();
    assert!(bpl.finish().is_ok());
    assert!(data(bpl.recv(0)).is_empty());
    assert_eq!(data(bpl.recv(usize::MAX)), allow(&first));
    assert_eq!(data(bpl.recv(24)), allow(&second));
    assert_eq!(data(bpl.recv(24)), allow(&packet(&[], 0x33)));
    assert!(matches!(bpl.recv(24), Recv::Eof));
    bpl.close();
    bpl.close();
    assert!(bpl.send(&first).is_err());
    assert!(matches!(bpl.recv(1), Recv::Error));
}
#[test]
fn malformed_and_over_limit_bpl_never_allocates_declared_body() {
    for size in [0u32, 8, 23, 65537, u32::MAX] {
        let mut request = packet(&[], 0x31);
        request[4..8].copy_from_slice(&size.to_le_bytes());
        let mut bpl = BplAllow::default();
        assert!(bpl.feed(&request).is_err());
        assert!(matches!(bpl.recv(24), Recv::Error));
        assert!(bpl.feed(&packet(&[], 0)).is_err());
    }
    for offset in [0, 1] {
        let mut request = packet(&[], 0);
        request[offset] = 8;
        assert!(BplAllow::default().feed(&request).is_err());
    }
    let mut bpl = BplAllow::new(24, 24).unwrap();
    bpl.feed(&packet(&[], 0)).unwrap();
    assert!(bpl.feed(&packet(&[], 1)).is_err());
    assert!(BplAllow::new(23, 24).is_err());
    assert!(BplAllow::new(24, usize::MAX).is_err());
    let mut bpl = BplAllow::new(65536, 24).unwrap();
    bpl.feed(&packet(&vec![0x11; 65536 - 24], 0)).unwrap();
    assert_eq!(data(bpl.recv(24)).len(), 24);
}
#[test]
fn builtin_bpl_bypasses_dns_and_socket_redirects_and_survives_empty_reads() {
    for profile in [Profile::REVIVAL, Profile::UPSTREAM] {
        let cfg = Config {
            profile,
            socket_redirects: vec![SocketRedirect {
                source: Endpoint { ip: IP, port: PORT },
                target: Endpoint {
                    ip: [192, 0, 2, 1],
                    port: 1,
                },
            }],
            ..Config::default()
        };
        let mut a = Adapter::new(cfg, Scripted::default());
        let host = include_bytes!("../data/bpl-hosts.txt")
            .split(|&b| b == b'\n')
            .next()
            .unwrap();
        let dns = call(&mut a, 8, &[host, &[0]].concat());
        assert_eq!(dns[0], 1);
        assert_eq!(&dns[dns.len() - 4..], &IP);
        connect(&mut a, IP, PORT);
        let id = profile.first_socket;
        assert_eq!(
            call(&mut a, 0xf, &[id, 24]),
            [
                0,
                if profile.eof == ws_wondergate::profile::Eof::Ff {
                    255
                } else {
                    0
                }
            ]
        );
        for marker in [0x12, 0x34] {
            let request = packet(b"url", marker);
            assert_eq!(
                call(&mut a, 0xe, &[&[id], request.as_slice()].concat()),
                [0, request.len() as u8]
            );
            assert_eq!(
                call(&mut a, 0xf, &[id, 24]),
                [vec![0, 24], allow(&request)].concat()
            );
            assert_eq!(call(&mut a, 0xf, &[id, 0]), [0, 0]);
        }
        call(&mut a, 7, &[id]);
        connect(&mut a, IP, PORT);
    }
}
#[test]
fn redirects_are_ordered_exact_single_pass_and_match_c_port_rules() {
    let hosts = [
        HostRedirect {
            source: b"old.invalid".to_vec(),
            target: b"new.invalid".to_vec(),
        },
        HostRedirect {
            source: b"new.invalid".to_vec(),
            target: b"again.invalid".to_vec(),
        },
    ];
    assert_eq!(redirect::hostname(b"old.invalid", &hosts), b"new.invalid");
    assert_eq!(redirect::hostname(b"OLD.invalid", &hosts), b"OLD.invalid");
    let rules = [
        SocketRedirect {
            source: Endpoint { ip: IP, port: 0 },
            target: Endpoint {
                ip: [127, 0, 0, 2],
                port: 5000,
            },
        },
        SocketRedirect {
            source: Endpoint { ip: IP, port: 80 },
            target: Endpoint {
                ip: [127, 0, 0, 3],
                port: 8080,
            },
        },
    ];
    assert_eq!(
        redirect::socket(Endpoint { ip: IP, port: 80 }, &rules),
        Endpoint {
            ip: [127, 0, 0, 2],
            port: 80
        }
    );
    assert_eq!(
        redirect::socket(Endpoint { ip: IP, port: 80 }, &rules[1..]),
        Endpoint {
            ip: [127, 0, 0, 3],
            port: 8080
        }
    );
    assert_eq!(
        redirect::socket(Endpoint { ip: IP, port: 81 }, &rules[1..]),
        Endpoint { ip: IP, port: 81 }
    );
    assert_eq!(
        redirect::socket(Endpoint { ip: IP, port: 0 }, &rules),
        Endpoint {
            ip: [127, 0, 0, 2],
            port: 5000
        }
    );
}
#[test]
fn ini_legacy_sections_prefixes_comments_and_swankit_settings() {
    let cfg = Config::from_ini("\u{feff}; comment\r\n[pdc]\nreception = $e ; level\n[ppp]\nuser = unit#name\npass = unit;value\n[bpl_proxy]\nenabled = 0b0\n[hostname_hack]\nold.invalid = 127.0.0.1\n[socket_hack]\n192.0.2.1:0x50 = 127.0.0.1:0o17520\n[swankit]\nheader = len16be\nprofile = upstream\nbackend = transcript\ntranscript_file = replay.log\nmax_frame = 0x1000\nmax_sockets = 0b100\nmax_bpl_message = 4096\nmax_bpl_pending = 48\nmax_receive = 256\nconnect_timeout_ms = 100\nrecv_timeout_ms = 200\nsend_timeout_ms = 300\nallow_system_dns = false\nresolve_all = 127.0.0.2\nresolve.exact.invalid = 127.0.0.1 # local\n").unwrap();
    assert_eq!(cfg.pdc_reception, 14);
    assert_eq!(cfg.ppp.user, b"unit#name");
    assert_eq!(cfg.ppp.password, b"unit;value");
    assert!(!cfg.builtin_bpl);
    assert_eq!(cfg.profile.header, HeaderMode::Len16Be);
    assert_eq!(cfg.profile.first_socket, 0x30);
    assert_eq!(cfg.backend, Backend::Transcript);
    assert_eq!(cfg.transcript_file.unwrap().to_str().unwrap(), "replay.log");
    assert_eq!(
        cfg.limits,
        Limits {
            max_frame: 4096,
            max_sockets: 4,
            max_bpl_message: 4096,
            max_bpl_pending: 48
        }
    );
    assert_eq!(cfg.socket_redirects[0].source.port, 80);
    assert_eq!(cfg.socket_redirects[0].target.port, 8016);
    assert_eq!(cfg.tcp.recv_timeout.as_millis(), 200);
    assert!(!cfg.tcp.allow_system_dns);
    assert_eq!(cfg.tcp.resolutions[b"exact.invalid".as_slice()], IP);
    for (text, value) in [
        ("007", 7),
        ("0xfF", 255),
        ("$A", 10),
        ("0b11", 3),
        ("0o77", 63),
    ] {
        assert_eq!(integer(text), Some(value));
    }
    for text in [
        "",
        "-1",
        "+1",
        "0x",
        "0b2",
        "0o9",
        "0XFF",
        "5garbage",
        "18446744073709551616",
    ] {
        assert!(integer(text).is_none());
    }
}
#[test]
fn ini_works_without_ppp_and_rejects_invalid_input_with_redacted_errors() {
    let cfg = Config::from_ini("").unwrap();
    assert_eq!(cfg.profile, Profile::REVIVAL);
    assert!(cfg.builtin_bpl);
    assert!(cfg.ppp.user.is_empty());
    for text in [
        "[unknown]",
        "reception=2",
        "[pdc]\nreception=16",
        "[pdc]\nreception=-1",
        "[ppp\npass=credential",
        "[ppp]\nunknown=credential",
        "[swankit]\nallow_system_dns=credential",
        "[swankit]\nmax_frame=65536",
        "[swankit]\nrecv_timeout_ms=0",
        "[socket_hack]\n1.2.3.999=127.0.0.1",
        "[hostname_hack]\nold.invalid=",
        "[swankit]\nbackend=unknown",
        "[swankit]\nheader=unknown",
        "[swankit]\nprofile=unknown",
        "[swankit]\nresolve.=127.0.0.1",
        "[ppp]\npass=a\0b",
    ] {
        let error = Config::from_ini(text).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("credential"));
    }
    let cfg = Config::from_ini("[ppp]\npass = private-token\n").unwrap();
    assert!(!format!("{cfg:?}").contains("private-token"));
    assert!(Config::from_ini(&" ".repeat(1024 * 1024 + 1)).is_err());
}
#[test]
fn configured_dns_and_socket_redirects_apply_to_adapter_and_bpl_can_be_external() {
    let cfg = Config::from_ini("[bpl_proxy]\nenabled=0\n[hostname_hack]\nold.invalid = 127.0.0.1\n[socket_hack]\n192.0.2.1:80 = 127.0.0.1:8080\n").unwrap();
    let mut a = Adapter::new(cfg.clone(), Tcp::default());
    let reply = call(&mut a, 8, b"old.invalid\0");
    assert_eq!(reply, [b"\x01127.0.0.1\0".as_slice(), &IP].concat());
    let mut net = Scripted::default();
    let log = net.queue(ScriptedPeer::new(IP, 8080, []));
    let mut a = Adapter::new(cfg.clone(), net);
    connect(&mut a, [192, 0, 2, 1], 80);
    call(&mut a, 7, &[0]);
    assert_eq!(log.close_calls(), 1);
    let mut net = Scripted::default();
    let log = net.queue(ScriptedPeer::new(
        IP,
        PORT,
        [Recv::Data(b"external".to_vec())],
    ));
    let mut a = Adapter::new(cfg, net);
    connect(&mut a, IP, PORT);
    assert_eq!(call(&mut a, 0xf, &[0, 24]), b"\0\x08external");
    assert_eq!(log.recv_calls(), 1);
}
#[test]
fn redirected_dns_reply_is_bounded_before_backend_lookup() {
    let mut cfg = Config::default();
    cfg.limits.max_frame = 8;
    cfg.host_redirects.push(HostRedirect {
        source: b"x".to_vec(),
        target: b"long.invalid".to_vec(),
    });
    let mut a = Adapter::new(cfg, Scripted::default());
    assert!(a
        .try_handle(&Frame {
            kind: 0x11,
            cmd: 8,
            params: b"x\0".to_vec()
        })
        .is_err());
}

#[test]
fn hostname_redirect_authorizes_only_its_explicit_target_dns() {
    let cfg = Config::from_ini("[hostname_hack]\nold.invalid=localhost\n").unwrap();
    let mut a = Adapter::new(cfg, Tcp::default());
    assert_eq!(call(&mut a, 8, b"localhost\0"), [0]);
    assert_eq!(
        call(&mut a, 8, b"old.invalid\0"),
        [b"\x01localhost\0".as_slice(), &IP].concat()
    );
    assert_eq!(call(&mut a, 8, b"localhost\0"), [0]);
    assert_eq!(call(&mut a, 8, b"unknown.invalid\0"), [0]);
}

#[test]
fn invalid_typed_bpl_limits_and_host_redirects_fail_without_backend_io() {
    let mut cfg = Config::default();
    cfg.limits.max_bpl_message = 0;
    let mut a = Adapter::new(cfg, Scripted::default());
    call(&mut a, 1, &[0]);
    let mut address = vec![0, 0, 2];
    address.extend_from_slice(&PORT.to_le_bytes());
    address.extend_from_slice(&IP);
    address.extend_from_slice(&[0; 8]);
    assert_eq!(call(&mut a, 3, &address), [0, 0]);
    for target in [vec![], b"bad\0name".to_vec()] {
        let cfg = Config {
            host_redirects: vec![HostRedirect {
                source: b"x".to_vec(),
                target,
            }],
            ..Config::default()
        };
        let mut a = Adapter::new(cfg, Scripted::default());
        assert!(a
            .try_handle(&Frame {
                kind: 0x11,
                cmd: 8,
                params: b"x\0".to_vec()
            })
            .is_err());
    }
    assert!(Config::from_ini("[swankit]\ntranscript_file=\n").is_err());
}
