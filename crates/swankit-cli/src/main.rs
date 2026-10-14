#![forbid(unsafe_code)]
mod args;
mod assets;
mod discover;
mod io;
mod link;
mod png;
mod png_read;
#[cfg(feature = "saves")]
mod save;
mod wg_server;
use args::Args;
use serde_json::{json, Value};
use std::{env, path::Path};
use swankit::{extract, gfx, patch, text, Toolkit};
use swankit_cli::wg;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MAX_FILE: usize = 64 * 1024 * 1024;
const HELP: &str = "swankit: WonderSwan format tools
  info ROM [--json]
  detect FILE... [--json]
  tiles FILE --format 2bpp|4bpp-planar|4bpp-packed --offset N --length N --output PNG
  tiles SNAPSHOT --snapshot [--color] [--profile hardware|java] [--line-ports FILE] --output PNG
  extract text ROM --def JSON_OR_ID --table NAME [--output JSON] [--json]
  extract font ROM --def JSON_OR_ID --font NAME --output PNG [--columns N]
  extract sound FILE --def JSON [--output JSON] [--json]
  extract codec FILE --def JSON --output FILE [--json]
  save decode DEF_OR_AUTO SAVE [--context JSON] [--json]
  save set DEF_OR_AUTO SAVE --field INDEX --value JSON --output SAVE [--context JSON]
  save fix DEF_OR_AUTO SAVE --output SAVE [--context JSON]
  patch apply PATCH BASE --output FILE [--overlap reject|last-write-wins] [--growth reject|zero-fill]
  patch create BASE TARGET --output IPS
  assets discover|extract|replace|verify ROM MAP ... (see swankit assets --help)
Options: --json, --force (replace output); integers accept decimal or 0x hexadecimal.
Experimental: interop --json [--server] (versioned JSON-lines requests on stdin).
  wg-server --data DIR --wire DIR [--bind IP] [--http-port N] [--bpl-port N]
            [--clock system|fixed:YYYYMMDDhhmmss] [--seed N] [--timeout SECONDS]
The server loads bundled shared services and validated title definitions.
--timeout sets the per-request read window in seconds (1..=60, default 5).
  wg-adapter [CONFIG.ini | ASDF] [--config CONFIG.ini] [--tap FILE] [--net tcp]
  wg-decode SERIAL.log [--header len16be|zero-then-len8]
The same commands are available as stand-alone wg-adapter and wg-decode binaries.
  link ENDPOINT_A ENDPOINT_B [--log FILE.tsv] [--latency-ms N] [--jitter-ms N --seed S] [--baud B]
Endpoints: tcp:HOST:PORT, listen:[HOST:]PORT (default 127.0.0.1), stdio,
           exec:COMMAND, device:PATH, wondergate[:CONFIG.ini].
Raw data bytes only; no speed or timing. --baud applies only to devices.
See swankit link --help and docs/link.md for details.
See docs/cli.md for definition schemas and bounds.";

