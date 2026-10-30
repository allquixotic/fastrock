use crate::{app::AppController, ui::GooseState};
use serde_json::json;
use slint::{ComponentHandle, Model};

impl AppController {
    pub(super) fn effort_controls_install(&mut self) {
        self.window.global::<GooseState>().on_save_effort_default(
            crate::ui_thread::scoped_callback!(|clear| {
                crate::ui_thread::with_app(move |app| app.effort_default_save(clear));
            }),
        );
    }

    fn effort_default_save(&mut self, clear: bool) {
        let state = self.window.global::<GooseState>();
        if state.get_busy() || state.get_changing() {
            return;
        }
        let Some(id) = self
            .active_thread_index()
            .and_then(|i| self.thread_tab(i))
            .and_then(|t| t.thread_id.clone())
        else {
            return;
        };
        let options = state.get_options();
        let selected = |name: &str| {
            options
                .iter()
                .find(|o| o.id == name)
                .and_then(|o| o.values.row_data(o.selected as usize))
                .unwrap_or_default()
                .to_string()
        };
        let params = json!({"threadId":id, "provider":selected("provider"),
            "model":selected("model"), "value":selected("thinking_effort"), "clear":clear});
        let context = state.get_effort_context().to_string();
        state.set_changing(true);
        state.set_effort_status("".into());
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = backend.goose_request("fastrock/goose/saveEffortDefault", params).await;
            crate::ui_thread::post(move |app| {
                let state = app.window.global::<GooseState>();
                state.set_changing(false);
                if state.get_effort_context() != context { return; }
                state.set_effort_status(match result {
                    Ok(_) if clear => "Saved effort default cleared for this provider/model. Current conversation is unchanged.".into(),
                    Ok(_) => "Effort saved for this provider/model. New conversations and model switches use this default; existing conversations keep their settings.".into(),
                    Err(error) => error.into(),
                });
            });
        });
    }
}
