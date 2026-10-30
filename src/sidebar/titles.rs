//! Explicit, sequential batches keep provider work bounded and the UI responsive.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub(super) struct Controller {
    pub running: bool,
    pub pending: HashSet<String>,
    pub status: String,
    cancel: Arc<AtomicBool>,
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl AppController {
    pub(super) fn sidebar_cancel_titles(&mut self) {
        self.sidebar.titles.cancel.store(true, Ordering::Relaxed);
        self.sidebar.titles.status =
            "Finishing the current title; remaining requests canceled".into();
        self.organization_render();
    }
    pub(super) fn sidebar_regenerate_titles(&mut self, mut ids: Vec<String>) {
        if self.sidebar.titles.running {
            self.toast("Wait for this title batch to finish, or cancel its remaining requests");
            return;
        }
        ids.retain(|id| self.sidebar.row_index.contains_key(id));
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return;
        }
        if ids.len() > 1000 {
            self.toast("Select at most 1,000 conversations for a title batch");
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.sidebar.titles.cancel = cancel.clone();
        self.sidebar.titles.running = true;
        self.sidebar.titles.pending = ids.iter().cloned().collect();
        self.sidebar.titles.status = "Generating titles with the cheap model…".into();
        self.sidebar_render();
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let total = ids.len();
            let mut done = 0;
            let mut failed = 0;
            let mut last_error = String::new();
            for id in ids {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                match backend
                    .goose_request(
                        "fastrock/thread/regenerateTitle",
                        serde_json::json!({"threadId":id}),
                    )
                    .await
                {
                    Ok(_) => done += 1,
                    Err(error) => {
                        failed += 1;
                        last_error = error.chars().take(512).collect();
                    }
                }
                let detail = format!("Generated {done} of {total} titles; {failed} failed");
                crate::ui_thread::post(move |app| {
                    app.sidebar.titles.pending.remove(&id);
                    app.sidebar.titles.status = detail;
                    app.sidebar_render();
                });
            }
            let remaining = total - done - failed;
            crate::ui_thread::post(move |app| {
                app.sidebar.titles.running = false;
                app.sidebar.titles.pending.clear();
                app.sidebar.titles.status = format!(
                    "Generated {done} titles; {failed} failed; {remaining} canceled{}",
                    if last_error.is_empty() {
                        String::new()
                    } else {
                        format!(". {last_error}")
                    }
                );
                app.sidebar_render();
            });
        });
    }
}
