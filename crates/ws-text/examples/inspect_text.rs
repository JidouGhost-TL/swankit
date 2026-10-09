//! Read a caller-selected text definition and write one raw RGBA glyph.
use formatkit_core::{Error, Result};
use std::{env, fs, io::Read};
use ws_text::{builtin_definition, FontFormat, TextLimits};

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
    let mut bytes = Vec::new();
    fs::File::open(&args[1])
        .map_err(io_error)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(Error::Malformed("input exceeds cartridge limit".into()));
    }
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
    let colors = match font.format {
        FontFormat::Bpp1 => 2,
        FontFormat::Bpp2 => 4,
        FontFormat::Bpp4Planar | FontFormat::Bpp4Packed => 16,
    };
    let palette: Vec<_> = (0..colors)
        .map(|v| {
            let grey = 255 - v * (255 / (colors - 1));
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
