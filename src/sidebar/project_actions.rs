//! Project editor: metadata IO stays on the shared embedded runtime.
use super::*;
use crate::ui::ProjectState;
use fastrock_workspaces::{projects::Details, session::WorkspaceSession};

#[derive(Default)]
pub(super) struct Editor {
    generation: u64,
    path: PathBuf,
    details: Option<Details>,
    busy: bool,
}

impl AppController {
    pub(crate) fn project_open_active(&mut self) {
        if let Some(path) = self
            .active_thread_index()
            .and_then(|i| self.thread_tab(i))
            .map(|thread| {
                thread
                    .extras
                    .project_id
                    .as_ref()
                    .and_then(|id| self.sidebar.projects.iter().find(|p| &p.id == id))
                    .map(|p| p.root.clone())
                    .unwrap_or_else(|| thread.cwd.clone())
            })
        {
            self.project_open_editor(path);
        }
    }
    pub(crate) fn project_automation(&mut self, args: &[String]) {
        use slint::Model;
        let state = self.window.global::<ProjectState>();
        match args.first().map(String::as_str) {
            Some("open") => {
                if let Some(path) = args.get(1) {
                    self.project_open_editor(path.into());
                }
            }
            Some("reference") => {
                if let Some(value) = args.get(1) {
                    state.set_reference(value.as_str().into());
                }
            }
            Some("action") => {
                if let Some(action) = args.get(1) {
                    state.invoke_action(action.as_str().into());
                }
            }
            Some("branch") => {
                if let Some(index) = args.get(1).and_then(|v| v.parse().ok()) {
                    state.invoke_select_branch(index);
                }
            }
            Some("dump") => {
                if let Some(path) = args.get(1) {
                    let path = path.clone();
                    let data = serde_json::json!({"open":state.get_open(),"busy":state.get_busy(),"ready":state.get_ready(),"name":state.get_name().as_str(),"path":state.get_path().as_str(),"reference":state.get_reference().as_str(),"branches":state.get_branches().iter().map(|s|s.to_string()).collect::<Vec<_>>(),"error":state.get_error().as_str(),"status":state.get_status().as_str(),"revision":self.sidebar.project_editor.details.as_ref().map(|d|d.settings.revision)}).to_string();
                    self.backend.spawn(async move {
                        let _ = tokio::fs::write(path, data).await;
                    });
                }
            }
            _ => {}
        }
    }
    pub(super) fn project_actions_bind(&mut self) {
        use slint::Model;
        let state = self.window.global::<ProjectState>();
        state.on_reference_index(|branches, reference| {
            branches
                .iter()
                .position(|value| value == reference)
                .map_or(-1, |i| i as i32)
        });
        state.on_action(crate::ui_thread::scoped_callback!(|action| {
            let action = action.to_string();
            crate::ui_thread::with_app(move |app| app.project_action(&action));
        }));
        state.on_select_branch(crate::ui_thread::scoped_callback!(|index| {
            crate::ui_thread::with_app(move |app| {
                let Some(details) = app.sidebar.project_editor.details.as_ref() else {
                    return;
                };
                let reference = if index == 0 {
                    Some("HEAD")
                } else {
                    details
                        .branches
                        .get(index.saturating_sub(1) as usize)
                        .map(String::as_str)
                };
                if let Some(reference) = reference {
                    app.window
                        .global::<ProjectState>()
                        .set_reference(reference.into());
                }
            });
        }));
    }

    pub(super) fn project_open_editor(&mut self, path: PathBuf) {
        self.sidebar.project_editor.path = path;
        self.sidebar.project_editor.details = None;
        self.sidebar.project_editor.busy = false;
        let state = self.window.global::<ProjectState>();
        state.set_open(true);
        state.set_ready(false);
        state.set_name(crate::app::folder_label(&self.sidebar.project_editor.path).into());
        state.set_path(
            self.sidebar
                .project_editor
                .path
                .to_string_lossy()
                .as_ref()
                .into(),
        );
        state.set_reference("HEAD".into());
        state.set_branches(ModelRc::default());
        self.project_editor_request(false);
    }

