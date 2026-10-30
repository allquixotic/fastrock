//! Non-secret provider setup through unchanged Goose's public configuration API.
//! Secret setters are intentionally excluded: upstream may fall back to a file.
use anyhow::{Result, bail, ensure};
use goose_runtime::config::{Config, providers};
use std::collections::BTreeMap;
use std::sync::Mutex;

pub(crate) const PROVIDERS: [&str; 3] = ["aws_bedrock", "openai", "anthropic"];
pub(crate) const FIELDS: [(&str, &str); 5] = [
    ("AWS_PROFILE", "AWS profile"),
    ("AWS_REGION", "AWS region"),
    ("AWS_ENDPOINT_URL_BEDROCK_RUNTIME", "Bedrock Runtime URL"),
    ("OPENAI_BASE_URL", "OpenAI base URL"),
    ("ANTHROPIC_HOST", "Anthropic host"),
];
static SETUP_GATE: Mutex<()> = Mutex::new(());

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Setup {
    pub provider: String,
    pub model: String,
    pub fields: BTreeMap<String, String>,
}

fn bounded(value: &str, limit: usize) -> bool {
    value.len() <= limit && !value.chars().any(char::is_control)
}

pub(super) fn validate_field(key: &str, value: &str) -> Result<()> {
    ensure!(
        FIELDS.iter().any(|(name, _)| *name == key),
        "Unsupported provider setting"
    );
    ensure!(
        bounded(value, 2048),
        "Provider settings must be single-line values of at most 2048 bytes"
    );
    if value.is_empty() {
        return Ok(());
    }
    match key {
        "AWS_PROFILE" => {}
        "AWS_REGION" => ensure!(
            value.len() <= 64
                && value
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
            "Enter a valid AWS region"
        ),
        _ => {
            let url = url::Url::parse(value)
                .map_err(|_| anyhow::anyhow!("Enter a valid provider URL"))?;
            let loopback = url
                .host_str()
                .is_some_and(|h| matches!(h, "localhost" | "127.0.0.1" | "[::1]"));
            ensure!(
                url.scheme() == "https" || (url.scheme() == "http" && loopback),
                "Provider URLs require HTTPS (local fixtures may use HTTP)"
            );
            ensure!(
                url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none(),
                "Provider URLs cannot contain credentials, query parameters or fragments"
            );
        }
    }
    Ok(())
}

impl Setup {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            super::provider_policy::allowed(&self.provider),
            "Select Bedrock, OpenAI or Anthropic"
        );
        ensure!(
            !self.model.trim().is_empty() && bounded(&self.model, 1024),
            "Enter a model ID of at most 1024 bytes"
        );
        ensure!(
            self.fields.len() == FIELDS.len(),
            "Reload the provider settings before saving"
        );
        for (key, value) in &self.fields {
            validate_field(key, value)?;
        }
        Ok(())
    }
}

fn read(config: &Config) -> Result<Setup> {
    let values = config
        .all_values()
        .map_err(|_| anyhow::anyhow!("Could not read Goose configuration"))?;
    let mut fields = BTreeMap::new();
    for (key, _) in FIELDS {
        let value = match values.get(key) {
            None => "",
            Some(value) => value.as_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "A provider setting has an unsupported value; correct it in Goose configuration"
                )
            })?,
        };
        // Never put credentials embedded in a legacy endpoint into the UI.
        validate_field(key, value)?;
        fields.insert(key.into(), value.into());
    }
    Ok(Setup {
        provider: config.get_goose_provider().unwrap_or_default(),
        model: config.get_goose_model().unwrap_or_default(),
        fields,
    })
}

fn write(config: &Config, expected: &Setup, desired: &Setup) -> Result<Setup> {
    desired.validate()?;
    ensure!(
        read(config)? == *expected,
        "Provider settings changed in another window. Reload before saving"
    );
    // Each operation preserves unrelated settings via Goose. Its public API
    // offers no transaction spanning provider selection and deleted fields.
    let result = (|| -> Result<()> {
        let updates = desired
            .fields
            .iter()
            .filter(|(key, value)| !value.is_empty() && expected.fields.get(*key) != Some(*value))
            .map(|(key, value)| (key.clone(), serde_json::json!(value)))
            .collect::<Vec<_>>();
        config.set_param_values(&updates)?;
        for (key, value) in &desired.fields {
            if value.is_empty() && expected.fields.get(key).is_some_and(|old| !old.is_empty()) {
                config.delete(key)?;
            }
        }
        if desired.provider != expected.provider || desired.model != expected.model {
            providers::set_active_provider(config, &desired.provider, &desired.model)?;
        }
        Ok(())
    })();
    if result.is_err() {
        bail!(
            "Provider settings could not be fully saved. Reload to review any changes before retrying"
        );
    }
    read(config)
}

