//! One in-process owner per canonical vwork home. All disk work runs on workers.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, OnceLock, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{OnceCell, Semaphore};
use vwork_engine::engine::Engine;

type Registry = Mutex<HashMap<PathBuf, Weak<OwnerSlot>>>;
static STORES: OnceLock<Registry> = OnceLock::new();

#[derive(Default)]
struct OwnerSlot {
    // Some(expired Weak) means the old owner is still closing. Do not reopen
    // until Drop has explicitly cleared this slot after releasing the database.
    owner: Mutex<Option<Weak<Store>>>,
    closed: Condvar,
}

pub struct Store {
    engine: Option<RwLock<Engine>>,
    slot: Arc<OwnerSlot>,
    failed: AtomicBool,
    workers: Arc<Semaphore>,
    next_compact: Mutex<std::time::Instant>,
}

impl Drop for Store {
    fn drop(&mut self) {
        drop(self.engine.take());
        *self.slot.owner.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.slot.closed.notify_all();
    }
}

impl Store {
    fn open(home: &Path) -> Result<Arc<Self>> {
        std::fs::create_dir_all(home)?;
        let home = home.canonicalize()?;
        let slot = {
            let mut stores = STORES
                .get_or_init(Default::default)
                .lock()
                .map_err(|_| anyhow::anyhow!("Workspace owner registry unavailable"))?;
            stores.retain(|_, slot| slot.strong_count() > 0);
            if let Some(slot) = stores.get(&home).and_then(Weak::upgrade) {
                slot
            } else {
                let slot = Arc::new(OwnerSlot::default());
                stores.insert(home.clone(), Arc::downgrade(&slot));
                slot
            }
        };
        let mut owner = slot
            .owner
            .lock()
            .map_err(|_| anyhow::anyhow!("Workspace ownership unavailable"))?;
        while let Some(previous) = owner.as_ref() {
            if let Some(store) = previous.upgrade() {
                return Ok(store);
            }
            owner = slot
                .closed
                .wait(owner)
                .map_err(|_| anyhow::anyhow!("Workspace ownership unavailable"))?;
        }
        let config = vwork_engine::transport::load_config(&home)?;
        let interval = std::time::Duration::from_secs(config.compact_interval_seconds);
        let store = Arc::new(Self {
            engine: Some(RwLock::new(Engine::open(&home, config)?)),
            slot: slot.clone(),
            failed: AtomicBool::new(false),
            workers: Arc::new(Semaphore::new(8)),
            next_compact: Mutex::new(std::time::Instant::now() + interval),
        });
        *owner = Some(Arc::downgrade(&store));
        drop(owner);
        Self::start_maintenance(Arc::downgrade(&store));
        Ok(store)
    }

