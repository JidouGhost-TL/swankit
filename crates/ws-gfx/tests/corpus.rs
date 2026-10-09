//! External Java oracle expectations; no corpus files or populations in this crate.
use formatkit_core::{Error, Image, Reader, Result};
use formatkit_corpus_test::{hash_bytes, run_dir, verify_versioned_samples, Args, Digest};
use serde_json::{json, Value};
use ws_gfx::{
    decode_tile, render_atlas, render_tile, Arrangement, AtlasLayout, ColorPalettes, MonoPalettes,
    RenderProfile, Snapshot, TileFormat,
};

fn format(args: &Args) -> Result<TileFormat> {
    match args.get("tile_format").and_then(Value::as_str) {
        Some("2bpp") => Ok(TileFormat::Bpp2),
        Some("4bpp-planar") => Ok(TileFormat::Bpp4Planar),
        Some("4bpp-packed") => Ok(TileFormat::Bpp4Packed),
        _ => Err(Error::Malformed("spec lacks tile format".into())),
    }
}
fn atlas(
    data: &[u8],
    fmt: TileFormat,
    palette: &[[u8; 4]],
    arrangement: Arrangement,
    columns: u32,
    scale: u32,
) -> Result<Value> {
    let image = render_atlas(
        data,
        fmt,
        palette,
        AtlasLayout {
            arrangement,
            columns,
            scale,
            max_bytes: 4 * 1024 * 1024,
        },
    )?;
    Ok(
        json!({"width": image.width, "height": image.height, "rgba_sha256": hash_bytes(&image.rgba)}),
    )
}

struct GraphicsDigest;

