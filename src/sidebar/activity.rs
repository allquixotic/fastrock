//! GUI read state and meaningful activity; opening/resuming is never an edit.
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use codex_app_server_protocol::{
    ServerNotification, Thread, ThreadStatus, ThreadTurnsListResponse,
};
use serde::{Deserialize, Serialize};

use super::{ThreadMark, unix_now};
use crate::app::AppController;
use crate::backend::BackendError;
use crate::session;
use crate::ui::SidebarThreadStatus;

const FILE: &str = "gui-thread-activity.json";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Entry {
    at: i64,
    read_at: Option<i64>,
    unread: bool,
    observed_updated: Option<i64>,
}
impl Entry {
    fn observe(&mut self, at: i64) {
        self.at = self.at.max(at);
        if self.read_at.is_some_and(|read| self.at > read) {
            self.unread = true;
        }
    }
    fn read(&mut self) {
        self.read_at = Some(self.at);
        self.unread = false;
    }
    fn status(&self) -> SidebarThreadStatus {
        if self.unread {
            SidebarThreadStatus::Unread
        } else if self.read_at.is_some() {
            SidebarThreadStatus::Idle
        } else {
            SidebarThreadStatus::NotOpen
        }
    }
}

#[derive(Default)]
pub(super) struct ActivityController {
    entries: BTreeMap<String, Entry>,
    statuses: HashMap<String, SidebarThreadStatus>,
    pending: VecDeque<(String, i64)>,
    loading: HashSet<String>,
    goals: HashSet<String>,
    save_revision: Arc<AtomicU64>,
    save_lock: Arc<Mutex<()>>,
}
impl ActivityController {
    pub(super) fn load(home: Option<&Path>) -> Self {
        Self {
            entries: home
                .and_then(|home| std::fs::read(home.join(FILE)).ok())
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default(),
            ..Self::default()
        }
    }
    pub(super) fn at(&self, id: &str) -> Option<i64> {
        self.entries
            .get(id)
            .map(|entry| entry.at)
            .filter(|at| *at > 0)
    }
    pub(super) fn marks(&self) -> HashMap<String, ThreadMark> {
        self.entries
            .iter()
            .map(|(id, entry)| {
                let runtime = self
                    .statuses
                    .get(id)
                    .copied()
                    .unwrap_or(SidebarThreadStatus::NotOpen);
                let status = if matches!(
                    runtime,
                    SidebarThreadStatus::Starting
                        | SidebarThreadStatus::Running
                        | SidebarThreadStatus::Waiting
                        | SidebarThreadStatus::Error
                ) {
                    runtime
                } else {
                    entry.status()
                };
                (
                    id.clone(),
                    ThreadMark {
                        status,
                        selected: false,
                    },
                )
            })
            .collect()
    }
    pub(super) fn idle_status(&self, id: &str) -> SidebarThreadStatus {
        self.entries
            .get(id)
            .map(Entry::status)
            .unwrap_or(SidebarThreadStatus::Idle)
    }

