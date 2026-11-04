//! Native progress UI receives worker storage events; no filesystem IO on UI.
use crate::{app::AppController, ui::AppState};
use slint::ComponentHandle;
use std::path::PathBuf;

impl AppController {
    pub(crate) fn storage_migration_bind(&self) {
        let window = self.window.as_weak();
        fastrock_workspaces::migration::observe(move |progress| {
            window
                .upgrade_in_event_loop(move |window| {
                    let state = window.global::<AppState>();
                    state.set_storage_migration_visible(true);
                    state.set_storage_migration_active(progress.active);
                    state.set_storage_migration_retryable(progress.error.is_some());
                    state.set_storage_migration_source(
                        progress.source.to_string_lossy().as_ref().into(),
                    );
                    state.set_storage_migration_indeterminate(progress.total == 0);
                    state.set_storage_migration_progress(if progress.total == 0 {
                        0.0
                    } else {
                        (progress.completed as f64 / progress.total as f64).clamp(0.0, 1.0) as f32
                    });
                    state.set_storage_migration_destination(
                        progress.destination.to_string_lossy().as_ref().into(),
                    );
                    state.set_storage_migration_message(
                        progress.error.unwrap_or(progress.phase).into(),
                    );
                })
                .is_ok()
        });
    }
    pub(crate) fn storage_migration_pick(&mut self) {
        let future = rfd::AsyncFileDialog::new()
            .set_title("Select legacy workspace database")
            .add_filter("Workspace database", &["redb"])
            .pick_file();
        let _ = crate::ui_thread::spawn_local(async move {
            let Some(source) = future.await else {
                return;
            };
            let Some(destination) = rfd::AsyncFileDialog::new()
                .set_title("Choose a workspace home without an existing Fjall store")
                .pick_folder()
                .await
            else {
                return;
            };
            let source = source.path().to_path_buf();
            let destination = destination.path().to_path_buf();
            crate::ui_thread::with_app(move |app| app.storage_migration_start(source, destination));
        });
    }
    pub(crate) fn storage_migration_start(&mut self, source: PathBuf, destination: PathBuf) {
        self.backend.spawn(async move {
            let result = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                let report = fastrock_workspaces::migration::migrate(&source, &destination)?;
                fastrock_workspaces::vwork::register_imported_store(&destination)?;
                Ok(report)
            })
            .await;
            let error = match result {
                Ok(Ok(_)) => None,
                Ok(Err(e)) => Some(format!("{e:#}")),
                Err(e) => Some(e.to_string()),
            };
            if let Some(error) = error {
                crate::ui_thread::post(move |app| {
                    app.toast(format!("Migration could not complete: {error}"))
                });
            }
        });
    }
}
