//! Read a caller-selected text definition and write one raw RGBA glyph.
use formatkit_core::{Error, Result};
use std::{env, fs};
use ws_text::{builtin_definition, TextLimits};

fn run() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() != 7 {
        return Err(Error::Malformed(
            "usage: inspect_text DEFINITION INPUT OFFSET_HEX BYTES FONT GLYPH_HEX RGBA_OUTPUT"
                .into(),
        ));
    }
    let number = |s: &str, radix| {
        usize::from_str_radix(s, radix).map_err(|e| Error::Malformed(e.to_string()))
    };
    let io_error = |e: std::io::Error| Error::Malformed(e.to_string());
    let size = fs::metadata(&args[1]).map_err(io_error)?.len();
    if size > 64 * 1024 * 1024 {
        return Err(Error::Malformed("input exceeds cartridge limit".into()));
    }
    let bytes = fs::read(&args[1]).map_err(io_error)?;
    let definition = builtin_definition(&args[0])?;
    let map = definition.character_map()?;
    let input = formatkit_core::bytes_at(&bytes, number(&args[2], 16)?, number(&args[3], 10)?)?;
    let decoded = map.decode(input, TextLimits::default())?;
    println!(
        "{} bytes, terminated={}: {}",
        decoded.consumed, decoded.terminated, decoded.text
    );
    println!("normalized: {}", decoded.normalized);
    let font = definition
        .fonts
        .get(&args[4])
        .ok_or_else(|| Error::Malformed("unknown font".into()))?;
    let palette: Vec<_> = (0..16)
        .map(|v| {
            let grey = 255 - v * 17;
            [grey, grey, grey, 255]
        })
        .collect();
    let glyph = font.glyph(&bytes, number(&args[5], 16)?, &palette, 65536)?;
    fs::write(&args[6], &glyph.image.rgba).map_err(io_error)?;
    println!(
        "glyph: {}x{}, advance {}",
        glyph.image.width, glyph.image.height, glyph.advance
    );
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
