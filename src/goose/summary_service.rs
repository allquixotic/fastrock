//! A shared, cancellable background worker. Only IDs wait in its queue.
use super::{cheap_profile, summary_worker};
use anyhow::Result;
use fastrock_workspaces::{embedded::Connection, summaries::Summaries};
use goose_context_management::ProviderModel;
use goose_provider_types::model::ModelConfig;
use goose_runtime::{
    config::paths::Paths,
    session::{SessionManager, SessionType},
};
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, Notify};

static SERVICE: OnceLock<Mutex<Weak<Service>>> = OnceLock::new();
static FAILED: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Pending {
    ids: VecDeque<String>,
    queued: HashSet<String>,
    active: Option<(String, Arc<AtomicBool>)>,
}
impl Pending {
    fn push(&mut self, id: String, completed_turn: bool) {
        if let Some((active, cancel)) = &self.active {
            if *active == id {
                if !completed_turn {
                    return;
                }
                cancel.store(true, Ordering::Relaxed);
            }
        }
        if self.queued.insert(id.clone()) {
            if completed_turn {
                self.ids.push_front(id);
            } else {
                self.ids.push_back(id);
            }
        }
    }
    fn pop(&mut self) -> Option<(String, Arc<AtomicBool>)> {
        let id = self.ids.pop_front()?;
        self.queued.remove(&id);
        let cancel = Arc::new(AtomicBool::new(false));
        self.active = Some((id.clone(), cancel.clone()));
        Some((id, cancel))
    }
}
#[derive(Default)]
struct Queue {
    pending: Mutex<Pending>,
    notify: Notify,
    // A completion cannot race between the obsolete check and the DB commit.
    commit: AsyncMutex<()>,
    ready: AtomicBool,
    stage: AtomicU8,
    storage_failure: Mutex<Option<&'static str>>,
}

