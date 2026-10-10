use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use ws_link::{Endpoint, Options, Relay, Schedule, Stats};

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);
impl Write for Log {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct Running {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<std::io::Result<Stats>>>,
    log: Log,
}
impl Running {
    fn start(relay: Relay, log: Log) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        Self {
            stop,
            worker: Some(thread::spawn(move || relay.run(&flag))),
            log,
        }
    }
    fn finish(mut self) -> (Stats, String) {
        self.stop.store(true, Ordering::Relaxed);
        let stats = self.worker.take().unwrap().join().unwrap().unwrap();
        let log = String::from_utf8(self.log.0.lock().unwrap().clone()).unwrap();
        (stats, log)
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn peer(address: std::net::SocketAddr) -> TcpStream {
    let p = TcpStream::connect(address).unwrap();
    p.set_nodelay(true).unwrap();
    p.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    p
}
fn wait_connections(running: &Running, end: &str, count: usize) {
    let start = Instant::now();
    loop {
        let log = String::from_utf8(running.log.0.lock().unwrap().clone()).unwrap();
        if log
            .lines()
            .filter(|l| l.starts_with(&format!("# {end}\t")) && l.ends_with("connected"))
            .count()
            >= count
        {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "connection timeout: {log}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn listener() -> Endpoint {
    Endpoint::Listen("127.0.0.1:0".into())
}

#[test]
fn endpoints_reject_malformed_configuration() {
    for bad in [
        "",
        "tcp:",
        "tcp:host",
        "tcp:host:0",
        "tcp:host:65536",
        "listen:",
        "listen::1",
        "exec:",
        "device:",
        "wondergate:",
        "other:x",
        "stdio:x",
        "tcp:a\0:2",
    ] {
        assert!(bad.parse::<Endpoint>().is_err(), "{bad:?}");
    }
    assert_eq!(
        "listen:3000".parse::<Endpoint>().unwrap(),
        Endpoint::Listen("127.0.0.1:3000".into())
    );
    assert!("tcp:[::1]:3000".parse::<Endpoint>().is_ok());
    assert!(Options {
        baud: 123,
        ..Options::default()
    }
    .validate()
    .is_err());
    assert!(Options {
        jitter: Duration::MAX,
        ..Options::default()
    }
    .validate()
    .is_err());
    assert!(Relay::new(
        [Endpoint::Stdio, Endpoint::Stdio],
        Options::default(),
        Box::new(std::io::sink())
    )
    .is_err());
}

#[test]
fn latency_jitter_order_bounds_and_independent_sequences() {
    let options = Options {
        latency: Duration::from_millis(20),
        jitter: Duration::from_millis(30),
        seed: 42,
        ..Options::default()
    };
    let mut a = Schedule::new(options, 0).unwrap();
    let mut b = Schedule::new(options, 0).unwrap();
    let mut reverse = Schedule::new(options, 1).unwrap();
    let mut previous = 0;
    let mut differs = false;
    for arrival in (0..100000).step_by(100) {
        let due = a.deadline_us(arrival);
        assert!((arrival..=arrival + 50000).contains(&due));
        assert!(due >= previous);
        assert_eq!(due, b.deadline_us(arrival));
        differs |= due != reverse.deadline_us(arrival);
        previous = due;
    }
    assert!(differs);
    let mut fixed = Schedule::new(
        Options {
            latency: Duration::from_millis(100),
            ..Options::default()
        },
        0,
    )
    .unwrap();
    assert_eq!(fixed.deadline_us(1000), 101000);
    assert_eq!(fixed.deadline_us(u64::MAX), u64::MAX);
}

#[test]
fn listen_bidirectional_binary_order_and_tsv_with_latency() {
    let log = Log::default();
    let relay = Relay::new(
        [listener(), listener()],
        Options {
            latency: Duration::from_millis(20),
            jitter: Duration::from_millis(10),
            seed: 7,
            ..Options::default()
        },
        Box::new(log.clone()),
    )
    .unwrap();
    let mut a = peer(relay.listen_address(0).unwrap());
    let mut b = peer(relay.listen_address(1).unwrap());
    let running = Running::start(relay, log);
    wait_connections(&running, "A", 1);
    wait_connections(&running, "B", 1);
    let bytes: Vec<u8> = (0..=255).cycle().take(8192).collect();
    let reverse: Vec<u8> = bytes.iter().rev().copied().collect();
    a.write_all(&bytes).unwrap();
    b.write_all(&reverse).unwrap();
    let mut received = vec![0; bytes.len()];
    b.read_exact(&mut received).unwrap();
    assert_eq!(received, bytes);
    a.read_exact(&mut received).unwrap();
    assert_eq!(received, reverse);
    let (stats, log) = running.finish();
    assert_eq!(stats.forwarded, [8192, 8192]);
    assert_eq!(stats.dropped, [0, 0]);
    assert!(log.starts_with("direction\ttime_us\tvalue\tresult\n"));
    for line in log.lines().skip(1).filter(|s| !s.starts_with('#')) {
        let cols: Vec<_> = line.split('\t').collect();
        assert_eq!(cols.len(), 4);
        assert!(matches!(cols[0], "A>B" | "B>A"));
        assert!(cols[1].parse::<u64>().unwrap() >= 10000);
        assert_eq!(cols[2].len(), 2);
        assert!(u8::from_str_radix(cols[2], 16).is_ok());
        assert_eq!(cols[3], "FORWARDED");
    }
}

#[test]
fn listen_disconnect_drops_only_read_bytes_and_preserves_unread_partner_bytes() {
    let log = Log::default();
    let relay = Relay::new(
        [listener(), listener()],
        Options {
            latency: Duration::from_millis(500),
            ..Options::default()
        },
        Box::new(log.clone()),
    )
    .unwrap();
    let address = relay.listen_address(1).unwrap();
    let mut a = peer(relay.listen_address(0).unwrap());
    let b = peer(address);
    let running = Running::start(relay, log);
    wait_connections(&running, "A", 1);
    wait_connections(&running, "B", 1);
    a.write_all(b"old").unwrap();
    thread::sleep(Duration::from_millis(100));
    drop(b);
    thread::sleep(Duration::from_millis(30));
    a.write_all(b"gone").unwrap();
    thread::sleep(Duration::from_millis(20));
    let mut b = peer(address);
    wait_connections(&running, "B", 2);
    a.write_all(b"new").unwrap();
    let mut received = [0; 7];
    b.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"gonenew");
    let mut bytes = [0; 3];
    b.write_all(b"ack").unwrap();
    a.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"ack");
    let (stats, log) = running.finish();
    assert_eq!(stats.dropped, [3, 0]);
    assert_eq!(stats.forwarded, [7, 3]);
    assert!(log.contains("DROPPED: end disconnected"));
}

#[test]
fn tcp_retries_initial_failure_and_reconnects_after_eof() {
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let log = Log::default();
    let relay = Relay::new(
        [listener(), Endpoint::Tcp(address.to_string())],
        Options::default(),
        Box::new(log.clone()),
    )
    .unwrap();
    let mut a = peer(relay.listen_address(0).unwrap());
    let running = Running::start(relay, log);
    thread::sleep(Duration::from_millis(150));
    let listener = TcpListener::bind(address).unwrap();
    listener.set_nonblocking(true).unwrap();
    let accept = || {
        let start = Instant::now();
        loop {
            if let Ok((p, _)) = listener.accept() {
                p.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                return p;
            }
            assert!(start.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(10));
        }
    };
    let mut b = accept();
    wait_connections(&running, "B", 1);
    a.write_all(b"one").unwrap();
    let mut bytes = [0; 3];
    b.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"one");
    drop(b);
    let mut b = accept();
    wait_connections(&running, "B", 2);
    b.write_all(b"two").unwrap();
    a.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"two");
    let (stats, _) = running.finish();
    assert_eq!(stats.connections, [1, 2]);
}

