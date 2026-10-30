//! Per-provider/model defaults; choices remain authoritative from unchanged Goose.
use super::*;
use anyhow::{Context, Result, ensure};
use goose_runtime::config::{Config, ConfigError, paths::Paths};
use goose_runtime::session::SessionManager;
use std::{collections::BTreeMap, sync::Mutex};

const KEY: &str = "FASTROCK_EFFORT_DEFAULTS";
type Defaults = BTreeMap<String, BTreeMap<String, String>>;
static GATE: Mutex<()> = Mutex::new(());

fn read(config: &Config) -> Result<Defaults> {
    match config.get_param(KEY) {
        Ok(value) => Ok(value),
        Err(ConfigError::NotFound(_)) => Ok(BTreeMap::new()),
        Err(_) => anyhow::bail!("Could not read saved reasoning defaults in Goose configuration"),
    }
}

fn write(config: &Config, provider: &str, model: &str, value: Option<&str>) -> Result<()> {
    for field in [provider, model].into_iter().chain(value) {
        ensure!(
            !field.is_empty() && field.len() <= 1024 && !field.chars().any(char::is_control),
            "Invalid reasoning default"
        );
    }
    let mut defaults = read(config)?;
    if let Some(value) = value {
        defaults
            .entry(provider.into())
            .or_default()
            .insert(model.into(), value.into());
    } else if let Some(models) = defaults.get_mut(provider) {
        models.remove(model);
        if models.is_empty() {
            defaults.remove(provider);
        }
    }
    ensure!(
        defaults.values().map(BTreeMap::len).sum::<usize>() <= 4096,
        "Too many saved reasoning defaults"
    );
    config
        .set_param(KEY, defaults)
        .map_err(|_| anyhow::anyhow!("Could not save reasoning defaults in Goose configuration"))?;
    Ok(())
}

async fn identity(id: &str) -> Result<(String, String)> {
    let session = SessionManager::new(Paths::data_dir())
        .get_session(id, false)
        .await?;
    Ok((
        session
            .provider_name
            .context("Goose has not selected a provider")?,
        session
            .model_config
            .context("Goose has not selected a model")?
            .model_name,
    ))
}

fn offered(options: &[Value], value: &str) -> bool {
    options
        .iter()
        .find(|option| option["id"] == "thinking_effort")
        .is_some_and(|option| {
            sessions::option_values(option)
                .iter()
                .any(|v| v["value"] == value)
        })
}

impl Adapter {
    /// Caller holds the session gate. Apply only to new sessions or explicit
    /// provider/model changes, never restoration or forks of existing settings.
    pub(super) async fn apply_effort_default(
        &self,
        id: &str,
    ) -> Result<Option<Value>, JSONRPCErrorError> {
        if !self.embedded_runtime {
            return Ok(None);
        }
        let (provider, model) = identity(id).await.map_err(|_| {
            error("Could not resolve this Goose provider/model for reasoning defaults")
        })?;
        let desired = tokio::task::spawn_blocking(move || {
            let _guard = GATE
                .lock()
                .map_err(|_| anyhow::anyhow!("Reasoning defaults unavailable"))?;
            Ok::<_, anyhow::Error>(
                read(Config::global())?
                    .get(&provider)
                    .and_then(|models| models.get(&model))
                    .cloned(),
            )
        })
        .await
        .map_err(|_| error("Reasoning defaults unavailable"))?
        .map_err(|e| error(e.to_string()))?;
        let Some(desired) = desired else {
            return Ok(None);
        };
        let supported = self.sessions.lock().await.get(id).is_some_and(|s| {
            offered(&s.config, &desired) && s.config_value("thinking_effort") != desired
        });
        if !supported {
            return Ok(None);
        }
        self.set_config_raw(id, "thinking_effort", &json!(desired))
            .await
            .map(Some)
    }