pub(super) struct Service {
    queue: Arc<Queue>,
    task: tokio::task::JoinHandle<()>,
    home: PathBuf,
}
impl Drop for Service {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Service {
    pub fn start(home: PathBuf) -> Arc<Self> {
        let mut singleton = SERVICE.get_or_init(Default::default).lock().unwrap();
        if let Some(service) = singleton.upgrade() {
            return service;
        }
        let queue = Arc::new(Queue::default());
        let worker = queue.clone();
        let summaries = Summaries(Connection::new(home.clone()));
        let task = tokio::spawn(async move {
            let manager = SessionManager::new(Paths::data_dir());
            // Keep errors out of logs: provider errors may contain credentials.
            loop {
                let _ = run(worker.clone(), &summaries, &manager).await;
                FAILED.store(true, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
        let service = Arc::new(Self { queue, task, home });
        *singleton = Arc::downgrade(&service);
        service
    }

    pub async fn completed(&self, id: String) {
        let _guard = self.queue.commit.lock().await;
        self.queue.pending.lock().unwrap().push(id, true);
        self.queue.notify.notify_one();
    }

    #[cfg(test)]
    pub async fn wait_idle(&self) {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let idle = {
                    let pending = self.queue.pending.lock().unwrap();
                    self.queue.ready.load(Ordering::Relaxed)
                        && pending.active.is_none()
                        && pending.ids.is_empty()
                };
                if idle {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Summary fixture did not become idle");
    }
}

pub(super) fn idle() -> bool {
    let Some(service) = SERVICE
        .get()
        .and_then(|service| service.lock().ok()?.upgrade())
    else {
        return false;
    };
    let pending = service.queue.pending.lock().unwrap();
    service.queue.ready.load(Ordering::Relaxed)
        && pending.active.is_none()
        && pending.ids.is_empty()
}

/// Fixture diagnostics contain stages/counts only, never provider errors or IDs.
pub(super) fn diagnostic() -> String {
    let Some(service) = SERVICE.get().and_then(|s| s.lock().ok()?.upgrade()) else {
        return "unavailable".into();
    };
    let pending = service.queue.pending.lock().unwrap();
    let stages = [
        "backfill",
        "source",
        "previous summary",
        "cheap profile",
        "provider",
        "revision",
        "generation",
        "latest source",
        "commit",
    ];
    format!(
        "ready={}, active={}, pending={}, retry={}, stage={}, storage={}",
        service.queue.ready.load(Ordering::Relaxed),
        pending.active.is_some(),
        pending.ids.len(),
        FAILED.load(Ordering::Relaxed),
        stages
            .get(service.queue.stage.load(Ordering::Relaxed) as usize)
            .unwrap_or(&"unknown"),
        service
            .queue
            .storage_failure
            .lock()
            .unwrap()
            .unwrap_or("ok")
    )
}

pub(super) fn home() -> Option<PathBuf> {
    SERVICE
        .get()?
        .lock()
        .ok()?
        .upgrade()
        .map(|service| service.home.clone())
}

pub(super) fn status() -> &'static str {
    if FAILED.load(Ordering::Relaxed) {
        "Background summaries are unavailable or retrying. Existing summaries and local search remain available."
    } else {
        "Background summaries use this profile; searches run locally."
    }
}

async fn backfill(manager: &SessionManager, queue: &Queue) -> Result<()> {
    let sessions = manager.list_all_sessions().await?;
    let mut pending = queue.pending.lock().unwrap();
    for session in sessions {
        if session.session_type != SessionType::Hidden && session.message_count > 0 {
            pending.push(session.id, false);
        }
    }
    Ok(())
}

async fn run(queue: Arc<Queue>, summaries: &Summaries, manager: &SessionManager) -> Result<()> {
    queue.stage.store(0, Ordering::Relaxed);
    backfill(manager, &queue).await?;
    queue.ready.store(true, Ordering::Relaxed);
    loop {
        // Register notification before checking the queue, avoiding a lost wakeup.
        let notified = queue.notify.notified();
        let job = queue.pending.lock().unwrap().pop();
        if let Some((id, cancelled)) = job {
            let result = process(manager, summaries, &queue, &id, &cancelled).await;
            queue.pending.lock().unwrap().active = None;
            if result.is_err() {
                FAILED.store(true, Ordering::Relaxed);
                // Missing credentials must not hammer the provider for every
                // imported conversation. Retry once per minute, latest data only.
                queue.pending.lock().unwrap().push(id, false);
                tokio::time::sleep(Duration::from_secs(60)).await;
            } else {
                FAILED.store(false, Ordering::Relaxed);
            }
        } else {
            notified.await;
        }
    }
}

async fn process(
    manager: &SessionManager,
    summaries: &Summaries,
    queue: &Queue,
    id: &str,
    cancelled: &AtomicBool,
) -> Result<()> {
    queue.stage.store(1, Ordering::Relaxed);
    let Some(source) = summary_worker::snapshot(manager, id).await? else {
        return Ok(());
    };
    let key = super::model::ui_id(id);
    queue.stage.store(2, Ordering::Relaxed);
    let previous = summaries.get(key.clone()).await.map_err(|error| {
        *queue.storage_failure.lock().unwrap() =
            Some(fastrock_workspaces::summaries::failure_kind(&error));
        error
    })?;
    *queue.storage_failure.lock().unwrap() = None;
    if previous.as_ref().is_some_and(|old| source.current(old)) {
        return Ok(());
    }
    if cancelled.load(Ordering::Relaxed) {
        return Ok(());
    }
    queue.stage.store(3, Ordering::Relaxed);
    let profile = cheap_profile::load().await?;
    queue.stage.store(4, Ordering::Relaxed);
    let provider = goose_runtime::providers::create(&profile.provider, Vec::new()).await?;
    let model = ProviderModel::new(
        provider,
        ModelConfig::new(&profile.model).with_max_tokens(Some(8192)),
    );
    queue.stage.store(5, Ordering::Relaxed);
    let revision = summaries.advance(key.clone()).await?;
    queue.stage.store(6, Ordering::Relaxed);
    let Some(result) = summary_worker::generate(
        &model,
        &source,
        previous.as_ref(),
        format!(
            "{} · {} / {}",
            profile.name, profile.provider, profile.model
        ),
        revision,
        || cancelled.load(Ordering::Relaxed),
    )
    .await?
    else {
        return Ok(());
    };
    // Another window, compaction, or an external edit may have changed history.
    queue.stage.store(7, Ordering::Relaxed);
    let Some(latest) = summary_worker::snapshot(manager, id).await? else {
        return Ok(());
    };
    let _guard = queue.commit.lock().await;
    if !cancelled.load(Ordering::Relaxed) && latest.current(&result) {
        queue.stage.store(8, Ordering::Relaxed);
        summaries.replace(key, result).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v44_completed_turns_cancel_old_work_and_coalesce_backfill() {
        let mut queue = Pending::default();
        queue.push("old".into(), false);
        queue.push("live".into(), true);
        let (id, cancelled) = queue.pop().unwrap();
        assert_eq!(id, "live");
        queue.push(id.clone(), false);
        assert!(!cancelled.load(Ordering::Relaxed));
        queue.push(id.clone(), true);
        queue.push(id.clone(), true);
        assert!(cancelled.load(Ordering::Relaxed));
        assert_eq!(queue.pop().unwrap().0, "live");
        assert_eq!(queue.pop().unwrap().0, "old");
        assert!(queue.pop().is_none());
    }
}
