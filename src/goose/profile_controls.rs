use super::profiles::{self, Profile};
use crate::{
    app::AppController,
    ui::{ProfileRow, ProfileState},
};
use serde_json::json;
use slint::{ComponentHandle, ModelRc, VecModel};
impl AppController {
    pub(super) fn profiles_install(&mut self) {
        let state = self.window.global::<ProfileState>();
        state.on_load(crate::ui_thread::scoped_callback!(|| {
            crate::ui_thread::with_app(AppController::profiles_load)
        }));
        state.on_select(crate::ui_thread::scoped_callback!(|id| {
            let id = id.to_string();
            crate::ui_thread::with_app(move |app| app.profiles_select(&id));
        }));
        state.on_save(crate::ui_thread::scoped_callback!(|| {
            crate::ui_thread::with_app(AppController::profiles_save)
        }));
        self.profiles_select("");
    }
    pub(crate) fn profiles_load(&mut self) {
        let state = self.window.global::<ProfileState>();
        if state.get_busy() {
            return;
        }
        state.set_busy(true);
        self.backend.spawn(async move {
            let result = async {
                super::provider_policy::refresh().await?;
                profiles::load().await
            }
            .await;
            crate::ui_thread::post(move |app| {
                let state = app.window.global::<ProfileState>();
                state.set_busy(false);
                match result {
                    Ok(profiles) => {
                        state.set_rows(ModelRc::new(VecModel::from(
                            profiles
                                .iter()
                                .map(|p| ProfileRow {
                                    id: p.id.as_str().into(),
                                    name: p.name.as_str().into(),
                                    detail: format!("{} · {} · {}", p.engine, p.model, p.url)
                                        .into(),
                                })
                                .collect::<Vec<_>>(),
                        )));
                        app.settings.profiles = profiles;
                        super::setup_controls::publish_choices(
                            &app.window.global::<crate::ui::GooseState>(),
                        );
                    }
                    Err(e) => state.set_status(e.to_string().into()),
                }
            });
        });
    }
    fn profiles_select(&mut self, id: &str) {
        let state = self.window.global::<ProfileState>();
        if state.get_busy() {
            return;
        }
        let profile = self
            .settings
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .unwrap_or_default();
        state.set_edit_id(profile.id.into());
        state.set_revision(profile.revision.into());
        state.set_create_id(uuid::Uuid::new_v4().to_string().into());
        state.set_name(profile.name.into());
        state.set_engine(if profile.engine == "anthropic" { 1 } else { 0 });
        state.set_url(profile.url.into());
        state.set_model(profile.model.into());
        state.set_secret("".into());
        state.set_status("".into());
    }
    fn profiles_save(&mut self) {
        let state = self.window.global::<ProfileState>();
        if state.get_busy() {
            return;
        }
        let profile = Profile {
            id: state.get_edit_id().to_string(),
            revision: state.get_revision().to_string(),
            name: state.get_name().trim().to_owned(),
            engine: if state.get_engine() == 1 {
                "anthropic"
            } else {
                "openai"
            }
            .into(),
            url: state.get_url().trim().to_owned(),
            model: state.get_model().trim().to_owned(),
            credential_id: String::new(),
        };
        if let Err(e) = profile.validate() {
            state.set_status(e.to_string().into());
            return;
        }
        let secret = state.get_secret().to_string();
        let create_id = state.get_create_id().to_string();
        state.set_secret("".into());
        state.set_busy(true);
        self.backend.spawn(async move {
            let result = profiles::save(profile, create_id, secret).await;
            crate::ui_thread::post(move |app| {
                let state = app.window.global::<ProfileState>();
                state.set_busy(false);
                match result {
                    Ok(p) => {
                        state.set_edit_id(p.id.into());
                        state.set_revision(p.revision.into());
                        state.set_create_id(uuid::Uuid::new_v4().to_string().into());
                        state.set_status(
                            "Profile saved in Goose; credential stored in the OS vault".into(),
                        );
                    }
                    Err(e) => state.set_status(e.to_string().into()),
                }
                app.profiles_load();
            });
        });
    }
    pub(crate) fn profiles_automation(&mut self, args: &[String]) {
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        let state = self.window.global::<ProfileState>();
        match arg(0) {
            "new" => state.invoke_select("".into()),
            "select" => {
                let id = self
                    .settings
                    .profiles
                    .iter()
                    .find(|p| p.name == arg(1) || p.id == arg(1))
                    .map(|p| p.id.as_str())
                    .unwrap_or(arg(1));
                state.invoke_select(id.into());
            }
            "name" => state.set_name(arg(1).into()),
            "engine" => state.set_engine(if arg(1) == "anthropic" { 1 } else { 0 }),
            "url" => state.set_url(arg(1).into()),
            "model" => state.set_model(arg(1).into()),
            // Automation logs step arguments. Pass only an environment name,
            // never a credential value in the fixture script or diagnostics.
            "secret-env" => state.set_secret(std::env::var(arg(1)).unwrap_or_default().into()),
            "save" => state.invoke_save(),
            "cheap" => {
                if let Some(p) = self.settings.profiles.iter().find(|p| p.name == arg(1)) {
                    let goose = self.window.global::<crate::ui::GooseState>();
                    super::setup_controls::publish_choices(&goose);
                    goose.set_cheap_name(p.name.as_str().into());
                    goose.set_cheap_model(p.model.as_str().into());
                    goose.set_cheap_provider(
                        super::provider_policy::choices()
                            .iter()
                            .position(|(id, _)| *id == p.id)
                            .map_or(-1, |i| i as i32),
                    );
                    goose.invoke_save_cheap();
                }
            }
            "load" => state.invoke_load(),
            "dump" => {
                let value = json!({"profiles":self.settings.profiles,"status":state.get_status().as_str(),"busy":state.get_busy(),"secretEmpty":state.get_secret().is_empty()});
                let path = std::path::PathBuf::from(arg(1));
                self.backend.spawn(async move {
                    let _ = tokio::fs::write(path, value.to_string()).await;
                });
            }
            _ => {}
        }
    }
}
