//! Dump a caller-selected reviewed leaf table; source paths remain external.
use formatkit_core::{Error, Result};
use std::{env, fs::File, io::Read};
use ws_text::{builtin_definition, TextLimits};

fn run() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() != 3 {
        return Err(Error::Malformed(
            "usage: dump_table DEFINITION INPUT TABLE".into(),
        ));
    }
    let io_error = |e: std::io::Error| Error::Malformed(e.to_string());
    let mut source = Vec::new();
    File::open(&args[1])
        .map_err(io_error)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut source)
        .map_err(io_error)?;
    if source.len() > 64 * 1024 * 1024 {
        return Err(Error::Malformed("input exceeds cartridge limit".into()));
    }
    let definition = builtin_definition(&args[0])?;
    let map = definition.character_map()?;
    let table = definition
        .tables
        .get(&args[2])
        .ok_or_else(|| Error::Malformed("unknown table".into()))?;
    for string in table.dump(&source, &map, TextLimits::default(), 65536)? {
        println!(
            "{:06x}\t{}\t{}",
            string.offset,
            string.decoded.consumed,
            serde_json::to_string(&string.decoded.text)
                .map_err(|e| Error::Malformed(e.to_string()))?
        );
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
