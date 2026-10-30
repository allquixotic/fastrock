//! Optional virtual files. The daemon owns storage; the UI never opens redb.
mod browser;
#[cfg(test)]
mod browser_tests;

use crate::app::AppController;
use crate::ui::{AppState, VirtualWorkspaceWindow, WorkspaceRowData};
use browser::{Browser, Event, Work};
use slint::{ComponentHandle, ModelRc, VecModel};

#[derive(Default)]
pub(crate) struct Controller {
    browser: Browser,
    window: Option<VirtualWorkspaceWindow>,
    discovery: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Controller {
    fn drop(&mut self) {
        if let Some(task) = self.discovery.take() {
            task.abort();
        }
    }
}

impl AppController {
    pub(crate) fn virtual_workspaces_automation(&mut self, args: &[String]) {
        let Some(command) = args.first().map(String::as_str) else {
            return;
        };
        match command {
            "open" => self.virtual_workspaces_open(),
            "action" => {
                if let Some(action) = args.get(1) {
                    self.virtual_workspaces_action(action);
                }
            }
            "select" | "file" => {
                let index = args
                    .get(1)
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                let view = self.virtual_workspaces.browser.snapshot();
                let rows = if command == "select" {
                    view.catalog
                } else {
                    view.files
                };
                if let Some(row) = rows.get(index) {
                    self.virtual_workspaces_action(&row.action);
                }
            }
            "dump" => {
                if let Some(path) = args.get(1) {
                    let view = self.virtual_workspaces.browser.snapshot();
                    let value = serde_json::json!({"visible":view.visible,"dirty":view.dirty,"text":view.text,"status":view.status,"report":view.report,"title":view.title,"canCompact":view.can_compact,"databaseIndex":view.database_index,"mergeSource":view.merge_source,"canMerge":view.can_merge,"widgetCanMerge":self.virtual_workspaces.window.as_ref().is_some_and(|window| window.get_can_merge())});
                    let path = path.clone();
                    self.backend.spawn(async move {
                        let _ = tokio::fs::write(path, value.to_string()).await;
                    });
                }
            }
            "snapshot" => {
                if let (Some(path), Some(window)) = (args.get(1), &self.virtual_workspaces.window) {
                    match window.window().take_snapshot() {
                        Ok(buffer) => {
                            let path = path.clone();
                            self.backend.spawn(async move {
                                let _ = image::save_buffer(
                                    path,
                                    buffer.as_bytes(),
                                    buffer.width(),
                                    buffer.height(),
                                    image::ColorType::Rgba8,
                                );
                            });
                        }
                        Err(error) => tracing::warn!(%error,"Virtual workspace snapshot failed"),
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn virtual_workspaces_bind(&mut self) {
        self.window.global::<AppState>().on_open_virtual_workspaces(
            crate::ui_thread::scoped_callback!(|| {
                crate::ui_thread::with_app(Self::virtual_workspaces_open);
            }),
        );
        self.virtual_workspaces.discovery = Some(self.backend.spawn(async {
            loop {
                let event = browser::execute(Work::Discover).await;
                if !crate::ui_thread::post(move |app| app.virtual_workspaces_event(event)) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        }));
    }

    fn virtual_workspaces_open(&mut self) {
        if !self.virtual_workspaces.browser.snapshot().visible {
            return;
        }
        if self.virtual_workspaces.window.is_none() {
            let window = match VirtualWorkspaceWindow::new() {
                Ok(window) => window,
                Err(error) => {
                    self.push_warning(format!("Cannot open virtual workspaces: {error}"));
                    return;
                }
            };
            window.on_action(crate::ui_thread::scoped_callback!(|action| {
                crate::ui_thread::with_app(move |app| app.virtual_workspaces_action(&action));
            }));
            self.virtual_workspaces.window = Some(window);
        }
        self.virtual_workspaces_render();
        if let Some(window) = &self.virtual_workspaces.window {
            let _ = window.show();
        }
    }

    fn virtual_workspaces_action(&mut self, action: &str) {
        let work = self.virtual_workspaces.browser.action(action);
        if action.starts_with("virtual-edit:") {
            if let Some(window) = &self.virtual_workspaces.window {
                let (dirty, status) = self.virtual_workspaces.browser.edit_status();
                window.set_workspace_dirty(dirty);
                window.set_can_merge(false);
                window.set_workspace_status(status.into());
                if window.get_workspace_text().as_str() != self.virtual_workspaces.browser.text() {
                    window.set_workspace_text(self.virtual_workspaces.browser.text().into());
                }
            }
        } else {
            self.virtual_workspaces_render();
        }
        self.virtual_workspaces_enqueue(work);
    }

    fn virtual_workspaces_event(&mut self, event: Event) {
        let work = self.virtual_workspaces.browser.apply(event);
        self.virtual_workspaces_render();
        self.virtual_workspaces_enqueue(work);
    }

    fn virtual_workspaces_enqueue(&self, work: Vec<Work>) {
        for work in work {
            self.backend.spawn(async move {
                let event = browser::execute(work).await;
                crate::ui_thread::post(move |app| app.virtual_workspaces_event(event));
            });
        }
    }

    fn virtual_workspaces_render(&self) {
        let view = self.virtual_workspaces.browser.snapshot();
        self.window
            .global::<AppState>()
            .set_virtual_workspaces_available(view.visible);
        let Some(window) = &self.virtual_workspaces.window else {
            return;
        };
        if !view.visible {
            let _ = window.hide();
        }
        window.set_workspace_busy(view.busy);
        window.set_workspace_status(view.status.into());
        window.set_workspace_title(view.title.into());
        if window.get_workspace_text().as_str() != view.text {
            window.set_workspace_text(view.text.into());
        }
        window.set_workspace_dirty(view.dirty);
        window.set_workspace_editable(view.editable);
        window.set_workspace_selected(view.selected);
        window.set_workspace_can_create(view.can_create);
        window.set_workspace_has_next(view.has_next);
        window.set_workspace_has_previous(view.has_previous);
        window.set_workspace_report(view.report.into());
        window.set_merge_source(view.merge_source.into());
        window.set_can_merge(view.can_merge);
        window.set_database_can_compact(view.can_compact);
        window.set_database_index(view.database_index);
        window.set_database_labels(ModelRc::new(VecModel::from(
            view.databases
                .iter()
                .map(|d| slint::SharedString::from(d.label.as_str()))
                .collect::<Vec<_>>(),
        )));
        window.set_database_actions(ModelRc::new(VecModel::from(
            view.databases
                .iter()
                .map(|d| slint::SharedString::from(d.action.as_str()))
                .collect::<Vec<_>>(),
        )));
        let row = |item: browser::BrowserRow| WorkspaceRowData {
            label: item.label.into(),
            action: item.action.into(),
            selected: item.selected,
        };
        window.set_workspace_catalog(ModelRc::new(VecModel::from(
            view.catalog.into_iter().map(row).collect::<Vec<_>>(),
        )));
        window.set_workspace_files(ModelRc::new(VecModel::from(
            view.files.into_iter().map(row).collect::<Vec<_>>(),
        )));
    }
}
