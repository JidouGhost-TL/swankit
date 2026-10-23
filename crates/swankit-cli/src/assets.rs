//! `swankit assets`: per-ROM asset maps (docs/asset-maps.md).
mod output;
mod search;
pub use search::run as search_command;

use crate::{args::Args, io, png, png_read, Result};
use serde_json::json;
use std::path::{Path, PathBuf};
use ws_assets::{
    extract, replace_with, verify, AssetMap, Charmaps, ExtractedFile, ReplaceInput, ReplaceOptions,
    Replacement,
};

pub const HELP: &str = "swankit assets: per-ROM asset maps (swankit-asset-map/1)
  assets search ROM [--config JSON] [--output JSON] [--json]
  assets search-corpus MANIFEST --output DIR [--config JSON] [--jobs N] [--resume] [--json]
  assets extract ROM MAP OUTDIR [--force] [--json]
  assets replace ROM MAP --id ID --from FILE -o PATCHED [--text-framing lf] [--ips PATCH] [--map-out MAP] [--allocate] [--force] [--json]
  assets verify ROM MAP [--json]
--from: .png (tiles/font sheet, tilemap or palette swatch as extracted), .txt (text in
the charmap's spelling) or any other file
(decoded payload bytes). Replacement re-encodes with the asset's codec, writes
in place when it fits, relocates into declared free space only when every
reference is listed, and always recomputes the final two checksum bytes (minimum
ROM size: 64 KiB). Text export appends one framing LF. Default .txt import removes
one optional final LF, then one optional CR (even without LF). --text-framing lf
is for .txt replacement only: remove one optional final LF, preserve content CR
and LF. Without a final LF, this mode preserves the whole string. Text that breaks
its declared line limits (line_width, lines) is refused; --force writes it and
reports warnings (and overwrites existing output files). --allocate gives cells of
a shared tile that were edited differently new tiles from declared free tile
space and rewrites those tilemap cells.";

const MAX_ROM: usize = 64 * 1024 * 1024;

fn load(rom: &str, map_path: &str) -> Result<(Vec<u8>, AssetMap, Charmaps)> {
    let rom = io::read(rom, MAX_ROM)?;
    let map = AssetMap::from_json(&io::read(map_path, ws_assets::map::MAX_MAP_BYTES)?)?;
    let base: PathBuf = Path::new(map_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let charmaps = Charmaps::load(&map, |name| {
        if Path::new(name).is_absolute() || name.split('/').any(|c| c == "..") {
            return Err(formatkit_core::Error::Malformed(format!(
                "charmap path {name:?} must be relative and stay beside the map"
            )));
        }
        let path = base.join(name);
        io::read(&path.to_string_lossy(), 8 * 1024 * 1024)
            .map_err(|e| formatkit_core::Error::Malformed(e.to_string()))
    })?;
    Ok((rom, map, charmaps))
}

fn input_paths(rom: &str, map_path: &str, map: &AssetMap) -> Vec<PathBuf> {
    let base = Path::new(map_path).parent().unwrap_or(Path::new("."));
    let mut inputs = vec![PathBuf::from(rom), PathBuf::from(map_path)];
    inputs.extend(
        map.charmaps
            .values()
            .filter_map(|source| source.file.as_ref())
            .map(|file| base.join(file)),
    );
    inputs
}

pub fn run(words: Vec<String>) -> Result<()> {
    if words.len() < 2 || matches!(words[1].as_str(), "--help" | "-h" | "help") {
        println!("{HELP}");
        return Ok(());
    }
    let words = words
        .into_iter()
        .map(|w| if w == "-o" { "--output".to_owned() } else { w })
        .collect();
    let mut a = Args::parse(words)?;
    let json_mode = a.flag("--json");
    let force = a.flag("--force");
    match a.positional[1].as_str() {
        "extract" => {
            a.finish(5)?;
            let (rom, map, charmaps) = load(&a.positional[2], &a.positional[3])?;
            let out = Path::new(&a.positional[4]);
            std::fs::create_dir_all(out).map_err(|e| format!("create {}: {e}", out.display()))?;
            let result = extract(&map, &charmaps, &rom)?;
            let mut files = Vec::new();
            for (name, file) in &result.files {
                let bytes = match file {
                    ExtractedFile::Image(image) => png::encode(image)?,
                    ExtractedFile::Bytes(bytes) => bytes.clone(),
                };
                files.push((out.join(name), bytes));
            }
            files.push((
                out.join("index.json"),
                serde_json::to_vec_pretty(&result.index)?,
            ));
            output::write(
                files,
                &input_paths(&a.positional[2], &a.positional[3], &map),
                force,
            )?;
            let summary = json!({"files": result.files.len(), "failures": result.failures});
            if json_mode {
                println!("{summary}");
            } else {
                println!(
                    "Extracted {} assets and {} screens into {} files ({} failures)",
                    map.assets.len(),
                    map.screens.len(),
                    result.files.len() + 1,
                    result.failures
                );
            }
            if result.failures > 0 {
                return Err(format!(
                    "{} assets or screens failed; see index.json",
                    result.failures
                )
                .into());
            }
            Ok(())
        }
        "replace" => {
            let id = a.require("--id")?;
            let from = a.require("--from")?;
            let output = a.require("--output")?;
            let ips = a.value("--ips");
            let map_out = a.value("--map-out");
            let allocate = a.flag("--allocate");
            let text_framing = a.value("--text-framing");
            a.finish(4)?;
            let extension = Path::new(&from).extension().and_then(|e| e.to_str());
            if let Some(framing) = text_framing.as_deref() {
                if framing != "lf" {
                    return Err("--text-framing must be lf".into());
                }
                if extension != Some("txt") {
                    return Err("--text-framing lf requires a .txt replacement file".into());
                }
            }
            let (rom, map, charmaps) = load(&a.positional[2], &a.positional[3])?;
            let raw = io::read(&from, MAX_ROM)?;
            let input = match extension {
                Some("png") => ReplaceInput::Image(png_read::decode(&raw)?),
                Some("txt") => {
                    let text = String::from_utf8(raw).map_err(|_| "text file is not UTF-8")?;
                    let text = text.strip_suffix('\n').unwrap_or(&text);
                    let text = if text_framing.is_some() {
                        text
                    } else {
                        text.strip_suffix('\r').unwrap_or(text)
                    };
                    ReplaceInput::Text(text.to_owned())
                }
                _ => ReplaceInput::Bytes(raw),
            };
            let options = ReplaceOptions { force, allocate };
            let outcome =
                replace_with(&map, &charmaps, &rom, &Replacement { id, input }, &options)?;
            let mut files = Vec::new();
            if let Some(path) = &ips {
                files.push((
                    PathBuf::from(path),
                    swankit::patch::create(&rom, &outcome.rom)?,
                ));
            }
            if let Some(path) = &map_out {
                files.push((PathBuf::from(path), outcome.map.to_json()?));
            }
            files.insert(0, (PathBuf::from(&output), outcome.rom));
            let mut inputs = input_paths(&a.positional[2], &a.positional[3], &map);
            inputs.push(PathBuf::from(&from));
            output::write(files, &inputs, force)?;
            let summary = json!({
                "output": output, "ips": ips, "map": map_out,
                "actions": outcome.actions, "approximated_pixels": outcome.approximated_pixels,
                "warnings": outcome.warnings,
                "sha256": outcome.map.rom.sha256,
            });
            if json_mode {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                for action in &outcome.actions {
                    println!("{action}");
                }
                for warning in &outcome.warnings {
                    println!("warning: {warning}");
                }
                if outcome.approximated_pixels > 0 {
                    println!(
                        "{} pixels used the nearest palette colour",
                        outcome.approximated_pixels
                    );
                }
                println!("Wrote {output}");
            }
            Ok(())
        }
        "verify" => {
            a.finish(4)?;
            let (rom, map, charmaps) = load(&a.positional[2], &a.positional[3])?;
            let report = verify(&map, &charmaps, &rom)?;
            if json_mode {
                let mut value = serde_json::to_value(&report)?;
                value["coverage"] = json!(report.coverage());
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                println!(
                    "{} assets, {} references; mapped {} of {} bytes ({:.2}%), {} free bytes declared",
                    report.assets,
                    report.references,
                    report.mapped_bytes,
                    report.rom_bytes,
                    report.coverage() * 100.0,
                    report.free_bytes
                );
                for (kind, (bytes, count)) in &report.by_kind {
                    println!("  {kind}: {count} assets, {bytes} stored bytes");
                }
                for (level, bytes) in &report.by_confidence {
                    println!("  confidence {level}: {bytes} bytes");
                }
                for f in &report.errors {
                    println!("ERROR {}: {}", f.id, f.message);
                }
            }
            if !report.ok() {
                return Err(format!("{} verification errors", report.errors.len()).into());
            }
            Ok(())
        }
        other => Err(format!("unknown assets action {other:?}; see swankit assets --help").into()),
    }
}
