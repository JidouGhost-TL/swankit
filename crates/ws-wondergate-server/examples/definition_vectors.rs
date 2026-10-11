// SPDX-License-Identifier: MIT OR Apache-2.0
//! External oracle driver: synthetic histories and exact HTTP bytes, one JSON line per case.
use serde::Deserialize;
use std::io::{self, BufRead, Write};
use ws_wondergate_server::{
    store::Submission, Definition, FixedClock, SeededRng, Server, Timestamp,
};
#[path = "support/history.rs"]
mod history;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    definitions: Vec<String>,
    submissions: Vec<Submission>,
    request: Vec<u8>,
    clock: String,
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut out = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let case: Case = serde_json::from_str(&line?)?;
        let definitions = case
            .definitions
            .iter()
            .map(|s| Definition::from_json(s))
            .collect::<Result<Vec<_>, _>>()?;
        let store = history::FixtureStore::new(case.submissions)?;
        let app = Server::from_definitions(
            definitions,
            Box::new(store),
            Box::new(FixedClock(Timestamp::parse_fixed(&case.clock)?)),
            Box::new(SeededRng::new(24)),
        )?;
        serde_json::to_writer(&mut out, &app.dispatch_http(&case.request))?;
        writeln!(out)?;
        out.flush()?;
    }
    Ok(())
}
