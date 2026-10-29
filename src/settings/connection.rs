//! Embedded runtime status; provider configuration lives in Goose settings.
use crate::app::AppController;
use crate::ui::SettingsState;
use slint::ComponentHandle;

#[derive(Default)]
pub(crate) struct ConnectionState;

impl AppController {
    pub(super) fn settings_connection_bind(&mut self) {
        self.window
            .global::<SettingsState>()
            .on_connection_revert(|| {
                crate::ui_thread::with_app(AppController::settings_connection_activate);
            });
    }
    pub(super) fn settings_connection_activate(&mut self) {
        let state = self.window.global::<SettingsState>();
        state.set_connection_mode(3);
        state.set_connection_address("".into());
        state.set_connection_token_env("".into());
        state.set_connection_dirty(false);
        state.set_connection_valid(true);
        state.set_connection_testing(false);
        state.set_connection_saved("".into());
        self.settings_connection_refresh_status();
    }
    pub(super) fn settings_connection_refresh_status(&mut self) {
        let state = self.window.global::<SettingsState>();
        state.set_connection_current("Embedded Goose".into());
        state.set_connection_state(
            match (&self.settings.server_error, self.backend.is_ready()) {
                (Some(_), _) => "Not running",
                (None, true) => "Running",
                (None, false) => "Starting",
            }
            .into(),
        );
    }
    pub(super) fn settings_connection_automation(&mut self, _args: &[String]) {
        self.settings_connection_activate();
    }
}