pub(crate) async fn load() -> Result<Setup> {
    tokio::task::spawn_blocking(|| {
        let _guard = SETUP_GATE
            .lock()
            .map_err(|_| anyhow::anyhow!("Provider settings are unavailable"))?;
        read(Config::global())
    })
    .await?
}

pub(crate) async fn save(expected: Setup, desired: Setup) -> Result<Setup> {
    tokio::task::spawn_blocking(move || {
        let _guard = SETUP_GATE
            .lock()
            .map_err(|_| anyhow::anyhow!("Provider settings are unavailable"))?;
        write(Config::global(), &expected, &desired)
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v52_setup_preserves_credentials_extensions_and_other_providers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let secrets = dir.path().join("secrets.yaml");
        let original = "ANTHROPIC_API_KEY: fixture-do-not-touch\n";
        std::fs::write(&secrets, original).unwrap();
        std::fs::write(&path, "GOOSE_PROVIDER: aws_bedrock\nGOOSE_MODEL: openai.gpt-6-luna\nAWS_REGION: us-east-1\nextensions: {fixture: {enabled: true}}\nCUSTOM_SETTING: keep-me\n").unwrap();
        let config = Config::new_with_file_secrets(&path, &secrets).unwrap();
        providers::set_provider_entry(
            &config,
            "anthropic",
            &providers::ProviderEntry {
                enabled: true,
                configured: true,
                model: "keep-model".into(),
            },
        )
        .unwrap();
        let before = read(&config).unwrap();
        let mut desired = before.clone();
        desired.provider = "openai".into();
        desired.model = "gpt-6.1-sol".into();
        desired
            .fields
            .insert("OPENAI_BASE_URL".into(), "https://api.openai.com/v1".into());
        desired.fields.insert("AWS_REGION".into(), String::new());
        let after = write(&config, &before, &desired).unwrap();
        assert_eq!(after.provider, "openai");
        assert_eq!(after.model, "gpt-6.1-sol");
        assert_eq!(
            config.get_param::<String>("CUSTOM_SETTING").unwrap(),
            "keep-me"
        );
        assert_eq!(
            config.get_param::<serde_json::Value>("extensions").unwrap()["fixture"]["enabled"],
            true
        );
        assert_eq!(
            providers::get_provider_entry(&config, "anthropic")
                .unwrap()
                .model,
            "keep-model"
        );
        assert!(!config.all_values().unwrap().contains_key("AWS_REGION"));
        assert_eq!(std::fs::read_to_string(secrets).unwrap(), original);
        assert!(
            write(&config, &before, &desired).is_err(),
            "stale window must not overwrite new settings"
        );
    }
    #[test]
    fn v52_setup_rejects_secrets_and_invalid_endpoints_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            "GOOSE_PROVIDER: aws_bedrock\nGOOSE_MODEL: fixture-model\n",
        )
        .unwrap();
        let config = Config::new_with_file_secrets(&path, dir.path().join("secrets.yaml")).unwrap();
        let before = read(&config).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        for url in [
            "https://key@example.com",
            "https://example.com?key=secret",
            "http://example.com",
            "https://example.com#secret",
            "not a URL",
        ] {
            let mut desired = before.clone();
            desired.fields.insert("OPENAI_BASE_URL".into(), url.into());
            assert!(write(&config, &before, &desired).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        let mut desired = before.clone();
        desired.fields.remove("AWS_PROFILE");
        desired
            .fields
            .insert("OPENAI_API_KEY".into(), "secret".into());
        assert!(write(&config, &before, &desired).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(!dir.path().join("secrets.yaml").exists());
        assert!(validate_field("OPENAI_BASE_URL", "http://127.0.0.1:1234/v1").is_ok());
    }
}
