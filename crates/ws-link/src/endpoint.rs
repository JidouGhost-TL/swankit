use crate::{os, Endpoint};
use std::{
    fs::OpenOptions,
    io,
    net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    os::{
        fd::{AsRawFd, OwnedFd, RawFd},
        unix::{fs::OpenOptionsExt, net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) struct Duplex {
    pub read: OwnedFd,
    pub write: OwnedFd,
    restore: Option<(i32, i32)>,
}
impl Duplex {
    fn new(read: OwnedFd, write: OwnedFd, restore: bool) -> io::Result<Self> {
        // Snapshot both descriptions before changing either: duped tty/socket
        // descriptors can share the same file status flags.
        let r = os::flags(read.as_raw_fd())?;
        let w = os::flags(write.as_raw_fd())?;
        os::nonblocking(read.as_raw_fd())?;
        match os::nonblocking(write.as_raw_fd()) {
            Ok(_) => {}
            Err(e) => {
                os::restore_flags(read.as_raw_fd(), r);
                return Err(e);
            }
        };
        Ok(Self {
            read,
            write,
            restore: restore.then_some((r, w)),
        })
    }
    fn socket(stream: impl Into<OwnedFd>) -> io::Result<Self> {
        let read = stream.into();
        let write = os::duplicate(read.as_raw_fd())?;
        Self::new(read, write, false)
    }
}
impl Drop for Duplex {
    fn drop(&mut self) {
        if let Some((r, w)) = self.restore {
            os::restore_flags(self.read.as_raw_fd(), r);
            os::restore_flags(self.write.as_raw_fd(), w);
        }
    }
}

pub(crate) enum Kind {
    Tcp {
        addresses: Vec<SocketAddr>,
        cursor: usize,
        pending: Option<TcpStream>,
        retry: Instant,
    },
    Listen(TcpListener),
    Fixed,
}
pub(crate) struct End {
    pub kind: Kind,
    pub io: Option<Duplex>,
    child: Option<ChildGroup>,
    adapter_config: Option<Option<std::path::PathBuf>>,
    pub read_eof: bool,
}

struct ChildGroup(Child);
impl Drop for ChildGroup {
    fn drop(&mut self) {
        // The group is ours even if the shell already exited: descendants may
        // still own its pipes. Terminate them as well, then reap our child.
        os::terminate_group(self.0.id());
        // Keep the shell unreaped until both signals have been delivered, so
        // its pid/group id cannot be reused. Shell exit does not prove that
        // descendants exited; a background child may ignore SIGTERM.
        std::thread::sleep(Duration::from_millis(250));
        os::kill_group(self.0.id());
        let _ = self.0.wait();
    }
}

impl End {
    pub fn new(endpoint: Endpoint, baud: u32) -> io::Result<Self> {
        let mut out = Self {
            kind: Kind::Fixed,
            io: None,
            child: None,
            adapter_config: None,
            read_eof: false,
        };
        match endpoint {
            Endpoint::Tcp(address) => {
                let addresses: Vec<_> = address.to_socket_addrs()?.collect();
                if addresses.is_empty() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "TCP address has no resolution",
                    ));
                }
                out.kind = Kind::Tcp {
                    addresses,
                    cursor: 0,
                    pending: None,
                    retry: Instant::now(),
                };
            }
            Endpoint::Listen(address) => {
                let listener = TcpListener::bind(address)?;
                listener.set_nonblocking(true)?;
                out.kind = Kind::Listen(listener);
            }
            Endpoint::Stdio => {
                out.io = Some(Duplex::new(os::duplicate(0)?, os::duplicate(1)?, true)?);
            }
            Endpoint::Exec(command) => {
                let mut command_process = Command::new("/bin/sh");
                command_process
                    .arg("-c")
                    .arg(command)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::inherit());
                command_process.process_group(0);
                let mut child = ChildGroup(command_process.spawn()?);
                let input = child
                    .0
                    .stdout
                    .take()
                    .ok_or_else(|| io::Error::other("missing child stdout"))?;
                let output = child
                    .0
                    .stdin
                    .take()
                    .ok_or_else(|| io::Error::other("missing child stdin"))?;
                out.io = Some(Duplex::new(input.into(), output.into(), false)?);
                out.child = Some(child);
            }
            Endpoint::Device(path) => {
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
                    .open(path)?;
                os::raw_device(file.as_raw_fd(), baud)?;
                out.io = Some(Duplex::socket(file)?);
            }
            Endpoint::Wondergate(path) => {
                out.adapter_config = Some(path.clone());
                let config = if let Some(path) = path {
                    ws_wondergate::raw::load_config(&path)?
                } else {
                    ws_wondergate::Config::default()
                };
                let net = ws_wondergate::raw::tcp_network(&config)?;
                let (relay, peer) = UnixStream::pair()?;
                let reader = peer.try_clone()?;
                // Existing adapter core owns protocol parsing; only bytes cross
                // this socket. Its blocking network waits cannot stall the relay.
                std::thread::Builder::new()
                    .name("serial-adapter".into())
                    .spawn(move || {
                        let adapter = ws_wondergate::Adapter::new(config, net);
                        let _ = ws_wondergate::raw::serve(
                            adapter,
                            reader,
                            peer,
                            ws_wondergate::transcript::Tap::new(io::sink()),
                        );
                    })?;
                out.io = Some(Duplex::socket(relay)?);
            }
        }
        Ok(out)
    }
    pub fn reset_adapter(&mut self) -> io::Result<()> {
        if let Some(path) = self.adapter_config.clone() {
            self.io = None; // EOF stops the old pump; old replies cannot reach the new socket.
            *self = Self::new(Endpoint::Wondergate(path), 9600)?;
        }
        Ok(())
    }
    pub fn can_return(&self) -> bool {
        !matches!(self.kind, Kind::Fixed)
    }
    pub fn disconnect(&mut self) {
        self.io = None;
        self.child = None;
        self.read_eof = false;
        if let Kind::Tcp { retry, pending, .. } = &mut self.kind {
            *pending = None;
            *retry = Instant::now() + Duration::from_secs(1);
        }
    }
    pub fn connect_due(&mut self, now: Instant) {
        if self.io.is_some() {
            return;
        }
        if let Kind::Tcp {
            addresses,
            cursor,
            pending,
            retry,
        } = &mut self.kind
        {
            if now >= *retry {
                *pending = None;
                let address = addresses[*cursor]; // nonempty, cursor always modulo len
                *cursor = (*cursor + 1) % addresses.len();
                *pending = os::connect(address).ok();
                *retry = now + Duration::from_secs(1);
            }
        }
    }
    pub fn connector(&self) -> Option<(RawFd, bool)> {
        if self.io.is_some() {
            return None;
        }
        match &self.kind {
            Kind::Listen(listener) => Some((os::fd(listener), false)),
            Kind::Tcp {
                pending: Some(stream),
                ..
            } => Some((os::fd(stream), true)),
            _ => None,
        }
    }
    pub fn connected(&mut self) -> io::Result<bool> {
        let stream = match &mut self.kind {
            Kind::Listen(listener) => match listener.accept() {
                Ok((stream, _)) => stream,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(e) => return Err(e),
            },
            Kind::Tcp { pending, .. } => {
                let Some(stream) = pending.take() else {
                    return Ok(false);
                };
                if stream.take_error()?.is_some() || stream.peer_addr().is_err() {
                    return Ok(false);
                }
                stream
            }
            _ => return Ok(false),
        };
        stream.set_nodelay(true)?;
        self.io = Some(Duplex::socket(stream)?);
        self.read_eof = false;
        Ok(true)
    }
}