#[test]
fn exec_cat_echoes_every_data_value() {
    let log = Log::default();
    let relay = Relay::new(
        [listener(), Endpoint::Exec("cat".into())],
        Options::default(),
        Box::new(log.clone()),
    )
    .unwrap();
    let mut a = peer(relay.listen_address(0).unwrap());
    let running = Running::start(relay, log);
    wait_connections(&running, "A", 1);
    let bytes: Vec<u8> = (0..=255).collect();
    a.write_all(&bytes).unwrap();
    let mut received = vec![0; 256];
    a.read_exact(&mut received).unwrap();
    assert_eq!(received, bytes);
    let (stats, _) = running.finish();
    assert_eq!(stats.forwarded, [256, 256]);
}

#[test]
fn both_permanent_ends_gone_returns_error() {
    let relay = Relay::new(
        [
            Endpoint::Exec("exit 0".into()),
            Endpoint::Exec("exit 0".into()),
        ],
        Options::default(),
        Box::new(std::io::sink()),
    )
    .unwrap();
    assert_eq!(
        relay.run(&AtomicBool::new(false)).unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
}

#[test]
fn wondergate_existing_core_receives_fragmented_command_and_replies() {
    let log = Log::default();
    let relay = Relay::new(
        [listener(), Endpoint::Wondergate(None)],
        Options::default(),
        Box::new(log.clone()),
    )
    .unwrap();
    let mut a = peer(relay.listen_address(0).unwrap());
    let running = Running::start(relay, log);
    wait_connections(&running, "A", 1);
    // Documented PDC reception query, revival profile: no new adapter behavior.
    a.write_all(&[1, 0]).unwrap();
    a.write_all(&[1, 2]).unwrap();
    let mut reply = [0; 6];
    a.read_exact(&mut reply).unwrap();
    assert_eq!(reply, [1, 0, 3, 2, b'1', b'0']);
    let (stats, _) = running.finish();
    assert_eq!(stats.forwarded, [4, 6]);
}

#[cfg(target_os = "linux")]
#[test]
fn device_pty_is_raw_bidirectional_at_all_supported_bauds() {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    for baud in [9600, 38400, 192000] {
        let (mut master_fd, mut slave_fd) = (-1, -1);
        let mut name = [0i8; 256];
        // SAFETY: initialized output slots and sufficient tty name storage;
        // null optional termios/winsize arguments request default setup.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master_fd,
                    &mut slave_fd,
                    name.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            },
            0
        );
        let master: OwnedFd = unsafe { OwnedFd::from_raw_fd(master_fd) };
        let slave: OwnedFd = unsafe { OwnedFd::from_raw_fd(slave_fd) };
        let path = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }
            .to_str()
            .unwrap();
        let log = Log::default();
        let relay = Relay::new(
            [listener(), Endpoint::Device(path.into())],
            Options {
                baud,
                ..Options::default()
            },
            Box::new(log.clone()),
        )
        .unwrap();
        let mut termios: libc::termios2 = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe { libc::ioctl(slave.as_raw_fd(), libc::TCGETS2, &mut termios) },
            0
        );
        assert_eq!(termios.c_ispeed, baud);
        assert_eq!(termios.c_ospeed, baud);
        assert_eq!(termios.c_lflag, 0);
        assert_eq!(termios.c_iflag, 0);
        assert_eq!(termios.c_oflag, 0);
        assert_eq!(
            termios.c_cflag & (libc::CSIZE | libc::PARENB | libc::CSTOPB | libc::CRTSCTS),
            libc::CS8
        );
        let mut a = peer(relay.listen_address(0).unwrap());
        let running = Running::start(relay, log);
        wait_connections(&running, "A", 1);
        let mut device = std::fs::File::from(master);
        let bytes = b"\0\r\n\x11\x13\xffabc";
        device.write_all(bytes).unwrap();
        let mut received = [0; 9];
        a.read_exact(&mut received).unwrap();
        assert_eq!(&received, bytes);
        a.write_all(bytes).unwrap();
        let mut poll = libc::pollfd {
            fd: device.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut poll, 1, 3000) }, 1);
        device.read_exact(&mut received).unwrap();
        assert_eq!(&received, bytes);
        let (stats, _) = running.finish();
        assert_eq!(stats.forwarded, [9, 9]);
    }
}

