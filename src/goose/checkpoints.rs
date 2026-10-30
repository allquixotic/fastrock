//! Durable file/conversation checkpoints and explicit interrupted-restore recovery.
use super::checkpoint_storage::{self as storage, Boundary};
use super::*;
use anyhow::{Context, Result, ensure};
use fastrock_workspaces::{
    checkpoints::{self as disk, Capture, Checkpoint, Review, journal},
    embedded::Connection,
};
use goose_provider_types::conversation::Conversation;
use goose_runtime::{config::paths::Paths, session::SessionManager};

fn manager() -> SessionManager {
    SessionManager::new(Paths::data_dir())
}
fn capture_record(
    id: &str,
    turn: &str,
    phase: &str,
    label: &str,
    conversation: &Conversation,
) -> Result<Capture> {
    let ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    Ok(Capture {
        id: format!("{ms:020}-{}", uuid::Uuid::new_v4()),
        session: id.into(),
        created_ms: ms,
        turn: turn.into(),
        phase: phase.into(),
        label: label.chars().take(512).collect(),
        boundary: serde_json::to_value(storage::boundary(conversation)?)?,
    })
}
impl Adapter {
    async fn checkpoint_context(&self, id: &str) -> Result<(Connection, Option<String>)> {
        let binding = self
            .binding_record(id)
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        let (home, workspace) = match binding.workspace {
            Some(w) => (w.selection.home, Some(w.selection.workspace)),
            None => (self.vwork_paths.default_vwork_home.clone(), None),
        };
        Ok((Connection::new(home), workspace))
    }
    pub(super) async fn checkpoint_pending(&self, id: &str) -> Result<bool> {
        if !self.embedded_runtime {
            return Ok(false);
        }
        let (connection, _) = self.checkpoint_context(id).await?;
        Ok(journal::pending(&connection, id.into()).await?.is_some())
    }
    pub(super) async fn checkpoint_ready(&self, id: &str) -> Result<()> {
        ensure!(
            !self.checkpoint_pending(id).await?,
            "An interrupted restore needs review in Checkpoints before work can resume"
        );
        Ok(())
    }
    pub(super) async fn checkpoint_capture(
        &self,
        id: &str,
        turn: &str,
        phase: &str,
        label: &str,
    ) -> Result<Option<Checkpoint>> {
        if !self.embedded_runtime
            || self
                .sessions
                .lock()
                .await
                .get(id)
                .is_some_and(|s| s.ephemeral)
        {
            return Ok(None);
        }
        self.checkpoint_ready(id).await?;
        let conversation = storage::read(&manager(), id).await?;
        let (connection, workspace) = self.checkpoint_context(id).await?;
        Ok(Some(
            disk::capture(
                &connection,
                workspace,
                capture_record(id, turn, phase, label, &conversation)?,
            )
            .await?,
        ))
    }
    async fn checkpoint_idle(&self, id: &str) -> Result<()> {
        let sessions = self.sessions.lock().await;
        let s = sessions.get(id).context("Open the conversation first")?;
        ensure!(
            !s.ephemeral && s.active.is_none(),
            "Wait for the current turn before using checkpoints"
        );
        Ok(())
    }
    async fn checkpoint_isolated(&self, id: &str) -> Result<()> {
        let binding = self
            .binding_record(id)
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        let Some(workspace) = binding.workspace else {
            anyhow::bail!("No virtual workspace is bound to this conversation")
        };
        // Include closed conversations, not just loaded UI sessions.
        let mut entries = tokio::fs::read_dir(&self.queue_root).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".binding.json") {
                continue;
            }
            if entry.path() == self.queue_path(id).with_extension("binding.json") {
                continue;
            }
            ensure!(
                entry.metadata().await?.len() <= 64 * 1024,
                "Stored workspace binding exceeds limit"
            );
            let other: workspace::BindingRecord =
                serde_json::from_slice(&tokio::fs::read(entry.path()).await?)?;
            if let Some(other) = other.workspace {
                ensure!(
                    other.selection != workspace.selection,
                    "This workspace is shared with another conversation; fork into an isolated workspace before restoring files"
                );
            }
        }
        Ok(())
    }
    async fn checkpoint_target(
        connection: &Connection,
        point: &Checkpoint,
        current: Conversation,
    ) -> Result<Conversation> {
        if point.capture.phase == "restore-backup" {
            let backup = journal::get(
                connection,
                point.capture.session.clone(),
                point.capture.id.clone(),
            )
            .await?;
            Ok(serde_json::from_value(backup.original)?)
        } else {
            storage::prefix(
                current,
                &serde_json::from_value(point.capture.boundary.clone())?,
            )
        }
    }
    pub(super) async fn checkpoint_request(
        self: &Arc<Self>,
        params: &Value,
        action: &str,
    ) -> Reply {
        if !self.embedded_runtime {
            return Err(unsupported("Checkpoints"));
        }
        let id = session_id(text(params, "threadId"))?.to_owned();
        let gate = self.gate(&id).await;
        let lock = gate.lock().await;
        self.deletion_ready(&id).await?;
        let result = self
            .checkpoint_request_locked(&id, params, action)
            .await
            .map_err(|e| error(format!("{e:#}")));
        drop(lock);
        if result.as_ref().is_ok_and(|r| r["reload"] == true) {
            // Loading outside the store guard avoids vwork MCP initialization deadlocks.
            let loaded = self
                .load_session(&json!({"threadId":ui_id(&id)}), true)
                .await?;
            return Ok(json!({"reload":true,"thread":loaded["thread"]}));
        }
        result
    }
    async fn checkpoint_request_locked(
        &self,
        id: &str,
        params: &Value,
        action: &str,
    ) -> Result<Value> {
        let (connection, workspace) = self.checkpoint_context(id).await?;
        if action == "list" {
            let page = disk::list(
                &connection,
                id.into(),
                params["before"].as_str().map(str::to_owned),
                50,
            )
            .await?;
            let pending = journal::pending(&connection, id.into()).await?;
            return Ok(
                json!({"page":page,"pending":pending.map(|j|json!({"id":j.backup.capture.id,"target":j.target.capture.label,"files":j.files,"conversation":j.conversation}))}),
            );
        }
        self.checkpoint_idle(id).await?;
        if action == "capture" {
            return Ok(
                json!({"checkpoint":self.checkpoint_capture(id,"","manual",params["label"].as_str().filter(|s|!s.trim().is_empty()).unwrap_or("Manual checkpoint")).await?}),
            );
        }
        if action == "review" {
            let review =
                disk::review(&connection, id.into(), text(params, "id").into(), workspace).await?;
            let current = storage::read(&manager(), id).await?;
            let current_boundary = storage::boundary(&current)?;
            let target = Self::checkpoint_target(&connection, &review.checkpoint, current).await;
            let (count, reason) = match target {
                Ok(c) => (Some(c.len()), String::new()),
                Err(e) => (None, e.to_string()),
            };
            let pending = journal::pending(&connection, id.into()).await?;
            return Ok(
                json!({"review":review,"current":current_boundary,"targetCount":count,"conversationError":reason,"pending":pending.map(|j|j.backup.capture.id)}),
            );
        }
        ensure!(
            matches!(action, "restore" | "rollback" | "keep"),
            "Unknown checkpoint action"
        );
        let review: Review = serde_json::from_value(params["review"].clone())?;
        ensure!(
            review.checkpoint.capture.session == id && review.checkpoint.workspace == workspace,
            "Checkpoint belongs to a different conversation or workspace"
        );
        let expected: Boundary = serde_json::from_value(params["current"].clone())?;
        let original = storage::read(&manager(), id).await?;
        ensure!(
            storage::boundary(&original)? == expected,
            "Conversation changed after review; review it again"
        );
        let runtime = tokio::runtime::Handle::current();
        let session = id.to_owned();
        self.pause_agent_queue(id)
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        if action == "restore" {
            let files = params["files"] == true;
            let conversation = params["conversation"] == true;
            ensure!(
                files || conversation,
                "Choose files or conversation to restore"
            );
            if files {
                self.checkpoint_isolated(id).await?;
            }
            self.checkpoint_ready(id).await?;
            let target = if conversation {
                Some(
                    Self::checkpoint_target(&connection, &review.checkpoint, original.clone())
                        .await?,
                )
            } else {
                None
            };
            let backup = disk::capture(
                &connection,
                workspace,
                capture_record(
                    id,
                    "",
                    "restore-backup",
                    "Before restore — recoverable backup",
                    &original,
                )?,
            )
            .await?;
            let record = journal::Journal {
                backup,
                target: review.checkpoint.clone(),
                files,
                conversation,
                original: serde_json::to_value(original)?,
                status: "pending".into(),
            };
            journal::apply(&connection, review, record, move || {
                runtime.block_on(async move {
                    // Deliberate fault injection requires the explicitly enabled UI automation harness.
                    if std::env::var_os("CODEX_GUI_AUTOMATION").is_some()
                        && std::env::var_os("FASTROCK_CHECKPOINT_TEST_FAIL").as_deref()
                            == Some(std::ffi::OsStr::new("1"))
                    {
                        anyhow::bail!("Injected fixture interruption after file restore");
                    }
                    if let Some(target) = target {
                        storage::replace(&manager(), &session, &expected, &target).await?;
                    } else {
                        ensure!(
                            storage::boundary(&storage::read(&manager(), &session).await?)?
                                == expected,
                            "Conversation changed during restore"
                        );
                    }
                    Ok(())
                })
            })
            .await?;
        } else {
            let pending = journal::pending(&connection, id.into())
                .await?
                .context("No interrupted restore needs recovery")?;
            if action == "rollback" && pending.files {
                self.checkpoint_isolated(id).await?;
            }
            let rollback = action == "rollback";
            journal::resolve(
                &connection,
                review,
                pending.backup.capture.id.clone(),
                rollback,
                move |j| {
                    runtime.block_on(async {
                        if rollback && j.conversation {
                            storage::replace(
                                &manager(),
                                &session,
                                &expected,
                                &serde_json::from_value(j.original.clone())?,
                            )
                            .await?;
                        } else {
                            ensure!(
                                storage::boundary(&storage::read(&manager(), &session).await?)?
                                    == expected,
                                "Conversation changed after review"
                            );
                        }
                        Ok(())
                    })
                },
            )
            .await?;
        }
        if let Some(s) = self.sessions.lock().await.get_mut(id) {
            s.loaded = false;
            s.turns.clear();
        }
        if let Some(service) = &self.summaries {
            service.completed(id.into()).await;
        }
        Ok(json!({"reload":true}))
    }
}
