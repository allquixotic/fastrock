//! Explicit permanent deletion, with recoverable cleanup across Goose and vwork.
use super::*;
use base64::Engine as _;
use fastrock_workspaces::{checkpoints, embedded::Connection, summaries::Summaries};
use goose_runtime::{config::paths::Paths, session::SessionManager};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    id: String,
    checkpoint_home: PathBuf,
    summary_home: PathBuf,
    parent: Option<String>,
    complete: bool,
}
#[derive(Default)]
pub(super) struct State {
    records: HashMap<String, Record>,
}
pub(super) async fn open(root: PathBuf) -> anyhow::Result<State> {
    tokio::task::spawn_blocking(move || {
        let mut state = State::default();
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(state),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            if entry.path().extension().is_none_or(|e| e != "json") {
                continue;
            }
            anyhow::ensure!(
                entry.metadata()?.len() <= 16 * 1024,
                "Deletion record exceeds limit; preserve recovery files"
            );
            let record: Record = serde_json::from_slice(&std::fs::read(entry.path())?)?;
            anyhow::ensure!(
                !record.id.is_empty()
                    && record.id.len() <= 256
                    && !record.id.chars().any(char::is_control)
                    && record.parent.as_ref().is_none_or(|p| !p.is_empty()
                        && p.len() <= 256
                        && !p.chars().any(char::is_control)
                        && p != &record.id)
                    && entry.file_name()
                        == std::ffi::OsString::from(format!(
                            "{}.json",
                            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&record.id)
                        ))
                    && record.checkpoint_home.is_absolute()
                    && record.summary_home.is_absolute(),
                "Invalid deletion recovery record"
            );
            anyhow::ensure!(
                state.records.len() < 100_000 && !state.records.contains_key(&record.id),
                "Too many or duplicate deletion records"
            );
            state.records.insert(record.id.clone(), record);
        }
        Ok(state)
    })
    .await?
}
fn manager() -> SessionManager {
    SessionManager::new(Paths::data_dir())
}
async fn exists(manager: &SessionManager, id: &str) -> Result<bool, JSONRPCErrorError> {
    match manager.get_session(id, false).await {
        Ok(_) => Ok(true),
        Err(e) if e.to_string() == "Session not found" => Ok(false),
        Err(_) => Err(error(
            "Cannot inspect stored conversation; deletion stopped",
        )),
    }
}
impl Adapter {
    pub(super) async fn deletion_page(&self, params: &Value) -> Reply {
        let after = params["after"].as_str().unwrap_or("");
        let state = self.deletions.lock().await;
        let mut ids: Vec<_> = state
            .records
            .values()
            .filter(|r| r.complete && r.id.as_str() > after)
            .map(|r| r.id.clone())
            .collect();
        ids.sort();
        let more = ids.len() > 256;
        ids.truncate(256);
        let cursor = more.then(|| ids.last().cloned()).flatten();
        Ok(json!({"data":ids.iter().map(|id|ui_id(id)).collect::<Vec<_>>(), "nextCursor":cursor}))
    }

