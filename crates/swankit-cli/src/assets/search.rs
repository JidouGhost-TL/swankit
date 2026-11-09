//! Read-only format leads and bounded, resumable manifest batches.
use crate::{args::Args, io, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};
use ws_discover::retained::{self, Attachment, ProvenanceReference};
use ws_discover::search::{self, Options, SearchReport};

const HELP: &str = "swankit assets search ROM [--config JSON] [--retained-source JSON] [--provenance-output JSON] [--output JSON] [--json]
swankit assets search-corpus MANIFEST --output DIR [--config JSON] [--jobs N] [--resume] [--json]
Search results are leads, never verified asset maps. Configurations select bounded
decoder probes, layout heuristics and external reader/reference patterns.
Batch manifest schema: swankit-format-search-inputs/1; inputs contain id, path,
optional sha256, config and retained_source paths (relative to the manifest).
Each successful input is checkpointed independently; --resume verifies identities,
configuration, executable hash and report digest. Changed inputs/configurations
need a new output directory. Failures are reported and make the command fail.
See docs/format-search.md for coverage limits, evidence and confirmation steps.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    inputs: Vec<Input>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    id: String,
    path: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    config: Option<String>,
    #[serde(default)]
    retained_source: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct FileReport {
    schema: String,
    executable_sha256: String,
    configuration_sha256: String,
    report_sha256: String,
    report: SearchReport,
    batch_manifest_sha256: Option<String>,
    batch_index: Option<usize>,
    batch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retained_source: Option<SourceBinding>,
}
#[derive(Debug, Serialize)]
struct BatchEntry {
    index: usize,
    id: String,
    source: String,
    report: Option<String>,
    rom_sha256: Option<String>,
    leads: usize,
    resumed: bool,
    complete_for_configured_scan: bool,
    error: Option<String>,
}
#[derive(Serialize)]
struct BatchReport {
    schema: &'static str,
    executable_sha256: String,
    manifest_sha256: String,
    default_configuration_sha256: String,
    inputs: usize,
    succeeded: usize,
    failed: usize,
    incomplete: usize,
    resumed: usize,
    leads: usize,
    entries: Vec<BatchEntry>,
}

fn configuration(path: Option<&str>) -> Result<Options> {
    let options: Options = match path {
        Some(path) => serde_json::from_slice(&io::read(path, 1024 * 1024)?)?,
        None => Options::default(),
    };
    options.validate()?;
    Ok(options)
}
fn digest<T: Serialize>(value: &T) -> Result<String> {
    Ok(ws_assets::sha256_hex(&serde_json::to_vec(value)?))
}
fn executable_digest() -> Result<String> {
    let exe = std::env::current_exe()?;
    Ok(ws_assets::sha256_hex(&io::read(
        exe.to_str().ok_or("non-UTF8 executable path")?,
        256 * 1024 * 1024,
    )?))
}
fn envelope(rom: &[u8], options: &Options, executable: &str) -> Result<FileReport> {
    let report = search::search(rom, options)?;
    Ok(FileReport {
        schema: "swankit-format-search-file/1".into(),
        executable_sha256: executable.into(),
        configuration_sha256: digest(options)?,
        report_sha256: digest(&report)?,
        report,
        batch_manifest_sha256: None,
        batch_index: None,
        batch_id: None,
        retained_source: None,
    })
}
fn resolve(base: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}
fn string(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| "non-UTF8 input/output path".into())
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SourceBinding {
    attachment_sha256: String,
    listing_sha256: String,
    sidecar: String,
    sidecar_sha256: String,
}
struct LoadedSource {
    attachment: Attachment,
    attachment_sha256: String,
    listing: Vec<u8>,
    protected: Vec<PathBuf>,
}
fn source_attachment(path: &Path) -> Result<LoadedSource> {
    let bytes = io::read(string(path)?, 1024 * 1024)?;
    let attachment: Attachment = serde_json::from_slice(&bytes)?;
    let listing_path = resolve(
        path.parent().unwrap_or(Path::new(".")),
        &attachment.listing.path,
    );
    let listing = io::read(string(&listing_path)?, retained::MAX_LISTING_BYTES)?;
    Ok(LoadedSource {
        attachment,
        attachment_sha256: ws_assets::sha256_hex(&bytes),
        listing,
        protected: vec![path.to_path_buf(), listing_path],
    })
}
fn attach_source(
    found: &mut FileReport,
    rom: &[u8],
    input: &LoadedSource,
    sidecar: &str,
) -> Result<Vec<u8>> {
    let provenance = retained::analyze(
        rom,
        &found.report,
        &input.attachment,
        &input.attachment_sha256,
        &input.listing,
    )?;
    let bytes = serde_json::to_vec_pretty(&provenance)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("caller provenance exceeds 64 MiB checkpoint bound".into());
    }
    let hash = ws_assets::sha256_hex(&bytes);
    for lead in &mut found.report.leads {
        if lead.family != search::Family::BankedQueueConstructor {
            continue;
        }
        let indices = provenance
            .constructors
            .iter()
            .enumerate()
            .filter(|(_, c)| c.lead_offset == lead.offset && c.lead_sha256 == lead.stored_sha256)
            .map(|(n, _)| n)
            .collect::<Vec<_>>();
        if !indices.is_empty() {
            lead.source_provenance = Some(ProvenanceReference {
                sidecar: sidecar.into(),
                sha256: hash.clone(),
                constructor_indices: indices,
            });
        }
    }
    found.retained_source = Some(SourceBinding {
        attachment_sha256: input.attachment_sha256.clone(),
        listing_sha256: input.attachment.listing.sha256.to_ascii_lowercase(),
        sidecar: sidecar.into(),
        sidecar_sha256: hash,
    });
    found.report_sha256 = digest(&found.report)?;
    Ok(bytes)
}