    fn notification_at(&mut self, notification: &ServerNotification, now: i64) -> Option<i64> {
        use ServerNotification as N;
        match notification {
            // Resume sends goal snapshots too. Use the goal's actual timestamp,
            // and do not treat an initial/repeated empty snapshot as a mutation.
            N::ThreadGoalUpdated(updated) => {
                self.goals.insert(updated.thread_id.clone());
                (updated.goal.updated_at > self.at(&updated.thread_id).unwrap_or(0))
                    .then_some(updated.goal.updated_at)
            }
            N::ThreadGoalCleared(cleared) => self.goals.remove(&cleared.thread_id).then_some(now),
            N::ItemStarted(_)
            | N::ItemCompleted(_)
            | N::AgentMessageDelta(_)
            | N::TurnStarted(_)
            | N::TurnCompleted(_)
            | N::ThreadNameUpdated(_) => Some(now),
            _ => None,
        }
    }
}
impl AppController {
    pub(super) fn activity_observe(&mut self, threads: &[Thread]) {
        for thread in threads.iter().filter(|thread| !thread.ephemeral) {
            let activity = &mut self.sidebar.activity;
            let entry = activity.entries.entry(thread.id.clone()).or_default();
            // Legacy rollouts expose file mtime as recency_at, including resume
            // and configuration writes. Only creation and actual turn timestamps
            // below are trustworthy evidence of conversation activity.
            entry.observe(thread.created_at);
            activity.statuses.insert(
                thread.id.clone(),
                match &thread.status {
                    ThreadStatus::Active { active_flags } if !active_flags.is_empty() => {
                        SidebarThreadStatus::Waiting
                    }
                    ThreadStatus::Active { .. } => SidebarThreadStatus::Running,
                    ThreadStatus::SystemError => SidebarThreadStatus::Error,
                    _ => SidebarThreadStatus::NotOpen,
                },
            );
            if entry.observed_updated != Some(thread.updated_at)
                && !activity.loading.contains(&thread.id)
                && !activity.pending.iter().any(|(id, _)| id == &thread.id)
            {
                activity
                    .pending
                    .push_back((thread.id.clone(), thread.updated_at));
            }
        }
        self.activity_pump();
    }

    fn activity_pump(&mut self) {
        while self.sidebar.activity.loading.len() < 2 {
            let Some((id, updated)) = self.sidebar.activity.pending.pop_front() else {
                break;
            };
            self.sidebar.activity.loading.insert(id.clone());
            let request = session::turns_page(self.backend.next_request_id(), &id, None, 1);
            self.backend.call(
                move |_| request,
                move |app, result: Result<ThreadTurnsListResponse, BackendError>| {
                    app.sidebar.activity.loading.remove(&id);
                    let entry = app.sidebar.activity.entries.entry(id.clone()).or_default();
                    if let Ok(page) = result {
                        entry.observed_updated = Some(updated);
                        for turn in page.data {
                            for at in [turn.started_at, turn.completed_at].into_iter().flatten() {
                                entry.observe(at);
                            }
                        }
                        if app.window_in_foreground()
                            && app
                                .active_thread_index()
                                .and_then(|i| app.thread_tab(i))
                                .and_then(|t| t.thread_id.as_deref())
                                == Some(&id)
                            && let Some(entry) = app.sidebar.activity.entries.get_mut(&id)
                        {
                            entry.read();
                        }
                        app.activity_save();
                        app.sidebar_render();
                    }
                    app.activity_pump();
                },
            );
        }
    }

    pub(crate) fn activity_read_active(&mut self) {
        if !self.window_in_foreground() {
            return;
        }
        let Some(id) = self
            .active_thread_index()
            .and_then(|i| self.thread_tab(i))
            .and_then(|t| t.thread_id.clone())
        else {
            return;
        };
        self.sidebar.activity.entries.entry(id).or_default().read();
        self.activity_save();
    }

    pub(super) fn activity_notification(&mut self, notification: &ServerNotification) {
        use ServerNotification as N;
        if let N::ThreadStatusChanged(changed) = notification {
            self.sidebar.activity.statuses.insert(
                changed.thread_id.clone(),
                match &changed.status {
                    ThreadStatus::Active { active_flags } if !active_flags.is_empty() => {
                        SidebarThreadStatus::Waiting
                    }
                    ThreadStatus::Active { .. } => SidebarThreadStatus::Running,
                    ThreadStatus::SystemError => SidebarThreadStatus::Error,
                    _ => SidebarThreadStatus::NotOpen,
                },
            );
        }
        let Some(at) = self
            .sidebar
            .activity
            .notification_at(notification, unix_now())
        else {
            return;
        };
        let Some(id) = crate::app::notification_thread_id(notification) else {
            return;
        };
        if self.sidebar.unlisted.contains(id) {
            return;
        }
        let active = self.window_in_foreground()
            && self
                .active_thread_index()
                .and_then(|i| self.thread_tab(i))
                .and_then(|t| t.thread_id.as_deref())
                == Some(id);
        let entry = self
            .sidebar
            .activity
            .entries
            .entry(id.to_string())
            .or_default();
        entry.observe(at);
        if active {
            entry.read();
        } else {
            entry.unread = true;
        }
        // Streamed chunks update memory only; item/turn boundaries persist.
        if !matches!(notification, N::AgentMessageDelta(_)) {
            self.activity_save();
        }
    }

