// SPDX-License-Identifier: MIT OR Apache-2.0
//! Shared blocking byte pump for stdio adapters and serial link endpoints.
use crate::{
    backend::Tcp,
    config::Backend,
    transcript::{Direction, Tap},
    Adapter, Config, Network,
};
use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
};

pub fn load_config(path: &Path) -> io::Result<Config> {
    let mut text = String::new();
    File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_string(&mut text)?;
    Config::from_ini(&text).map_err(io::Error::other)
}

/// Each reply is flushed immediately. EOF closes sockets and validates the
/// final frame; an incomplete frame returns InvalidData instead of spinning.
pub fn serve<N: Network>(
    mut adapter: Adapter<N>,
    mut input: impl Read,
    mut output: impl Write,
    mut tap: Tap<impl Write>,
) -> io::Result<()> {
    let mut bytes = [0; 4096];
    loop {
        let n = match input.read(&mut bytes) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            other => other?,
        };
        if n == 0 {
            return match adapter.finish() {
                Some(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
                None => Ok(()),
            };
        }
        tap.record(Direction::ToAdapter, &bytes[..n])?;
        let mut reply = Vec::new();
        adapter.push(&bytes[..n], &mut reply, &mut |_| {});
        if !reply.is_empty() {
            output.write_all(&reply)?;
            output.flush()?;
            tap.record(Direction::FromAdapter, &reply)?;
        }
    }
}

pub fn serve_tcp(
    config: Config,
    input: impl Read,
    output: impl Write,
    tap: impl Write,
) -> io::Result<()> {
    let net = tcp_network(&config)?;
    serve(Adapter::new(config, net), input, output, Tap::new(tap))
}

pub fn tcp_network(config: &Config) -> io::Result<Tcp> {
    if config.backend != Backend::Tcp {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "raw endpoint requires TCP backend",
        ));
    }
    Tcp::new(config.tcp.clone()).map_err(|_| io::Error::other("invalid adapter TCP options"))
}