    fn start_maintenance(weak: Weak<Self>) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                let Some(store) = weak.upgrade() else {
                    break;
                };
                let Ok(permit) = Arc::clone(&store.workers).try_acquire_owned() else {
                    continue;
                };
                let _ = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let config = store.with_engine(|engine| {
                        engine.reap_temporary()?;
                        Ok(engine.config.clone())
                    })?;
                    let now = std::time::Instant::now();
                    let mut next = store
                        .next_compact
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Maintenance schedule unavailable"))?;
                    if now < *next {
                        return Ok::<_, anyhow::Error>(());
                    }
                    *next = now + std::time::Duration::from_secs(config.compact_interval_seconds);
                    if config.auto_compact
                        && vwork_engine::maintenance::idle_time()
                            .is_some_and(|idle| idle.as_secs() > config.compact_idle_seconds)
                    {
                        store.call("database_compact", &json!({}))?;
                    }
                    Ok(())
                })
                .await;
            }
        });
    }

    /// Keep the database borrowed for the entire operation, including summary
    /// transactions. Compaction can never overlap a reader or retain an Arc.
    pub fn with_engine<T>(&self, f: impl FnOnce(&Engine) -> Result<T>) -> Result<T> {
        ensure!(
            !self.failed.load(Ordering::Acquire),
            "Workspace maintenance failed; restart Fastrock for recovery"
        );
        let engine = self
            .engine
            .as_ref()
            .expect("live workspace engine")
            .try_read()
            .map_err(|_| anyhow::anyhow!("Workspace maintenance is in progress"))?;
        ensure!(
            !self.failed.load(Ordering::Acquire),
            "Workspace maintenance failed; restart Fastrock for recovery"
        );
        f(&engine)
    }

    /// Review and integration need one consistent pair of workspace heads.
    /// Every regular operation holds a read guard, including MCP mutations;
    /// this guard therefore excludes tools, compaction and maintenance too.
    pub(crate) fn with_exclusive_engine<T>(
        &self,
        f: impl FnOnce(&Engine) -> Result<T>,
    ) -> Result<T> {
        ensure!(
            !self.failed.load(Ordering::Acquire),
            "Workspace maintenance failed; restart Fastrock for recovery"
        );
        let engine = self
            .engine
            .as_ref()
            .expect("live workspace engine")
            .try_write()
            .map_err(|_| {
                anyhow::anyhow!("A workspace operation is active; review again after it finishes")
            })?;
        ensure!(
            !self.failed.load(Ordering::Acquire),
            "Workspace maintenance failed; restart Fastrock for recovery"
        );
        f(&engine)
    }

    fn call(&self, op: &str, args: &Value) -> Result<Value> {
        if op == "database_compact" {
            ensure!(
                !self.failed.load(Ordering::Acquire),
                "Workspace maintenance failed; restart Fastrock for recovery"
            );
            let Ok(mut engine) = self
                .engine
                .as_ref()
                .expect("live workspace engine")
                .try_write()
            else {
                return Ok(json!({"compacted":false,"reason":"busy"}));
            };
            ensure!(
                !self.failed.load(Ordering::Acquire),
                "Workspace maintenance failed; restart Fastrock for recovery"
            );
            let result = engine.compact_database();
            if result.is_err() {
                self.failed.store(true, Ordering::Release);
            }
            return result;
        }
        self.with_engine(|engine| {
            if op == "mcp" {
                let request = args.get("request").context("Missing MCP request")?;
                let bound = args["workspace"].as_str();
                Ok(
                    vwork_engine::protocol::mcp_response(request, bound, |op, args| {
                        engine.call(op, args)
                    })
                    .unwrap_or(Value::Null),
                )
            } else {
                engine.call(op, args)
            }
        })
    }
}

/// Lazy opening keeps construction safe on the Slint UI thread. A connection
/// retains its owner until the workspace browser and all agent bindings close.
#[derive(Clone)]
pub struct Connection {
    home: PathBuf,
    store: Arc<OnceCell<Arc<Store>>>,
}

impl Connection {
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            store: Arc::new(OnceCell::new()),
        }
    }

    pub async fn store(&self) -> Result<Arc<Store>> {
        let home = self.home.clone();
        let store = self
            .store
            .get_or_try_init(|| async {
                tokio::task::spawn_blocking(move || Store::open(&home)).await?
            })
            .await?;
        Ok(Arc::clone(store))
    }

    pub async fn call(&self, op: String, args: Value) -> Result<Value> {
        self.run(move |store| store.call(&op, &args)).await
    }

    pub async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = self.store().await?;
        let permit = Arc::clone(&store.workers).acquire_owned().await?;
        // Own the permit inside the blocking worker: cancellation must not let
        // another call bypass the bound while a mutation is still committing.
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work(&store)
        })
        .await?
    }
}

pub fn hook(payload: &Value) -> Value {
    vwork_engine::protocol::hook(payload)
}

pub fn hook_matcher() -> String {
    vwork_engine::protocol::hook_matcher()
}

/// Configured Goose command hooks may invoke this lightweight helper mode.
/// It evaluates vwork's original policy without opening a database or UI.
pub fn run_hook(input: impl std::io::Read, mut output: impl std::io::Write) -> Result<()> {
    use std::io::Read;
    let mut bytes = Vec::new();
    input.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "Hook input exceeds 1 MiB");
    let payload: Value = serde_json::from_slice(&bytes)?;
    serde_json::to_writer(&mut output, &hook(&payload))?;
    output.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn v40_reopen_waits_until_previous_owner_has_closed() {
        let temp = tempfile::tempdir().unwrap();
        let owner = Store::open(temp.path()).unwrap();
        // Deterministically model the last Arc reaching zero before its Engine
        // destructor has released redb's OS file lock.
        let closing = Arc::try_unwrap(owner).unwrap_or_else(|_| panic!("unexpected owner"));
        let connection = Connection::new(temp.path().to_owned());
        let mut reopen = tokio::spawn(async move { connection.store().await });
        let early = tokio::time::timeout(std::time::Duration::from_millis(200), &mut reopen).await;
        drop(closing);
        assert!(
            early.is_err(),
            "reopening must wait for the old owner to finish closing"
        );
        let reopened = tokio::time::timeout(std::time::Duration::from_secs(5), reopen)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!reopened.failed.load(Ordering::Acquire));
    }
}
