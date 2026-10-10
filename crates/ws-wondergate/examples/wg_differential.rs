// SPDX-License-Identifier: MIT OR Apache-2.0
//! Private differential-test driver. The production adapter CLI is separate.
use std::io::{self, Read, Write};
use ws_wondergate::{backend::Tcp, Adapter, Config};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        return Err("expected one INI path".into());
    }
    let mut text = String::new();
    std::fs::File::open(&args[0])?
        .take(1024 * 1024 + 1)
        .read_to_string(&mut text)?;
    let config = Config::from_ini(&text)?;
    let net = Tcp::new(config.tcp.clone()).map_err(|_| "invalid TCP options")?;
    let mut adapter = Adapter::new(config, net);
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut buffer = [0; 4096];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        let mut replies = Vec::new();
        adapter.push(&buffer[..n], &mut replies, &mut |_| {});
        output.write_all(&replies)?;
        output.flush()?;
    }
    if let Some(error) = adapter.finish() {
        return Err(error.into());
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("differential adapter: {error}");
        std::process::exit(64);
    }
}