    fn project_editor_request(&mut self, save: bool) {
        let editor = &mut self.sidebar.project_editor;
        if editor.busy {
            return;
        }
        editor.busy = true;
        editor.generation = editor.generation.wrapping_add(1);
        let generation = editor.generation;
        let state = self.window.global::<ProjectState>();
        state.set_busy(true);
        state.set_error("".into());
        state.set_status("".into());
        let params = serde_json::json!({
            "path":editor.path,
            "projectId":editor.details.as_ref().map(|d| &d.project.id),
            "expectedRevision":editor.details.as_ref().map(|d|d.settings.revision),
            "reference":state.get_reference().as_str(),
        });
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = backend
                .goose_request(
                    if save {
                        "fastrock/project/settings"
                    } else {
                        "fastrock/project/details"
                    },
                    params,
                )
                .await
                .and_then(|v| serde_json::from_value::<Details>(v).map_err(|e| e.to_string()));
            crate::ui_thread::post(move |app| {
                if app.sidebar.project_editor.generation != generation {
                    return;
                }
                app.sidebar.project_editor.busy = false;
                let state = app.window.global::<ProjectState>();
                state.set_busy(false);
                match result {
                    Ok(details) => {
                        state.set_ready(true);
                        state.set_name(app.sidebar_project_name(&details.project).into());
                        state.set_path(
                            dunce::simplified(&details.project.root)
                                .to_string_lossy()
                                .as_ref()
                                .into(),
                        );
                        state.set_reference(details.settings.default_reference.as_str().into());
                        state.set_branches(ModelRc::new(VecModel::from(
                            std::iter::once("HEAD".into())
                                .chain(details.branches.iter().map(|s| s.as_str().into()))
                                .collect::<Vec<slint::SharedString>>(),
                        )));
                        if save {
                            state.set_status("Default saved".into());
                        }
                        app.sidebar.project_editor.path = details.project.root.clone();
                        app.sidebar.project_editor.details = Some(details);
                        app.sidebar_refresh_projects();
                    }
                    Err(error) => state.set_error(error.into()),
                }
            });
        });
    }

    fn project_action(&mut self, action: &str) {
        if action == "close" {
            // Invalidate delayed UI replies; a requested durable save still completes.
            self.sidebar.project_editor.generation =
                self.sidebar.project_editor.generation.wrapping_add(1);
            self.sidebar.project_editor.busy = false;
            self.window.global::<ProjectState>().set_open(false);
            self.restore_focus();
            return;
        }
        if self.sidebar.project_editor.busy {
            return;
        }
        let path = self.sidebar.project_editor.path.clone();
        match action {
            "reload" => self.project_editor_request(false),
            "save" => self.project_editor_request(true),
            "copy-path" => self.copy_to_clipboard(&path.to_string_lossy()),
            "open-folder" => self.open_in_file_manager(&path),
            "rename" => {
                self.project_action("close");
                self.organization_rename_project(&path);
            }
            "new-thread" => {
                let state = self.window.global::<ProjectState>();
                let reference = state.get_reference().to_string();
                let project_id = self
                    .sidebar
                    .project_editor
                    .details
                    .as_ref()
                    .map(|d| d.project.id.clone());
                state.set_busy(true);
                state.set_error("".into());
                self.sidebar.project_editor.busy = true;
                self.sidebar.project_editor.generation =
                    self.sidebar.project_editor.generation.wrapping_add(1);
                let generation = self.sidebar.project_editor.generation;
                let backend = self.backend.clone();
                self.backend.spawn(async move {
                    let result = backend.goose_request("fastrock/project/open", serde_json::json!({"path":path,"reference":reference,"projectId":project_id})).await
                        .and_then(|v|serde_json::from_value::<WorkspaceSession>(v).map_err(|e|e.to_string()));
                    crate::ui_thread::post(move |app| {
                        // A user-requested new chat remains owned even if the editor closed.
                        let current = app.sidebar.project_editor.generation == generation;
                        if current {
                            app.sidebar.project_editor.busy = false;
                            app.window.global::<ProjectState>().set_busy(false);
                        }
                        match result {
                            Ok(workspace) => {
                                if current { app.window.global::<ProjectState>().set_open(false); }
                                app.start_thread_in_workspace(workspace);
                                app.sidebar_refresh();
                            }
                            Err(error) if current => app.window.global::<ProjectState>().set_error(error.into()),
                            Err(error) => app.toast(error),
                        }
                    });
                });
            }
            _ => {}
        }
    }

    pub(super) fn project_copy_reference(&mut self, thread_id: String) {
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = backend
                .goose_request(
                    "fastrock/thread/location",
                    serde_json::json!({"threadId":thread_id}),
                )
                .await;
            crate::ui_thread::post(move |app| match result {
                Ok(value) => {
                    if let Some(reference) = value["reference"].as_str() {
                        app.copy_to_clipboard(reference);
                    }
                }
                Err(error) => app.toast(error),
            });
        });
    }
}
