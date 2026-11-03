use crate::{
    app::AppController,
    ui::{LearnedMemoryRow, LearnedMemoryState},
};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
impl AppController {
    pub(super) fn memory_install(&mut self) {
        let state = self.window.global::<LearnedMemoryState>();
        state.on_action(crate::ui_thread::scoped_callback!(|action| {
            let action = action.to_string();
            crate::ui_thread::with_app(move |app| app.memory_action(&action));
        }));
        state.on_select(crate::ui_thread::scoped_callback!(|id| {
            let id = id.to_string();
            crate::ui_thread::with_app(move |app| app.memory_select(&id));
        }));
    }
    pub(crate) fn memory_load(&mut self) {
        self.memory_action("list");
    }
    fn memory_select(&self, id: &str) {
        let state = self.window.global::<LearnedMemoryState>();
        if state.get_busy() || state.get_dirty() {
            return;
        }
        if let Some(row) = state.get_rows().iter().find(|r| r.id == id) {
            state.set_selected(row.id);
            state.set_text(row.text);
            state.set_pinned(row.pinned);
            state.set_global(row.global);
        }
    }
    fn memory_action(&mut self, action: &str) {
        let state = self.window.global::<LearnedMemoryState>();
        if state.get_busy() {
            return;
        }
        if action == "discard" {
            state.set_dirty(false);
            self.memory_select(state.get_selected().as_str());
            return;
        }
        if state.get_dirty() && action != "update" {
            return;
        }
        let params = json!({"revision":state.get_revision().parse::<u64>().ok(),"id":state.get_selected().as_str(),
            "text":state.get_text().as_str(),"pinned":state.get_pinned(),"global":state.get_global(),
            "learning":if action == "toggle-learning" { !state.get_learning() } else { state.get_learning() },
            "recall":if action == "toggle-recall" { !state.get_recall() } else { state.get_recall() }});
        let action = if action.starts_with("toggle-") {
            "settings"
        } else {
            action
        }
        .to_owned();
        state.set_busy(true);
        state.set_status("".into());
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = backend
                .goose_request(&format!("fastrock/memory/{action}"), params)
                .await;
            crate::ui_thread::post(move |app| {
                let state = app.window.global::<LearnedMemoryState>();
                state.set_busy(false);
                match result {
                    Err(error) => state.set_status(error.into()),
                    Ok(value) => {
                        state.set_dirty(false);
                        state.set_confirm_clear(false);
                        publish(&state, &value);
                        let selected = state.get_selected().to_string();
                        if state.get_rows().iter().any(|r| r.id == selected) {
                            app.memory_select(&selected);
                        } else {
                            state.set_selected("".into());
                            state.set_text("".into());
                        }
                    }
                }
            });
        });
    }
    pub(crate) fn memory_automation(&mut self, args: &[String]) {
        let arg = |i| args.get(i).map(String::as_str).unwrap_or("");
        let state = self.window.global::<LearnedMemoryState>();
        match arg(0) {
            "select" => state.invoke_select(arg(1).into()),
            "select-first" => {
                if let Some(row) = state.get_rows().row_data(0) {
                    state.invoke_select(row.id);
                }
            }
            "text" => {
                state.set_text(arg(1).into());
                state.set_dirty(true);
            }
            "pin" => {
                state.set_pinned(arg(1) == "true");
                state.set_dirty(true);
            }
            "dump" => {
                let value = json!({"revision":state.get_revision().as_str(),"busy":state.get_busy(),"learning":state.get_learning(),"recall":state.get_recall(),"status":state.get_status().as_str(),"count":state.get_rows().row_count(),"selected":state.get_selected().as_str()});
                let path = arg(1).to_owned();
                self.backend.spawn(async move {
                    let _ = tokio::fs::write(path, value.to_string()).await;
                });
            }
            _ => state.invoke_action(arg(0).into()),
        }
    }
}
fn publish(state: &LearnedMemoryState<'_>, value: &Value) {
    let snapshot = &value["snapshot"];
    state.set_revision(snapshot["revision"].to_string().into());
    state.set_learning(snapshot["learning"] == true);
    state.set_recall(snapshot["recall"] == true);
    state.set_rows(ModelRc::new(VecModel::from(
        snapshot["facts"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| LearnedMemoryRow {
                id: f["id"].as_str().unwrap_or("").into(),
                title: format!(
                    "{}{}",
                    if f["pinned"] == true {
                        "Pinned · "
                    } else {
                        ""
                    },
                    f["key"].as_str().unwrap_or("")
                )
                .into(),
                text: f["text"].as_str().unwrap_or("").into(),
                pinned: f["pinned"] == true,
                global: f["global"] == true,
                detail: format!(
                    "{} · {} · {}",
                    f["scope_name"].as_str().unwrap_or("Project"),
                    f["kind"].as_str().unwrap_or("fact"),
                    f["source"]
                        .as_str()
                        .unwrap_or("")
                        .strip_prefix("goose:")
                        .unwrap_or_else(|| f["source"].as_str().unwrap_or(""))
                )
                .into(),
            })
            .collect::<Vec<_>>(),
    )));
    state.set_status(if value["unavailable"] == true { "Learning is unavailable or retrying. Saved memories remain available." } else if state.get_rows().row_count() == 0 { "No memories yet. Learning starts with future completed turns." } else { "Forgotten entries are blocked from automatic recreation. Existing conversation text remains unchanged." }.into());
}