fn emit(
    value: &Value,
    plain: &str,
    json_mode: bool,
    output: Option<&str>,
    force: bool,
) -> Result<()> {
    if let Some(path) = output {
        io::write(path, &serde_json::to_vec_pretty(value)?, force)?;
    } else if json_mode {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{plain}");
    }
    Ok(())
}
fn artifact(path: &str, bytes: &[u8], force: bool, json_mode: bool, extra: Value) -> Result<()> {
    io::write(path, bytes, force)?;
    if json_mode {
        println!(
            "{}",
            json!({"output":path,"bytes":bytes.len(),"details":extra})
        );
    } else {
        println!("Wrote {} bytes to {path}", bytes.len());
    }
    Ok(())
}
fn text_definition(id: &str) -> Result<text::TextDefinition> {
    if Path::new(id).is_file() {
        Ok(text::TextDefinition::from_json(&io::read(
            id,
            extract::MAX_DEFINITION_BYTES,
        )?)?)
    } else {
        Ok(text::builtin_definition(id)?)
    }
}
fn run(words: Vec<String>) -> Result<()> {
    if words.first().is_some_and(|s| s == "wg-adapter") {
        return wg::adapter(words.into_iter().skip(1).collect());
    }
    if words.first().is_some_and(|s| s == "assets") {
        if words.get(1).is_some_and(|s| s == "discover") {
            return discover::run(words);
        }
        return assets::run(words);
    }
    if words.first().is_some_and(|s| s == "wg-decode") {
        return wg::decode(words.into_iter().skip(1).collect());
    }
    if words.first().is_some_and(|s| s == "link")
        && words.len() == 2
        && matches!(words[1].as_str(), "--help" | "-h")
    {
        println!("{}", link::HELP);
        return Ok(());
    }
    if words.is_empty() || words == ["--help"] || words == ["help"] || words == ["-h"] {
        println!("{HELP}");
        return Ok(());
    }
    let mut a = Args::parse(words)?;
    let json_mode = a.flag("--json");
    let command = a
        .positional
        .first()
        .map(String::as_str)
        .unwrap_or("")
        .to_owned();
    match command.as_str() {
        "link" => {
            if json_mode {
                return Err("link does not accept --json".into());
            }
            link::run(&mut a)
        }
        "wg-server" => {
            if json_mode {
                return Err("wg-server does not accept --json".into());
            }
            wg_server::run(&mut a)
        }
        "interop" => {
            let server = a.flag("--server");
            a.finish(1)?;
            if !json_mode {
                return Err("interop requires --json".into());
            }
            swankit_ffi::protocol::serve(
                std::io::stdin().lock(),
                std::io::stdout().lock(),
                !server,
            )?;
            Ok(())
        }
        "info" => {
            a.finish(2)?;
            let info = Toolkit::new()?.info(&io::read(&a.positional[1], MAX_FILE)?)?;
            let plain = format!("{}: {} bytes, {}\nPublisher: {} ({:02x})\nGame ID: {:02x}, version: {}\nMapper: {}, save: {}\nChecksum: {:04x}, computed {:04x}, valid: {}\nOrientation: {}, bus: {} bit, ROM access: {} cycles\nDeclared ROM bytes: {:?}, matches file: {}\nReset: {:04x}:{:04x}",
                info.format, info.size_bytes, if info.color { "color" } else { "monochrome" }, info.publisher.unwrap_or("unknown"), info.publisher_id,
                info.game_id, info.version, info.mapper, info.save, info.checksum_footer, info.checksum_computed, info.checksum_ok,
                info.orientation, info.bus_width, info.rom_speed_cycles, info.declared_rom_bytes, info.declared_size_matches, info.reset_segment, info.reset_offset);
            emit(&serde_json::to_value(info)?, &plain, json_mode, None, false)
        }
        "detect" => {
            if a.positional.len() < 2 {
                return Err("detect requires at least one file".into());
            }
            #[cfg(feature = "saves")]
            let save_name = a.value("--name");
            #[cfg(feature = "saves")]
            let context = if let Some(path) = a.value("--context") {
                swankit::save::context_from_json(&io::read(&path, extract::MAX_DEFINITION_BYTES)?)?
            } else {
                swankit::saves::DecodeContext::default()
            };
            #[cfg(feature = "saves")]
            if save_name.is_some() && a.positional.len() != 2 {
                return Err("detect --name requires exactly one input".into());
            }
            a.finish(a.positional.len())?;
            let toolkit = Toolkit::new()?;
            let mut results = Vec::new();
            let mut lines = Vec::new();
            for file in &a.positional[1..] {
                let bytes = io::read(file, MAX_FILE)?;
                let extension = Path::new(file).extension().and_then(|s| s.to_str());
                #[cfg(feature = "saves")]
                let found = toolkit.detect_with_save_context(
                    &bytes,
                    extension,
                    save_name.as_deref().unwrap_or(file),
                    &context,
                )?;
                #[cfg(not(feature = "saves"))]
                let found = toolkit.detect(&bytes, extension);
                lines.push(format!(
                    "{file}: {}{}",
                    found
                        .selected
                        .as_deref()
                        .unwrap_or(if found.candidates.is_empty() {
                            "unknown"
                        } else {
                            "ambiguous"
                        }),
                    if found.confidence.as_deref() == Some("structural") {
                        " (structural size candidate)"
                    } else {
                        ""
                    }
                ));
                #[cfg(feature = "saves")]
                if !found.save_definitions.is_empty() {
                    lines.push(format!(
                        "  Save definitions{}: {}",
                        if found.save_definitions.len() > 1 {
                            " (ambiguous)"
                        } else {
                            ""
                        },
                        found
                            .save_definitions
                            .iter()
                            .map(|c| c.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                results.push(json!({"file":file,"bytes":bytes.len(),"detection":found}));
            }
            emit(&json!(results), &lines.join("\n"), json_mode, None, false)
        }
        "tiles" => {
            let output = a.require("--output")?;
            let force = a.flag("--force");
            let snapshot = a.flag("--snapshot");
            let image = if snapshot {
                let color = a.flag("--color");
                let profile = match a.value("--profile").as_deref().unwrap_or("hardware") {
                    "hardware" => gfx::RenderProfile::Hardware,
                    "java" => gfx::RenderProfile::JavaDiagnostic,
                    _ => return Err("invalid render profile".into()),
                };
                let line_ports = a.value("--line-ports");
                a.finish(2)?;
                let bytes = io::read(&a.positional[1], 65536 + 256)?;
                if let Some(path) = line_ports {
                    let ports = io::read(&path, 144 * 256)?;
                    if ports.len() != 144 * 256 {
                        return Err("line ports must contain 144 rows of 256 bytes".into());
                    }
                    let rows: Vec<[u8; 256]> = ports
                        .chunks_exact(256)
                        .map(|row| row.try_into().unwrap())
                        .collect();
                    Toolkit::new()?.snapshot_with_line_ports(&bytes, color, profile, &rows)?
                } else {
                    Toolkit::new()?.snapshot(&bytes, color, profile)?
                }
            } else {
                let format = match a.require("--format")?.as_str() {
                    "2bpp" => gfx::TileFormat::Bpp2,
                    "4bpp-planar" => gfx::TileFormat::Bpp4Planar,
                    "4bpp-packed" => gfx::TileFormat::Bpp4Packed,
                    _ => return Err("invalid tile format".into()),
                };
                let offset = a.number("--offset", 0)?;
                let length = a.require("--length").and_then(|s| args::number(&s))?;
                let columns = u32::try_from(a.number("--columns", 16)?)?;
                let scale = u32::try_from(a.number("--scale", 1)?)?;
                a.finish(2)?;
                let bytes = io::read(&a.positional[1], MAX_FILE)?;
                Toolkit::new()?.tiles(
                    formatkit_core::bytes_at(&bytes, offset, length)?,
                    format,
                    &extract::grayscale(format.colors())?,
                    gfx::AtlasLayout {
                        arrangement: gfx::Arrangement::PLAIN,
                        columns,
                        scale,
                        max_bytes: extract::MAX_IMAGE_BYTES,
                    },
                )?
            };
            artifact(
                &output,
                &png::encode(&image)?,
                force,
                json_mode,
                json!({"width":image.width,"height":image.height}),
            )
        }
        "extract" => {
            let kind = a
                .positional
                .get(1)
                .cloned()
                .ok_or("extract requires text, font, sound or codec")?;
            let def = a.require("--def")?;
            let output = a.value("--output");
            let force = a.flag("--force");
            match kind.as_str() {
                "text" => {
                    let table = a.require("--table")?;
                    a.finish(3)?;
                    let value = Toolkit::new()?.extract_text(
                        &io::read(&a.positional[2], MAX_FILE)?,
                        &text_definition(&def)?,
                        &table,
                    )?;
                    let plain = value["strings"]
                        .as_array()
                        .ok_or("invalid text result")?
                        .iter()
                        .map(|s| {
                            format!(
                                "{:08x}\t{}",
                                s["offset"].as_u64().unwrap_or(0),
                                s["text"].as_str().unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    emit(&value, &plain, json_mode, output.as_deref(), force)
                }
                "font" => {
                    let font = a.require("--font")?;
                    let columns = u32::try_from(a.number("--columns", 16)?)?;
                    a.finish(3)?;
                    let output = output.ok_or("font extraction requires --output")?;
                    let image = Toolkit::new()?.extract_font(
                        &io::read(&a.positional[2], MAX_FILE)?,
                        &text_definition(&def)?,
                        &font,
                        columns,
                    )?;
                    artifact(
                        &output,
                        &png::encode(&image)?,
                        force,
                        json_mode,
                        json!({"width":image.width,"height":image.height}),
                    )
                }
                "codec" => {
                    a.finish(3)?;
                    let output = output.ok_or("codec extraction requires --output")?;
                    let def: extract::CodecDefinition =
                        extract::definition(&io::read(&def, extract::MAX_DEFINITION_BYTES)?)?;
                    let decoded = Toolkit::new()?
                        .extract_codec(&io::read(&a.positional[2], MAX_FILE)?, &def)?;
                    artifact(
                        &output,
                        &decoded.bytes,
                        force,
                        json_mode,
                        json!({"consumed":decoded.consumed}),
                    )
                }
                "sound" => {
                    a.finish(3)?;
                    let def: extract::SoundDefinition =
                        extract::definition(&io::read(&def, extract::MAX_DEFINITION_BYTES)?)?;
                    let value = Toolkit::new()?
                        .extract_sound(&io::read(&a.positional[2], MAX_FILE)?, &def)?;
                    let plain = serde_json::to_string_pretty(&value)?;
                    emit(&value, &plain, json_mode, output.as_deref(), force)
                }
                _ => Err("unknown extraction kind".into()),
            }
        }
        "patch" => {
            let action = a
                .positional
                .get(1)
                .cloned()
                .ok_or("patch requires apply or create")?;
            let output = a.require("--output")?;
            let force = a.flag("--force");
            let bytes = match action.as_str() {
                "apply" => {
                    let overlaps = match a.value("--overlap").as_deref().unwrap_or("reject") {
                        "reject" => patch::OverlapPolicy::Reject,
                        "last-write-wins" => patch::OverlapPolicy::LastWriteWins,
                        _ => return Err("invalid overlap policy".into()),
                    };
                    let growth = match a.value("--growth").as_deref().unwrap_or("reject") {
                        "reject" => patch::GrowthPolicy::Reject,
                        "zero-fill" => patch::GrowthPolicy::ZeroFill,
                        _ => return Err("invalid growth policy".into()),
                    };
                    a.finish(4)?;
                    let ips = io::read(&a.positional[2], MAX_FILE)?;
                    let base = io::read(&a.positional[3], MAX_FILE)?;
                    Toolkit::new()?.apply_patch(
                        &ips,
                        &base,
                        patch::ApplyOptions {
                            overlaps,
                            growth,
                            ..patch::ApplyOptions::default()
                        },
                    )?
                }
                "create" => {
                    a.finish(4)?;
                    Toolkit::new()?.create_patch(
                        &io::read(&a.positional[2], MAX_FILE)?,
                        &io::read(&a.positional[3], MAX_FILE)?,
                    )?
                }
                _ => return Err("unknown patch action".into()),
            };
            artifact(&output, &bytes, force, json_mode, Value::Null)
        }
        "save" => {
            #[cfg(feature = "saves")]
            {
                save::run(a, json_mode)
            }
            #[cfg(not(feature = "saves"))]
            {
                Err("save commands require the saves feature".into())
            }
        }
        _ => Err("unknown command; see swankit --help".into()),
    }
}
fn main() {
    let words: Vec<_> = env::args_os().skip(1).collect();
    let json_mode = words.iter().any(|s| s == "--json");
    let parsed: Result<Vec<String>> = words
        .into_iter()
        .map(|s| {
            s.into_string()
                .map_err(|_| "arguments must be valid UTF-8".into())
        })
        .collect();
    if let Err(e) = parsed.and_then(run) {
        if json_mode {
            eprintln!("{}", json!({"error":e.to_string()}));
        } else {
            eprintln!("error: {e}");
        }
        std::process::exit(1);
    }
}
