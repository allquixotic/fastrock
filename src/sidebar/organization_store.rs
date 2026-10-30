//! Process-owned, serialized GUI organization. Never writes Goose history.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{mpsc, oneshot};

const FILE: &str = "fastrock-sidebar.json";
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
fn version() -> u32 {
    1
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Entry {
    pub pinned: bool,
    pub settled_at: Option<i64>,
    pub snoozed_until: Option<i64>,
    pub auto_settle_disabled: bool,
    pub last_woken_at: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Policy {
    pub group_projects: bool,
    pub working_section: bool,
    pub auto_settle_days: Option<u32>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            group_projects: true,
            working_section: true,
            auto_settle_days: Some(3),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct State {
    pub version: u32,
    pub revision: u64,
    pub entries: BTreeMap<String, Entry>,
    pub order: Vec<String>,
    pub collapsed: BTreeSet<String>,
    pub project_names: BTreeMap<String, String>,
    pub policy: Policy,
}
impl Default for State {
    fn default() -> Self {
        Self {
            version: version(),
            revision: 0,
            entries: BTreeMap::new(),
            order: vec![],
            collapsed: BTreeSet::new(),
            project_names: BTreeMap::new(),
            policy: Policy::default(),
        }
    }
}
impl State {
    pub fn section_at(
        &self,
        id: &str,
        working: bool,
        attention: bool,
        activity: i64,
        now: i64,
    ) -> &'static str {
        let base = self.section(id, working, now);
        if base == "active"
            && !working
            && !attention
            && self
                .auto_settle_at(id, activity)
                .is_some_and(|at| at <= now)
        {
            "settled"
        } else {
            base
        }
    }
    pub fn auto_settle_at(&self, id: &str, activity: i64) -> Option<i64> {
        let days = self.policy.auto_settle_days?;
        let entry = self.entries.get(id);
        if entry.is_some_and(|e| e.auto_settle_disabled || e.pinned || e.settled_at.is_some()) {
            return None;
        }
        let activity = activity
            .max(entry.and_then(|e| e.last_woken_at).unwrap_or(0))
            .max(entry.and_then(|e| e.snoozed_until).unwrap_or(0));
        (activity > 0).then(|| activity.saturating_add(i64::from(days) * 86400))
    }
    pub fn section(&self, id: &str, working: bool, now: i64) -> &'static str {
        if let Some(e) = self.entries.get(id) {
            if e.snoozed_until.is_some_and(|at| at > now) {
                return "snoozed";
            }
            if e.settled_at.is_some() {
                return "settled";
            }
            if e.pinned {
                return "pinned";
            }
        }
        if working && self.policy.working_section {
            "working"
        } else {
            "active"
        }
    }
    pub fn next_wake(&self, now: i64) -> Option<i64> {
        self.entries
            .values()
            .filter_map(|e| e.snoozed_until)
            .filter(|at| *at > now)
            .min()
    }
    fn apply(&mut self, mutation: Mutation, now: i64) -> Result<(), String> {
        match mutation {
            Mutation::Threads { ids, action } => {
                if ids.len() > 10_000
                    || ids
                        .iter()
                        .any(|id| id.len() > 512 || !id.starts_with("goose:"))
                {
                    return Err("Invalid conversation selection".into());
                }
                if let Action::Snooze(until) = action {
                    if until <= now || until > now.saturating_add(366 * 86400) {
                        return Err("Choose a snooze time within the next year".into());
                    }
                }
                for id in ids {
                    let e = self.entries.entry(id).or_default();
                    match action {
                        Action::Pin(value) => {
                            e.pinned = value;
                            e.last_woken_at = Some(now);
                            if value {
                                e.snoozed_until = None;
                                e.settled_at = None;
                            }
                        }
                        Action::Settle(value) => {
                            e.settled_at = value.then_some(now);
                            e.snoozed_until = None;
                            if !value {
                                e.last_woken_at = Some(now);
                            }
                        }
                        Action::Snooze(until) => {
                            e.snoozed_until = Some(until);
                            e.settled_at = None;
                        }
                        Action::Wake => {
                            e.snoozed_until = None;
                            e.settled_at = None;
                            e.last_woken_at = Some(now);
                        }
                        Action::AutoSettle(enabled) => e.auto_settle_disabled = !enabled,
                    }
                }
            }
            Mutation::Order(ids) => {
                if ids.len() > 10_000
                    || ids
                        .iter()
                        .any(|id| id.len() > 512 || !id.starts_with("goose:"))
                {
                    return Err("Invalid conversation order".into());
                }
                let mut seen = BTreeSet::new();
                self.order = ids
                    .into_iter()
                    .chain(std::mem::take(&mut self.order))
                    .filter(|id| seen.insert(id.clone()))
                    .collect();
            }
            Mutation::Place { id, section, order } => {
                if !["pinned", "active", "settled"].contains(&section.as_str())
                    || !id.starts_with("goose:")
                    || id.len() > 512
                    || order.len() > 10_000
                    || !order.contains(&id)
                    || order
                        .iter()
                        .any(|id| !id.starts_with("goose:") || id.len() > 512)
                {
                    return Err("Invalid sidebar drop".into());
                }
                let entry = self.entries.entry(id).or_default();
                entry.pinned = section == "pinned";
                entry.settled_at = (section == "settled").then_some(now);
                entry.snoozed_until = None;
                entry.last_woken_at = Some(now);
                let mut seen = BTreeSet::new();
                self.order = order
                    .into_iter()
                    .chain(std::mem::take(&mut self.order))
                    .filter(|id| seen.insert(id.clone()))
                    .collect();
            }
            Mutation::Policy { key, value } => match (key.as_str(), value) {
                ("group-projects", 0..=1) => self.policy.group_projects = value != 0,
                ("working-section", 0..=1) => self.policy.working_section = value != 0,
                ("auto-settle-days", 0..=365) => {
                    self.policy.auto_settle_days = (value > 0).then_some(value as u32)
                }
                _ => return Err("Invalid sidebar preference".into()),
            },
            Mutation::Collapse { section, collapsed } => {
                if !["pinned", "active", "working", "snoozed", "settled"]
                    .contains(&section.as_str())
                {
                    return Err("Invalid sidebar group".into());
                }
                if collapsed {
                    self.collapsed.insert(section);
                } else {
                    self.collapsed.remove(&section);
                }
            }
            Mutation::ProjectName { id, name } => {
                if id.len() > 512 || name.len() > 256 || name.chars().any(char::is_control) {
                    return Err(
                        "Project name must be at most 256 bytes without control characters".into(),
                    );
                }
                if name.trim().is_empty() {
                    self.project_names.remove(&id);
                } else {
                    self.project_names.insert(id, name.trim().into());
                }
            }
        }
        if self.entries.len() > MAX_ENTRIES
            || self.order.len() > MAX_ENTRIES
            || self.project_names.len() > 10_000
        {
            return Err("Sidebar metadata capacity exceeded".into());
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("Sidebar revision exhausted")?;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Action {
    Pin(bool),
    Settle(bool),
    Snooze(i64),
    Wake,
    AutoSettle(bool),
}
#[derive(Clone, Debug)]
pub(crate) enum Mutation {
    Threads {
        ids: Vec<String>,
        action: Action,
    },
    Order(Vec<String>),
    Policy {
        key: String,
        value: i32,
    },
    Place {
        id: String,
        section: String,
        order: Vec<String>,
    },
    Collapse {
        section: String,
        collapsed: bool,
    },
    ProjectName {
        id: String,
        name: String,
    },
}
fn read(home: &Path) -> Result<State, String> {
    let file = match std::fs::File::open(home.join(FILE)) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = vec![];
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Sidebar metadata exceeds size limit".into());
    }
    let state: State = serde_json::from_slice(&bytes)
        .map_err(|_| "Sidebar metadata is invalid; original file preserved")?;
    if state.version != 1
        || state.entries.len() > MAX_ENTRIES
        || state.order.len() > MAX_ENTRIES
        || state
            .policy
            .auto_settle_days
            .is_some_and(|days| !(1..=365).contains(&days))
    {
        return Err("Unsupported sidebar metadata; original file preserved".into());
    }
    Ok(state)
}
fn write(home: &Path, state: &State) -> Result<(), String> {
    let bytes = serde_json::to_vec(state).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Sidebar metadata exceeds size limit".into());
    }
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    crate::rally::store::atomic_write(&home.join(FILE), &bytes).map_err(|e| e.to_string())
}
struct Request {
    home: Option<PathBuf>,
    mutation: Option<Mutation>,
    reply: oneshot::Sender<Result<Arc<State>, String>>,
}
/// Bounded actor owns the latest snapshot. Commands merge into its current state,
/// not a window's potentially stale copy. Replies follow durable writes.
pub(crate) struct Store {
    requests: mpsc::Sender<Request>,
}
impl Store {
    pub fn new(runtime: &tokio::runtime::Handle) -> Self {
        let (requests, mut pending) = mpsc::channel::<Request>(64);
        runtime.spawn(async move {
            let mut cache: Option<(PathBuf, Arc<State>)> = None;
            while let Some(request) = pending.recv().await {
                let old = cache.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let Some(home) = request.home else {
                        return Ok(old);
                    };
                    let mut state = match old {
                        Some((path, state)) if path == home => state,
                        _ => Arc::new(read(&home)?),
                    };
                    if let Some(mutation) = request.mutation {
                        if read(&home)? != *state {
                            return Err("Sidebar metadata changed outside Fastrock; restart to reload it. Original file preserved.".into());
                        }
                        let mut next = (*state).clone();
                        next.apply(mutation, super::unix_now())?;
                        write(&home, &next)?;
                        state = Arc::new(next);
                    }
                    Ok::<_, String>(Some((home, state)))
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|v| v);
                let reply = match result {
                    Ok(next) => {
                        cache = next;
                        Ok(cache.as_ref().map(|(_, s)| s.clone()).unwrap_or_default())
                    }
                    Err(e) => Err(e),
                };
                let _ = request.reply.send(reply);
            }
        });
        Self { requests }
    }
    pub async fn request(
        &self,
        home: PathBuf,
        mutation: Option<Mutation>,
    ) -> Result<Arc<State>, String> {
        self.send(Some(home), mutation).await
    }
    async fn send(
        &self,
        home: Option<PathBuf>,
        mutation: Option<Mutation>,
    ) -> Result<Arc<State>, String> {
        let (reply, receive) = oneshot::channel();
        self.requests
            .send(Request {
                home,
                mutation,
                reply,
            })
            .await
            .map_err(|_| "Sidebar writer stopped")?;
        receive.await.map_err(|_| "Sidebar writer stopped")?
    }
    pub async fn flush(&self) -> Result<(), String> {
        self.send(None, None).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v68_auto_settle_deadlines_attention_opt_out_and_activity() {
        let mut state = State::default();
        let activity = 1000;
        let deadline = activity + 3 * 86400;
        assert_eq!(
            state.section_at("goose:a", false, false, activity, deadline - 1),
            "active"
        );
        assert_eq!(
            state.section_at("goose:a", false, false, activity, deadline),
            "settled"
        );
        assert_eq!(
            state.section_at("goose:a", true, false, activity, deadline),
            "working"
        );
        assert_eq!(
            state.section_at("goose:a", false, true, activity, deadline),
            "active"
        );
        assert_eq!(
            state.section_at("goose:a", false, false, deadline, deadline),
            "active"
        );
        state
            .apply(action(&["goose:a"], Action::AutoSettle(false)), deadline)
            .unwrap();
        assert_eq!(state.auto_settle_at("goose:a", activity), None);
        assert_eq!(
            state.section_at("goose:a", false, false, activity, deadline),
            "active"
        );
        state
            .apply(action(&["goose:a"], Action::AutoSettle(true)), deadline)
            .unwrap();
        state
            .apply(action(&["goose:a"], Action::Wake), deadline)
            .unwrap();
        assert_eq!(
            state.auto_settle_at("goose:a", activity),
            Some(deadline + 3 * 86400)
        );
        state
            .apply(
                action(&["goose:b"], Action::Snooze(deadline + 3600)),
                deadline,
            )
            .unwrap();
        assert_eq!(
            state.section_at("goose:b", false, false, activity, deadline),
            "snoozed"
        );
        assert_eq!(
            state.section_at("goose:b", false, false, activity, deadline + 3600),
            "active"
        );
        state
            .apply(action(&["goose:c"], Action::Pin(true)), activity)
            .unwrap();
        assert_eq!(
            state.section_at("goose:c", false, false, activity, deadline),
            "pinned"
        );
        state
            .apply(
                Mutation::Policy {
                    key: "working-section".into(),
                    value: 0,
                },
                deadline,
            )
            .unwrap();
        assert_eq!(
            state.section_at("goose:running", true, false, activity, deadline),
            "active"
        );
        state
            .apply(
                Mutation::Policy {
                    key: "auto-settle-days".into(),
                    value: 0,
                },
                deadline,
            )
            .unwrap();
        assert_eq!(
            state.section_at("goose:old", false, false, activity, deadline),
            "active"
        );
        let saved = state.clone();
        assert!(
            state
                .apply(
                    Mutation::Policy {
                        key: "auto-settle-days".into(),
                        value: 366
                    },
                    deadline
                )
                .is_err()
        );
        assert_eq!(state, saved);
    }
    #[tokio::test]
    async fn v68_drop_updates_group_and_order_atomically() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().to_path_buf();
        let store = Store::new(&tokio::runtime::Handle::current());
        let now = chrono::Utc::now().timestamp();
        store
            .request(
                home.clone(),
                Some(action(&["goose:a"], Action::Snooze(now + 3600))),
            )
            .await
            .unwrap();
        for section in ["pinned", "settled", "active"] {
            let state = store
                .request(
                    home.clone(),
                    Some(Mutation::Place {
                        id: "goose:a".into(),
                        section: section.into(),
                        order: vec!["goose:b".into(), "goose:a".into(), "goose:b".into()],
                    }),
                )
                .await
                .unwrap();
            assert_eq!(state.section("goose:a", false, now), section);
            assert_eq!(state.entries["goose:a"].snoozed_until, None);
            assert_eq!(state.entries["goose:a"].pinned, section == "pinned");
            assert_eq!(state.order, ["goose:b", "goose:a"]);
            assert_eq!(read(&home).unwrap(), *state);
        }
        let before = std::fs::read(home.join(FILE)).unwrap();
        assert!(
            store
                .request(
                    home.clone(),
                    Some(Mutation::Place {
                        id: "goose:a".into(),
                        section: "snoozed".into(),
                        order: vec!["goose:a".into()],
                    })
                )
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(home.join(FILE)).unwrap(), before);
        // External edits must survive a rejected drop, even with a cached snapshot.
        std::fs::write(home.join(FILE), b"external invalid edit").unwrap();
        assert!(
            store
                .request(
                    home.clone(),
                    Some(Mutation::Place {
                        id: "goose:a".into(),
                        section: "pinned".into(),
                        order: vec!["goose:a".into()],
                    })
                )
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(home.join(FILE)).unwrap(),
            b"external invalid edit"
        );
    }
    fn action(ids: &[&str], action: Action) -> Mutation {
        Mutation::Threads {
            ids: ids.iter().map(|s| s.to_string()).collect(),
            action,
        }
    }
    #[test]
    fn v68_sections_wake_without_dispatch_and_preserve_pin() {
        let mut s = State::default();
        s.apply(action(&["goose:a"], Action::Pin(true)), 100)
            .unwrap();
        assert_eq!(s.section("goose:a", true, 100), "pinned");
        s.apply(action(&["goose:a"], Action::Snooze(200)), 100)
            .unwrap();
        assert_eq!(s.section("goose:a", true, 199), "snoozed");
        assert_eq!(s.next_wake(199), Some(200));
        assert_eq!(s.next_wake(200), None);
        assert_eq!(s.section("goose:a", false, 200), "pinned");
        s.apply(action(&["goose:a"], Action::Settle(true)), 201)
            .unwrap();
        assert_eq!(s.section("goose:a", true, 201), "settled");
        s.apply(action(&["goose:a"], Action::Wake), 202).unwrap();
        assert_eq!(s.section("goose:a", false, 202), "pinned");
        assert_eq!(s.section("goose:b", true, 202), "working");
        assert_eq!(s.section("goose:b", false, 202), "active");
        assert!(
            s.apply(action(&["goose:b"], Action::Snooze(201)), 202)
                .is_err()
        );
    }
    #[tokio::test]
    async fn v68_store_serializes_windows_reopens_and_preserves_invalid_file() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().to_path_buf();
        let store = Store::new(&tokio::runtime::Handle::current());
        let (a, b) = tokio::join!(
            store.request(home.clone(), Some(action(&["goose:a"], Action::Pin(true)))),
            store.request(
                home.clone(),
                Some(action(&["goose:b"], Action::Settle(true)))
            )
        );
        a.unwrap();
        b.unwrap();
        store
            .request(
                home.clone(),
                Some(Mutation::Order(vec![
                    "goose:b".into(),
                    "goose:a".into(),
                    "goose:b".into(),
                ])),
            )
            .await
            .unwrap();
        store.flush().await.unwrap();
        let saved = read(&home).unwrap();
        assert_eq!(saved.entries.len(), 2);
        assert_eq!(saved.order, vec!["goose:b", "goose:a"]);
        let reopened = Store::new(&tokio::runtime::Handle::current());
        assert!(reopened.request(home.clone(), None).await.unwrap().entries["goose:a"].pinned);
        let invalid = tempfile::tempdir().unwrap();
        std::fs::write(invalid.path().join(FILE), b"broken original").unwrap();
        assert!(
            store
                .request(
                    invalid.path().into(),
                    Some(action(&["goose:a"], Action::Pin(true)))
                )
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(invalid.path().join(FILE)).unwrap(),
            b"broken original"
        );
        // Failed load must not replace the existing owner's good state.
        assert_eq!(store.request(home, None).await.unwrap().entries.len(), 2);
    }
}
