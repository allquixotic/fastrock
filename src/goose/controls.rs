use crate::app::AppController;
use crate::ui::{GooseOption, GooseState};
use serde_json::json;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

impl AppController {
    pub(crate) fn goose_controls_install(&mut self) {
        self.goose_setup_install();
        self.effort_controls_install();
        self.profiles_install();
        self.capabilities_install();
        self.schedules_install();
        self.window
            .global::<GooseState>()
            .on_resume_recovered(crate::ui_thread::scoped_callback!(|| {
                crate::ui_thread::with_app(|app| app.goose_recovery_action("resume"));
            }));
        self.window
            .global::<GooseState>()
            .on_copy_recovery(crate::ui_thread::scoped_callback!(|| {
                crate::ui_thread::with_app(|app| app.goose_recovery_action("copy"));
            }));
        self.window
            .global::<GooseState>()
            .on_dismiss_recovery(crate::ui_thread::scoped_callback!(|| {
                crate::ui_thread::with_app(|app| app.goose_recovery_action("dismiss"));
            }));
        self.window
            .global::<GooseState>()
            .on_choose(crate::ui_thread::scoped_callback!(|id, value| {
                let (id, value) = (id.to_string(), value.to_string());
                crate::ui_thread::with_app(move |app| app.goose_choose(id, value));
            }));
        self.window
            .global::<GooseState>()
            .on_save_cheap(crate::ui_thread::scoped_callback!(|| {
                crate::ui_thread::with_app(AppController::goose_cheap_save);
            }));
    }
    pub(crate) fn goose_controls_refresh(&self) {
        let state = self.window.global::<GooseState>();
        state.set_active(self.backend.is_goose());
        let thread = self.active_thread_index().and_then(|i| self.thread_tab(i));
        let combined = thread.map(|thread| self.combined_recovery(thread));
        let recovery = combined.as_ref();
        state.set_recovery_paused(recovery.is_some_and(|r| r.paused));
        state
            .set_recovered_pending(recovery.map_or(0, |r| r.pending.min(i32::MAX as usize) as i32));
        state.set_uncertain_count(
            recovery.map_or(0, |r| r.uncertain.len().min(i32::MAX as usize) as i32),
        );
        state.set_recovery_details(recovery.map(recovery_preview).unwrap_or_default().into());
        state.set_busy(
            thread.is_none_or(|t| t.is_busy() || t.phase == crate::app::ThreadPhase::Starting),
        );
        let options = thread
            .and_then(|t| t.thread_id.as_ref())
            .and_then(|id| self.composer_shared.goose_configs.get(id));
        let rows = options
            .into_iter()
            .flatten()
            .filter(|o| o["type"] == "select")
            .map(|o| {
                let values = super::sessions::option_values(o);
                GooseOption {
                    id: super::text(o, "id").into(),
                    title: super::text(o, "name").into(),
                    selected: values
                        .iter()
                        .position(|v| v["value"] == o["currentValue"])
                        .map(|i| i as i32)
                        .unwrap_or(-1),
                    names: ModelRc::new(VecModel::from(
                        values
                            .iter()
                            .map(|v| SharedString::from(super::text(v, "name")))
                            .collect::<Vec<_>>(),
                    )),
                    values: ModelRc::new(VecModel::from(
                        values
                            .iter()
                            .map(|v| SharedString::from(super::text(v, "value")))
                            .collect::<Vec<_>>(),
                    )),
                }
            })
            .collect::<Vec<_>>();
        let model = options
            .into_iter()
            .flatten()
            .find(|o| o["id"] == "model")
            .map(|o| super::text(o, "currentValue"))
            .unwrap_or("");
        state.set_summary(if model.is_empty() {
            "Goose options".into()
        } else {
            format!("Goose · {model}").into()
        });
        let provider = options
            .into_iter()
            .flatten()
            .find(|o| o["id"] == "provider")
            .map(|o| super::text(o, "currentValue"))
            .unwrap_or("");
        let context = format!(
            "{}\n{provider}\n{model}",
            thread.and_then(|t| t.thread_id.as_deref()).unwrap_or("")
        );
        if state.get_effort_context() != context {
            state.set_effort_context(context.into());
            state.set_effort_status("".into());
            state.set_custom_model(model.into());
        }
        state.set_effort_available(
            rows.iter()
                .any(|r| r.id == "thinking_effort" && r.selected >= 0),
        );
        state.set_options(ModelRc::new(VecModel::from(rows)));
    }
    fn goose_choose(&mut self, config_id: String, value: String) {
        let Some(id) = self
            .active_thread_index()
            .and_then(|i| self.thread_tab(i))
            .and_then(|t| t.thread_id.clone())
        else {
            return;
        };
        let state = self.window.global::<GooseState>();
        if state.get_changing() || state.get_busy() {
            return;
        }
        state.set_changing(true);
        state.set_error("".into());
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = backend
                .goose_request(
                    "fastrock/goose/setConfig",
                    json!({"threadId":id,"configId":config_id,"value":value}),
                )
                .await;
            crate::ui_thread::post(move |app| {
                let state = app.window.global::<GooseState>();
                state.set_changing(false);
                match result {
                    Ok(reply) => {
                        app.composer_shared.goose_configs.insert(
                            id,
                            reply["configOptions"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default(),
                        );
                    }
                    Err(e) => state.set_error(e.into()),
                }
                app.goose_controls_refresh();
            });
        });
    }

    pub(crate) fn thread_recovery_paused(&self, thread: &crate::app::ThreadTab) -> bool {
        thread.recovery_paused
            || thread
                .thread_id
                .as_ref()
                .and_then(|id| self.composer_shared.goose_recovery.get(id))
                .is_some_and(|recovery| recovery.paused)
    }

    fn combined_recovery(&self, thread: &crate::app::ThreadTab) -> super::Recovery {
        let mut recovery = thread
            .thread_id
            .as_ref()
            .and_then(|id| self.composer_shared.goose_recovery.get(id))
            .cloned()
            .unwrap_or_default();
        recovery.paused |= thread.recovery_paused;
        recovery.pending += thread.pending_inputs.len();
        recovery.uncertain.extend(thread.recovered_sends.clone());
        recovery
    }
    fn goose_recovery_action(&mut self, action: &str) {
        let Some(index) = self.active_thread_index() else {
            return;
        };
        let Some(thread) = self.thread_tab(index) else {
            return;
        };
        let tab_id = self.tabs[index].id;
        let id = thread.thread_id.clone();
        let recovery = self.combined_recovery(thread);
        match action {
            "copy" => {
                if let Ok(text) = serde_json::to_string_pretty(&recovery.uncertain) {
                    self.copy_to_clipboard(&text);
                }
            }
            "resume" => {
                if let Some(id) = id {
                    self.goose_recovery_request(id, "fastrock/goose/resumeQueue", json!({}));
                } else {
                    self.resume_local_inputs(tab_id);
                }
            }
            "dismiss" => {
                let ids = recovery
                    .uncertain
                    .iter()
                    .map(|item| item["id"].clone())
                    .collect::<Vec<_>>();
                self.show_dialog(crate::app::DialogRequest::confirm(
                    "Dismiss reviewed records?",
                    "This removes the selected recovery records. Conversation history and queued messages remain. Copy the records first if you need them.".to_owned(),
                ).accept_label("Dismiss").destructive(), Box::new(move |app, accepted| {
                    if accepted.is_some() {
                        if let Some(index) = app.tab_index_by_id(tab_id) {
                            if let Some(thread) = app.thread_tab_mut(index) { thread.recovered_sends.retain(|item| !ids.contains(&item["id"])); }
                        }
                        app.save_open_chats();
                        if let Some(id) = id { app.goose_recovery_request(id, "fastrock/goose/dismissRecovery", json!({"ids":ids})); }
                        app.goose_controls_refresh();
                    }
                }));
            }
            _ => {}
        }
    }
    fn goose_recovery_request(
        &mut self,
        id: String,
        method: &'static str,
        mut params: serde_json::Value,
    ) {
        params["threadId"] = json!(id);
        self.window.global::<GooseState>().set_changing(true);
        self.window.global::<GooseState>().set_error("".into());
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = backend
                .goose_request(method, params)
                .await
                .and_then(|value| {
                    serde_json::from_value::<super::Recovery>(value).map_err(|e| e.to_string())
                });
            crate::ui_thread::post(move |app| {
                app.window.global::<GooseState>().set_changing(false);
                // Ordered backend events carry the current state. A response
                // snapshot can arrive after the resumed queue advances.
                match result {
                    Err(error) => app.window.global::<GooseState>().set_error(error.into()),
                    Ok(_) if method == "fastrock/goose/resumeQueue" => {
                        if let Some(index) = app.tab_index_for_thread(&id) {
                            app.resume_local_inputs(app.tabs[index].id);
                        }
                    }
                    Ok(_) => {}
                }
                app.goose_controls_refresh();
            });
        });
    }

    pub(crate) fn goose_cheap_load(&mut self) {
        let state = self.window.global::<GooseState>();
        if state.get_cheap_loading() {
            return;
        }
        state.set_cheap_loading(true);
        self.backend.spawn(async move {
            let result = super::cheap_profile::load().await;
            crate::ui_thread::post(move |app| {
                let state = app.window.global::<GooseState>();
                state.set_cheap_loading(false);
                match result {
                    Ok(profile) => {
                        state.set_cheap_name(profile.name.into());
                        state.set_cheap_model(profile.model.into());
                        state.set_cheap_provider(
                            super::provider_policy::choices()
                                .iter()
                                .position(|(p, _)| *p == profile.provider)
                                .unwrap_or(0) as i32,
                        );
                        state.set_cheap_status(super::summary_service::status().into());
                    }
                    Err(_) => state.set_cheap_status(
                        "Could not read the designated cheap profile. Search continues locally."
                            .into(),
                    ),
                }
            });
        });
    }

    fn goose_cheap_save(&mut self) {
        let state = self.window.global::<GooseState>();
        if state.get_cheap_loading() {
            return;
        }
        let profile = super::cheap_profile::CheapProfile {
            name: state.get_cheap_name().trim().to_owned(),
            provider: state
                .get_provider_ids()
                .row_data(state.get_cheap_provider() as usize)
                .unwrap_or_default()
                .to_string(),
            model: state.get_cheap_model().trim().to_owned(),
        };
        if let Err(error) = profile.validate() {
            state.set_cheap_status(error.to_string().into());
            return;
        }
        state.set_cheap_loading(true);
        self.backend.spawn(async move {
            let result = super::cheap_profile::save(profile).await;
            crate::ui_thread::post(move |app| {
                let state = app.window.global::<GooseState>();
                state.set_cheap_loading(false);
                state.set_cheap_status(
                    if result.is_ok() {
                        "Saved the cheap profile in Goose settings."
                    } else {
                        "Could not save the cheap profile."
                    }
                    .into(),
                );
            });
        });
    }
}

fn recovery_preview(recovery: &super::Recovery) -> String {
    recovery
        .uncertain
        .iter()
        .take(12)
        .map(|item| {
            let text = item["input"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|input| {
                    input["text"]
                        .as_str()
                        .unwrap_or("[attachment or structured input]")
                })
                .flat_map(|text| text.chars().chain(std::iter::once('\n')))
                .take(2048)
                .collect::<String>();
            format!(
                "{}\n{}",
                item["recoveryReason"]
                    .as_str()
                    .unwrap_or("Interrupted send"),
                text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
