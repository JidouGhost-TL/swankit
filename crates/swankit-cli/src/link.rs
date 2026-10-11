use crate::{args::Args, Result};
use std::{fs::OpenOptions, time::Duration};
use ws_link::{Endpoint, Options, Relay};

pub const HELP: &str = "swankit link ENDPOINT_A ENDPOINT_B [--log FILE.tsv]
  [--latency-ms N] [--jitter-ms N --seed S] [--baud B]
tcp:HOST:PORT           Connect; retry every second, including after disconnect.
listen:[HOST:]PORT      Accept one peer at a time; default host is 127.0.0.1.
stdio                  Raw stdin/stdout; diagnostics go to stderr.
exec:COMMAND           Run /bin/sh -c COMMAND; terminate child group on exit.
device:PATH            Raw 8N1 serial device; baud defaults to 9600.
wondergate[:CONFIG.ini] Existing adapter core with its configured TCP backend.
--baud B               Device endpoints only: 9600, 38400, 192000.
--latency-ms N          Extra delay per direction (0..600000).
--jitter-ms N --seed S  Deterministic uniform +/- jitter, clamped at zero;
                       order preserved; independent sequence per direction.
The stream carries data bytes only, with no speed or timing; ends own pacing.
TSV: direction, time_us since start, two-digit hex value, result.
Connection events are # comments. A log must be a new file.
Real console hardware is untested; see docs/link.md for interoperability notes.";

pub fn run(a: &mut Args) -> Result<()> {
    let path = a.value("--log");
    let latency_ms = u64::try_from(a.number("--latency-ms", 0)?)?;
    let jitter_ms = u64::try_from(a.number("--jitter-ms", 0)?)?;
    let seed = match a.value("--seed") {
        Some(s) => s.parse::<u64>().map_err(|_| "invalid seed")?,
        None if jitter_ms != 0 => return Err("--jitter-ms requires --seed".into()),
        None => 0,
    };
    let baud = u32::try_from(a.number("--baud", 9600)?)?;
    a.finish(3)?;
    let endpoints = [
        a.positional[1].parse::<Endpoint>()?,
        a.positional[2].parse()?,
    ];
    let options = Options {
        latency: Duration::from_millis(latency_ms),
        jitter: Duration::from_millis(jitter_ms),
        seed,
        baud,
    };
    options.validate()?;
    let log: Box<dyn std::io::Write + Send> = match path {
        Some(p) => Box::new(OpenOptions::new().write(true).create_new(true).open(p)?),
        None => Box::new(std::io::sink()),
    };
    Relay::start_with_signals(endpoints, options, log)?;
    Ok(())
}
