use super::speed::{change, option, tier};
use super::{Adapter, Reply, Session, error};
use serde_json::{Value, json};

fn manager() -> goose_runtime::session::SessionManager {
    goose_runtime::session::SessionManager::new(goose_runtime::config::paths::Paths::data_dir())
}
impl Adapter {
    pub(super) fn visible_config(&self, session: &Session) -> Vec<Value> {
        if !self.embedded_runtime {
            return session.config.clone();
        }
        let mut options = super::provider_policy::visible_options(session.config.clone());
        options.retain(|o| o["id"] != "speed");
        options.push(option(
            session.config_value("provider"),
            session.config_value("model"),
            session.service_tier.as_deref(),
        ));
        options.sort_by_key(|o| match o["id"].as_str() {
            Some("provider") => 0,
            Some("mode") => 1,
            Some("model") => 2,
            Some("thinking_effort") => 3,
            Some("speed") => 4,
            _ => 5,
        });
        options
    }
    pub(super) async fn refresh_speed(
        &self,
        id: &str,
    ) -> Result<(), codex_app_server_protocol::JSONRPCErrorError> {
        if !self.embedded_runtime {
            return Ok(());
        }
        let persisted = manager()
            .get_session(id, false)
            .await
            .map_err(|_| error("Could not read saved model settings"))?;
        let value = persisted.model_config.as_ref().and_then(tier);
        if let Some(s) = self.sessions.lock().await.get_mut(id) {
            s.service_tier = value;
            self.config_event(id, s)?;
        }
        Ok(())
    }
    pub(super) async fn set_speed(&self, id: &str, value: &Value) -> Reply {
        let manager = manager();
        let persisted = manager
            .get_session(id, false)
            .await
            .map_err(|_| error("Could not read saved model settings"))?;
        let provider = persisted.provider_name.as_deref().unwrap_or_default();
        let mut model = persisted
            .model_config
            .ok_or_else(|| error("Select a model before changing speed"))?;
        let supported = super::provider_policy::ultrafast(provider, &model.model_name);
        change(&mut model, value.as_str().unwrap_or_default(), supported)
            .map_err(|e| error(e.to_string()))?;
        manager
            .update(id)
            .model_config(model)
            .apply()
            .await
            .map_err(|_| error("Could not save speed; previous setting retained"))?;
        self.refresh_speed(id).await?;
        let sessions = self.sessions.lock().await;
        let s = sessions
            .get(id)
            .ok_or_else(|| error("Session unavailable"))?;
        Ok(json!({"configOptions":self.visible_config(s)}))
    }
}
