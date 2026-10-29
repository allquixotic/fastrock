use crate::app::AppController;
use crate::ui::{GooseOption, GooseState};
use serde_json::json;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

impl AppController {
    pub(crate) fn goose_controls_install(&mut self) {
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
        state.set_busy(thread.is_none_or(|t| t.is_busy()));
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
                            ["aws_bedrock", "openai", "anthropic"]
                                .iter()
                                .position(|p| *p == profile.provider)
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
            provider: ["aws_bedrock", "openai", "anthropic"]
                .get(state.get_cheap_provider() as usize)
                .unwrap_or(&"")
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
