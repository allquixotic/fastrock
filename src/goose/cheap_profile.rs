//! The designated summary model is a Goose setting, separate from UI preferences.
use anyhow::{Context, Result, ensure};
use goose_runtime::config::Config;
use serde::{Deserialize, Serialize};

const KEY: &str = "FASTROCK_CHEAP_PROFILE";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheapProfile {
    pub name: String,
    pub provider: String,
    pub model: String,
}
impl Default for CheapProfile {
    fn default() -> Self {
        Self {
            name: "Bedrock Luna".into(),
            provider: "aws_bedrock".into(),
            model: "openai.gpt-6-luna".into(),
        }
    }
}
impl CheapProfile {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty() && self.name.len() <= 128,
            "Enter a profile name of at most 128 bytes"
        );
        ensure!(
            super::provider_policy::allowed(&self.provider),
            "Select a supported Goose provider"
        );
        ensure!(
            !self.model.trim().is_empty()
                && self.model.len() <= 1024
                && !self.model.chars().any(char::is_control),
            "Enter a valid model ID"
        );
        Ok(())
    }
}
fn read(config: &Config) -> Result<CheapProfile> {
    let profile = match config.get_param::<CheapProfile>(KEY) {
        Ok(profile) => profile,
        Err(goose_runtime::config::ConfigError::NotFound(_)) => CheapProfile::default(),
        Err(_) => anyhow::bail!("The saved cheap model profile is invalid"),
    };
    profile.validate()?;
    Ok(profile)
}
pub(super) async fn load() -> Result<CheapProfile> {
    tokio::task::spawn_blocking(|| read(Config::global())).await?
}
pub(super) async fn save(profile: CheapProfile) -> Result<()> {
    profile.validate()?;
    tokio::task::spawn_blocking(move || {
        super::config_guard::validate(Config::global())?;
        Config::global()
            .set_param(KEY, &profile)
            .context("Save cheap model profile")
    })
    .await?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v44_cheap_profile_defaults_to_luna_and_never_expensive_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "GOOSE_MODEL: expensive-existing\nextensions: {}\n").unwrap();
        let config =
            Config::new_with_file_secrets(&path, dir.path().join("fixture-secrets.yaml")).unwrap();
        assert_eq!(read(&config).unwrap(), CheapProfile::default());
        config.set_param(KEY, serde_json::json!({"name":"invalid","provider":"external-agent","model":"expensive"})).unwrap();
        assert!(read(&config).is_err());
        config
            .set_param(
                KEY,
                CheapProfile {
                    name: "My cheap model".into(),
                    provider: "openai".into(),
                    model: "gpt-6-luna".into(),
                },
            )
            .unwrap();
        assert_eq!(read(&config).unwrap().provider, "openai");
        assert_eq!(
            config.get_param::<String>("GOOSE_MODEL").unwrap(),
            "expensive-existing"
        );
        assert!(!dir.path().join("fixture-secrets.yaml").exists());
    }
}
