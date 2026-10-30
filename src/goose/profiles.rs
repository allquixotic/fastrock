//! Named Goose declarative providers with a bundled, vault-only auth helper.
//! We never call Goose's secret setters (which permit plaintext fallback).
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
}
impl Profile {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.engine.as_str(), "openai" | "anthropic"),
            "Choose OpenAI or Anthropic"
        );
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 128
                && !self.name.chars().any(char::is_control),
            "Enter a profile name of 1–128 bytes"
        );
        ensure!(
            !self.model.trim().is_empty()
                && self.model.len() <= 1024
                && !self.model.chars().any(char::is_control),
            "Enter a model ID of 1–1024 bytes"
        );
        super::setup::validate_field("OPENAI_BASE_URL", &self.url)?;
        ensure!(!self.url.is_empty(), "Enter the API base URL");
        Ok(())
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
    let auth = config.auth.as_ref()?;
    (engine(config).is_some()
        && config.api_key_env.is_empty()
        && auth.args.len() == 2
        && auth.args[0] == "--fastrock-provider-credential"
        && uuid::Uuid::parse_str(&auth.args[1]).is_ok())
    .then(|| auth.args[1].as_str())
}
fn snapshot(config: &api::DeclarativeProviderConfig) -> Result<Profile> {
    let credential_id =
        owned(config).ok_or_else(|| anyhow::anyhow!("This profile is managed outside Fastrock"))?;
    ensure!(
        config.models.len() == 1 && config.headers.is_none() && config.env_vars.is_none(),
        "This profile has custom settings; preserve it and create a new profile to use the simple editor"
    );
    let p = Profile {
        id: config.name.clone(),
        revision: fingerprint(serde_json::to_value(config)?)?,
        credential_id: credential_id.into(),
        name: config.display_name.clone(),
        engine: engine(config).unwrap().into(),
        url: config.base_url.clone(),
        model: config.models[0].name.clone(),
    };
    p.validate()?;
    Ok(p)
}
fn fingerprint(mut value: serde_json::Value) -> Result<String> {
    // Goose model metadata contains HashMaps. Their iteration order must not
    // make an unchanged profile look like a stale edit after another load.
    value.sort_all_objects();
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&value)?)))
}
fn helper(id: &str) -> Result<api::AuthConfig> {
    ensure!(
        uuid::Uuid::parse_str(id).is_ok(),
        "Invalid credential identity"
    );
    Ok(api::AuthConfig {
        command: std::env::current_exe()?
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Fastrock path must be valid Unicode"))?
            .into(),
        args: vec!["--fastrock-provider-credential".into(), id.into()],
        refresh_interval: 300,
        timeout_seconds: Some(10),
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
        engine: desired.engine.clone(),
        display_name: desired.name.clone(),
        api_url: desired.url.clone(),
        api_key: None,
        models: vec![ModelInfo::new(&desired.model)],
        supports_streaming: Some(true),
        headers: None,
        requires_auth: true,
        catalog_provider_id: config.catalog_provider_id.clone(),
        base_path: config.base_path.clone(),
        toolshim: config.toolshim,
        preserves_thinking: None,
        auth: Some(auth),
    })
}
fn read() -> Result<Vec<Profile>> {
    let configs = api::load_custom_providers(&api::custom_providers_dir())
        .map_err(|_| anyhow::anyhow!("Could not read Goose provider profiles"))?;
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
            let auth = helper(id)?;
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
        let existing = if desired.id.is_empty() {
            None
        } else {
            let loaded = api::load_provider(&desired.id)
                .map_err(|_| anyhow::anyhow!("Profile no longer exists"))?;
            ensure!(
                loaded.is_editable && snapshot(&loaded.config)?.revision == desired.revision,
                "Profile changed; reload it before saving"
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
                        && old.model == desired.model,
                    "This form already created a profile; reload it to edit"
                );
                return Ok(old);
            }
            ensure!(!secret.is_empty(), "Enter an API key for the new profile");
        }
        let new_key = !secret.is_empty();
        let credential_id = if new_key {
            create_id
        } else {
            existing.as_ref().and_then(owned).unwrap_or("").into()
        };
        ensure!(
            !new_key || existing.as_ref().and_then(owned) != Some(credential_id.as_str()),
            "Credential replacement requires a fresh identity; reload the profile"
        );
        let auth = helper(&credential_id)?;
        if new_key {
            crate::vault::put("provider", &credential_id, &secret)?;
        }
        let saved = (|| -> Result<String> {
            if let Some(config) = &existing {
                update(config, &desired, auth)?;
                Ok(config.name.clone())
            } else {
                Ok(
                    api::create_custom_provider(api::CreateCustomProviderParams {
                        engine: desired.engine.clone(),
                        display_name: desired.name.clone(),
                        api_url: desired.url.clone(),
                        api_key: None,
                        models: vec![ModelInfo::new(&desired.model)],
                        supports_streaming: Some(true),
                        headers: None,
                        requires_auth: true,
                        catalog_provider_id: None,
                        base_path: None,
                        toolshim: false,
                        preserves_thinking: None,
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
                    let _ = crate::vault::remove("provider", &credential_id);
                }
                bail!("Could not save Goose provider profile; reload to inspect the configuration")
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
            anyhow::anyhow!(
                "Profile saved; restart Fastrock to refresh the Goose provider registry"
            )
        })?;
    super::provider_policy::refresh().await?;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
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