#[test]
fn early_bytes_wait_in_socket_until_partner_connects() {
    let log = Log::default();
    let relay = Relay::new(
        [listener(), listener()],
        Options::default(),
        Box::new(log.clone()),
    )
    .unwrap();
    let a_address = relay.listen_address(0).unwrap();
    let b_address = relay.listen_address(1).unwrap();
    let running = Running::start(relay, log);
    let mut a = peer(a_address);
    wait_connections(&running, "A", 1);
    let bytes: Vec<u8> = (0..=255).cycle().take(8192).collect();
    a.write_all(&bytes).unwrap();
    thread::sleep(Duration::from_millis(100));
    let before = String::from_utf8(running.log.0.lock().unwrap().clone()).unwrap();
    assert!(
        !before.contains("A>B"),
        "read bytes before partner connected: {before}"
    );
    let mut b = peer(b_address);
    let mut received = vec![0; bytes.len()];
    b.read_exact(&mut received).unwrap();
    assert_eq!(received, bytes);
    let (stats, _) = running.finish();
    assert_eq!(stats.forwarded, [8192, 0]);
    assert_eq!(stats.dropped, [0, 0]);
}

#[test]
fn payload_larger_than_pending_limit_survives_delayed_backpressure() {
    let log = Log::default();
    let relay = Relay::new(
        [listener(), listener()],
        Options {
            latency: Duration::from_millis(200),
            ..Options::default()
        },
        Box::new(log.clone()),
    )
    .unwrap();
    let mut a = peer(relay.listen_address(0).unwrap());
    let mut b = peer(relay.listen_address(1).unwrap());
    let running = Running::start(relay, log);
    wait_connections(&running, "A", 1);
    wait_connections(&running, "B", 1);
    let bytes: Vec<u8> = (0..=255).cycle().take(3 * ws_link::MAX_PENDING).collect();
    a.write_all(&bytes).unwrap();
    b.write_all(b"reverse during backpressure").unwrap();
    let mut received = vec![0; bytes.len()];
    b.read_exact(&mut received).unwrap();
    assert_eq!(received, bytes);
    let mut reverse = [0; 27];
    a.read_exact(&mut reverse).unwrap();
    assert_eq!(&reverse, b"reverse during backpressure");
    let (stats, log) = running.finish();
    assert_eq!(stats.forwarded, [bytes.len() as u64, 27]);
    assert_eq!(stats.dropped, [0, 0]);
    assert!(!log.contains("DROPPED"));
}

