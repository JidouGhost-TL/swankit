// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::{args::Args, Result};
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use ws_wondergate_server::{
    net::{Config, TcpServer, WireLog},
    store::JournalStore,
    Clock, FixedClock, OsRng, Rng, SeededRng, Server, SystemClock, Timestamp,
};

pub fn run(args: &mut Args) -> Result<()> {
    let bind: IpAddr = args
        .value("--bind")
        .unwrap_or_else(|| "127.0.0.1".into())
        .parse()?;
    let http_port = u16::try_from(args.number("--http-port", 80)?)?;
    let bpl_port = u16::try_from(args.number("--bpl-port", 5555)?)?;
    let data = PathBuf::from(args.require("--data")?);
    let wire = PathBuf::from(
        args.value("--wire")
            .or_else(|| args.value("--logs"))
            .ok_or("required option: --wire")?,
    );
    let clock: Box<dyn Clock> = match args.value("--clock").as_deref().unwrap_or("system") {
        "system" => Box::new(SystemClock),
        value if value.starts_with("fixed:") => {
            Box::new(FixedClock(Timestamp::parse_fixed(&value[6..])?))
        }
        _ => return Err("clock must be system or fixed:YYYYMMDDhhmmss (UTC)".into()),
    };
    let rng: Box<dyn Rng> = match args.value("--seed") {
        Some(value) => Box::new(SeededRng::new(value.parse()?)),
        None => Box::new(OsRng::new()?),
    };
    let timeout_secs = args.number("--timeout", 5)?;
    if timeout_secs == 0 || timeout_secs > 60 {
        return Err("timeout must be 1..=60 seconds".into());
    }
    args.finish(1)?;
    std::fs::create_dir_all(&data)?;
    let app = Server::bundled(
        Box::new(JournalStore::open(data.join("journal.jsonl"))?),
        clock,
        rng,
    )?;
    let server = TcpServer::bind(
        app,
        Config {
            http: SocketAddr::new(bind, http_port),
            bpl: SocketAddr::new(bind, bpl_port),
            timeout: Duration::from_secs(timeout_secs as u64),
            ..Config::default()
        },
        Arc::new(WireLog::open(wire)?),
    )?;
    println!(
        "WonderGate HTTP:{} BPL:{}",
        server.http_addr(),
        server.bpl_addr()
    );
    std::io::Write::flush(&mut std::io::stdout())?;
    loop {
        std::thread::park_timeout(Duration::from_secs(1));
        let errors = server.errors();
        if !errors.is_empty() {
            return Err(errors.join("; ").into());
        }
    }
}
