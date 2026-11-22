// SPDX-License-Identifier: MIT
// Ported from the private WonderGate revival server, wgserver/runtime.py (MIT).
// Copyright (C) 2026 jidoughost. Thread lifecycle and bounds are new code.
//! Threaded inbound TCP only; raw transactions match the Python wire schema.
use crate::{http, services::bpl, Server, Timestamp};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
fn other(message: &str) -> io::Error {
    io::Error::other(message)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub const MAX_WIRE_RECORDS: usize = 256;
pub const MAX_WIRE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_BPL_MESSAGES: usize = 32;
pub const MAX_HTTP_MESSAGES: usize = 128;
pub const MAX_PEER_CONNECTIONS: usize = 4;
struct Log {
    file: File,
    next_id: u64,
    records: usize,
    bytes: u64,
    refused: bool,
}
pub struct WireLog {
    directory: PathBuf,
    log: Mutex<Log>,
}
impl WireLog {
    pub fn open(directory: impl AsRef<Path>) -> io::Result<Self> {
        fs::create_dir_all(directory.as_ref())?;
        let mut options = File::options();
        // Windows cannot lock a handle that was opened for append access alone.
        #[cfg(windows)]
        options.read(true);
        let file = options
            .append(true)
            .create(true)
            .open(directory.as_ref().join("wire.jsonl"))?;
        file.try_lock()?;
        let records = fs::read_dir(directory.as_ref())?
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".request.bin"))
            .count();
        let bytes = fs::read_dir(directory.as_ref())?
            .filter_map(Result::ok)
            .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
            .sum();
        Ok(Self {
            directory: directory.as_ref().into(),
            log: Mutex::new(Log {
                file,
                next_id: 0,
                records,
                bytes,
                refused: false,
            }),
        })
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    /// Files are exclusive-created, so restarts never overwrite evidence.
    /// The timestamp is injected by the server, making wire times testable.
    /// At the cumulative quota logging is refused, with one diagnostic per
    /// open log. Serving continues and normal shutdown remains successful.
    pub fn record(
        &self,
        protocol: &str,
        peer: SocketAddr,
        request: &[u8],
        response: &[u8],
        error: Option<&str>,
        now: Timestamp,
    ) -> io::Result<()> {
        let mut log = self
            .log
            .lock()
            .map_err(|_| other("wire log lock poisoned"))?;
        let reservation = (request.len() as u64)
            .saturating_add(response.len() as u64)
            .saturating_add(4096);
        if log.records >= MAX_WIRE_RECORDS || log.bytes.saturating_add(reservation) > MAX_WIRE_BYTES
        {
            if !log.refused {
                eprintln!("wire logging refused: cumulative record or byte capacity reached");
                log.refused = true;
            }
            return Ok(());
        }
        // Reserve before creating any file, including when later I/O fails.
        log.records += 1;
        log.bytes += reservation;
        let (id, mut req) = loop {
            let id = format!("{:016x}{:016x}", std::process::id(), log.next_id);
            log.next_id = log
                .next_id
                .checked_add(1)
                .ok_or_else(|| other("wire sequence exhausted"))?;
            match File::options()
                .write(true)
                .create_new(true)
                .open(self.directory.join(format!("{id}.request.bin")))
            {
                Ok(file) => break (id, file),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        };
        let req_file = format!("{id}.request.bin");
        let res_file = format!("{id}.response.bin");
        let mut res = File::options()
            .write(true)
            .create_new(true)
            .open(self.directory.join(&res_file))?;
        req.write_all(request)?;
        req.sync_all()?;
        res.write_all(response)?;
        res.sync_all()?;
        let entry = json!({"id": id, "time": now.iso_utc(), "protocol": protocol,
            "peer": [json!(peer.ip().to_string()), json!(peer.port())], "request": req_file, "response": res_file,
            "request_length": request.len(), "response_length": response.len(), "error": error,
            "request_sha256": digest(request), "response_sha256": digest(response)});
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        log.file.write_all(&line)?;
        log.file.sync_all()
    }
}
#[derive(Clone, Debug)]
pub struct Config {
    pub http: SocketAddr,
    pub bpl: SocketAddr,
    pub timeout: Duration,
    pub max_connections: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            http: ([127, 0, 0, 1], 80).into(),
            bpl: ([127, 0, 0, 1], 5555).into(),
            timeout: Duration::from_secs(5),
            max_connections: 32,
        }
    }
}
struct Shared {
    stopped: AtomicBool,
    active: [AtomicUsize; 2],
    peers: Mutex<std::collections::BTreeMap<std::net::IpAddr, usize>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    errors: Mutex<Vec<String>>,
}
impl Shared {
    fn error(&self, error: impl ToString) {
        if let Ok(mut errors) = self.errors.lock() {
            if errors.len() < 128 {
                errors.push(error.to_string());
            }
        }
    }
    fn reap(&self) {
        if let Ok(mut workers) = self.workers.lock() {
            let mut index = 0;
            while index < workers.len() {
                if workers[index].is_finished() {
                    if workers.swap_remove(index).join().is_err() {
                        self.error("TCP worker panicked");
                    }
                } else {
                    index += 1;
                }
            }
        }
    }
}
struct Active(Arc<Shared>, usize, std::net::IpAddr);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active[self.1].fetch_sub(1, Ordering::AcqRel);
        if let Ok(mut peers) = self.0.peers.lock() {
            if let Some(n) = peers.get_mut(&self.2) {
                *n -= 1;
                if *n == 0 {
                    peers.remove(&self.2);
                }
            }
        }
    }
}
pub struct TcpServer {
    http: SocketAddr,
    bpl: SocketAddr,
    shared: Arc<Shared>,
    listeners: Vec<JoinHandle<()>>,
}
impl TcpServer {
    /// Binds both listeners before starting either. Physical ports may be zero.
    pub fn bind(server: Server, config: Config, wire: Arc<WireLog>) -> io::Result<Self> {
        if config.timeout.is_zero()
            || config.timeout > Duration::from_secs(60)
            || config.max_connections == 0
            || config.max_connections > 1024
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid TCP limits",
            ));
        }
        let http = TcpListener::bind(config.http)?;
        let bpl = TcpListener::bind(config.bpl)?;
        http.set_nonblocking(true)?;
        bpl.set_nonblocking(true)?;
        let http_address = http.local_addr()?;
        let bpl_address = bpl.local_addr()?;
        let shared = Arc::new(Shared {
            stopped: AtomicBool::new(false),
            active: std::array::from_fn(|_| AtomicUsize::new(0)),
            peers: Mutex::new(std::collections::BTreeMap::new()),
            workers: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
        });
        let mut result = Self {
            http: http_address,
            bpl: bpl_address,
            shared,
            listeners: Vec::new(),
        };
        for (protocol_index, (listener, protocol)) in
            [(http, "HTTP"), (bpl, "BPL")].into_iter().enumerate()
        {
            let app = server.clone();
            let log = wire.clone();
            let control = result.shared.clone();
            let config = config.clone();
            let handle = thread::Builder::new()
                .name(format!("wg-{protocol}-listener"))
                .spawn(move || {
                    while !control.stopped.load(Ordering::Acquire) {
                        control.reap();
                        match listener.accept() {
                            Ok((stream, peer)) => {
                                if control.active[protocol_index]
                                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                                        (n < config.max_connections).then_some(n + 1)
                                    })
                                    .is_err()
                                {
                                    drop(stream);
                                    continue;
                                }
                                let peer_allowed = match control.peers.lock() {
                                    Ok(mut peers) => {
                                        let n = peers.entry(peer.ip()).or_default();
                                        if *n >= MAX_PEER_CONNECTIONS {
                                            false
                                        } else {
                                            *n += 1;
                                            true
                                        }
                                    }
                                    Err(_) => false,
                                };
                                if !peer_allowed {
                                    control.active[protocol_index].fetch_sub(1, Ordering::AcqRel);
                                    drop(stream);
                                    continue;
                                }
                                let worker_app = app.clone();
                                let worker_log = log.clone();
                                let worker_control = control.clone();
                                let timeout = config.timeout;
                                // The guard is captured before spawn, including spawn failure.
                                let guard = Active(control.clone(), protocol_index, peer.ip());
                                let worker = thread::Builder::new()
                                    .name(format!("wg-{protocol}-connection"))
                                    .spawn(move || {
                                        let _guard = guard;
                                        if let Err(e) = serve(
                                            stream,
                                            peer,
                                            protocol,
                                            &worker_app,
                                            &worker_log,
                                            timeout,
                                            &worker_control,
                                        ) {
                                            worker_control.error(e);
                                        }
                                    });
                                match worker {
                                    Ok(worker) => match control.workers.lock() {
                                        Ok(mut workers) => workers.push(worker),
                                        Err(_) => {
                                            let _ = worker.join();
                                            control.error("worker list lock poisoned");
                                        }
                                    },
                                    Err(e) => control.error(e),
                                }
                            }
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(10))
                            }
                            Err(e) => {
                                control.error(e);
                                break;
                            }
                        }
                    }
                })?;
            result.listeners.push(handle);
        }
        Ok(result)
    }
    pub fn http_addr(&self) -> SocketAddr {
        self.http
    }
    pub fn bpl_addr(&self) -> SocketAddr {
        self.bpl
    }
    pub fn errors(&self) -> Vec<String> {
        self.shared
            .errors
            .lock()
            .map(|e| e.clone())
            .unwrap_or_else(|_| vec!["error list lock poisoned".into()])
    }
    /// Joins owned listener and connection threads; idle reads finish at timeout.
    pub fn shutdown(&mut self) -> io::Result<()> {
        self.shared.stopped.store(true, Ordering::Release);
        for listener in self.listeners.drain(..) {
            if listener.join().is_err() {
                self.shared.error("TCP listener panicked");
            }
        }
        let workers = std::mem::take(
            &mut *self
                .shared
                .workers
                .lock()
                .map_err(|_| other("worker list lock poisoned"))?,
        );
        for worker in workers {
            if worker.join().is_err() {
                self.shared.error("TCP worker panicked");
            }
        }
        let errors = self.errors();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(other(&errors.join("; ")))
        }
    }
}
impl Drop for TcpServer {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
fn read_window(stream: &TcpStream, deadline: Instant, cap: Duration) -> io::Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now()).min(cap);
    if remaining.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "transaction timeout",
        ));
    }
    stream.set_read_timeout(Some(remaining))
}
fn exact(
    stream: &mut TcpStream,
    raw: &mut Vec<u8>,
    length: usize,
    deadline: Instant,
) -> io::Result<()> {
    while raw.len() < length {
        read_window(stream, deadline, Duration::from_secs(60))?;
        let mut chunk = [0u8; 4096];
        let count = (length - raw.len()).min(chunk.len());
        let n = stream.read(&mut chunk[..count])?;
        if n == 0 {
            return Err(other("early EOF"));
        }
        raw.extend_from_slice(&chunk[..n]);
    }
    Ok(())
}
fn read_http(
    stream: &mut TcpStream,
    raw: &mut Vec<u8>,
    server: &Server,
    timeout: Duration,
) -> io::Result<()> {
    let mut input = http::Assembler::new(server.limits());
    let deadline = Instant::now() + timeout;
    loop {
        read_window(stream, deadline, timeout)?;
        let mut chunk = [0u8; 2048];
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Err(other("early EOF"));
        }
        raw.extend_from_slice(&chunk[..n]);
        let complete = match input.feed(&chunk[..n]) {
            Ok(complete) => complete,
            Err(e) => {
                return server
                    .parse_http(raw)
                    .map(|_| ())
                    .map_err(|_| io::Error::other(e));
            }
        };
        if complete {
            break;
        }
    }
    // The client's strlen+1 byte can be a separate TCP segment. Keep it in raw.
    if input.body_size() == Some(0) && raw.ends_with(b"\r\n\r\n") {
        for _ in 0..2 {
            read_window(stream, deadline, Duration::from_millis(50))?;
            let mut byte = [0];
            let read = stream.read(&mut byte);
            stream.set_read_timeout(Some(timeout))?;
            match read {
                Ok(1) => {
                    raw.push(byte[0]);
                    input.feed(&byte).map_err(io::Error::other)?;
                    if byte[0] != b'\r' {
                        break;
                    }
                }
                Ok(_) => break,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    break
                }
                Err(e) => return Err(e),
            }
        }
    }
    input.finish().map_err(io::Error::other)?;
    http::parse_with_aliases(raw, &server.limits(), &server.services().http_aliases)
        .map_err(io::Error::other)?;
    Ok(())
}
fn serve(
    mut stream: TcpStream,
    peer: SocketAddr,
    protocol: &str,
    server: &Server,
    wire: &WireLog,
    timeout: Duration,
    control: &Shared,
) -> io::Result<()> {
    // Accepted streams inherit nonblocking mode on some platforms.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    if protocol == "HTTP" {
        // Sequential reuse is opt-in per response; pipelining remains rejected.
        for transaction in 0..MAX_HTTP_MESSAGES {
            if control.stopped.load(Ordering::Acquire) {
                break;
            }
            let mut raw = Vec::new();
            let (reply, mut error) = match read_http(&mut stream, &mut raw, server, timeout) {
                Ok(()) => match server.try_dispatch_http(&raw) {
                    Ok(reply) => (reply, None),
                    Err(e) => (http::empty_response(500), Some(e.to_string())),
                },
                Err(_) if transaction > 0 && raw.is_empty() => break,
                Err(e) => (http::empty_response(400), Some(e.to_string())),
            };
            if let Err(e) = stream.write_all(&reply) {
                error = Some(e.to_string());
            }
            wire.record(
                protocol,
                peer,
                &raw,
                &reply,
                error.as_deref(),
                server.clock().now_utc(),
            )?;
            let persistent = reply
                .split(|b| *b == b'\n')
                .take_while(|line| *line != b"\r")
                .any(|line| line == b"Connection: keep-alive\r");
            if error.is_some() || !persistent {
                break;
            }
        }
    } else {
        let total_deadline = Instant::now() + timeout * 4;
        let mut messages = 0;
        while !control.stopped.load(Ordering::Acquire)
            && Instant::now() < total_deadline
            && messages < MAX_BPL_MESSAGES
        {
            messages += 1;
            let mut raw = Vec::new();
            let mut reply = Vec::new();
            let deadline = (Instant::now() + timeout).min(total_deadline);
            let result: io::Result<()> = (|| {
                read_window(&stream, deadline, timeout)?;
                let mut first = [0];
                if stream.read(&mut first)? == 0 {
                    return Ok(());
                }
                raw.push(first[0]);
                exact(&mut stream, &mut raw, 8, deadline)?;
                let size = bpl::length(&raw, &server.limits()).map_err(io::Error::other)?;
                exact(&mut stream, &mut raw, size, deadline)?;
                reply = bpl::reply(&raw, &server.limits()).map_err(io::Error::other)?;
                stream.write_all(&reply)
            })();
            if !raw.is_empty() {
                wire.record(
                    protocol,
                    peer,
                    &raw,
                    &reply,
                    result.as_ref().err().map(ToString::to_string).as_deref(),
                    server.clock().now_utc(),
                )?;
            }
            if raw.is_empty() || result.is_err() {
                break;
            }
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

#[cfg(test)]
#[path = "review_portability.rs"]
mod review_portability;
