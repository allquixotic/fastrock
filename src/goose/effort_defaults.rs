//! Per-provider/model defaults and manual effort for Bedrock profiles.
use super::*;
use anyhow::{Context, Result, ensure};
use goose_provider_types::{model::ModelConfig, thinking::ThinkingEffort};
use goose_runtime::config::{Config, ConfigError};
use goose_runtime::session::SessionManager;
use std::{collections::BTreeMap, sync::Mutex};

const KEY: &str = "FASTROCK_EFFORT_DEFAULTS";
type Defaults = BTreeMap<String, BTreeMap<String, String>>;
static GATE: Mutex<()> = Mutex::new(());

fn read(config: &Config) -> Result<Defaults> {
    match config.get_param(KEY) {
        Ok(value) => Ok(value),
        Err(ConfigError::NotFound(_)) => Ok(BTreeMap::new()),
        Err(_) => {
            anyhow::bail!("Could not read saved reasoning defaults in provider configuration")
        }
    }
}

fn write(config: &Config, provider: &str, model: &str, value: Option<&str>) -> Result<()> {
    super::config_guard::validate(config)?;
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
    super::config_guard::edit(config, |config| Ok(config.set_param(KEY, defaults)?))
        .map_err(|error| anyhow::anyhow!("Could not save reasoning defaults: {error}"))?;
    Ok(())
}

async fn identity(id: &str) -> Result<(String, String)> {
    let session = SessionManager::instance().get_session(id, false).await?;
    Ok((
        session.provider_name.context("No provider selected")?,
        session
            .model_config
            .context("No model selected")?
            .model_name,
    ))
}

fn effort_option(options: &[Value]) -> Option<&Value> {
    options
        .iter()
        .find(|option| option["id"] == "thinking_effort")
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ManualEffort {
    Goose,
    Responses,
}

impl ManualEffort {
    pub(super) fn for_session(session: &Session) -> Option<Self> {
        let provider = session.config_value("provider");
        if session.config_value("model").is_empty()
            || effort_option(&session.config)
                .is_some_and(|option| sessions::option_values(option).len() > 1)
        {
            return None;
        }
        if provider == "aws_bedrock" {
            Some(Self::Goose)
        } else if super::provider_policy::bedrock_responses(provider) {
            Some(Self::Responses)
        } else {
            None
        }
    }

    fn parameter(self) -> &'static str {
        match self {
            Self::Goose => "thinking_effort",
            Self::Responses => "reasoning_effort",
        }
    }

    pub(super) fn accepts(self, value: &Value) -> bool {
        match self {
            Self::Goose => value
                .as_str()
                .is_some_and(|value| value.parse::<ThinkingEffort>().is_ok()),
            Self::Responses => sessions::valid_model_id(value),
        }
    }

    pub(super) fn saved(self, model: &ModelConfig) -> Option<String> {
        if self == Self::Responses
            && let Some(reasoning) = model.request_param::<Value>("reasoning")
            && let Some(effort) = reasoning["effort"].as_str()
        {
            return Some(effort.to_owned());
        }
        model.request_param::<String>(self.parameter())
    }

    pub(super) fn visible(self, session: &Session) -> Value {
        json!({
            "id":"thinking_effort", "name":"Thinking effort", "type":"select",
            "currentValue":current_manual(session), "options":[],
            "manual":true, "gooseEffort":self == Self::Goose
        })
    }
}

fn current_manual(session: &Session) -> &str {
    session
        .manual_effort
        .as_ref()
        .filter(|(provider, model, _)| {
            provider == session.config_value("provider") && model == session.config_value("model")
        })
        .map_or("", |(_, _, value)| value.as_str())
}

fn accepts(session: &Session, manual: Option<ManualEffort>, value: &str) -> bool {
    if let Some(manual) = manual {
        return manual.accepts(&json!(value));
    }
    effort_option(&session.config).is_some_and(|option| {
        sessions::option_values(option)
            .iter()
            .any(|v| v["value"] == value)
    })
}

fn current(session: &Session, manual: Option<ManualEffort>) -> &str {
    if manual.is_some() {
        current_manual(session)
    } else {
        session.config_value("thinking_effort")
    }
}

