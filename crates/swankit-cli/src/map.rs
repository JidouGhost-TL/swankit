//! `swankit map`: universal declarative asset-map verification and Markdown reporting.
use crate::{args::Args, io, Result};
use serde_json::json;
use std::path::{Path, PathBuf};
use ws_assets::{Charmaps, LoadedAssetMap};

pub const HELP: &str =
    "swankit map: declarative asset maps (formatkit.asset-map.v1 / swankit-asset-map/1)
  map verify ROM MAP [--json]
  map markdown MAP
Accepts both formatkit.asset-map.v1 and swankit-asset-map/1 JSON documents.";

const MAX_ROM: usize = 64 * 1024 * 1024;

fn load_charmaps(map_path: &str, map: &ws_assets::AssetMap) -> Result<Charmaps> {
    let base: PathBuf = Path::new(map_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let charmaps = Charmaps::load(map, |name| {
        if Path::new(name).is_absolute() || name.split('/').any(|c| c == "..") {
            return Err(formatkit_core::Error::Malformed(format!(
                "charmap path {name:?} must be relative and stay beside the map"
            )));
        }
        let path = base.join(name);
        io::read(&path.to_string_lossy(), 8 * 1024 * 1024)
            .map_err(|e| formatkit_core::Error::Malformed(e.to_string()))
    })?;
    Ok(charmaps)
}

pub fn run(words: Vec<String>) -> Result<()> {
    if words.len() < 2 || matches!(words[1].as_str(), "--help" | "-h" | "help") {
        println!("{HELP}");
        return Ok(());
    }
    let mut a = Args::parse(words)?;
    let json_mode = a.flag("--json");
    match a.positional[1].as_str() {
        "verify" => {
            a.finish(4)?;
            let rom = io::read(&a.positional[2], MAX_ROM)?;
            let map_bytes = io::read(&a.positional[3], ws_assets::map::MAX_MAP_BYTES)?;
            let loaded = LoadedAssetMap::from_json(&map_bytes)?;
            let asset_map = loaded.to_asset_map()?;
            let charmaps = load_charmaps(&a.positional[3], &asset_map)?;
            let report = loaded.verify(&charmaps, &rom)?;
            if json_mode {
                let mut value = serde_json::to_value(&report)?;
                value["schema"] = json!(loaded.schema());
                value["ok"] = json!(report.ok());
                value["coverage"] = json!(report.coverage());
                value["coverage_pct"] = json!(report.coverage() * 100.0);
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                println!(
                    "{} ({}): {} assets, {} references; mapped {} of {} bytes ({:.2}%), {} free bytes declared",
                    a.positional[3],
                    loaded.schema(),
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
                for f in &report.errors {
                    println!("ERROR {}: {}", f.id, f.message);
                }
            }
            if !report.ok() {
                return Err(format!("{} verification errors", report.errors.len()).into());
            }
            Ok(())
        }
        "markdown" => {
            if json_mode {
                return Err("map markdown does not accept --json".into());
            }
            a.finish(3)?;
            let map_bytes = io::read(&a.positional[2], ws_assets::map::MAX_MAP_BYTES)?;
            let loaded = LoadedAssetMap::from_json(&map_bytes)?;
            print!("{}", loaded.render_markdown());
            Ok(())
        }
        other => Err(format!("unknown map action {other:?}; see swankit map --help").into()),
    }
}
