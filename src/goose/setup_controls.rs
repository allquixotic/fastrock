use super::setup::{self, Setup};
use crate::app::AppController;
use crate::ui::{GooseSetupField, GooseState};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

fn publish(state: &GooseState<'_>, setup: &Setup) {
    state.set_setup_provider(
        setup::PROVIDERS
            .iter()
            .position(|name| *name == setup.provider)
            .map_or(-1, |i| i as i32),
    );
    state.set_setup_model(setup.model.clone().into());
    state.set_setup_fields(ModelRc::new(VecModel::from(
        setup::FIELDS
            .iter()
            .map(|(key, title)| GooseSetupField {
                key: (*key).into(),
                title: (*title).into(),
                value: setup.fields.get(*key).cloned().unwrap_or_default().into(),
            })
            .collect::<Vec<_>>(),
    )));
    state.set_setup_loaded(true);
}

impl AppController {
    pub(super) fn goose_setup_install(&mut self) {
        self.window
            .global::<GooseState>()
            .on_load_setup(crate::ui_thread::scoped_callback!(|| {
                crate::ui_thread::with_app(AppController::goose_setup_load);
            }));
        self.window
            .global::<GooseState>()
            .on_save_setup(crate::ui_thread::scoped_callback!(|| {
                crate::ui_thread::with_app(AppController::goose_setup_save);
            }));
    }
    pub(crate) fn goose_setup_load(&mut self) {
        let state = self.window.global::<GooseState>();
        if state.get_setup_loading() {
            return;
        }
        state.set_setup_loading(true);
        state.set_setup_status("Loading provider defaults…".into());
        self.backend.spawn(async move {
            let result = setup::load().await;
            crate::ui_thread::post(move |app| {
                let state = app.window.global::<GooseState>();
                state.set_setup_loading(false);
                match result {
                    Ok(setup) => {
                        publish(&state, &setup);
                        app.settings.goose_setup = Some(setup);
                        state.set_setup_status("".into());
                    }
                    Err(error) => state.set_setup_status(error.to_string().into()),
                }
            });
        });
    }
    fn goose_setup_save(&mut self) {
        let state = self.window.global::<GooseState>();
        if state.get_setup_loading() {
            return;
        }
        let Some(expected) = self.settings.goose_setup.clone() else {
            return;
        };
        let desired = Setup {
            provider: setup::PROVIDERS
                .get(state.get_setup_provider() as usize)
                .unwrap_or(&"")
                .to_string(),
            model: state.get_setup_model().trim().to_owned(),
            fields: state
                .get_setup_fields()
                .iter()
                .map(|field| (field.key.to_string(), field.value.trim().to_owned()))
                .collect(),
        };
        if let Err(error) = desired.validate() {
            state.set_setup_status(error.to_string().into());
            return;
        }
        state.set_setup_loading(true);
        self.backend.spawn(async move {
            let result = setup::save(expected, desired).await;
            crate::ui_thread::post(move |app| {
                let state = app.window.global::<GooseState>();
                state.set_setup_loading(false);
                match result {
                    Ok(setup) => {
                        publish(&state, &setup);
                        app.settings.goose_setup = Some(setup);
                        state.set_setup_status("Saved in Goose configuration. Quit and restart Fastrock to use the new defaults. Environment overrides still apply.".into());
                    }
                    Err(error) => state.set_setup_status(error.to_string().into()),
                }
            });
        });
    }

    pub(crate) fn goose_setup_automation(&mut self, args: &[String]) {
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        let state = self.window.global::<GooseState>();
        match arg(0) {
            "provider" => state.set_setup_provider(
                setup::PROVIDERS
                    .iter()
                    .position(|name| *name == arg(1))
                    .map_or(-1, |i| i as i32),
            ),
            "model" => state.set_setup_model(arg(1).into()),
            "field" => {
                let fields = state.get_setup_fields();
                for i in 0..fields.row_count() {
                    if let Some(mut field) = fields.row_data(i) {
                        if field.key == arg(1) {
                            field.value = arg(2).into();
                            fields.set_row_data(i, field);
                            break;
                        }
                    }
                }
            }
            "save" => state.invoke_save_setup(),
            "reload" => state.invoke_load_setup(),
            "dump" => {
                let value = serde_json::json!({"loaded":state.get_setup_loaded(), "busy":state.get_setup_loading(),
                    "provider":state.get_setup_provider(), "model":state.get_setup_model().as_str(),
                    "fields":state.get_setup_fields().iter().map(|field| (field.key.to_string(), field.value.to_string())).collect::<std::collections::BTreeMap<_,_>>(),
                    "status":state.get_setup_status().as_str()});
                let path = std::path::PathBuf::from(arg(1));
                self.backend.spawn(async move {
                    if let Ok(bytes) = serde_json::to_vec_pretty(&value) {
                        let _ = tokio::fs::write(path, bytes).await;
                    }
                });
            }
            _ => {}
        }
    }
}
