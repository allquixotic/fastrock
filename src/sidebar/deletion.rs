//! Explicit, guarded history deletion. Never discard an unsent draft.
use super::*;
use crate::app::{ThreadPhase, ThreadTab};

fn local_guard(thread: &ThreadTab) -> Option<&'static str> {
    if thread.extras.deleting {
        Some("deletion is already pending")
    } else if thread.extras.side_parent.is_some() {
        Some("close this temporary side-chat tab to discard it")
    } else if thread.is_busy()
        || thread.phase == ThreadPhase::Starting
        || thread.extras.turn_starts_in_flight > 0
        || thread.extras.recap.is_some()
    {
        Some("stop or finish active work first")
    } else if !thread.composer.text.is_empty()
        || !thread.composer.attachments.is_empty()
        || thread.composer.preparing.is_some()
        || thread.extras.editing_pending.is_some()
    {
        Some("save or clear the unsent draft first")
    } else if !thread.pending_inputs.is_empty() || !thread.recovered_sends.is_empty() {
        Some("review pending or recovered messages first")
    } else {
        None
    }
}

impl AppController {
    pub(super) fn sidebar_sync_deletions(&self) {
        if !self.backend.is_embedded() {
            return;
        }
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let mut after = None::<String>;
            loop {
                let page = match backend
                    .goose_request(
                        "fastrock/thread/deletions",
                        serde_json::json!({"after":after}),
                    )
                    .await
                {
                    Ok(page) => page,
                    Err(error) => {
                        crate::ui_thread::post(move |app| {
                            app.toast(format!("Deleted conversation cleanup unavailable: {error}"))
                        });
                        return;
                    }
                };
                let ids: Vec<String> = page["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|id| id.as_str().map(str::to_owned))
                    .collect();
                crate::ui_thread::post(move |app| {
                    for thread_id in ids {
                        app.sidebar_on_notification(&ServerNotification::ThreadDeleted(
                            codex_app_server_protocol::ThreadDeletedNotification { thread_id },
                        ));
                    }
                });
                let next = page["nextCursor"].as_str().map(str::to_owned);
                if next.is_none() || next == after {
                    break;
                }
                after = next;
            }
        });
    }

    fn deletion_local_guard(&mut self, id: &str) -> Result<(), String> {
        if self.terminal_delete_guard(id) {
            return Err("close the conversation's terminal first".into());
        }
        if let Some(index) = self.tab_index_for_thread(id) {
            if self.active == Some(index) {
                self.composer_stash();
            }
            if self.approvals_pending_count(index) > 0 {
                return Err("answer pending requests first".into());
            }
            if let Some(reason) = self.thread_tab(index).and_then(local_guard) {
                return Err(reason.into());
            }
        }
        Ok(())
    }
    fn deletion_reserve(&mut self, id: &str) -> Result<(), String> {
        self.deletion_local_guard(id)?;
        let own = self.backend.window_id();
        for window in crate::ui_thread::ids().into_iter().filter(|w| *w != own) {
            // Fail closed if another controller is temporarily borrowed.
            let mut checked = Err("another window is busy; retry Delete".into());
            crate::ui_thread::with_app_now_for(window, |app| {
                checked = app.deletion_local_guard(id)
            });
            checked?;
        }
        for window in crate::ui_thread::ids().into_iter().filter(|w| *w != own) {
            crate::ui_thread::with_app_now_for(window, |app| app.deletion_flag(id, true));
        }
        self.deletion_flag(id, true);
        Ok(())
    }
    fn deletion_flag(&mut self, id: &str, value: bool) {
        if let Some(index) = self.tab_index_for_thread(id) {
            if let Some(thread) = self.thread_tab_mut(index) {
                thread.extras.deleting = value;
            }
        }
    }
    pub(super) fn sidebar_delete_many(&mut self, mut ids: Vec<String>) {
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return;
        }
        let title = if ids.len() == 1 {
            self.sidebar
                .threads
                .iter()
                .find(|t| t.id == ids[0])
                .map(|t| t.title.clone())
                .unwrap_or_else(|| "this conversation".into())
        } else {
            format!("{} selected conversations", ids.len())
        };
        self.show_dialog(
            DialogRequest::confirm("Delete permanently?", format!(
                "Delete {title} and its history, search summary and checkpoints? This cannot be undone. Repository files and workspaces stay intact. Conversations with drafts or unfinished work will be kept."
            )).accept_label("Delete").destructive(),
            Box::new(move |app, accepted| {
                if accepted.is_some() { app.deletion_next(ids.into(), 0, Vec::new()); }
            }),
        );
    }
    fn deletion_next(
        &mut self,
        mut ids: std::collections::VecDeque<String>,
        done: usize,
        mut failed: Vec<String>,
    ) {
        let Some(id) = ids.pop_front() else {
            let message = if failed.is_empty() {
                format!("Deleted {done} conversation(s).")
            } else {
                format!(
                    "Deleted {done}; kept {}. {}",
                    failed.len(),
                    failed
                        .iter()
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            };
            self.show_dialog(
                DialogRequest::confirm("Deletion results", message).accept_label("OK"),
                Box::new(|_, _| {}),
            );
            return;
        };
        if let Err(reason) = self.deletion_reserve(&id) {
            failed.push(format!("{id}: {reason}"));
            // Avoid recursive stacks for a large guarded selection.
            crate::ui_thread::post(move |app| app.deletion_next(ids, done, failed));
            return;
        }
        let params = ThreadDeleteParams {
            thread_id: id.clone(),
        };
        self.backend.call(
            |request_id| ClientRequest::ThreadDelete { request_id, params },
            move |app, result: Result<ThreadDeleteResponse, BackendError>| {
                let success = result.is_ok();
                if success {
                    app.sidebar_deleted(&id);
                } else if let Err(error) = result {
                    failed.push(format!("{id}: {}", error.user_message()));
                }
                let released = id.clone();
                crate::ui_thread::post_all(move |app| {
                    app.deletion_flag(&released, false);
                    if success {
                        app.sidebar_deleted(&released);
                    }
                });
                app.deletion_flag(&id, false);
                app.deletion_next(ids, done + usize::from(success), failed);
            },
        );
    }
    pub(crate) fn sidebar_deleted(&mut self, id: &str) {
        if !self.sidebar.deleted.insert(id.into()) {
            return;
        }
        // Invalidate results fetched before deletion, including fuzzy search.
        self.sidebar.search_run = None;
        self.sidebar.generation += 1;
        self.sidebar.loading = false;
        self.sidebar.loading_more = false;
        self.organization_deselect(id);
        self.sidebar_remove_thread(id);
        self.composer_shared.goose_configs.remove(id);
        self.composer_shared.goose_recovery.remove(id);
        if let Some(index) = self.tab_index_for_thread(id) {
            if self.active == Some(index) {
                self.composer_stash();
            }
            let preserve = self.thread_tab(index).is_some_and(|t| {
                !t.composer.text.is_empty()
                    || !t.composer.attachments.is_empty()
                    || t.composer.preparing.is_some()
            });
            self.terminal_close_thread(id);
            if preserve {
                let old = self.thread_tab_mut(index).unwrap();
                let mut fresh = ThreadTab::new(old.cwd.clone());
                fresh.composer = std::mem::take(&mut old.composer);
                fresh.extras.workspace = old.extras.workspace.clone();
                fresh.extras.project_id = old.extras.project_id.clone();
                fresh.model = old.model.clone();
                fresh.effort = old.effort.clone();
                fresh.model_provider = old.model_provider.clone();
                fresh.turn_overrides = old.turn_overrides.clone();
                *old = fresh;
                self.start_thread_in_tab(index);
                self.toast("Conversation deleted. New draft preserved in a fresh conversation.");
            } else {
                self.thread_tab_mut(index).unwrap().thread_id = None;
                self.close_tab(index);
            }
        }
        self.organization_request(Some(organization_store::Mutation::Forget(vec![id.into()])));
        self.save_open_chats();
        self.sidebar_refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v77_deletion_rejects_local_work_without_discarding_drafts() {
        let mut tab = ThreadTab::new(PathBuf::from("fixture"));
        tab.phase = ThreadPhase::Idle;
        tab.thread_id = Some("goose:fixture".into());
        assert!(local_guard(&tab).is_none());
        tab.composer.text = "unsent".into();
        assert!(local_guard(&tab).unwrap().contains("draft"));
        assert_eq!(tab.composer.text, "unsent");
        tab.composer.text.clear();
        tab.extras.deleting = true;
        assert!(local_guard(&tab).unwrap().contains("pending"));
        tab.extras.deleting = false;
        tab.recovered_sends
            .push(serde_json::json!({"text":"uncertain"}));
        assert!(local_guard(&tab).unwrap().contains("recovered"));
        tab.recovered_sends.clear();
        tab.phase = ThreadPhase::Running;
        assert!(local_guard(&tab).unwrap().contains("active"));
    }
}
