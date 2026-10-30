//! Conversation and virtual-workspace forks share the existing in-process owners.
use super::*;
use fastrock_workspaces::{embedded::Connection, session::Selection, vwork::McpBinding};
use goose_runtime::session::{SessionManager, SessionType};

impl Adapter {
    pub(super) async fn fork_session(self: &Arc<Self>, params: &Value) -> Reply {
        self.fork_bound(params, None).await
    }

    pub(super) async fn fork_bound(
        self: &Arc<Self>,
        params: &Value,
        delegation: Option<super::delegation::Delegation>,
    ) -> Reply {
        if !self.embedded_runtime {
            return Err(unsupported("Fork requires embedded Goose"));
        }
        let side = params["ephemeral"] == true && delegation.is_none();
        let source_id = session_id(text(params, "threadId"))?;
        self.load_session(&json!({"threadId":ui_id(source_id)}), false)
            .await?;
        let gate = self.gate(source_id).await;
        let _lock = gate.lock().await;
        if let Some(task) = &delegation {
            if let Some(child) = self.delegation_index(source_id).await?.get(&task.task_id) {
                let manager = SessionManager::new(goose_runtime::config::paths::Paths::data_dir());
                let stored = manager
                    .get_session(child, false)
                    .await
                    .map_err(|_| error("Saved delegate is unavailable; do not repeat the task"))?;
                if stored.session_type == SessionType::Hidden {
                    return Err(error(
                        "Delegate setup was interrupted; inspect the retained child before retrying",
                    ));
                }
                let record = self.binding_record(child).await?;
                if record
                    .delegation
                    .as_ref()
                    .is_none_or(|old| !old.same_task(task))
                {
                    return Err(error(
                        "Task identity already exists with different instructions or settings",
                    ));
                }
                return Ok(json!({"thread":{"id":ui_id(child)},"reused":true}));
            }
        }
        let (binding, before, parent_active) = {
            let mut sessions = self.sessions.lock().await;
            let source = sessions
                .get_mut(source_id)
                .ok_or_else(|| error("Source conversation unavailable"))?;
            if source.ephemeral || (!side && delegation.is_none() && source.active.is_some()) {
                return Err(error(
                    "Finish the current turn before forking a persistent conversation",
                ));
            }
            if let Some(task) = &delegation {
                let allowed = if task.scheduled {
                    source.active.is_none()
                } else {
                    source.active.as_deref() == Some(task.source_turn.as_str())
                        && !source.cancel_requested
                };
                if !allowed {
                    return Err(error("The delegating turn has stopped or changed"));
                }
            }
            let before = if let Some(turn_id) = params["beforeTurnId"].as_str() {
                let boundary = (0..source.turns.len()).step_by(200).find_map(|offset| {
                    source
                        .turns
                        .turns_page(offset, 200, false)
                        .into_iter()
                        .find(|t| t["id"] == turn_id)
                        .and_then(|t| t["gooseMessageId"].as_str().map(str::to_string))
                });
                Some(boundary.ok_or_else(|| {
                    error("Reload this conversation before forking at the selected turn")
                })?)
            } else {
                None
            };
            (source.binding.clone(), before, source.active.is_some())
        };
        let workspace = binding.workspace;
        if workspace.is_none() && !side {
            return Err(error(
                "Open a conversation from a virtual workspace before forking; this conversation has no workspace binding",
            ));
        }
        // Validate all persisted inputs before creating a workspace or conversation.
        let bindings_path = self.queue_path(source_id).with_extension("vwork.json");
        let mut bindings: Vec<McpBinding> = read_saved(&bindings_path, 1024 * 1024)
            .await?
            .unwrap_or_default();
        if workspace.is_some() && bindings.is_empty() {
            return Err(error("Source conversation has no saved workspace tools"));
        }
        let mut tools: Option<Vec<Value>> = read_saved(
            &self.queue_path(source_id).with_extension("tools.json"),
            8 * 1024 * 1024,
        )
        .await?;
        let manager = SessionManager::new(goose_runtime::config::paths::Paths::data_dir());
        let mut source = manager
            // Delegates need configuration, never the parent's history. Avoid
            // decoding a huge conversation just to discard it below.
            .get_session(source_id, delegation.is_none())
            .await
            .map_err(|_| error("Cannot read the stored source conversation"))?;
        if delegation.is_some() {
            // Never copy a pending tool invocation into another conversation.
            source.conversation = None;
        }
        if side {
            let unbound: Vec<_> = bindings
                .iter()
                .filter(|b| b.workspace.is_none())
                .map(|b| b.name.clone())
                .collect();
            bindings.retain(|b| b.workspace.is_some());
            super::side::reference_configuration(&mut source, &unbound)
                .map_err(|e| error(e.to_string()))?;
            super::side::reference_history(&mut source, parent_active);
            if let Some(tools) = &mut tools {
                super::side::filter_tools(tools);
            }
        }
        if let Some(message) = &before {
            if !source
                .conversation
                .as_ref()
                .is_some_and(|c| c.messages().iter().any(|m| m.id.as_ref() == Some(message)))
            {
                return Err(error(
                    "The selected turn no longer has a stored message; reload before forking",
                ));
            }
        }
        let mut primary = None;
        let mut forks: HashMap<(std::path::PathBuf, String), String> = HashMap::new();
        if bindings.iter().any(|b| b.workspace.is_none()) {
            return Err(error(
                "Every workspace extension must have an explicit workspace before forking",
            ));
        }
        for binding in &mut bindings {
            let home = tokio::fs::canonicalize(&binding.home)
                .await
                .map_err(|_| error("Workspace store is unavailable"))?;
            let parent = binding
                .workspace
                .as_ref()
                .ok_or_else(|| {
                    error(
                        "Every workspace extension must have an explicit workspace before forking",
                    )
                })?
                .clone();
            let key = (home.clone(), parent.clone());
            let child: String = if let Some(child) = forks.get(&key) {
                child.clone()
            } else {
                let child_name = format!("fork-{}", uuid::Uuid::new_v4().simple());
                Connection::new(home.clone())
                    .call(
                        "workspace_fork".into(),
                        json!({"workspace":parent,"name":child_name}),
                    )
                    .await
                    .map_err(|e| error(format!("Cannot fork workspace: {e:#}")))?;
                forks.insert(key, child_name.clone());
                child_name.clone()
            };
            if workspace.as_ref().is_some_and(|workspace| {
                home == workspace.selection.home && parent == workspace.selection.workspace
            }) {
                primary = Some(
                    Selection {
                        home,
                        workspace: child.clone(),
                    }
                    .resolve()
                    .await
                    .map_err(|e| error(format!("Resolve child workspace: {e:#}")))?,
                );
            }
            binding.workspace = Some(child);
        }
        if workspace.is_some() && primary.is_none() {
            return Err(error(
                "Source workspace binding does not match its saved tools",
            ));
        }
        let cwd = match &primary {
            Some(workspace) => workspace.cwd.clone(),
            None => {
                let cwd = self
                    .queue_root
                    .join("side-workdirs")
                    .join(uuid::Uuid::new_v4().to_string());
                tokio::fs::create_dir_all(&cwd)
                    .await
                    .map_err(|e| error(format!("Create side directory: {e}")))?;
                tokio::fs::canonicalize(cwd)
                    .await
                    .map_err(|e| error(e.to_string()))?
            }
        };
        let reference_cwd = source.working_dir.clone();
        let child = super::fork_storage::copy_hidden(&manager, source, cwd, before.as_deref())
            .await
            .map_err(|e| {
                error(format!(
                    "Fork conversation: {e:#}. The isolated workspace was retained for recovery"
                ))
            })?;
        if let Some(workspace) = &primary {
            manager
                .set_system_prompt_extra(
                    &child,
                    "fastrock-workspace",
                    Some(super::workspace::instructions(workspace)),
                )
                .await
                .map_err(|_| error("Could not save child workspace instructions"))?;
        }
        if side {
            if primary.is_none() {
                manager.set_system_prompt_extra(&child, "fastrock-side-reference", Some(format!(
                    "The parent conversation worked in {}. Treat that directory as reference only; your own current directory is isolated. Do not change parent files without an explicit request in this side chat.", reference_cwd.display()
                ))).await.map_err(|e|error(e.to_string()))?;
            }
            super::side::prepare(&manager, &child)
                .await
                .map_err(|e| error(format!("Prepare side boundary: {e:#}")))?;
        }
        self.save_vwork_bindings(&child, &bindings)?;
        self.save_binding_record(
            &child,
            &super::workspace::BindingRecord {
                workspace: primary,
                forked_from: delegation.is_none().then(|| source_id.into()),
                delegation: delegation.clone(),
                side,
            },
        )?;
        if let Some(tools) = tools {
            crate::rally::store::atomic_write(
                &self.queue_path(&child).with_extension("tools.json"),
                &serde_json::to_vec(&tools).map_err(|e| error(e.to_string()))?,
            )
            .map_err(|e| error(format!("Save fork tools: {e}")))?;
        }
        if let Some(task) = &delegation {
            manager
                .update(&child)
                .parent_session_id(Some(source_id.into()))
                .system_generated_name(format!(
                    "Task: {}",
                    task.label.as_deref().unwrap_or(&task.task_id)
                ))
                .apply()
                .await
                .map_err(|_| error("Save delegate identity failed"))?;
            self.save_delegation_index(source_id, &task.task_id, &child)
                .await?;
        }
        // Open side chats remain hidden but durable for restart recovery.
        if !side {
            manager
                .update(&child)
                .session_type(SessionType::User)
                .apply()
                .await
                .map_err(|_| error("Fork saved but could not be made visible"))?;
        }
        let result = self
            .load_session(&json!({"threadId":ui_id(&child)}), true)
            .await?;
        self.emit(json!({"method":"thread/started","params":{"thread":result["thread"]}}))?;
        Ok(result)
    }
}

pub(super) async fn read_saved<T: serde::de::DeserializeOwned>(
    path: &std::path::Path,
    limit: u64,
) -> Result<Option<T>, JSONRPCErrorError> {
    match tokio::fs::metadata(path).await {
        Ok(meta) if meta.len() <= limit => serde_json::from_slice(
            &tokio::fs::read(path)
                .await
                .map_err(|e| error(e.to_string()))?,
        )
        .map(Some)
        .map_err(|_| error("Invalid saved source configuration; preserve it for recovery")),
        Ok(_) => Err(error(
            "Saved source configuration exceeds the supported size",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(error(e.to_string())),
    }
}
