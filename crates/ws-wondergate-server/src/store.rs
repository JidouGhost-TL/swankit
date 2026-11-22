// SPDX-License-Identifier: MIT
// Ported from the private WonderGate revival server, wgserver/store.py (MIT).
// Copyright (C) 2026 jidoughost. Journal persistence is a new implementation.
//! Empty histories, append-only records, serialized token creation and fsync.
use crate::Timestamp;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    sync::Mutex,
};
pub type Result<T> = io::Result<T>;
const MAX_RECORD: usize = 128 * 1024;
/// Refusal limits preserve durable history and prevent unbounded growth.
pub const MAX_TOKENS: usize = 1024;
pub const MAX_HOST_TOKENS: usize = 256;
pub const MAX_SUBMISSIONS: usize = 4096;
pub const MAX_BOARD_SUBMISSIONS: usize = 2048;
pub const MAX_IDENTITY_SUBMISSIONS: usize = 64;
pub const MAX_JOURNAL_BYTES: u64 = 16 * 1024 * 1024;
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub definition: String,
    pub board: String,
    pub course: String,
}
impl Scope {
    pub fn new(definition: &str, board: &str, course: &str) -> Self {
        Self {
            definition: definition.into(),
            board: board.into(),
            course: course.into(),
        }
    }
    fn validate(&self) -> Result<()> {
        if [&self.definition, &self.board, &self.course]
            .iter()
            .any(|value| {
                value.is_empty()
                    || value.len() > 128
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            })
        {
            return Err(invalid("invalid board scope"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    pub scope: Scope,
    pub identity: Vec<u8>,
    pub name: Vec<u8>,
    pub score: u64,
    pub extra: i64,
    pub submitted_at: Timestamp,
}
impl Submission {
    fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        if self.identity.len() > 512 || self.name.len() > 512 {
            return Err(invalid("oversized identity or name"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredSubmission {
    pub id: u64,
    pub provenance: String,
    pub submission: Submission,
}
/// Implementations serialize mutation; a returned token is durable and unique
/// for (host, sid). Retry candidates never replace the first issued token.
pub trait Store: Send + Sync {
    fn submit(&self, submission: Submission) -> Result<u64>;
    fn submissions(&self, scope: &Scope) -> Result<Vec<StoredSubmission>>;
    fn aid(&self, host: &str, sid: &str, candidate: &str, now: Timestamp) -> Result<String>;
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: String,
    id: u64,
    provenance: String,
    data: RecordData,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum RecordData {
    Submission {
        submission: Submission,
    },
    Token {
        host: String,
        sid: String,
        aid: String,
        created_at: Timestamp,
    },
}
#[derive(Default)]
struct Index {
    last_id: u64,
    submissions: BTreeMap<Scope, Vec<StoredSubmission>>,
    tokens: BTreeMap<(String, String), String>,
}
fn validate_token(host: &str, sid: &str, aid: &str) -> Result<()> {
    if host.is_empty()
        || host.len() > 253
        || !host.is_ascii()
        || sid.len() > 32768
        || aid.len() != 80
        || !aid
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(invalid("invalid token record"));
    }
    Ok(())
}
impl Index {
    fn record(&self, data: RecordData) -> Result<Record> {
        self.capacity(&data)?;
        Ok(Record {
            schema: "swankit-wondergate-journal/1".into(),
            id: self
                .last_id
                .checked_add(1)
                .ok_or_else(|| invalid("journal sequence exhausted"))?,
            provenance: "new".into(),
            data,
        })
    }
    fn capacity(&self, data: &RecordData) -> Result<()> {
        match data {
            RecordData::Token { host, .. } => {
                if self.tokens.len() >= MAX_TOKENS
                    || self.tokens.keys().filter(|(h, _)| h == host).count() >= MAX_HOST_TOKENS
                {
                    return Err(invalid("token capacity reached"));
                }
            }
            RecordData::Submission { submission } => {
                let rows = self.submissions.get(&submission.scope);
                if self.submissions.values().map(Vec::len).sum::<usize>() >= MAX_SUBMISSIONS
                    || rows.is_some_and(|rows| {
                        rows.len() >= MAX_BOARD_SUBMISSIONS
                            || rows
                                .iter()
                                .filter(|r| r.submission.identity == submission.identity)
                                .count()
                                >= MAX_IDENTITY_SUBMISSIONS
                    })
                {
                    return Err(invalid("submission capacity reached"));
                }
            }
        }
        Ok(())
    }
    fn validate(&self, record: &Record) -> Result<()> {
        if record.schema != "swankit-wondergate-journal/1"
            || record.provenance != "new"
            || Some(record.id) != self.last_id.checked_add(1)
        {
            return Err(invalid("invalid journal schema, provenance or sequence"));
        }
        self.capacity(&record.data)?;
        match &record.data {
            RecordData::Submission { submission } => submission.validate(),
            RecordData::Token { host, sid, aid, .. } => {
                validate_token(host, sid, aid)?;
                if self.tokens.contains_key(&(host.clone(), sid.clone())) {
                    return Err(invalid("duplicate journal token"));
                }
                Ok(())
            }
        }
    }
    fn apply(&mut self, record: Record) {
        self.last_id = record.id;
        match record.data {
            RecordData::Submission { submission } => self
                .submissions
                .entry(submission.scope.clone())
                .or_default()
                .push(StoredSubmission {
                    id: record.id,
                    provenance: record.provenance,
                    submission,
                }),
            RecordData::Token { host, sid, aid, .. } => {
                self.tokens.insert((host, sid), aid);
            }
        }
    }
}
#[derive(Default)]
pub struct MemoryStore {
    index: Mutex<Index>,
}
impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}
impl Store for MemoryStore {
    fn submit(&self, submission: Submission) -> Result<u64> {
        submission.validate()?;
        let mut index = self
            .index
            .lock()
            .map_err(|_| invalid("memory store lock poisoned"))?;
        let record = index.record(RecordData::Submission { submission })?;
        let id = record.id;
        index.apply(record);
        Ok(id)
    }
    fn submissions(&self, scope: &Scope) -> Result<Vec<StoredSubmission>> {
        scope.validate()?;
        let index = self
            .index
            .lock()
            .map_err(|_| invalid("memory store lock poisoned"))?;
        Ok(index.submissions.get(scope).cloned().unwrap_or_default())
    }
    fn aid(&self, host: &str, sid: &str, candidate: &str, now: Timestamp) -> Result<String> {
        validate_token(host, sid, candidate)?;
        let mut index = self
            .index
            .lock()
            .map_err(|_| invalid("memory store lock poisoned"))?;
        if let Some(aid) = index.tokens.get(&(host.into(), sid.into())) {
            return Ok(aid.clone());
        }
        let record = index.record(RecordData::Token {
            host: host.into(),
            sid: sid.into(),
            aid: candidate.into(),
            created_at: now,
        })?;
        index.apply(record);
        Ok(candidate.into())
    }
}
struct Journal {
    file: File,
    index: Index,
    failed: bool,
}
pub struct JournalStore {
    journal: Mutex<Journal>,
}
impl JournalStore {
    /// Takes an exclusive nonblocking file lock. Corrupt or incomplete tails
    /// return an error; opening never truncates or rewrites existing records.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::options()
            .read(true)
            .append(true)
            .create(true)
            .open(path)?;
        file.try_lock()?;
        if file.metadata()?.len() > MAX_JOURNAL_BYTES {
            return Err(invalid("journal byte capacity reached"));
        }
        let mut input = BufReader::new(&file);
        let mut index = Index::default();
        loop {
            let mut line = Vec::new();
            let size = input
                .by_ref()
                .take((MAX_RECORD + 1) as u64)
                .read_until(b'\n', &mut line)?;
            if size == 0 {
                break;
            }
            if size > MAX_RECORD || line.last() != Some(&b'\n') {
                return Err(invalid("oversized or incomplete journal record"));
            }
            let record: Record =
                serde_json::from_slice(&line).map_err(|e| invalid(e.to_string()))?;
            index.validate(&record)?;
            index.apply(record);
        }
        Ok(Self {
            journal: Mutex::new(Journal {
                file,
                index,
                failed: false,
            }),
        })
    }
}
impl Journal {
    fn append(&mut self, record: Record) -> Result<u64> {
        if self.failed {
            return Err(invalid("journal requires reopening after a write failure"));
        }
        self.index.validate(&record)?;
        let mut line = serde_json::to_vec(&record).map_err(|e| invalid(e.to_string()))?;
        line.push(b'\n');
        if self
            .file
            .metadata()?
            .len()
            .saturating_add(line.len() as u64)
            > MAX_JOURNAL_BYTES
        {
            return Err(invalid("journal byte capacity reached"));
        }
        if line.len() > MAX_RECORD {
            return Err(invalid("oversized journal record"));
        }
        if let Err(error) = self
            .file
            .write_all(&line)
            .and_then(|()| self.file.sync_all())
        {
            self.failed = true;
            return Err(error);
        }
        let id = record.id;
        self.index.apply(record);
        Ok(id)
    }
}
impl Store for JournalStore {
    fn submit(&self, submission: Submission) -> Result<u64> {
        let mut journal = self
            .journal
            .lock()
            .map_err(|_| invalid("journal lock poisoned"))?;
        let record = journal
            .index
            .record(RecordData::Submission { submission })?;
        journal.append(record)
    }
    fn submissions(&self, scope: &Scope) -> Result<Vec<StoredSubmission>> {
        scope.validate()?;
        let journal = self
            .journal
            .lock()
            .map_err(|_| invalid("journal lock poisoned"))?;
        Ok(journal
            .index
            .submissions
            .get(scope)
            .cloned()
            .unwrap_or_default())
    }
    fn aid(&self, host: &str, sid: &str, candidate: &str, now: Timestamp) -> Result<String> {
        validate_token(host, sid, candidate)?;
        let mut journal = self
            .journal
            .lock()
            .map_err(|_| invalid("journal lock poisoned"))?;
        if let Some(aid) = journal.index.tokens.get(&(host.into(), sid.into())) {
            return Ok(aid.clone());
        }
        let record = journal.index.record(RecordData::Token {
            host: host.into(),
            sid: sid.into(),
            aid: candidate.into(),
            created_at: now,
        })?;
        journal.append(record)?;
        Ok(candidate.into())
    }
}
