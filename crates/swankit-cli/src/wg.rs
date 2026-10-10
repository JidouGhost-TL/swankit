// SPDX-License-Identifier: MIT OR Apache-2.0
use std::{
    env,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
use ws_wondergate::{frame::HeaderMode, transcript, Config, Limits};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub const ADAPTER_HELP: &str =
    "wg-adapter [CONFIG.ini | ASDF] [--config CONFIG.ini] [--tap FILE] [--net tcp]
Raw request/reply bytes on stdin/stdout; diagnostics on stderr; exits on EOF.
Config: --config, positional path, WG_INI, then bounded revival defaults.
ASDF is Mednafen's ignored sentinel. WG_SERIAL_LOG appends a tap; --tap creates
a new file. Taps retain all wire bytes, including credentials.
The TCP resolver denies unlisted names by default; configure redirects or
explicit mappings in the INI. --version credits the MIT WonderFence core.";
pub const DECODE_HELP: &str = "wg-decode SERIAL.log [--header len16be|zero-then-len8]
Print joined directional byte counts, offsets, Type:Cmd, names and parameters.
The default header is 16-bit big-endian; interframe zero padding is skipped.
Malformed taps or incomplete frames fail with a diagnostic.";

fn value(words: &mut impl Iterator<Item = String>, option: &str) -> Result<String> {
    let word = words
        .next()
        .ok_or_else(|| format!("missing value for {option}"))?;
    if word.starts_with("--") {
        return Err(format!("missing value for {option}").into());
    }
    Ok(word)
}

// The stand-alone binaries share this module with the product CLI.
pub fn adapter(words: Vec<String>) -> Result<()> {
    if words == ["--help"] || words == ["-h"] {
        println!("{ADAPTER_HELP}");
        return Ok(());
    }
    if words == ["--version"] {
        println!(
            "wg-adapter {} (WonderFence MIT core, trap15 / kconger)",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    let mut words = words.into_iter();
    let (mut config_path, mut positional, mut tap_path, mut net) = (None, None, None, None);
    while let Some(word) = words.next() {
        match word.as_str() {
            "--config" if config_path.is_none() => {
                config_path = Some(value(&mut words, "--config")?)
            }
            "--tap" if tap_path.is_none() => tap_path = Some(value(&mut words, "--tap")?),
            "--net" if net.is_none() => net = Some(value(&mut words, "--net")?),
            s if !s.starts_with('-') && positional.is_none() => positional = Some(word),
            _ => return Err("unknown, duplicate or extra adapter argument".into()),
        }
    }
    if net.as_deref().is_some_and(|s| s != "tcp") {
        return Err("raw mode supports --net tcp".into());
    }
    if config_path.is_some() && positional.as_deref().is_some_and(|s| s != "ASDF") {
        return Err("use only one config path".into());
    }
    let config_path = config_path
        .or(positional.filter(|s| s != "ASDF"))
        .map(PathBuf::from)
        .or_else(|| env::var_os("WG_INI").map(PathBuf::from));
    let config = match config_path {
        Some(path) => ws_wondergate::raw::load_config(&path)?,
        None => Config::default(),
    };
    let tap: Box<dyn Write> = match tap_path {
        Some(path) => Box::new(OpenOptions::new().write(true).create_new(true).open(path)?),
        None => match env::var_os("WG_SERIAL_LOG") {
            Some(path) => Box::new(OpenOptions::new().append(true).create(true).open(path)?),
            None => Box::new(io::sink()),
        },
    };
    ws_wondergate::raw::serve_tcp(config, io::stdin().lock(), io::stdout().lock(), tap)?;
    Ok(())
}

fn command_name(kind: u8, cmd: u8) -> &'static str {
    match (kind, cmd) {
        (1, 0) => "check-pdc",
        (1, 1) => "unk-01:01",
        (1, 2) => "init/poweron",
        (1, 3) => "unk-01:03",
        (1, 4) => "unk-01:04",
        (1, 8) => "dial",
        (1, 9) => "unk-01:09",
        (1, 10) => "hangup",
        (1, 11) => "dial/hangup-reply",
        (1, 16) => "set-ppp",
        (1, 17) => "set-dns",
        (2, 1) => "get-status",
        (2, 2) => "get-status-reply",
        (3, 0) => "unk-03:00",
        (15, 255) => "deinit",
        (17, 1) => "sock-new",
        (17, 3) => "sock-connect",
        (17, 6) => "sock-poll",
        (17, 7) => "sock-close",
        (17, 8) => "gethost",
        (17, 13) => "sock-readline",
        (17, 14) => "sock-write",
        (17, 15) => "sock-read",
        _ => "?",
    }
}

pub fn decode(words: Vec<String>) -> Result<()> {
    if words == ["--help"] || words == ["-h"] {
        println!("{DECODE_HELP}");
        return Ok(());
    }
    let mut words = words.into_iter();
    let (mut path, mut header) = (None, None);
    while let Some(word) = words.next() {
        match word.as_str() {
            "--header" if header.is_none() => {
                header = Some(match value(&mut words, "--header")?.as_str() {
                    "len16be" => HeaderMode::Len16Be,
                    "zero-then-len8" => HeaderMode::ZeroThenLen8,
                    _ => return Err("unknown header mode".into()),
                })
            }
            s if !s.starts_with('-') && path.is_none() => path = Some(word),
            _ => return Err("unknown, duplicate or extra decode argument".into()),
        }
    }
    let path = path.ok_or("wg-decode requires a tap path")?;
    let streams = transcript::read(
        File::open(Path::new(&path))?,
        transcript::TranscriptLimits::default(),
    )?;
    let header = header.unwrap_or(HeaderMode::Len16Be);
    let limits = Limits {
        max_frame: header.max_length(),
        ..Limits::default()
    };
    // Validate both directions before emitting a partial listing.
    let input = transcript::frames(&streams.input, header, true, limits)?;
    let output = transcript::frames(&streams.output, header, true, limits)?;
    let mut out = io::stdout().lock();
    for (who, bytes, frames) in [
        ("EMU->ADAPTER", &streams.input, input),
        ("ADAPTER->EMU", &streams.output, output),
    ] {
        writeln!(out, "=== {who} ({} bytes) ===", bytes.len())?;
        for located in frames {
            let f = located.frame;
            let high = bytes[located.offset + 1];
            write!(
                out,
                "  +{} {:02X}:{:02X} {:14} zero={high:02X} params[{}]: ",
                located.offset,
                f.kind,
                f.cmd,
                command_name(f.kind, f.cmd),
                f.params.len()
            )?;
            if f.params.is_empty() {
                writeln!(out, "(none)")?;
                continue;
            }
            for (i, byte) in f.params.iter().enumerate() {
                if i != 0 {
                    write!(out, " ")?;
                }
                write!(out, "{byte:02x}")?;
            }
            write!(out, "  |")?;
            for &byte in &f.params {
                write!(
                    out,
                    "{}",
                    if (32..127).contains(&byte) {
                        char::from(byte)
                    } else {
                        '.'
                    }
                )?;
            }
            writeln!(out, "|")?;
        }
    }
    Ok(())
}
