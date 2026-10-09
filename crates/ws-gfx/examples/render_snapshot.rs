//! Render a RAM+ports snapshot to raw RGBA for external comparison or PNG conversion.
use formatkit_core::Reader;
use std::{env, fs};
use ws_gfx::{RenderProfile, Snapshot};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 4 {
        return Err("usage: render_snapshot INPUT OUTPUT mono|color hardware|java".into());
    }
    let hardware = match args[2].to_str() {
        Some("mono") => false,
        Some("color") => true,
        _ => return Err("invalid hardware model".into()),
    };
    let profile = match args[3].to_str() {
        Some("hardware") => RenderProfile::Hardware,
        Some("java") => RenderProfile::JavaDiagnostic,
        _ => return Err("invalid render profile".into()),
    };
    let bytes = fs::read(&args[0])?;
    let mut r = Reader::new(&bytes);
    let ram = r.bytes(65536)?;
    let ports = r.bytes(256)?;
    if r.remaining() != 0 {
        return Err("trailing snapshot bytes".into());
    }
    let image = Snapshot::new(ram, ports, hardware)?.render(profile)?;
    fs::write(&args[1], image.rgba)?;
    Ok(())
}
