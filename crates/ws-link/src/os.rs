//! Small Unix boundary. Every unsafe call uses owned/live descriptors and
//! initialized buffers; no emulator implementation is involved.
use std::{
    io,
    net::{SocketAddr, TcpStream},
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    sync::atomic::{AtomicBool, Ordering},
};

pub fn flags(fd: RawFd) -> io::Result<i32> {
    // SAFETY: fcntl reads flags from a live descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(flags)
}

pub fn nonblocking(fd: RawFd) -> io::Result<i32> {
    // SAFETY: fcntl accepts a live descriptor and integer flag arguments.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(flags)
}

pub fn duplicate(fd: RawFd) -> io::Result<OwnedFd> {
    // SAFETY: dup returns a new descriptor, owned exactly once below.
    let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(copy) })
}

pub fn restore_flags(fd: RawFd, flags: i32) {
    // SAFETY: descriptor remains live until its owner drops.
    unsafe {
        libc::fcntl(fd, libc::F_SETFL, flags);
    }
}

pub fn read(fd: RawFd, bytes: &mut [u8]) -> io::Result<usize> {
    // SAFETY: output buffer is writable for its exact length.
    let n = unsafe { libc::read(fd, bytes.as_mut_ptr().cast(), bytes.len()) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

pub fn write(fd: RawFd, bytes: &[u8]) -> io::Result<usize> {
    // SAFETY: input buffer is readable for its exact length. SIGPIPE is ignored
    // by the Rust executable runtime; signal-enabled runs also install SIG_IGN.
    let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

pub fn poll(fds: &mut [libc::pollfd], timeout: i32) -> io::Result<()> {
    // SAFETY: poll receives the exact initialized mutable array length.
    let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
    if n < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn connect(addr: SocketAddr) -> io::Result<TcpStream> {
    let domain = if addr.is_ipv4() {
        libc::AF_INET
    } else {
        libc::AF_INET6
    };
    #[cfg(not(target_vendor = "apple"))]
    let socket_type = libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC;
    #[cfg(target_vendor = "apple")]
    let socket_type = libc::SOCK_STREAM;
    // SAFETY: socket has no pointer arguments; the resulting fd is owned once.
    let fd = unsafe { libc::socket(domain, socket_type, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    #[cfg(target_vendor = "apple")]
    {
        // Darwin socket creation accepts the type alone. Set descriptor flags
        // while the descriptor is still owned locally, before connecting.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        nonblocking(fd)?;
    }
    // SAFETY: correctly typed, initialized sockaddr structures and exact sizes.
    let status = match addr {
        SocketAddr::V4(a) => {
            let socket = libc::sockaddr_in {
                #[cfg(target_vendor = "apple")]
                sin_len: std::mem::size_of::<libc::sockaddr_in>() as u8,
                sin_family: libc::AF_INET as _,
                sin_port: a.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_ne_bytes(a.ip().octets()),
                },
                sin_zero: [0; 8],
            };
            unsafe {
                libc::connect(
                    fd,
                    (&socket as *const libc::sockaddr_in).cast(),
                    std::mem::size_of_val(&socket) as _,
                )
            }
        }
        SocketAddr::V6(a) => {
            let socket = libc::sockaddr_in6 {
                #[cfg(target_vendor = "apple")]
                sin6_len: std::mem::size_of::<libc::sockaddr_in6>() as u8,
                sin6_family: libc::AF_INET6 as _,
                sin6_port: a.port().to_be(),
                sin6_flowinfo: a.flowinfo(),
                sin6_addr: libc::in6_addr {
                    s6_addr: a.ip().octets(),
                },
                sin6_scope_id: a.scope_id(),
            };
            unsafe {
                libc::connect(
                    fd,
                    (&socket as *const libc::sockaddr_in6).cast(),
                    std::mem::size_of_val(&socket) as _,
                )
            }
        }
    };
    if status < 0 && io::Error::last_os_error().raw_os_error() != Some(libc::EINPROGRESS) {
        return Err(io::Error::last_os_error());
    }
    let stream = TcpStream::from(owned);
    stream.set_nodelay(true)?;
    Ok(stream)
}

pub fn raw_device(fd: RawFd, baud: u32) -> io::Result<()> {
    // Linux termios2/BOTHER accepts arbitrary adapter rates including 192000.
    // See Linux TCSETS(2); UART hardware details are cited in docs/link.md.
    #[cfg(target_os = "linux")]
    {
        // SAFETY: termios2 is a plain C record; ioctl reads/writes its exact size.
        let mut t: libc::termios2 = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(fd, libc::TCGETS2, &mut t) } < 0 {
            return Err(io::Error::last_os_error());
        }
        t.c_iflag = 0;
        t.c_oflag = 0;
        t.c_lflag = 0;
        t.c_cflag &= !(libc::CBAUD | libc::CSIZE | libc::PARENB | libc::CSTOPB | libc::CRTSCTS);
        t.c_cflag |= libc::BOTHER | libc::CS8 | libc::CREAD | libc::CLOCAL;
        t.c_ispeed = baud;
        t.c_ospeed = baud;
        t.c_cc[libc::VMIN] = 1;
        t.c_cc[libc::VTIME] = 0;
        if unsafe { libc::ioctl(fd, libc::TCSETS2, &t) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (fd, baud);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "serial devices require Linux termios2",
        ))
    }
}

pub fn terminate_group(pid: u32) {
    // SAFETY: the process group was created by this relay, never another job.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGTERM);
    }
}
pub fn kill_group(pid: u32) {
    // SAFETY: only called for the still-owned child process group.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);
extern "C" fn interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}
pub struct Signals(Vec<(libc::c_int, libc::sigaction)>);
impl Signals {
    pub fn install() -> io::Result<Self> {
        let mut guard = Self(Vec::new());
        INTERRUPTED.store(false, Ordering::Relaxed);
        for signal in [
            libc::SIGINT,
            libc::SIGTERM,
            libc::SIGHUP,
            libc::SIGQUIT,
            libc::SIGPIPE,
        ] {
            // SAFETY: initialized sigaction records, valid signal numbers.
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            let mut old: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = if signal == libc::SIGPIPE {
                libc::SIG_IGN
            } else {
                interrupt as *const () as usize
            };
            unsafe {
                libc::sigemptyset(&mut action.sa_mask);
            }
            if unsafe { libc::sigaction(signal, &action, &mut old) } < 0 {
                return Err(io::Error::last_os_error());
            }
            guard.0.push((signal, old));
        }
        Ok(guard)
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        for (signal, old) in &self.0 {
            // SAFETY: restore records returned by sigaction.
            unsafe {
                libc::sigaction(*signal, old, std::ptr::null_mut());
            }
        }
    }
}

pub fn fd<T: AsRawFd>(value: &T) -> RawFd {
    value.as_raw_fd()
}
