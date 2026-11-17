//! Named Goose declarative providers with bundled auth helpers.
//! We never call Goose's secret setters (which permit plaintext fallback).
use super::foundry;
use anyhow::{Result, bail, ensure};
use goose_runtime::config::{Config, declarative_providers as api, providers};
use goose_runtime::providers::base::ModelInfo;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;

static GATE: Mutex<()> = Mutex::new(());
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Profile {
    pub id: String,
    pub revision: String,
    pub credential_id: String,
    pub name: String,
    pub engine: String,
    pub url: String,
    pub model: String,
    #[serde(default)]
    pub base_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub azure_subscription: String,
}
impl Profile {
    fn is_foundry(&self) -> bool {
        self.engine == foundry::ENGINE
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(
                self.engine.as_str(),
                "openai" | "anthropic" | foundry::ENGINE
            ),
            "Choose OpenAI, Anthropic or Microsoft Foundry"
        );
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 128
                && !self.name.chars().any(char::is_control),
            "Enter a profile name of 1–128 bytes"
        );
        if self.is_foundry() {
            foundry::project_route(&self.url)?;
            foundry::subscription(&self.azure_subscription)?;
            ensure!(
                self.model.len() <= 64 && foundry::azure_name(&self.model),
                "Enter the MAI Code 2 Flash deployment name from Foundry"
            );
            return Ok(());
        }
        ensure!(
            !self.model.trim().is_empty()
                && self.model.len() <= 1024
                && !self.model.chars().any(char::is_control),
            "Enter a model ID of 1–1024 bytes"
        );
        ensure!(
            self.base_path.len() <= 512
                && !self.base_path.chars().any(char::is_control)
                && !self.base_path.contains(['?', '#', ':'])
                && !self.base_path.contains(".."),
            "Enter a relative API request path without query parameters"
        );
        super::setup::validate_field("OPENAI_BASE_URL", &self.url)?;
        ensure!(!self.url.is_empty(), "Enter the API base URL");
        Ok(())
    }

    fn provider_engine(&self) -> String {
        if self.is_foundry() {
            "openai".into()
        } else {
            self.engine.clone()
        }
    }

    fn request_path(&self) -> Result<Option<String>> {
        if self.is_foundry() {
            return foundry::project_route(&self.url).map(Some);
        }
        Ok((!self.base_path.is_empty()).then(|| self.base_path.clone()))
    }

    fn model_info(&self) -> ModelInfo {
        let model = ModelInfo::new(&self.model);
        if self.is_foundry() {
            let mut model = model.with_context_limit(922_000).with_vision_support(true);
            model.reasoning = true;
            return model;
        }
        model
    }
}
fn engine(config: &api::DeclarativeProviderConfig) -> Option<&'static str> {
    match config.engine {
        api::ProviderEngine::OpenAI => Some("openai"),
        api::ProviderEngine::Anthropic => Some("anthropic"),
        _ => None,
    }
}
fn owned(config: &api::DeclarativeProviderConfig) -> Option<&str> {
    super::provider_readiness::credential_id(config)
        .or_else(|| foundry::profile_args(config).map(|(id, _)| id))
}