struct BatchContext<'a> {
    base: &'a Path,
    output: &'a Path,
    manifest: &'a Path,
    config_path: Option<&'a str>,
    defaults: &'a Options,
    executable: &'a str,
    resume: bool,
    manifest_sha256: &'a str,
    previous_hashes: &'a BTreeMap<usize, String>,
}

fn one(index: usize, input: &Input, ctx: &BatchContext<'_>) -> Result<BatchEntry> {
    let source = resolve(ctx.base, &input.path);
    let rom = io::read(string(&source)?, 64 * 1024 * 1024)?;
    let hash = ws_assets::sha256_hex(&rom);
    if ctx
        .previous_hashes
        .get(&index)
        .is_some_and(|old| old != &hash)
    {
        return Err("input changed at an existing checkpoint index; use a new directory".into());
    }
    if let Some(expected) = &input.sha256 {
        if expected.to_ascii_lowercase() != hash {
            return Err(format!("input hash differs for {}", input.id).into());
        }
    }
    let per_input = input.config.as_ref().map(|p| resolve(ctx.base, p));
    let options = match &per_input {
        Some(path) => configuration(Some(string(path)?))?,
        None => ctx.defaults.clone(),
    };
    let relative = format!("leads/{index:05}-{hash}.json");
    let output = ctx.output.join(&relative);
    let mut protected = vec![source.clone(), ctx.manifest.to_path_buf()];
    if let Some(path) = ctx.config_path {
        protected.push(PathBuf::from(path));
    }
    if let Some(path) = per_input {
        protected.push(path);
    }
    let attached = input
        .retained_source
        .as_ref()
        .map(|p| source_attachment(&resolve(ctx.base, p)))
        .transpose()?;
    if let Some(source) = &attached {
        protected.extend(source.protected.iter().cloned());
    }
    let side_relative = format!("provenance/{index:05}-{hash}.json");
    let side_output = ctx.output.join(&side_relative);
    let mut resumed = false;
    let found = if output.try_exists()? {
        if !ctx.resume {
            return Err("checkpoint exists; use --resume or a new directory".into());
        }
        if std::fs::symlink_metadata(&output)?.file_type().is_symlink() {
            return Err("checkpoint must not be a symlink".into());
        }
        let found: FileReport =
            serde_json::from_slice(&io::read(string(&output)?, 64 * 1024 * 1024)?)?;
        if found.schema != "swankit-format-search-file/1"
            || found.executable_sha256 != ctx.executable
            || found.configuration_sha256 != digest(&options)?
            || found.report_sha256 != digest(&found.report)?
            || found.report.schema != search::SCHEMA
            || found.report.rom_sha256 != hash
            || found.report.rom_bytes != rom.len()
            || found.report.configuration_sha256 != digest(&options)?
            || found.batch_manifest_sha256.as_deref() != Some(ctx.manifest_sha256)
            || found.batch_index != Some(index)
            || found.batch_id.as_deref() != Some(input.id.as_str())
        {
            return Err(
                "checkpoint identity/configuration/executable/digest differs; use a new directory"
                    .into(),
            );
        }
        if let Some(attached) = &attached {
            if !side_output.is_file()
                || std::fs::symlink_metadata(&side_output)?
                    .file_type()
                    .is_symlink()
            {
                return Err("missing or symlinked caller provenance checkpoint".into());
            }
            let mut expected = found.clone();
            let side_bytes = attach_source(&mut expected, &rom, attached, &side_relative)?;
            if expected.retained_source != found.retained_source
                || expected.report_sha256 != found.report_sha256
                || io::read(string(&side_output)?, 64 * 1024 * 1024)? != side_bytes
            {
                return Err("retained source/provenance identity or recomputed facts differ; use a new directory".into());
            }
        } else if found.retained_source.is_some()
            || found
                .report
                .leads
                .iter()
                .any(|l| l.source_provenance.is_some())
        {
            return Err("unexpected retained provenance at unconfigured checkpoint".into());
        }
        resumed = true;
        found
    } else {
        let mut found = envelope(&rom, &options, ctx.executable)?;
        found.batch_manifest_sha256 = Some(ctx.manifest_sha256.into());
        found.batch_index = Some(index);
        found.batch_id = Some(input.id.clone());
        let mut files = Vec::new();
        if let Some(attached) = &attached {
            files.push((
                side_output,
                attach_source(&mut found, &rom, attached, &side_relative)?,
            ));
        }
        files.push((output, serde_json::to_vec_pretty(&found)?));
        super::output::write(files, &protected, false)?;
        found
    };
    Ok(BatchEntry {
        index,
        id: input.id.clone(),
        source: input.path.clone(),
        report: Some(relative),
        rom_sha256: Some(hash),
        leads: found.report.leads.len(),
        resumed,
        complete_for_configured_scan: found.report.complete_for_configured_scan,
        error: None,
    })
}

