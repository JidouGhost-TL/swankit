//! Discovery CLI edge; evidence collection stays in external adapters.
use crate::{args::Args, io, Result};
pub fn run(words: Vec<String>) -> Result<()> {
    if words.iter().any(|w| w == "--help" || w == "-h") {
        println!("swankit assets discover ROM EVIDENCE -o MAP [--report REPORT] [--force] [--json]\nEvidence schema: swankit-discovery-evidence/1; collect snapshots and explicit external profiles with tools/discover-assets.py. Maps are verified drafts; references remain incomplete.");
        return Ok(());
    }
    let words = words
        .into_iter()
        .map(|w| if w == "-o" { "--output".into() } else { w })
        .collect();
    let mut a = Args::parse(words)?;
    let output = a.require("--output")?;
    let report = a.value("--report");
    let force = a.flag("--force");
    let json = a.flag("--json");
    a.finish(4)?;
    let rom = io::read(&a.positional[2], 64 * 1024 * 1024)?;
    let evidence: ws_discover::Evidence =
        serde_json::from_slice(&io::read(&a.positional[3], 64 * 1024 * 1024)?)?;
    let found = ws_discover::discover(&rom, &evidence)?;
    io::write(&output, &serde_json::to_vec_pretty(&found.map)?, force)?;
    if let Some(p) = report {
        io::write(&p, &serde_json::to_vec_pretty(&found.report)?, force)?;
    }
    if json {
        println!("{}", serde_json::to_string(&found.report)?);
    } else {
        println!(
            "Verified draft: {} mapped bytes; {} compressed payloads; {} text entries",
            found.report.mapped_bytes, found.report.accepted_compressed, found.report.text_entries
        );
    }
    Ok(())
}