    pub(super) async fn save_effort_default(&self, params: &Value) -> Reply {
        if !self.embedded_runtime {
            return Err(unsupported("Saved reasoning defaults"));
        }
        let id = session_id(text(params, "threadId"))?;
        let gate = self.gate(id).await;
        let _guard = gate.lock().await;
        let clear = params["clear"].as_bool().unwrap_or(false);
        let value = text(params, "value").to_owned();
        {
            let sessions = self.sessions.lock().await;
            let session = sessions
                .get(id)
                .filter(|s| s.loaded)
                .ok_or_else(|| error("Open a Goose conversation first"))?;
            if session.active.is_some() {
                return Err(error("Wait for the current turn before saving defaults"));
            }
            if session.config_value("model") != text(params, "model")
                || session.config_value("provider") != text(params, "provider")
                || (!clear
                    && (session.config_value("thinking_effort") != value
                        || !offered(&session.config, &value)))
            {
                return Err(error(
                    "Goose options changed. Review the current options before saving",
                ));
            }
        }
        let (provider, model) = identity(id).await.map_err(|_| {
            error("Could not resolve this Goose provider/model for reasoning defaults")
        })?;
        tokio::task::spawn_blocking(move || {
            let _guard = GATE
                .lock()
                .map_err(|_| anyhow::anyhow!("Reasoning defaults unavailable"))?;
            write(
                Config::global(),
                &provider,
                &model,
                (!clear).then_some(value.as_str()),
            )
        })
        .await
        .map_err(|_| error("Reasoning defaults unavailable"))?
        .map_err(|e| error(e.to_string()))?;
        Ok(json!({"cleared":clear}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v59_effort_defaults_isolate_models_profiles_and_preserve_goose_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let secrets = dir.path().join("secrets.yaml");
        std::fs::write(
            &path,
            "GOOSE_MODEL: retained\nextensions: {fixture: {enabled: true}}\n",
        )
        .unwrap();
        std::fs::write(&secrets, "secret fixture unchanged").unwrap();
        let config = Config::new_with_file_secrets(&path, &secrets).unwrap();
        write(&config, "aws_bedrock", "model-a", Some("high")).unwrap();
        write(&config, "aws_bedrock", "model-b", Some("low")).unwrap();
        write(&config, "custom_fixture", "model-a", Some("max")).unwrap();
        write(&config, "aws_bedrock", "model-a", Some("medium")).unwrap();
        let saved = read(&config).unwrap();
        assert_eq!(saved["aws_bedrock"]["model-a"], "medium");
        assert_eq!(saved["aws_bedrock"]["model-b"], "low");
        assert_eq!(saved["custom_fixture"]["model-a"], "max");
        write(&config, "aws_bedrock", "model-a", None).unwrap();
        assert!(!read(&config).unwrap()["aws_bedrock"].contains_key("model-a"));
        assert_eq!(config.get_goose_model().unwrap(), "retained");
        assert_eq!(
            config.get_param::<Value>("extensions").unwrap()["fixture"]["enabled"],
            true
        );
        assert_eq!(
            std::fs::read_to_string(&secrets).unwrap(),
            "secret fixture unchanged"
        );
        let before = std::fs::read(&path).unwrap();
        assert!(write(&config, "aws_bedrock", "invalid\nmodel", Some("high")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[test]
    fn v39_effort_defaults_require_values_advertised_by_this_session() {
        let options =
            vec![json!({"id":"thinking_effort", "options":[{"value":"low"},{"value":"high"}]})];
        assert!(offered(&options, "high"));
        assert!(!offered(&options, "arbitrary"));
        assert!(!offered(&[], "high"));
    }

    #[test]
    fn v39_explicit_model_ids_preserve_runtime_routes_and_are_bounded() {
        for id in [
            "openai.gpt-6.1-sol",
            "openai.gpt-6-luna",
            "us.anthropic.claude-opus-5-5-v1:0",
            "arn:aws:bedrock:us-east-1:123456789012:inference-profile/custom",
        ] {
            assert!(sessions::valid_model_id(&json!(id)));
        }
        for value in [
            json!(null),
            json!(5),
            json!(""),
            json!(" model"),
            json!("a\nb"),
            json!("x".repeat(1025)),
        ] {
            assert!(!sessions::valid_model_id(&value));
        }
    }
}