fn batch(mut args: Args) -> Result<()> {
    let output = PathBuf::from(args.require("--output")?);
    let config_path = args.value("--config");
    let jobs = args.number("--jobs", 4)?;
    let resume = args.flag("--resume");
    let json = args.flag("--json");
    args.finish(3)?;
    if !(1..=16).contains(&jobs) {
        return Err("--jobs must be 1..16".into());
    }
    let manifest_path = Path::new(&args.positional[2]);
    let manifest_bytes = io::read(string(manifest_path)?, 8 * 1024 * 1024)?;
    let manifest_sha256 = ws_assets::sha256_hex(&manifest_bytes);
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.schema != "swankit-format-search-inputs/1"
        || manifest.inputs.is_empty()
        || manifest.inputs.len() > 10_000
    {
        return Err("invalid or empty format-search manifest".into());
    }
    let mut ids = BTreeSet::new();
    for input in &manifest.inputs {
        if input.id.is_empty() || input.id.len() > 256 || !ids.insert(&input.id) {
            return Err("manifest input ids must be nonempty and unique".into());
        }
        if let Some(hash) = &input.sha256 {
            if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err("manifest SHA256 must contain 64 hexadecimal digits".into());
            }
        }
    }
    let defaults = configuration(config_path.as_deref())?;
    let executable = executable_digest()?;
    let summary = output.join("summary.json");
    if summary.try_exists()? && !resume {
        return Err("batch summary already exists; use --resume or a new directory".into());
    }
    if summary.try_exists()? {
        if std::fs::symlink_metadata(&summary)?
            .file_type()
            .is_symlink()
        {
            return Err("batch summary must not be a symlink".into());
        }
        let previous: serde_json::Value =
            serde_json::from_slice(&io::read(string(&summary)?, 64 * 1024 * 1024)?)?;
        if previous["schema"] != "swankit-format-search-batch/1"
            || previous["manifest_sha256"] != manifest_sha256
            || previous["executable_sha256"] != executable
            || previous["default_configuration_sha256"] != digest(&defaults)?
        {
            return Err(
                "batch manifest/configuration/executable changed; use a new directory".into(),
            );
        }
    }
    std::fs::create_dir_all(output.join("leads"))?;
    if manifest.inputs.iter().any(|i| i.retained_source.is_some()) {
        let provenance = output.join("provenance");
        if provenance.try_exists()?
            && (std::fs::symlink_metadata(&provenance)?
                .file_type()
                .is_symlink()
                || !provenance.is_dir())
        {
            return Err("provenance directory must be a regular directory".into());
        }
        std::fs::create_dir_all(provenance)?;
    }
    let mut previous_hashes = BTreeMap::new();
    for entry in std::fs::read_dir(output.join("leads"))? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().ok_or("non-UTF8 checkpoint filename")?;
        if !name.ends_with(".json") {
            continue;
        }
        let (index, hash) = name
            .strip_suffix(".json")
            .and_then(|s| s.split_once('-'))
            .ok_or("unexpected checkpoint filename")?;
        if index.len() != 5
            || !index.bytes().all(|c| c.is_ascii_digit())
            || hash.len() != 64
            || !hash.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("unexpected checkpoint filename".into());
        }
        if entry.file_type()?.is_symlink() || !entry.file_type()?.is_file() {
            return Err("checkpoint must be a regular file".into());
        }
        let index: usize = index.parse()?;
        if index >= manifest.inputs.len()
            || previous_hashes.insert(index, hash.to_owned()).is_some()
        {
            return Err("checkpoint indices differ or duplicate; use a new directory".into());
        }
    }
    let ctx = BatchContext {
        base: manifest_path.parent().unwrap_or(Path::new(".")),
        output: &output,
        manifest: manifest_path,
        config_path: config_path.as_deref(),
        defaults: &defaults,
        executable: &executable,
        resume,
        manifest_sha256: &manifest_sha256,
        previous_hashes: &previous_hashes,
    };
    let next = AtomicUsize::new(0);
    let entries = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(manifest.inputs.len()) {
            let ctx = &ctx;
            let next = &next;
            let entries = &entries;
            let inputs = &manifest.inputs;
            scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(input) = inputs.get(index) else {
                    break;
                };
                let entry = one(index, input, ctx).unwrap_or_else(|error| BatchEntry {
                    index,
                    id: input.id.clone(),
                    source: input.path.clone(),
                    report: None,
                    rom_sha256: None,
                    leads: 0,
                    resumed: false,
                    complete_for_configured_scan: false,
                    error: Some(error.to_string()),
                });
                let mut completed = entries.lock().expect("worker result lock");
                eprintln!(
                    "format search {}/{}: {} {}",
                    completed.len() + 1,
                    inputs.len(),
                    input.id,
                    if entry.error.is_none() {
                        "done"
                    } else {
                        "FAILED"
                    }
                );
                completed.push(entry);
            });
        }
    });
    let mut entries = entries
        .into_inner()
        .map_err(|_| "worker result lock failed")?;
    entries.sort_by_key(|e| e.index);
    let failed = entries.iter().filter(|e| e.error.is_some()).count();
    let report = BatchReport {
        schema: "swankit-format-search-batch/1",
        executable_sha256: executable.clone(),
        manifest_sha256: manifest_sha256.clone(),
        default_configuration_sha256: digest(&defaults)?,
        inputs: manifest.inputs.len(),
        succeeded: entries.len() - failed,
        failed,
        incomplete: entries
            .iter()
            .filter(|e| e.error.is_none() && !e.complete_for_configured_scan)
            .count(),
        resumed: entries.iter().filter(|e| e.resumed).count(),
        leads: entries.iter().map(|e| e.leads).sum(),
        entries,
    };
    let mut protected = vec![manifest_path.to_path_buf()];
    if let Some(path) = &config_path {
        protected.push(PathBuf::from(path));
    }
    for input in &manifest.inputs {
        let path = resolve(ctx.base, &input.path);
        if path.try_exists()? {
            protected.push(path);
        }
        if let Some(config) = &input.config {
            let path = resolve(ctx.base, config);
            if path.try_exists()? {
                protected.push(path);
            }
        }
        if let Some(source) = &input.retained_source {
            let path = resolve(ctx.base, source);
            if path.try_exists()? {
                protected.push(path.clone());
                // Protect referenced files without rereading a failed listing; the
                // per-input error remains explicit in the batch summary.
                let attachment: Attachment =
                    serde_json::from_slice(&io::read(string(&path)?, 1024 * 1024)?)?;
                let listing = resolve(
                    path.parent().unwrap_or(Path::new(".")),
                    &attachment.listing.path,
                );
                if listing.try_exists()? {
                    protected.push(listing);
                }
            }
        }
    }
    super::output::write(
        vec![(summary, serde_json::to_vec_pretty(&report)?)],
        &protected,
        resume,
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "{} succeeded, {} failed, {} incomplete; {} leads; {} resumed",
            report.succeeded, report.failed, report.incomplete, report.leads, report.resumed
        );
    }
    if report.failed != 0 {
        return Err("one or more format searches failed; inspect summary.json".into());
    }
    Ok(())
}