    pub(super) async fn deletion_ready(&self, id: &str) -> Result<(), JSONRPCErrorError> {
        if let Some(record) = self.deletions.lock().await.records.get(id) {
            return Err(error(if record.complete {
                "This conversation was permanently deleted"
            } else {
                "Deletion was interrupted. Retry Delete to finish; no new work can start in this conversation"
            }));
        }
        Ok(())
    }
    pub(super) async fn deleted(&self, id: &str) -> bool {
        self.deletions
            .lock()
            .await
            .records
            .get(id)
            .is_some_and(|r| r.complete)
    }
    async fn save_deletion(&self, record: Record) -> Result<(), JSONRPCErrorError> {
        let file = self.queue_path(&record.id).file_name().unwrap().to_owned();
        let path = self.queue_root.join("deletions").join(file);
        let saved = record.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            std::fs::create_dir_all(path.parent().unwrap())?;
            crate::rally::store::atomic_write(&path, &serde_json::to_vec(&saved)?)?;
            Ok(())
        })
        .await
        .map_err(|e| error(e.to_string()))?
        .map_err(|_| {
            error("Could not save deletion recovery; history preserved until intent is durable")
        })?;
        self.deletions
            .lock()
            .await
            .records
            .insert(record.id.clone(), record);
        Ok(())
    }
    pub(super) async fn recover_deletions(&self) -> Result<(), JSONRPCErrorError> {
        let pending: Vec<_> = self
            .deletions
            .lock()
            .await
            .records
            .values()
            .filter(|r| !r.complete)
            .cloned()
            .collect();
        let manager = manager();
        for record in pending {
            // A crash before Goose deleted history requires another explicit
            // Delete. Once history is gone, only app metadata cleanup remains.
            if !exists(&manager, &record.id).await? {
                self.finish_deletion(record).await?;
            }
        }
        Ok(())
    }
    async fn finish_deletion(&self, mut record: Record) -> Result<(), JSONRPCErrorError> {
        if let Some(service) = &self.summaries {
            service
                .forget(&record.id)
                .await
                .map_err(|_| error("History deleted; summary cleanup needs a retry or restart"))?;
        }
        Summaries(Connection::new(record.summary_home.clone()))
            .forget(ui_id(&record.id))
            .await
            .map_err(|_| error("History deleted; summary cleanup needs a retry or restart"))?;
        checkpoints::forget(
            &Connection::new(record.checkpoint_home.clone()),
            record.id.clone(),
        )
        .await
        .map_err(|_| error("History deleted; checkpoint cleanup needs a retry or restart"))?;
        let base = self.queue_path(&record.id);
        // Only files owned by this conversation. Workspaces, source, leases,
        // Goose configuration and credential entries are never deleted.
        for suffix in [
            "json",
            "binding.json",
            "tools.json",
            "vwork.json",
            "delegates.json",
            "outcome.json",
            "delegate-result.json",
            "last-dispatch.json",
        ] {
            let path = base.with_extension(suffix);
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {
                    return Err(error(
                        "History deleted; conversation metadata cleanup needs a retry or restart",
                    ));
                }
            }
        }
        self.mcp.remove_session(&record.id).await;
        self.sessions.lock().await.remove(&record.id);
        self.queue.lock().await.remove(&record.id);
        record.complete = true;
        self.save_deletion(record).await
    }
    pub(super) async fn delete_session(self: &Arc<Self>, params: &Value) -> Reply {
        if !self.embedded_runtime {
            return Err(unsupported("Permanent deletion requires embedded Goose"));
        }
        let id = session_id(text(params, "threadId"))?.to_owned();
        // Same order as scheduling: schedule gate, parent gate, child gate.
        let _schedule = self.schedule_gate.lock().await;
        let previous = self.deletions.lock().await.records.get(&id).cloned();
        let binding = self.binding_record(&id).await?;
        let parent = previous
            .as_ref()
            .and_then(|r| r.parent.clone())
            .or_else(|| binding.delegation.as_ref().map(|d| d.parent.clone()));
        if parent.as_deref() == Some(&id) {
            return Err(error("Invalid self-parent; preserve conversation metadata"));
        }
        let parent_gate = match parent.as_deref() {
            Some(p) => Some(self.gate(p).await),
            None => None,
        };
        let _parent = match &parent_gate {
            Some(g) => Some(g.lock().await),
            None => None,
        };
        let gate = self.gate(&id).await;
        let _lock = gate.lock().await;
        let manager = manager();
        if previous.as_ref().is_some_and(|r| r.complete) {
            return Ok(json!({}));
        }
        if previous.is_none() || exists(&manager, &id).await? {
            {
                let sessions = self.sessions.lock().await;
                for target in std::iter::once(id.as_str()).chain(parent.as_deref()) {
                    if sessions
                        .get(target)
                        .is_some_and(|s| s.active.is_some() || s.shell.is_some())
                    {
                        return Err(error(
                            "Stop this conversation and its coordinating parent before deleting",
                        ));
                    }
                }
                if sessions.get(&id).is_some_and(|s| s.ephemeral) {
                    return Err(error("Close a temporary side-chat tab to discard it"));
                }
            }
            for target in std::iter::once(id.as_str()).chain(parent.as_deref()) {
                self.queue_delete_ready(target).await?;
            }
            let schedules = self.schedules.lock().await;
            schedules.available()?;
            if schedules.data.tasks.values().any(|task| {
                task.source == id
                    || task.active.as_ref().and_then(|r| r.child.as_deref()) == Some(&id)
                    || task.last.as_ref().and_then(|r| r.child.as_deref()) == Some(&id)
            }) {
                return Err(error(
                    "Remove schedules that reference this conversation before deleting it",
                ));
            }
            drop(schedules);
            for child in self.delegation_index(&id).await?.values() {
                if !self.deleted(child).await {
                    return Err(error("Delete this conversation's child agents first"));
                }
            }
            self.checkpoint_ready(&id)
                .await
                .map_err(|e| error(e.to_string()))?;
            let mut workspaces = Vec::new();
            if let Some(workspace) = &binding.workspace {
                workspaces.push((
                    workspace.selection.home.clone(),
                    workspace.selection.workspace.clone(),
                ));
            }
            let saved = self.queue_path(&id).with_extension("vwork.json");
            match tokio::fs::metadata(&saved).await {
                Ok(meta) => {
                    if meta.len() > 1024 * 1024 {
                        return Err(error("Stored vwork bindings exceed limit"));
                    }
                    let bindings: Vec<fastrock_workspaces::vwork::McpBinding> =
                        serde_json::from_slice(
                            &tokio::fs::read(saved)
                                .await
                                .map_err(|_| error("Cannot read workspace bindings"))?,
                        )
                        .map_err(|_| error("Invalid workspace bindings; deletion stopped"))?;
                    workspaces.extend(
                        bindings
                            .into_iter()
                            .filter_map(|b| b.workspace.map(|w| (b.home, w))),
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(error("Cannot check workspace bindings; deletion stopped")),
            }
            workspaces.sort();
            workspaces.dedup();
            for (home, workspace) in workspaces {
                let leases = Connection::new(home)
                    .call("temporary_list".into(), json!({}))
                    .await
                    .map_err(|_| error("Cannot check workspace leases; deletion stopped"))?;
                if leases["active"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|l| l["workspace"] == workspace)
                {
                    return Err(error(
                        "Close this workspace's terminal and temporary tools before deleting its conversation",
                    ));
                }
            }
            if previous.is_none() {
                super::id_reservation::before_delete(&manager, &id)
                    .await
                    .map_err(|_| {
                        error("Could not reserve a safe conversation identity; deletion stopped")
                    })?;
                let record = Record {
                    id: id.clone(),
                    checkpoint_home: binding
                        .workspace
                        .as_ref()
                        .map(|w| w.selection.home.clone())
                        .unwrap_or_else(|| self.vwork_paths.default_vwork_home.clone()),
                    summary_home: super::summary_service::home()
                        .unwrap_or_else(|| self.vwork_paths.default_vwork_home.clone()),
                    parent,
                    complete: false,
                };
                self.save_deletion(record).await?;
            }
            self.rpc
                .call("session/close", json!({"sessionId":id}))
                .await?;
            manager.delete_session(&id).await.map_err(|_| {
                error("Goose could not delete history; retry Delete. Recovery record preserved")
            })?;
        }
        let record = self
            .deletions
            .lock()
            .await
            .records
            .get(&id)
            .cloned()
            .ok_or_else(|| error("Deletion recovery record missing"))?;
        self.finish_deletion(record).await?;
        self.emit(json!({"method":"thread/deleted","params":{"threadId":ui_id(&id)}}))?;
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn v77_deletion_recovery_validates_identity_and_retains_corruption() {
        let root = tempfile::tempdir().unwrap();
        let record = Record {
            id: "session".into(),
            checkpoint_home: root.path().into(),
            summary_home: root.path().into(),
            parent: None,
            complete: false,
        };
        let bytes = serde_json::to_vec(&record).unwrap();
        let name = format!(
            "{}.json",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("session")
        );
        let path = root.path().join(name);
        std::fs::write(&path, &bytes).unwrap();
        let state = open(root.path().into()).await.unwrap();
        assert!(!state.records["session"].complete);
        let wrong = root.path().join("wrong.json");
        std::fs::rename(&path, &wrong).unwrap();
        assert!(open(root.path().into()).await.is_err());
        assert_eq!(std::fs::read(wrong).unwrap(), bytes);
    }
}
