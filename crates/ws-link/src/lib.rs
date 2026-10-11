//! Ordered, bounded serial byte relay. Linux/Unix endpoints carry data only.
mod endpoint;
mod os;
use endpoint::{End, Kind};
use std::{
    collections::VecDeque,
    io::{self, Write},
    os::fd::AsRawFd,
    path::PathBuf,
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub const MAX_PENDING: usize = 65536;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Tcp(String),
    Listen(String),
    Stdio,
    Exec(String),
    Device(PathBuf),
    Wondergate(Option<PathBuf>),
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn address(text: &str, listen: bool) -> io::Result<String> {
    let value = if listen && !text.contains(':') {
        format!("127.0.0.1:{text}")
    } else {
        text.to_owned()
    };
    let (host, port) = value
        .rsplit_once(':')
        .ok_or_else(|| invalid("endpoint needs host:port"))?;
    if host.is_empty()
        || host.contains(char::is_whitespace)
        || host.contains('\0')
        || port.parse::<u16>().ok().filter(|&n| n != 0).is_none()
    {
        return Err(invalid("invalid endpoint host or port"));
    }
    Ok(value)
}
impl FromStr for Endpoint {
    type Err = io::Error;
    fn from_str(text: &str) -> io::Result<Self> {
        if text.len() > 8192 || text.contains('\0') {
            return Err(invalid("endpoint exceeds bounds or contains NUL"));
        }
        if text == "stdio" {
            return Ok(Self::Stdio);
        }
        if text == "wondergate" {
            return Ok(Self::Wondergate(None));
        }
        let (kind, value) = text
            .split_once(':')
            .ok_or_else(|| invalid("unknown endpoint kind"))?;
        if value.is_empty() {
            return Err(invalid("empty endpoint value"));
        }
        match kind {
            "tcp" => Ok(Self::Tcp(address(value, false)?)),
            "listen" => Ok(Self::Listen(address(value, true)?)),
            "exec" => Ok(Self::Exec(value.to_owned())),
            "device" => Ok(Self::Device(value.into())),
            "wondergate" => Ok(Self::Wondergate(Some(value.into()))),
            _ => Err(invalid("unknown endpoint kind")),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub latency: Duration,
    pub jitter: Duration,
    pub seed: u64,
    pub baud: u32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            latency: Duration::ZERO,
            jitter: Duration::ZERO,
            seed: 0,
            baud: 9600,
        }
    }
}
impl Options {
    pub fn validate(&self) -> io::Result<()> {
        if self.latency > Duration::from_secs(600) || self.jitter > Duration::from_secs(600) {
            return Err(invalid("latency and jitter must be at most 600000 ms"));
        }
        if !matches!(self.baud, 9600 | 38400 | 192000) {
            return Err(invalid("baud must be 9600, 38400 or 192000"));
        }
        Ok(())
    }
}

/// Per-direction deterministic scheduling; equal seeds produce equal delay
/// sequences regardless of interleaving between directions. Microsecond ticks.
pub struct Schedule {
    options: Options,
    state: u64,
    last: u64,
}
impl Schedule {
    pub fn new(options: Options, direction: usize) -> io::Result<Self> {
        options.validate()?;
        Ok(Self {
            options,
            state: options.seed ^ (direction as u64).wrapping_mul(0x9e3779b97f4a7c15),
            last: 0,
        })
    }
    pub fn deadline_us(&mut self, arrival_us: u64) -> u64 {
        // SplitMix64: fixed wrapping arithmetic, no platform-dependent RNG.
        self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^= z >> 31;
        let jitter = self.options.jitter.as_micros() as i64;
        let delta = if jitter == 0 {
            0
        } else {
            (z % (2 * jitter as u64 + 1)) as i64 - jitter
        };
        let delay = (self.options.latency.as_micros() as i64 + delta).max(0) as u64;
        self.last = self.last.max(arrival_us.saturating_add(delay));
        self.last
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub forwarded: [u64; 2],
    pub dropped: [u64; 2],
    pub connections: [u64; 2],
}
struct Pending {
    value: u8,
    due: u64,
}
pub struct Relay {
    ends: [End; 2],
    queues: [VecDeque<Pending>; 2],
    schedules: [Schedule; 2],
    log: Box<dyn Write + Send>,
    start: Instant,
    stats: Stats,
}
impl Relay {
    pub fn new(
        endpoints: [Endpoint; 2],
        options: Options,
        mut log: Box<dyn Write + Send>,
    ) -> io::Result<Self> {
        options.validate()?;
        if endpoints
            .iter()
            .filter(|e| matches!(e, Endpoint::Stdio))
            .count()
            > 1
        {
            return Err(invalid("stdio can be used only once"));
        }
        let [a, b] = endpoints;
        let ends = [End::new(a, options.baud)?, End::new(b, options.baud)?];
        writeln!(log, "direction\ttime_us\tvalue\tresult")?;
        let mut relay = Self {
            ends,
            queues: std::array::from_fn(|_| VecDeque::new()),
            schedules: [Schedule::new(options, 0)?, Schedule::new(options, 1)?],
            log,
            start: Instant::now(),
            stats: Stats::default(),
        };
        for i in 0..2 {
            if relay.ends[i].io.is_some() {
                relay.connection(i)?;
            }
        }
        Ok(relay)
    }
    pub fn listen_address(&self, end: usize) -> Option<std::net::SocketAddr> {
        match &self.ends.get(end)?.kind {
            Kind::Listen(listener) => listener.local_addr().ok(),
            _ => None,
        }
    }
    fn now(&self) -> u64 {
        self.start.elapsed().as_micros().min(u64::MAX as u128) as u64
    }
    fn event(&mut self, end: usize, result: &str) -> io::Result<()> {
        writeln!(
            self.log,
            "# {}\t{}\t{result}",
            if end == 0 { "A" } else { "B" },
            self.now()
        )?;
        self.log.flush()
    }
    fn connection(&mut self, end: usize) -> io::Result<()> {
        self.stats.connections[end] += 1;
        self.event(end, "connected")
    }
    fn record(&mut self, direction: usize, value: u8, result: &str) -> io::Result<()> {
        writeln!(
            self.log,
            "{}\t{}\t{value:02X}\t{result}",
            if direction == 0 { "A>B" } else { "B>A" },
            self.now()
        )
    }
    fn drop_byte(&mut self, direction: usize, value: u8, reason: &str) -> io::Result<()> {
        self.stats.dropped[direction] += 1;
        self.record(direction, value, reason)
    }
    fn disconnect(&mut self, end: usize, reason: &str) -> io::Result<()> {
        self.ends[end].disconnect();
        self.ends[1 - end].reset_adapter()?;
        self.event(end, reason)?;
        for dir in 0..2 {
            while let Some(p) = self.queues[dir].pop_front() {
                self.drop_byte(dir, p.value, "DROPPED: end disconnected")?;
            }
        }
        self.log.flush()
    }
    fn paired(&self) -> bool {
        self.ends.iter().all(|e| e.io.is_some())
    }
    fn receive(&mut self, end: usize) -> io::Result<()> {
        // Recheck after every poll event: an earlier event may have removed
        // the partner. Unread bytes stay in the OS buffer until both return.
        if self.ends[end].read_eof || self.queues[end].len() == MAX_PENDING {
            return Ok(());
        }
        let Some(duplex) = &self.ends[end].io else {
            return Ok(());
        };
        let mut bytes = [0; 4096];
        let capacity = bytes.len().min(MAX_PENDING - self.queues[end].len());
        match os::read(duplex.read.as_raw_fd(), &mut bytes[..capacity]) {
            Ok(0) => {
                if self.queues[end].is_empty() {
                    self.disconnect(end, "disconnected: EOF")
                } else {
                    self.ends[end].read_eof = true;
                    Ok(())
                }
            }
            Ok(n) => {
                let arrival = self.now();
                for &value in &bytes[..n] {
                    let due = self.schedules[end].deadline_us(arrival);
                    self.queues[end].push_back(Pending { value, due });
                }
                self.log.flush()
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(())
            }
            Err(_) => self.disconnect(end, "disconnected: read error"),
        }
    }
    fn forward(&mut self, dir: usize) -> io::Result<()> {
        if !self.paired() {
            return Ok(());
        }
        let Some(io) = &self.ends[1 - dir].io else {
            return Ok(());
        };
        let now = self.now();
        let mut bytes = [0; 4096];
        let mut n = 0;
        for p in self.queues[dir].iter().take(bytes.len()) {
            if p.due > now {
                break;
            }
            bytes[n] = p.value;
            n += 1;
        }
        if n == 0 {
            return Ok(());
        }
        match os::write(io.write.as_raw_fd(), &bytes[..n]) {
            Ok(0) => self.disconnect(1 - dir, "disconnected: write EOF"),
            Ok(written) => {
                for value in &bytes[..written] {
                    self.queues[dir].pop_front();
                    self.stats.forwarded[dir] += 1;
                    self.record(dir, *value, "FORWARDED")?;
                }
                self.log.flush()
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(())
            }
            Err(_) => self.disconnect(1 - dir, "disconnected: write error"),
        }
    }
    /// Cancellation is checked at most every 100 ms. Child groups are owned
    /// by the relay and terminated on return, including on configuration errors.
    pub fn run(mut self, stop: &AtomicBool) -> io::Result<Stats> {
        while !stop.load(Ordering::Relaxed) {
            for i in 0..2 {
                self.ends[i].connect_due(Instant::now());
                self.forward(i)?;
                if self.ends[i].read_eof && self.queues[i].is_empty() {
                    self.disconnect(i, "disconnected: EOF after draining")?;
                }
            }
            // A permanent end cannot ever resume after EOF, regardless of
            // whether its partner can reconnect.
            // Closing the other end also closes owned child pipes/processes.
            if self.ends.iter().any(|e| !e.can_return() && e.io.is_none()) {
                for i in 0..2 {
                    if self.ends[i].io.is_some() {
                        self.disconnect(i, "disconnected: permanent partner ended")?;
                    }
                }
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "a permanent endpoint ended and cannot reconnect",
                ));
            }
            let now = self.now();
            let mut timeout = 100u64;
            let mut fds = Vec::new();
            let mut roles = Vec::new();
            for i in 0..2 {
                if let Some((fd, connecting)) = self.ends[i].connector() {
                    fds.push(libc::pollfd {
                        fd,
                        events: if connecting {
                            libc::POLLOUT
                        } else {
                            libc::POLLIN
                        },
                        revents: 0,
                    });
                    roles.push((i, true));
                }
                if let Some(io) = &self.ends[i].io {
                    // A full queue already supplies backpressure. Omit its
                    // input fd until space returns, including HUP, which would
                    // otherwise spin while delayed bytes await their deadline.
                    if !self.ends[i].read_eof && self.queues[i].len() < MAX_PENDING {
                        fds.push(libc::pollfd {
                            fd: io.read.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        });
                        roles.push((i, false));
                    }
                    if let Some(p) = self.queues[1 - i].front() {
                        if p.due <= now {
                            fds.push(libc::pollfd {
                                fd: io.write.as_raw_fd(),
                                events: libc::POLLOUT,
                                revents: 0,
                            });
                            roles.push((i, true)); // forward occurs at loop start
                        } else {
                            timeout = timeout.min((p.due - now).div_ceil(1000));
                        }
                    }
                }
            }
            os::poll(&mut fds, timeout as i32)?;
            for (fd, (end, connector)) in fds.iter().zip(roles) {
                if fd.revents == 0 {
                    continue;
                }
                if connector {
                    if self.ends[end].io.is_none() && self.ends[end].connected()? {
                        self.connection(end)?;
                    } else if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0
                        && self.ends[end].io.is_some()
                    {
                        self.disconnect(end, "disconnected: output closed")?;
                    }
                } else {
                    self.receive(end)?;
                }
            }
        }
        for dir in 0..2 {
            while let Some(p) = self.queues[dir].pop_front() {
                self.drop_byte(dir, p.value, "DROPPED: link ended")?;
            }
        }
        self.log.flush()?;
        Ok(self.stats)
    }
    /// Installs handlers before opening endpoints or spawning owned children.
    pub fn start_with_signals(
        endpoints: [Endpoint; 2],
        options: Options,
        log: Box<dyn Write + Send>,
    ) -> io::Result<Stats> {
        let _signals = os::Signals::install()?;
        Self::new(endpoints, options, log)?.run(&os::INTERRUPTED)
    }
    /// Installs process signal handlers for a single CLI relay. Do not run
    /// multiple signal-enabled relays concurrently in one process.
    pub fn run_with_signals(self) -> io::Result<Stats> {
        let _signals = os::Signals::install()?;
        self.run(&os::INTERRUPTED)
    }
}