pub(super) fn set_responses_effort(model: &mut ModelConfig, value: &Value) {
    let params = model.request_params.get_or_insert_with(Default::default);
    if let Some(reasoning) = params.get_mut("reasoning").and_then(Value::as_object_mut) {
        reasoning.insert("effort".into(), value.clone());
    } else {
        params.insert(ManualEffort::Responses.parameter().into(), value.clone());
    }
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
        let (provider, model) = identity(id)
            .await
            .map_err(|_| error("Could not resolve this provider/model for reasoning defaults"))?;
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
            let manual = ManualEffort::for_session(s);
            accepts(s, manual, &desired) && current(s, manual) != desired
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
                .ok_or_else(|| error("Open a conversation first"))?;
            if session.active.is_some() {
                return Err(error("Wait for the current turn before saving defaults"));
            }
            let manual = ManualEffort::for_session(session);
            if session.config_value("model") != text(params, "model")
                || session.config_value("provider") != text(params, "provider")
                || (!clear
                    && (current(session, manual) != value || !accepts(session, manual, &value)))
            {
                return Err(error(
                    "Model options changed. Review the current options before saving",
                ));
            }
        }
        let (provider, model) = identity(id)
            .await
            .map_err(|_| error("Could not resolve this provider/model for reasoning defaults"))?;
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
    fn v117_manual_bedrock_effort_preserves_exact_responses_payload() {
        use goose_provider_types::formats::openai_responses::create_responses_request;

        let model = "us.openai.gpt-future-fixture";
        for params in [
            BTreeMap::new(),
            BTreeMap::from([("reasoning".into(), json!({"effort":"low","summary":"auto"}))]),
        ] {
            let mut config =
                ModelConfig::new(model).with_merged_request_params(params.into_iter().collect());
            let effort = json!("xhigh");
            assert!(ManualEffort::Responses.accepts(&effort));
            set_responses_effort(&mut config, &effort);
            let request = create_responses_request(&config, "fixture", &[], &[]).unwrap();
            assert_eq!(request["model"], model);
            assert_eq!(request["reasoning"]["effort"], effort);
            assert_eq!(request["reasoning"]["summary"], "auto");
            assert_eq!(
                ManualEffort::Responses.saved(&config).as_deref(),
                Some("xhigh")
            );
        }
        assert!(!ManualEffort::Responses.accepts(&json!("high\n")));
        assert!(ManualEffort::Goose.accepts(&json!("high")));
        assert!(!ManualEffort::Goose.accepts(&json!("unknown")));
    }
    #[test]
    fn v83_clearing_flow_default_preserves_other_colon_model_ids() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.yaml");
        let opus = "us.anthropic.claude-opus-5-5-v1:0";
        let sonnet = "us.anthropic.claude-sonnet-4-5-20250929-v1:0";
        std::fs::write(
            &path,
            format!(
                "# retained user explanation\n{}\n",
                json!({"GOOSE_PROVIDER":"aws_bedrock","GOOSE_MODEL":opus,
                    "extensions":{},"FASTROCK_EFFORT_DEFAULTS":{"aws_bedrock":{opus:"high"}}})
            ),
        )
        .unwrap();
        let config =
            Config::new_with_file_secrets(&path, directory.path().join("secrets")).unwrap();
        assert_eq!(read(&config).unwrap()["aws_bedrock"][opus], "high");
        write(&config, "aws_bedrock", sonnet, Some("medium")).unwrap();
        write(&config, "aws_bedrock", opus, None).unwrap();
        let defaults = read(&config).unwrap();
        assert!(!defaults["aws_bedrock"].contains_key(opus));
        assert_eq!(defaults["aws_bedrock"][sonnet], "medium");
        assert!(
            std::fs::read_to_string(path)
                .unwrap()
                .contains("# retained user explanation")
        );
    }
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
        let session = Session {
            config: vec![
                json!({"id":"thinking_effort", "options":[{"value":"low"},{"value":"high"}]}),
            ],
            ..Session::default()
        };
        assert!(accepts(&session, None, "high"));
        assert!(!accepts(&session, None, "arbitrary"));
        assert!(!accepts(&Session::default(), None, "high"));
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