fn snapshot(config: &api::DeclarativeProviderConfig) -> Result<Profile> {
    let credential_id =
        owned(config).ok_or_else(|| anyhow::anyhow!("This profile is managed outside Fastrock"))?;
    ensure!(
        config.models.len() == 1 && config.headers.is_none() && config.env_vars.is_none(),
        "This profile has custom settings; preserve it and create a new profile to use the simple editor"
    );
    let foundry_subscription = foundry::profile_args(config).map(|(_, subscription)| subscription);
    let p = Profile {
        id: config.name.clone(),
        revision: fingerprint(serde_json::to_value(config)?)?,
        credential_id: credential_id.into(),
        name: config.display_name.clone(),
        engine: if foundry_subscription.is_some() {
            foundry::ENGINE
        } else {
            engine(config).unwrap()
        }
        .into(),
        url: config.base_url.clone(),
        model: config.models[0].name.clone(),
        base_path: if foundry_subscription.is_some() {
            String::new()
        } else {
            config.base_path.clone().unwrap_or_default()
        },
        azure_subscription: foundry_subscription.unwrap_or_default().into(),
    };
    p.validate()?;
    Ok(p)
}
fn fingerprint(mut value: serde_json::Value) -> Result<String> {
    // Goose model metadata contains HashMaps. Their iteration order must not
    // make an unchanged profile look like a stale edit after another load.
    value.sort_all_objects();
    Ok(format!(
        "{}",
        hex::encode(Sha256::digest(serde_json::to_vec(&value)?))
    ))
}
fn helper(id: &str, azure_subscription: Option<&str>) -> Result<api::AuthConfig> {
    ensure!(
        uuid::Uuid::parse_str(id).is_ok(),
        "Invalid credential identity"
    );
    let (args, timeout_seconds) = if let Some(subscription) = azure_subscription {
        let subscription = foundry::subscription(subscription)?.to_string();
        (vec![foundry::HELPER.into(), id.into(), subscription], 30)
    } else {
        (vec!["--fastrock-provider-credential".into(), id.into()], 10)
    };
    Ok(api::AuthConfig {
        command: std::env::current_exe()?
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Fastrock path must be valid Unicode"))?
            .into(),
        args,
        refresh_interval: 300,
        timeout_seconds: Some(timeout_seconds),
        cwd: None,
    })
}
fn update(
    config: &api::DeclarativeProviderConfig,
    desired: &Profile,
    auth: api::AuthConfig,
) -> Result<()> {
    api::update_custom_provider(api::UpdateCustomProviderParams {
        id: config.name.clone(),
        engine: desired.provider_engine(),
        display_name: desired.name.clone(),
        api_url: desired.url.clone(),
        api_key: None,
        models: vec![desired.model_info()],
        supports_streaming: Some(true),
        headers: None,
        requires_auth: true,
        catalog_provider_id: config.catalog_provider_id.clone(),
        base_path: desired.request_path()?,
        toolshim: config.toolshim,
        preserves_thinking: desired.is_foundry().then_some(true),
        auth: Some(auth),
    })
}
fn read() -> Result<Vec<Profile>> {
    let configs = api::load_custom_providers(&api::custom_providers_dir())
        .map_err(|_| anyhow::anyhow!("Could not read provider profiles"))?;
    let mut profiles = Vec::new();
    for config in configs {
        if owned(&config).is_some() {
            if let Ok(profile) = snapshot(&config) {
                profiles.push(profile);
            }
        }
    }
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(profiles)
}
/// Update only our helper paths after Fastrock is moved/upgraded. Existing
/// Goose-managed credentials and arbitrary configured helpers are untouched.
pub(super) async fn prepare() -> Result<()> {
    tokio::task::spawn_blocking(|| -> Result<()> {
        let _guard = GATE
            .lock()
            .map_err(|_| anyhow::anyhow!("Profile settings unavailable"))?;
        for config in api::load_custom_providers(&api::custom_providers_dir())? {
            let Some(id) = owned(&config) else {
                continue;
            };
            let subscription = foundry::profile_args(&config).map(|(_, subscription)| subscription);
            let auth = helper(id, subscription)?;
            if config
                .auth
                .as_ref()
                .is_some_and(|old| old.command != auth.command)
            {
                api::update_custom_provider(api::UpdateCustomProviderParams {
                    id: config.name.clone(),
                    engine: engine(&config).unwrap().into(),
                    display_name: config.display_name.clone(),
                    api_url: config.base_url.clone(),
                    api_key: None,
                    models: config.models.clone(),
                    supports_streaming: config.supports_streaming,
                    headers: None,
                    requires_auth: config.requires_auth,
                    catalog_provider_id: config.catalog_provider_id.clone(),
                    base_path: config.base_path.clone(),
                    toolshim: config.toolshim,
                    preserves_thinking: Some(config.preserves_thinking),
                    auth: Some(auth),
                })
                .map_err(|_| {
                    anyhow::anyhow!("Could not update the bundled credential helper path")
                })?;
            }
        }
        Ok(())
    })
    .await??;
    super::provider_policy::refresh().await
}
pub(crate) async fn load() -> Result<Vec<Profile>> {
    tokio::task::spawn_blocking(|| {
        let _guard = GATE
            .lock()
            .map_err(|_| anyhow::anyhow!("Profile settings unavailable"))?;
        read()
    })
    .await?
}
pub(crate) async fn save(desired: Profile, create_id: String, secret: String) -> Result<Profile> {
    let profile = tokio::task::spawn_blocking(move || -> Result<Profile> {
        let _guard = GATE
            .lock()
            .map_err(|_| anyhow::anyhow!("Profile settings unavailable"))?;
        desired.validate()?;
        ensure!(
            !desired.is_foundry() || secret.is_empty(),
            "Microsoft Foundry profiles use your Azure CLI sign-in"
        );
        let existing = if desired.id.is_empty() {
            None
        } else {
            let loaded = api::load_provider(&desired.id)
                .map_err(|_| anyhow::anyhow!("Profile no longer exists"))?;
            ensure!(
                loaded.is_editable && snapshot(&loaded.config)?.revision == desired.revision,
                "Profile changed; reload it before saving"
            );
            ensure!(
                foundry::profile_args(&loaded.config).is_some() == desired.is_foundry(),
                "Create a new profile to change between Azure CLI sign-in and API keys"
            );
            Some(loaded.config)
        };
        if existing.is_none() {
            // Native create identity survives a lost receipt; never create a
            // second provider or overwrite its credential on a retry.
            if let Some(old) = read()?.into_iter().find(|p| p.credential_id == create_id) {
                ensure!(
                    old.name == desired.name
                        && old.engine == desired.engine
                        && old.url == desired.url
                        && old.model == desired.model
                        && old.base_path == desired.base_path
                        && old.azure_subscription == desired.azure_subscription,
                    "This form already created a profile; reload it to edit"
                );
                return Ok(old);
            }
            ensure!(
                desired.is_foundry() || !secret.is_empty(),
                "Enter an API key for the new profile"
            );
        }
        let new_key = !secret.is_empty();
        let credential_id = if new_key || existing.is_none() {
            create_id.as_str()
        } else {
            existing.as_ref().and_then(owned).unwrap_or("")
        };
        ensure!(
            !new_key || existing.as_ref().and_then(owned) != Some(credential_id),
            "Credential replacement requires a fresh identity; reload the profile"
        );
        let auth = helper(
            credential_id,
            desired
                .is_foundry()
                .then_some(desired.azure_subscription.as_str()),
        )?;
        if new_key {
            crate::vault::put("provider", credential_id, &secret)?;
        }
        let saved = (|| -> Result<String> {
            if let Some(config) = &existing {
                update(config, &desired, auth)?;
                Ok(config.name.clone())
            } else {
                Ok(
                    api::create_custom_provider(api::CreateCustomProviderParams {
                        engine: desired.provider_engine(),
                        display_name: desired.name.clone(),
                        api_url: desired.url.clone(),
                        api_key: None,
                        models: vec![desired.model_info()],
                        supports_streaming: Some(true),
                        headers: None,
                        requires_auth: true,
                        catalog_provider_id: None,
                        base_path: desired.request_path()?,
                        toolshim: false,
                        preserves_thinking: desired.is_foundry().then_some(true),
                        auth: Some(auth),
                    })?
                    .name,
                )
            }
        })();
        let id = match saved {
            Ok(id) => id,
            Err(_) => {
                if new_key {
                    let _ = crate::vault::remove("provider", credential_id);
                }
                bail!("Could not save provider profile; reload to inspect the configuration")
            }
        };
        // Enable through Goose's public API; no credential is passed to Goose.
        if providers::set_provider_entry(
            Config::global(),
            &id,
            &providers::ProviderEntry {
                enabled: true,
                configured: true,
                model: desired.model.clone(),
            },
        )
        .is_err()
        {
            bail!("Profile was saved but could not be enabled; reload provider settings")
        }
        if new_key {
            if let Some(old) = existing
                .as_ref()
                .and_then(owned)
                .filter(|old| *old != credential_id)
            {
                let _ = crate::vault::remove("provider", old);
            }
        }
        snapshot(&api::load_provider(&id)?.config)
    })
    .await??;
    goose_runtime::providers::refresh_custom_providers()
        .await
        .map_err(|_| {
            anyhow::anyhow!("Profile saved; restart Fastrock to refresh the provider registry")
        })?;
    super::provider_policy::refresh().await?;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn foundry_profile() -> Profile {
        Profile {
            name: "Work Foundry".into(),
            engine: foundry::ENGINE.into(),
            url: "https://work.services.ai.azure.com/api/projects/coding".into(),
            model: "alias-for-flash".into(),
            azure_subscription: "12345678-1234-1234-1234-123456789abc".into(),
            ..Profile::default()
        }
    }

    #[test]
    fn v114_foundry_profile_round_trips_without_an_api_key() {
        let profile = foundry_profile();
        profile.validate().unwrap();
        let identity = uuid::Uuid::new_v4().to_string();
        let auth = helper(&identity, Some(&profile.azure_subscription)).unwrap();
        let mut config: api::DeclarativeProviderConfig =
            serde_json::from_value(serde_json::json!({
                "name": "custom_work_foundry",
                "display_name": profile.name,
                "engine": profile.provider_engine(),
                "base_url": profile.url,
                "base_path": profile.request_path().unwrap(),
                "models": [profile.model_info()],
                "requires_auth": true,
                "auth": auth,
            }))
            .unwrap();
        let restored = snapshot(&config).unwrap();
        assert_eq!(restored.engine, profile.engine);
        assert_eq!(restored.url, profile.url);
        assert_eq!(restored.model, profile.model);
        assert_eq!(restored.azure_subscription, profile.azure_subscription);
        assert_eq!(restored.credential_id, identity);
        assert!(super::super::provider_readiness::credential_id(&config).is_none());
        config.base_path = Some("openai/v1/chat/completions".into());
        assert!(snapshot(&config).is_err());
    }

    #[tokio::test]
    async fn v114_foundry_deployment_alias_streams_responses_with_cli_bearer() {
        use goose_provider_types::{
            base::Provider, conversation::message::Message, model::ModelConfig,
        };
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method, path},
        };
        let server = MockServer::start().await;
        let profile = foundry_profile();
        let response = [
            serde_json::json!({"type":"response.output_text.delta","sequence_number":1,
                "output_index":0,"content_index":0,"delta":"Foundry fixture reply","item_id":"msg-1"}),
            serde_json::json!({"type":"response.completed","sequence_number":2,"response":{
                "id":"resp-1","object":"response","status":"completed","model":profile.model,
                "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Foundry fixture reply"}]},
                    {"type":"function_call","status":"completed","call_id":"call-1","name":"fixture_tool","arguments":"{}"}],
                "usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}}),
        ].map(|event| format!("data: {event}\n\n")).join("");
        Mock::given(method("POST"))
            .and(path("/api/projects/coding/openai/v1/responses"))
            .and(header("Authorization", "Bearer fixture-foundry-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(response)
                    .insert_header("content-type", "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let python = std::env::var("FASTROCK_TEST_PYTHON").unwrap_or_else(|_| {
            if cfg!(windows) {
                "python".into()
            } else {
                "python3".into()
            }
        });
        let provider = goose_runtime::providers::openai_def::from_custom_config(
            serde_json::from_value(serde_json::json!({
                "name": "custom_foundry_fixture",
                "display_name": "Foundry fixture",
                "engine": profile.provider_engine(),
                "base_url": server.uri(),
                "base_path": profile.request_path().unwrap(),
                "models": [profile.model_info()],
                "requires_auth": true,
                "auth": {"command": python, "args": ["-c", "print('fixture-foundry-token')"], "refresh_interval": 300}
            })).unwrap(), None
        ).unwrap();
        let tool = serde_json::from_value(serde_json::json!({
            "name": "fixture_tool",
            "inputSchema": {"type": "object", "properties": {}}
        }))
        .unwrap();
        let (message, _) = provider
            .complete(
                &ModelConfig::new(&profile.model),
                "fixture",
                &[Message::user().with_text("hello")],
                &[tool],
            )
            .await
            .unwrap();
        assert_eq!(message.as_concat_text(), "Foundry fixture reply");
        assert!(message.get_tool_request_ids().contains("call-1"));
        let captured = server.received_requests().await.unwrap();
        let body: serde_json::Value = captured[0].body_json().unwrap();
        assert_eq!(body["model"], profile.model);
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert_eq!(body["input"][1]["content"][0]["text"], "hello");
        assert_eq!(body["tools"][0]["name"], "fixture_tool");
    }
    #[test]
    fn v59_revision_ignores_object_order_but_detects_changes() {
        let a = serde_json::from_str(r#"{"models":[{"request_params":{"x":1,"y":2}}],"name":"a"}"#)
            .unwrap();
        let b = serde_json::from_str(r#"{"name":"a","models":[{"request_params":{"y":2,"x":1}}]}"#)
            .unwrap();
        assert_eq!(fingerprint(a).unwrap(), fingerprint(b).unwrap());
        assert_ne!(
            fingerprint(serde_json::json!({"endpoint":"a"})).unwrap(),
            fingerprint(serde_json::json!({"endpoint":"b"})).unwrap()
        );
    }
    #[test]
    fn v59_profile_validation_excludes_agent_engines_and_secret_urls() {
        let mut p = Profile {
            name: "Fixture".into(),
            engine: "openai".into(),
            url: "http://127.0.0.1:1234/v1".into(),
            model: "gpt-6-luna".into(),
            ..Profile::default()
        };
        p.validate().unwrap();
        for url in [
            "http://example.com",
            "https://key@example.com",
            "https://example.com?token=secret",
        ] {
            p.url = url.into();
            assert!(p.validate().is_err());
        }
        p.url = "https://api.openai.com/v1".into();
        for engine in ["ollama", "codex", "claude_acp"] {
            p.engine = engine.into();
            assert!(p.validate().is_err());
        }
    }
}
