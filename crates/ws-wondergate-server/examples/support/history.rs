// SPDX-License-Identifier: MIT OR Apache-2.0
//! Bounded, immutable oracle history for development replays only.
//! Fresh writes and tokens retain the production MemoryStore quotas.
use std::io;
use ws_wondergate_server::{
    store::{MemoryStore, Scope, Store, StoredSubmission, Submission},
    Timestamp,
};
pub const MAX_HISTORY: usize = 32768;
pub struct FixtureStore {
    history: Vec<StoredSubmission>,
    live: MemoryStore,
}
impl FixtureStore {
    pub fn new(rows: Vec<Submission>) -> io::Result<Self> {
        if rows.len() > MAX_HISTORY {
            return Err(io::Error::other("oversized fixture history"));
        }
        let mut history = Vec::with_capacity(rows.len());
        for (index, row) in rows.into_iter().enumerate() {
            // Reuse the owner's validation without charging an old oracle row
            // against the current runtime's mutation quotas.
            MemoryStore::new().submit(row.clone())?;
            history.push(StoredSubmission {
                id: (index + 1) as u64,
                provenance: "new".into(),
                submission: row,
            });
        }
        Ok(Self {
            history,
            live: MemoryStore::new(),
        })
    }
    fn offset(&self, id: u64) -> io::Result<u64> {
        id.checked_add(self.history.len() as u64)
            .ok_or_else(|| io::Error::other("fixture sequence exhausted"))
    }
}
impl Store for FixtureStore {
    fn submit(&self, row: Submission) -> io::Result<u64> {
        self.offset(self.live.submit(row)?)
    }
    fn submissions(&self, scope: &Scope) -> io::Result<Vec<StoredSubmission>> {
        let mut rows: Vec<_> = self
            .history
            .iter()
            .filter(|row| &row.submission.scope == scope)
            .cloned()
            .collect();
        for mut row in self.live.submissions(scope)? {
            row.id = self.offset(row.id)?;
            rows.push(row);
        }
        Ok(rows)
    }
    fn aid(&self, host: &str, sid: &str, candidate: &str, now: Timestamp) -> io::Result<String> {
        self.live.aid(host, sid, candidate, now)
    }
}