#[cfg(target_os = "linux")]
#[test]
fn exec_cleanup_kills_term_ignoring_descendant_after_shell_exit() {
    use std::io::{BufRead, BufReader};
    let log = Log::default();
    let relay = Relay::new(
        [
            listener(),
            Endpoint::Exec("(trap '' TERM; echo ready; exec sleep 30) & echo $!".into()),
        ],
        Options::default(),
        Box::new(log.clone()),
    )
    .unwrap();
    let a = peer(relay.listen_address(0).unwrap());
    let running = Running::start(relay, log);
    let mut reader = BufReader::new(a);
    let mut lines = [String::new(), String::new()];
    for line in &mut lines {
        reader.read_line(line).unwrap();
    }
    assert!(lines.iter().any(|line| line.trim() == "ready"));
    let pid: i32 = lines
        .iter()
        .find_map(|line| line.trim().parse().ok())
        .unwrap();
    let _ = running.finish();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat"));
        // Container init may defer reaping an orphan; a zombie has terminated.
        if state.is_err() || state.as_ref().unwrap().split_whitespace().nth(2) == Some("Z") {
            break;
        }
        if Instant::now() >= deadline {
            // The fixture started this process; clean it up before failing.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            panic!("owned exec descendant survived relay cleanup");
        }
        thread::sleep(Duration::from_millis(10));
    }
}
