// SPDX-License-Identifier: MIT
// Ported from the private WonderGate revival server,
// wgserver/plugins/wave2_history.py and wgserver/store.py (MIT).
// Copyright (C) 2026 jidoughost.
use crate::store::{Scope, Store, StoredSubmission};
use std::{collections::BTreeMap, io};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Ascending,
    Descending,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ties {
    /// Earliest submission by that identity, matching the original store.
    FirstSubmission,
    /// Earliest winning score submission, matching the later history helper.
    FirstBestSubmission,
}
/// Identity selects its best score; associated name/extra come from that score.
/// Equal best scores retain the first record; global ties have explicit policy.
pub struct Board<'a> {
    store: &'a dyn Store,
    scope: Scope,
    order: Order,
    ties: Ties,
}
impl<'a> Board<'a> {
    pub fn new(store: &'a dyn Store, scope: Scope, order: Order) -> Self {
        Self {
            store,
            scope,
            order,
            ties: Ties::FirstSubmission,
        }
    }
    pub fn with_ties(mut self, ties: Ties) -> Self {
        self.ties = ties;
        self
    }
    fn rows(&self) -> io::Result<Vec<StoredSubmission>> {
        let mut best: BTreeMap<Vec<u8>, StoredSubmission> = BTreeMap::new();
        let mut first: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        for row in self.store.submissions(&self.scope)? {
            first
                .entry(row.submission.identity.clone())
                .or_insert(row.id);
            let replace = best
                .get(&row.submission.identity)
                .is_none_or(|old| match self.order {
                    Order::Ascending => row.submission.score < old.submission.score,
                    Order::Descending => row.submission.score > old.submission.score,
                });
            if replace {
                best.insert(row.submission.identity.clone(), row);
            }
        }
        let mut rows: Vec<_> = best.into_values().collect();
        rows.sort_by(|a, b| {
            let score = a.submission.score.cmp(&b.submission.score);
            (if self.order == Order::Ascending {
                score
            } else {
                score.reverse()
            })
            .then_with(|| match self.ties {
                Ties::FirstSubmission => first
                    .get(&a.submission.identity)
                    .cmp(&first.get(&b.submission.identity)),
                Ties::FirstBestSubmission => a.id.cmp(&b.id),
            })
        });
        Ok(rows)
    }
    pub fn count(&self) -> io::Result<usize> {
        Ok(self.rows()?.len())
    }
    pub fn leaders(&self, limit: usize) -> io::Result<Vec<StoredSubmission>> {
        check_slots(limit)?;
        let mut rows = self.rows()?;
        rows.truncate(limit);
        Ok(rows)
    }
    /// One-based rank, zero for an absent identity. Empty identity is a player.
    pub fn position(&self, identity: &[u8]) -> io::Result<usize> {
        Ok(self
            .rows()?
            .iter()
            .position(|row| row.submission.identity == identity)
            .map_or(0, |i| i + 1))
    }
    pub fn best(&self, identity: &[u8]) -> io::Result<Option<StoredSubmission>> {
        Ok(self
            .rows()?
            .into_iter()
            .find(|row| row.submission.identity == identity))
    }
    pub fn slots(&self, count: usize) -> io::Result<Vec<Option<StoredSubmission>>> {
        let mut rows: Vec<_> = self.leaders(count)?.into_iter().map(Some).collect();
        rows.resize_with(count, || None);
        Ok(rows)
    }
}
fn check_slots(count: usize) -> io::Result<()> {
    if count > 4096 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "too many board slots",
        ));
    }
    Ok(())
}