fn different_pixels(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

fn panel_response(image: &Image, lut: &[u8], color: bool) -> Result<Vec<u8>> {
    let mut out = Image::try_rgba_buffer(
        image.width,
        image.height,
        image.rgba.len(),
        "display response bytes",
    )?;
    let indices = image.rgba.chunks_exact(4).map(|rgba| {
        if color {
            (rgba[0] as usize / 17) * 256 + (rgba[1] as usize / 17) * 16 + rgba[2] as usize / 17
        } else {
            (255 - rgba[0] as usize) / 17
        }
    });
    formatkit_pixel::apply_palette(indices, &mut out, |index| -> Result<_> {
        let rgb = &lut[index * 3..index * 3 + 3];
        Ok([rgb[0], rgb[1], rgb[2], 255])
    })?;
    Ok(out)
}

struct DisplayDigest;
impl Digest for DisplayDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        let hardware = args
            .get("color_hardware")
            .and_then(Value::as_bool)
            .ok_or_else(|| Error::Malformed("spec lacks hardware model".into()))?;
        let mut reader = Reader::new(bytes);
        let ram = reader.bytes(65536)?;
        let ports = reader.bytes(256)?;
        let response = reader.bytes(if hardware { 4096 * 3 } else { 16 * 3 })?;
        let target = reader.bytes(224 * 144 * 4)?;
        if reader.remaining() != 0 {
            return Err(Error::Trailing(reader.remaining()));
        }
        let snapshot = Snapshot::new(ram, ports, hardware)?;
        let java = snapshot.render(RenderProfile::JavaDiagnostic)?;
        let documented = snapshot.render(RenderProfile::Hardware)?;
        let java_response = panel_response(&java, response, hardware)?;
        let hardware_response = panel_response(&documented, response, hardware)?;
        Ok(
            json!({"sha256": hash_bytes(bytes), "java_rgba_sha256": hash_bytes(&java.rgba),
            "hardware_rgba_sha256": hash_bytes(&documented.rgba), "java_response_sha256": hash_bytes(&java_response),
            "hardware_response_sha256": hash_bytes(&hardware_response),
            "java_raw_different_pixels": different_pixels(&java.rgba, target),
            "hardware_raw_different_pixels": different_pixels(&documented.rgba, target),
            "java_response_different_pixels": different_pixels(&java_response, target),
            "hardware_response_different_pixels": different_pixels(&hardware_response, target),
            "profile_different_pixels": different_pixels(&java.rgba, &documented.rgba)}),
        )
    }
}
impl Digest for GraphicsDigest {
    fn digest(&self, bytes: &[u8], args: &Args) -> Result<Value> {
        match args.get("operation").and_then(Value::as_str) {
            Some("tiles") => {
                let fmt = format(args)?;
                let indices = decode_tile(bytes, 0, fmt)?;
                let palette: Vec<_> = (0..fmt.colors() as u8)
                    .map(|i| [i * 17, 255 - i * 17, i * 17, 255])
                    .collect();
                let tile = render_tile(&indices, &palette)?;
                let atlases: Vec<_> = [
                    Arrangement::PLAIN,
                    Arrangement::TALL,
                    Arrangement::GLYPH_ROW,
                    Arrangement::GLYPH_COLUMN,
                ]
                .into_iter()
                .map(|a| atlas(bytes, fmt, &palette, a, 2, 2))
                .collect::<Result<_>>()?;
                Ok(
                    json!({"sha256": hash_bytes(bytes), "indices_sha256": hash_bytes(&indices), "tile_rgba_sha256": hash_bytes(&tile.rgba), "atlases": atlases}),
                )
            }
            Some("snapshot") => {
                let hardware = args
                    .get("color_hardware")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| Error::Malformed("spec lacks hardware model".into()))?;
                let mut reader = Reader::new(bytes);
                let ram = reader.bytes(65536)?;
                let ports = reader.bytes(256)?;
                if reader.remaining() != 0 {
                    return Err(Error::Trailing(reader.remaining()));
                }
                let snapshot = Snapshot::new(ram, ports, hardware)?;
                let screen = snapshot.render(RenderProfile::JavaDiagnostic)?;
                let layer0 = snapshot.render_layer(
                    ports[7] & 15,
                    RenderProfile::JavaDiagnostic,
                    [255, 0, 255, 255],
                )?;
                let layer1 = snapshot.render_layer(
                    ports[7] >> 4,
                    RenderProfile::JavaDiagnostic,
                    [255, 0, 255, 255],
                )?;
                let fmt = snapshot.tile_format();
                let (base, count) = if fmt == TileFormat::Bpp2 {
                    (0x2000, 512)
                } else {
                    (0x4000, 1024)
                };
                let data = &ram[base..base + count * fmt.bytes_per_tile()];
                let mut indices = Vec::with_capacity(count * 64);
                for tile in 0..count {
                    indices.extend_from_slice(&decode_tile(
                        data,
                        tile * fmt.bytes_per_tile(),
                        fmt,
                    )?);
                }
                let mono = MonoPalettes::from_ports(ports)?;
                let colors = ColorPalettes::parse(&ram[0xfe00..])?;
                let palette: &[[u8; 4]] = if hardware && ports[0x60] & 0x80 != 0 {
                    colors.palette(0)?
                } else {
                    mono.palette(0)?
                };
                Ok(
                    json!({"sha256": hash_bytes(bytes), "width": screen.width, "height": screen.height,
                    "screen_rgba_sha256": hash_bytes(&screen.rgba), "layer0_rgba_sha256": hash_bytes(&layer0.rgba),
                    "layer1_rgba_sha256": hash_bytes(&layer1.rgba), "indices_sha256": hash_bytes(&indices),
                    "atlas": atlas(data, fmt, palette, Arrangement::PLAIN, 32, 1)?}),
                )
            }
            _ => Err(Error::Malformed("unknown graphics corpus operation".into())),
        }
    }
}

#[test]
fn corpus_java_graphics() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen specs"
    );
    let inspected = run_dir("ws-gfx", &GraphicsDigest);
    let versioned = verify_versioned_samples("ws-gfx", "sha256", "ws-gfx", &["reference_sha256"]);
    assert_eq!(
        inspected, versioned,
        "every graphics sample must have oracle provenance"
    );
    if inspected > 0 {
        eprintln!("ws-gfx corpus: {inspected} snapshots/tile fixtures; every Java RGBA, decoded-index and atlas golden digest passed");
    }
}

#[test]
fn corpus_display_comparison() {
    assert_ne!(
        std::env::var("FORMATKIT_CORPUS_WRITE").as_deref(),
        Ok("1"),
        "never regenerate frozen specs"
    );
    let inspected = run_dir("ws-gfx-display", &DisplayDigest);
    let versioned = verify_versioned_samples(
        "ws-gfx-display",
        "sha256",
        "ws-gfx-display",
        &["reference_sha256"],
    );
    assert_eq!(
        inspected, versioned,
        "every screenshot comparison must have independent provenance"
    );
    if inspected > 0 {
        eprintln!("ws-gfx display corpus: {inspected} same-state screenshot comparisons; hardware Java oracle and independent measured panel-response digests/counts passed");
    }
}