    fn activity_save(&self) {
        let Some(home) = self.codex_home.clone() else {
            return;
        };
        let entries = self.sidebar.activity.entries.clone();
        let lock = Arc::clone(&self.sidebar.activity.save_lock);
        let revision = Arc::clone(&self.sidebar.activity.save_revision);
        let current = revision.fetch_add(1, Ordering::SeqCst) + 1;
        self.backend.spawn(async move {
            let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
                let _guard = lock
                    .lock()
                    .map_err(|_| std::io::Error::other("Activity cache lock poisoned"))?;
                if revision.load(Ordering::SeqCst) != current {
                    return Ok(());
                }
                std::fs::create_dir_all(&home)?;
                let temporary = home.join(format!("{FILE}.{}.tmp", std::process::id()));
                std::fs::write(&temporary, serde_json::to_vec(&entries)?)?;
                std::fs::rename(temporary, home.join(FILE))
            })
            .await;
            if !matches!(result, Ok(Ok(()))) {
                tracing::warn!(?result, "Could not persist GUI read state");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_app_server_protocol::{
        ThreadGoal, ThreadGoalClearedNotification, ThreadGoalStatus, ThreadGoalUpdatedNotification,
    };

    #[test]
    fn v13_goal_snapshots_do_not_become_new_activity() {
        let mut activity = ActivityController::default();
        let mut entry = Entry::default();
        entry.observe(100);
        entry.read();
        activity.entries.insert("thread".into(), entry);
        let clear = ServerNotification::ThreadGoalCleared(ThreadGoalClearedNotification {
            thread_id: "thread".into(),
        });
        assert_eq!(activity.notification_at(&clear, 200), None);
        let mut update = ServerNotification::ThreadGoalUpdated(ThreadGoalUpdatedNotification {
            thread_id: "thread".into(),
            turn_id: None,
            goal: ThreadGoal {
                thread_id: "thread".into(),
                objective: "Test goal".into(),
                status: ThreadGoalStatus::Active,
                token_budget: None,
                tokens_used: 0,
                time_used_seconds: 0,
                created_at: 90,
                updated_at: 90,
            },
        });
        assert_eq!(activity.notification_at(&update, 200), None);
        if let ServerNotification::ThreadGoalUpdated(update) = &mut update {
            update.goal.updated_at = 210;
        }
        assert_eq!(activity.notification_at(&update, 250), Some(210));
        activity.entries.get_mut("thread").unwrap().observe(210);
        assert_eq!(activity.notification_at(&update, 260), None);
        assert_eq!(activity.notification_at(&clear, 270), Some(270));
        assert_eq!(activity.notification_at(&clear, 280), None);
    }
    #[test]
    fn v13_reads_do_not_advance_activity_and_new_output_becomes_unread() {
        let mut entry = Entry::default();
        entry.observe(100);
        assert_eq!(entry.status(), SidebarThreadStatus::NotOpen);
        entry.read();
        entry.read();
        assert_eq!(entry.at, 100);
        assert_eq!(entry.status(), SidebarThreadStatus::Idle);
        entry.observe(200);
        assert_eq!(entry.status(), SidebarThreadStatus::Unread);
        entry.read();
        assert_eq!(entry.status(), SidebarThreadStatus::Idle);
        let restored: Entry = serde_json::from_slice(&serde_json::to_vec(&entry).unwrap()).unwrap();
        assert_eq!(restored.at, 200);
        assert!(!restored.unread);
    }
}