pub fn run(words: Vec<String>) -> Result<()> {
    if words.last().is_some_and(|s| s == "--help" || s == "-h") {
        println!("{HELP}");
        return Ok(());
    }
    let mut args = Args::parse(words)?;
    if args.positional.get(1).is_some_and(|s| s == "search-corpus") {
        return batch(args);
    }
    let config = args.value("--config");
    let output = args.value("--output");
    let retained_path = args.value("--retained-source");
    let provenance_output = args.value("--provenance-output");
    let json = args.flag("--json");
    args.finish(3)?;
    let options = configuration(config.as_deref())?;
    let source = &args.positional[2];
    let bytes = io::read(source, 64 * 1024 * 1024)?;
    let mut found = envelope(&bytes, &options, &executable_digest()?)?;
    let attached = retained_path
        .as_ref()
        .map(|p| source_attachment(Path::new(p)))
        .transpose()?;
    if attached.is_none() && provenance_output.is_some() {
        return Err("--provenance-output requires --retained-source".into());
    }
    let mut files = Vec::new();
    let mut protected = vec![PathBuf::from(source)];
    if let Some(attached) = &attached {
        protected.extend(attached.protected.iter().cloned());
        let side_path = match (&provenance_output, &output) {
            (Some(path), _) => PathBuf::from(path),
            (None, Some(path)) => PathBuf::from(path).with_extension("callers.json"),
            _ => return Err("--retained-source requires --output or --provenance-output".into()),
        };
        let locator = if provenance_output.is_some() {
            string(&side_path)?.to_owned()
        } else {
            side_path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("invalid sidecar path")?
                .to_owned()
        };
        files.push((
            side_path,
            attach_source(&mut found, &bytes, attached, &locator)?,
        ));
    }
    if let Some(path) = &config {
        protected.push(PathBuf::from(path));
    }
    if let Some(path) = output {
        files.push((PathBuf::from(path), serde_json::to_vec_pretty(&found)?));
    }
    if !files.is_empty() {
        super::output::write(files, &protected, false)?;
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&found)?);
    } else {
        println!(
            "{} leads; {} byte positions; {} decoder probes; configured scan complete: {}",
            found.report.leads.len(),
            found.report.positions_scanned,
            found.report.decoder_probes,
            found.report.complete_for_configured_scan
        );
    }
    Ok(())
}
