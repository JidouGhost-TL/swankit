// SPDX-License-Identifier: MIT OR Apache-2.0
use std::io::{self, Write};
use ws_wondergate::{
    backend::{Transcript, TranscriptPeer},
    bpl::BplAllow,
    frame::{Frame, HeaderMode},
    transcript::{self, Direction, Tap, TranscriptError, TranscriptLimits},
    Adapter, Config, Limits, NetError, Network, Recv,
};
const IP: [u8; 4] = [127, 0, 0, 1];
fn data(recv: Recv) -> Vec<u8> {
    match recv {
        Recv::Data(data) => data,
        _ => panic!("expected data"),
    }
}
#[test]
fn tap_roundtrips_directions_fragments_empty_records_and_wrapper_metadata() {
    let mut tap = Tap::new(Vec::new());
    tap.write_at(12, Direction::ToAdapter, &[1, 0]).unwrap();
    tap.write_at(13, Direction::FromAdapter, &[255, 0]).unwrap();
    tap.write_at(14, Direction::ToAdapter, &[1, 2]).unwrap();
    tap.write_at(15, Direction::FromAdapter, &[]).unwrap();
    let bytes = tap.into_inner();
    assert_eq!(
        bytes,
        b"T 00000012 > 0100\nT 00000013 < ff00\nT 00000014 > 0102\nT 00000015 < \n"
    );
    let text = format!(
        "T 00000000 # tap start metadata\n{}T 00000016 # tap end rc=0\n",
        String::from_utf8(bytes).unwrap()
    );
    let streams = transcript::parse(&text).unwrap();
    assert_eq!(streams.input, [1, 0, 1, 2]);
    assert_eq!(streams.output, [255, 0]);
    assert_eq!(streams.records.len(), 4);
    assert_eq!(streams.bytes(&streams.records[2]), Some([1, 2].as_slice()));
    assert_eq!(streams.records[2].ms, 14);
    let upper =
        transcript::read(b"T 1 < AAff\r\n".as_slice(), TranscriptLimits::default()).unwrap();
    assert_eq!(upper.output, [170, 255]);
}
#[test]
fn frame_listing_uses_core_decoder_with_both_headers_and_real_offsets() {
    for header in [HeaderMode::ZeroThenLen8, HeaderMode::Len16Be] {
        let first = Frame {
            kind: 1,
            cmd: 2,
            params: vec![],
        };
        let second = Frame {
            kind: 0x11,
            cmd: 0xe,
            params: vec![0, 0, 0x81],
        };
        let stream = [
            vec![0, 0],
            first.encode(header).unwrap(),
            vec![0],
            second.encode(header).unwrap(),
            vec![0],
        ]
        .concat();
        let listed = transcript::frames(&stream, header, true, Limits::default()).unwrap();
        assert_eq!(listed[0].offset, 2);
        assert_eq!(listed[0].frame, first);
        assert_eq!(listed[1].offset, 7);
        assert_eq!(listed[1].frame, second);
        let incomplete = &stream[..stream.len() - 2];
        assert!(matches!(
            transcript::frames(incomplete, header, true, Limits::default()),
            Err(TranscriptError::Frame { offset: 7, .. })
        ));
    }
    let long = Frame {
        kind: 0x11,
        cmd: 8,
        params: vec![b'x'; 300],
    };
    assert_eq!(
        transcript::frames(
            &long.encode(HeaderMode::Len16Be).unwrap(),
            HeaderMode::Len16Be,
            false,
            Limits::default()
        )
        .unwrap()[0]
            .frame,
        long
    );
    assert!(transcript::frames(&[1, 0, 0], HeaderMode::Len16Be, false, Limits::default()).is_err());
}
#[test]
fn transcript_malformed_text_and_bounds_return_errors() {
    for input in [
        "bad",
        "T",
        "T -1 > 00",
        "T 18446744073709551616 > 00",
        "T 0 ? 00",
        "T 0 > 0",
        "T 0 > zz",
        "T 0 > 00 junk",
        "T 0 > éé",
        "T 0 > \0",
    ] {
        assert!(transcript::parse(input).is_err(), "{input:?}");
    }
    let input = "T 0 > 010203\n";
    for limits in [
        TranscriptLimits {
            max_text: 2,
            ..TranscriptLimits::default()
        },
        TranscriptLimits {
            max_stream: 2,
            ..TranscriptLimits::default()
        },
        TranscriptLimits {
            max_records: 0,
            ..TranscriptLimits::default()
        },
    ] {
        assert!(transcript::read(input.as_bytes(), limits).is_err());
    }
    assert!(transcript::frames(
        &[1, 0, 2, 2, 0],
        HeaderMode::Len16Be,
        false,
        Limits {
            max_frame: 1,
            ..Limits::default()
        }
    )
    .is_err());
}
#[test]
fn replay_checks_fragmented_requests_and_independent_connections() {
    let mut net = Transcript::default();
    net.resolutions.insert(b"unit.invalid".to_vec(), IP);
    for response in [b"line\nbody".to_vec(), vec![0, 255]] {
        net.queue(TranscriptPeer::new(IP, 80, b"request".to_vec(), response).unwrap())
            .unwrap();
    }
    let log = net.log();
    assert_eq!(net.resolve(b"unit.invalid"), Some(IP));
    assert_eq!(net.resolve(b"other.invalid"), None);
    let mut first = net.connect(IP, 80).unwrap();
    assert_eq!(first.send(b"req").unwrap(), 3);
    assert!(data(first.recv(0)).is_empty());
    assert_eq!(first.send(b"uest").unwrap(), 4);
    let mut second = net.connect(IP, 80).unwrap();
    second.send(b"request").unwrap();
    assert_eq!(data(first.recv(5)), b"line\n");
    assert_eq!(data(second.recv(usize::MAX)), [0, 255]);
    assert_eq!(data(first.recv(usize::MAX)), b"body");
    assert!(matches!(first.recv(1), Recv::Eof));
    first.close();
    first.close();
    drop(first);
    drop(second);
    log.verify().unwrap();
    assert_eq!(log.snapshot().matched_requests, 2);
    assert_eq!(log.snapshot().closed, 2);
}
#[test]
fn replay_rejects_wrong_bytes_short_requests_early_reads_and_wrong_endpoints() {
    for mode in 0..5 {
        let mut net = Transcript::default();
        net.queue(TranscriptPeer::new(IP, 80, b"expected".to_vec(), b"reply".to_vec()).unwrap())
            .unwrap();
        let log = net.log();
        if mode == 4 {
            assert!(matches!(net.connect(IP, 81), Err(NetError::Denied)));
            assert!(log.verify().is_err());
            continue;
        }
        let mut peer = net.connect(IP, 80).unwrap();
        match mode {
            0 => assert!(peer.send(b"wrong").is_err()),
            1 => {
                peer.send(b"exp").unwrap();
            }
            2 => assert!(matches!(peer.recv(1), Recv::Error)),
            _ => assert!(peer.send(b"expectedmore").is_err()),
        }
        drop(peer);
        assert!(log.verify().is_err());
        assert_eq!(log.snapshot().mismatches, 1);
    }
    let mut net = Transcript::default();
    net.queue(TranscriptPeer::new(IP, 80, vec![], vec![]).unwrap())
        .unwrap();
    assert!(net.log().verify().is_err()); // Unused peer is also a failed replay.
}
#[test]
fn replay_composes_with_adapter_readline_eof_and_close() {
    let streams = transcript::parse("T 0 > 7175657279\nT 1 < 6c696e650a626f6479\n").unwrap();
    let mut net = Transcript::default();
    net.queue(TranscriptPeer::from_streams(IP, 80, streams).unwrap())
        .unwrap();
    let log = net.log();
    let mut adapter = Adapter::new(Config::default(), net);
    let mut call = |cmd, params: &[u8]| {
        adapter
            .try_handle(&Frame {
                kind: 0x11,
                cmd,
                params: params.to_vec(),
            })
            .unwrap()
            .unwrap()
            .params
    };
    assert_eq!(call(1, &[0]), [0]);
    assert_eq!(
        call(3, &[0, 0, 2, 80, 0, 127, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]),
        [0, 1]
    );
    assert_eq!(call(0xe, b"\0query"), [0, 5]);
    assert_eq!(call(0xd, &[0, 30]), b"\0\x05line\n");
    assert_eq!(call(0xf, &[0, 30]), b"\0\x04body");
    assert_eq!(call(0xf, &[0, 30]), [0, 255]);
    call(7, &[0]);
    drop(adapter);
    log.verify().unwrap();
}
#[test]
fn replay_loads_external_files_with_checked_limits() {
    let root = std::env::temp_dir().join(format!("wg-replay-test-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let request = root.join("request");
    let response = root.join("response");
    std::fs::write(&request, b"query").unwrap();
    std::fs::write(&response, b"reply").unwrap();
    let peer = TranscriptPeer::from_files(IP, 80, &request, &response).unwrap();
    let mut net = Transcript::default();
    net.queue(peer).unwrap();
    let mut conn = net.connect(IP, 80).unwrap();
    conn.send(b"query").unwrap();
    assert_eq!(data(conn.recv(10)), b"reply");
    std::fs::remove_dir_all(root).unwrap();
}
struct BrokenWriter;
impl Write for BrokenWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("synthetic failure"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn tap_reports_sink_errors() {
    assert!(Tap::new(BrokenWriter)
        .write_at(0, Direction::ToAdapter, &[0])
        .is_err());
}
#[test]
fn deterministic_malformed_bpl_ini_and_transcript_inputs_never_panic() {
    let mut seed = 0xc5d314a7u32;
    for i in 0..8192 {
        let bytes: Vec<_> = (0..i % 64)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as u8
            })
            .collect();
        let mut bpl = BplAllow::new(128, 48).unwrap();
        for chunk in bytes.chunks(3) {
            let _ = bpl.feed(chunk);
        }
        let _ = bpl.finish();
        let text = String::from_utf8_lossy(&bytes);
        let _ = Config::from_ini(&text);
        let _ = transcript::parse(&text);
        let _ = transcript::frames(
            &bytes,
            HeaderMode::Len16Be,
            true,
            Limits {
                max_frame: 64,
                ..Limits::default()
            },
        );
    }
}
