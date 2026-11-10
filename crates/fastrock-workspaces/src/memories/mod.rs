//! Small, independent memory records in the existing vwork database. No source scan.
use crate::embedded::Connection;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use vwork_engine::kv::TableDefinition;

mod recall;
pub use recall::recall;
#[cfg(test)]
mod tests;

const STATE: TableDefinition<&str, &[u8]> = TableDefinition::new("fastrock_memory_state_v1");
const FORGOTTEN: TableDefinition<&str, bool> = TableDefinition::new("fastrock_memory_forgotten_v1");
pub const MAX_FACTS: usize = 512;
pub const MAX_SCOPE: usize = 96;
pub const MAX_TEXT: usize = 384;
pub const MAX_RECALL: usize = 2048;
const MAX_STATE: usize = 2 * 1024 * 1024;
const LIFETIME: i64 = 180 * 86400;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Terminology,
    Preference,
    Convention,
    Correction,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub key: String,
    pub text: String,
    pub evidence: String,
    pub kind: Kind,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fact {
    pub id: String,
    pub scope: String,
    pub scope_name: String,
    pub key: String,
    pub text: String,
    pub kind: Kind,
    pub evidence: String,
    pub source: String,
    pub turn: String,
    pub updated: i64,
    pub pinned: bool,
    #[serde(default)]
    pub global: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub revision: u64,
    pub learning: bool,
    pub recall: bool,
    pub facts: Vec<Fact>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            learning: true,
            recall: true,
            facts: Vec::new(),
        }
    }
}
#[derive(Clone)]
pub struct Memories(pub Connection);

/// Secret-like material is rejected before extraction and before persistence.
/// This is intentionally conservative; semantic selection remains model-assisted.
pub fn sensitive(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "password",
        "api_key",
        "api key",
        "api-key",
        "secret",
        "private key",
        "bearer ",
        "access_token",
        "access token",
        "session_token",
        "credential",
        "sk-",
        "akia",
        "asia",
        "-----begin",
        "token=",
        "token:",
        "authorization:",
    ]
    .iter()
    .any(|p| lower.contains(p))
        || text.split_whitespace().any(|word| {
            word.len() >= 48
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-/+=.".contains(c))
        })
}
pub fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
pub fn identity(scope: &str, key: &str) -> String {
    format!(
        "{}",
        hex::encode(Sha256::digest(format!("{scope}\0{}", normalized(key))))
    )
}
fn valid_text(text: &str, max: usize) -> bool {
    !text.trim().is_empty()
        && text.len() <= max
        && !text.chars().any(char::is_control)
        && !sensitive(text)
}
pub fn valid_candidate(candidate: &Candidate, input: &str) -> bool {
    valid_text(&candidate.key, 64)
        && valid_text(&candidate.text, MAX_TEXT)
        && valid_text(&candidate.evidence, MAX_TEXT)
        && candidate.evidence.len() >= 8
        && input.contains(&candidate.evidence)
        && !sensitive(input)
}
fn decode(bytes: &[u8]) -> Result<Snapshot> {
    ensure!(bytes.len() <= MAX_STATE, "Memory state exceeds limit");
    let state: Snapshot = serde_json::from_slice(bytes)?;
    ensure!(
        state.facts.len() <= MAX_FACTS,
        "Memory collection exceeds limit"
    );
    ensure!(
        state
            .facts
            .iter()
            .all(|f| valid_text(&f.text, MAX_TEXT) && f.key.len() <= 64 && f.scope.len() <= 256),
        "Invalid memory record"
    );
    Ok(state)
}
fn maintain(state: &mut Snapshot, now: i64) {
    state
        .facts
        .retain(|f| f.pinned || now.saturating_sub(f.updated) < LIFETIME);
}
fn room(state: &mut Snapshot, scope: &str) -> bool {
    let full_scope = state.facts.iter().filter(|f| f.scope == scope).count() >= MAX_SCOPE;
    if !full_scope && state.facts.len() < MAX_FACTS {
        return true;
    }
    let oldest = state
        .facts
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.pinned && (!full_scope || f.scope == scope))
        .min_by_key(|(_, f)| (f.updated, f.id.as_str()))
        .map(|(i, _)| i);
    if let Some(i) = oldest {
        state.facts.remove(i);
        true
    } else {
        false
    }
}

#[derive(Clone)]
pub struct Learning {
    pub revision: u64,
    pub scope: String,
    pub scope_name: String,
    pub source: String,
    pub turn: String,
    pub input: String,
    pub candidates: Vec<Candidate>,
    pub now: i64,
}
#[derive(Clone)]
pub enum Edit {
    Settings {
        learning: bool,
        recall: bool,
    },
    Update {
        id: String,
        text: String,
        pinned: bool,
        global: bool,
    },
    Forget(String),
    Clear,
    Maintain(i64),
    Learn(Learning),
}
impl Memories {
    pub async fn load(&self) -> Result<Snapshot> {
        self.0
            .run(|store| {
                store.with_engine(|engine| {
                    let read = engine.db.begin_read()?;
                    let table = match read.open_table(STATE) {
                        Ok(table) => table,
                        Err(vwork_engine::kv::TableError::TableDoesNotExist(_)) => {
                            return Ok(Snapshot::default());
                        }
                        Err(error) => return Err(error.into()),
                    };
                    table
                        .get("current")?
                        .map(|v| decode(v.value()))
                        .transpose()
                        .map(|s| s.unwrap_or_default())
                })
            })
            .await
    }
    /// Cancellation is checked before commit too: dropping a
    /// spawn_blocking future alone cannot cancel a queued transaction. Learned
    /// facts use non-fsync commits; explicit user edits/forget remain durable.
    pub async fn edit(
        &self,
        expected: u64,
        edit: Edit,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Option<Snapshot>> {
        self.0.run(move |store| store.with_engine(|engine| {
            if cancelled.load(Ordering::Acquire) { return Ok(None); }
            let mut write = engine.db.begin_write()?;
            if cancelled.load(Ordering::Acquire) { return Ok(None); }
            let background = matches!(&edit, Edit::Learn(_) | Edit::Maintain(_));
            if !background { write.set_durability(vwork_engine::kv::Durability::Immediate)?; }
            let mut state = {
                let table = write.open_table(STATE)?;
                table.get("current")?.map(|v| decode(v.value())).transpose()?.unwrap_or_default()
            };
            if state.revision != expected { return Ok(None); }
            let previous_facts = background.then(|| state.facts.clone());
            {
                let mut forgotten = write.open_table(FORGOTTEN)?;
                match edit {
                    Edit::Settings { learning, recall } => { state.learning = learning; state.recall = recall; }
                    Edit::Clear => {
                        for fact in &state.facts { forgotten.insert(fact.id.as_str(), true)?; forgotten.insert(identity(&fact.scope, &format!("text:{}", normalized(&fact.text))).as_str(), true)?; }
                        state.facts.clear();
                    }
                    Edit::Forget(id) => {
                        ensure!(state.facts.iter().any(|f| f.id == id), "Memory no longer exists; refresh");
                        forgotten.insert(id.as_str(), true)?;
                        if let Some(fact) = state.facts.iter().find(|f| f.id == id) { forgotten.insert(identity(&fact.scope, &format!("text:{}", normalized(&fact.text))).as_str(), true)?; }
                        state.facts.retain(|f| f.id != id);
                    }
                    Edit::Update { id, text, pinned, global } => {
                        ensure!(valid_text(&text, MAX_TEXT), "Memory must be 1–384 bytes without secrets or control characters");
                        let fact = state.facts.iter_mut().find(|f| f.id == id).ok_or_else(|| anyhow::anyhow!("Memory no longer exists; refresh"))?;
                        fact.text = text.trim().into(); fact.pinned = pinned; fact.global = global;
                        fact.updated = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64;
                        // User edits are authoritative; automatic updates never overwrite them.
                        fact.source = "manual".into(); fact.evidence.clear(); fact.turn.clear();
                    }
                    Edit::Maintain(now) => maintain(&mut state, now),
                    Edit::Learn(job) => {
                        if !state.learning || job.revision != expected { return Ok(None); }
                        ensure!(job.scope.len() <= 256 && job.scope_name.len() <= 256 && job.source.len() <= 256 && job.turn.len() <= 256, "Invalid memory scope/source");
                        maintain(&mut state, job.now);
                        for candidate in job.candidates.into_iter().take(5) {
                            if !valid_candidate(&candidate, &job.input) { continue; }
                            let key = normalized(&candidate.key);
                            let id = identity(&job.scope, &key);
                            if forgotten.get(id.as_str())?.is_some() || forgotten.get(identity(&job.scope, &format!("text:{}", normalized(&candidate.text))).as_str())?.is_some() { continue; }
                            if let Some(old) = state.facts.iter_mut().find(|f| f.id == id) {
                                if old.pinned || old.source == "manual" || old.updated > job.now { continue; }
                                old.text = candidate.text; old.evidence = candidate.evidence; old.updated = job.now;
                                old.source = job.source.clone(); old.turn = job.turn.clone(); old.kind = candidate.kind;
                                continue;
                            }
                            // Same statement under a newly invented key is not a new fact.
                            if state.facts.iter().any(|f| f.scope == job.scope && normalized(&f.text) == normalized(&candidate.text)) { continue; }
                            if !room(&mut state, &job.scope) { continue; }
                            state.facts.push(Fact { id, scope: job.scope.clone(), scope_name: job.scope_name.clone(), key,
                                text: candidate.text, evidence: candidate.evidence, kind: candidate.kind,
                                source: job.source.clone(), turn: job.turn.clone(), updated: job.now, pinned: false, global: false });
                        }
                    }
                }
            }
            if previous_facts.as_ref().is_some_and(|old| old == &state.facts) { return Ok(Some(state)); }
            state.revision = state.revision.checked_add(1).ok_or_else(|| anyhow::anyhow!("Memory revision exhausted"))?;
            let bytes = serde_json::to_vec(&state)?;
            ensure!(bytes.len() <= MAX_STATE, "Memory state exceeds limit");
            write.open_table(STATE)?.insert("current", bytes.as_slice())?;
            if cancelled.load(Ordering::Acquire) { return Ok(None); }
            write.commit()?;
            Ok(Some(state))
        })).await
    }
}
