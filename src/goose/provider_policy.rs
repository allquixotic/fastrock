//! Apply Fastrock's provider scope before unchanged Goose constructs an agent.
//! ACP's provider factory only covers some paths; session creation, restoration
//! and switching also use Goose's internal registry directly.
use anyhow::{Result, ensure};
use goose_runtime::config::Config;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{LazyLock, RwLock},
};
#[derive(Default)]
struct Snapshot {
    custom: BTreeMap<String, String>,
    ready: BTreeSet<String>,
    routes: BTreeMap<String, super::speed::Route>,
}
static SNAPSHOT: LazyLock<RwLock<Snapshot>> = LazyLock::new(|| RwLock::new(Snapshot::default()));
static REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
pub(super) async fn refresh() -> Result<()> {
    let _guard = REFRESH.lock().await;
    let snapshot = tokio::task::spawn_blocking(|| -> Result<Snapshot> {
        use goose_runtime::config::declarative_providers as api;
        let config = Config::global();
        let mut result = Snapshot::default();
        for name in super::setup::PROVIDERS {
            if super::provider_readiness::builtin(config, name) {
                result.ready.insert(name.into());
            }
        }
        for p in api::load_custom_providers(&api::custom_providers_dir())? {
            if !matches!(
                p.engine,
                api::ProviderEngine::OpenAI | api::ProviderEngine::Anthropic
            ) {
                continue;
            }
            result.custom.insert(p.name.clone(), p.display_name.clone());
            if super::provider_readiness::custom(config, &p) {
                result.ready.insert(p.name.clone());
            }
            if p.engine == api::ProviderEngine::OpenAI {
                if let Ok(url) =
                    api::expand_env_vars(&p.base_url, p.env_vars.as_deref().unwrap_or_default())
                {
                    result.routes.insert(
                        p.name,
                        super::speed::Route {
                            url,
                            path: p.base_path.unwrap_or_default(),
                        },
                    );
                }
            }
        }
        // Built-in OpenAI derives a Chat Completions path for Bedrock's /openai/v1
        // prefix. It cannot select Ultrafast with this pinned dependency. A named
        // Responses profile uses the public declarative base_path option instead.
        Ok(result)
    })
    .await??;
    *SNAPSHOT
        .write()
        .map_err(|_| anyhow::anyhow!("Provider choices unavailable"))? = snapshot;
    Ok(())
}
pub(super) fn ultrafast(provider: &str, model: &str) -> bool {
    SNAPSHOT
        .read()
        .is_ok_and(|s| s.routes.get(provider).is_some_and(|r| r.ultrafast(model)))
}
pub(super) fn choices() -> Vec<(String, String)> {
    let mut choices = vec![
        ("aws_bedrock".into(), "Bedrock".into()),
        ("openai".into(), "OpenAI".into()),
        ("anthropic".into(), "Anthropic".into()),
    ];
    if let Ok(snapshot) = SNAPSHOT.read() {
        choices.extend(
            snapshot
                .custom
                .iter()
                .map(|(id, name)| (id.clone(), name.clone())),
        );
    }
    choices
}

pub(super) fn allowed(name: &str) -> bool {
    super::setup::PROVIDERS.contains(&name)
        || SNAPSHOT
            .read()
            .is_ok_and(|snapshot| snapshot.custom.contains_key(name))
}
fn resolve(config: &Config, requested: Option<&str>) -> Result<String> {
    let name = match requested.filter(|s| !s.is_empty() && *s != "goose") {
        Some(name) => name.to_owned(),
        None => config.get_goose_provider().map_err(|_| {
            anyhow::anyhow!("Select a provider in Settings → Providers → Provider defaults first")
        })?,
    };
    ensure!(
        super::embedded::supported_provider(&name),
        "Fastrock supports Bedrock, OpenAI and Anthropic model providers. External agent backends and other providers are disabled; choose a supported provider in provider settings."
    );
    Ok(name)
}
pub(super) async fn selected(requested: Option<String>) -> Result<String> {
    tokio::task::spawn_blocking(move || resolve(Config::global(), requested.as_deref())).await?
}
pub(super) async fn restored(id: &str) -> Result<()> {
    let manager = goose_runtime::session::SessionManager::new(
        goose_runtime::config::paths::Paths::data_dir(),
    );
    let session = manager
        .get_session(id, false)
        .await
        .map_err(|_| anyhow::anyhow!("Cannot inspect the saved conversation provider"))?;
    selected(session.provider_name).await?;
    Ok(())
}
pub(super) fn visible_options(options: Vec<Value>) -> Vec<Value> {
    let ready = SNAPSHOT.read().map(|s| s.ready.clone()).unwrap_or_default();
    filter_options(options, &ready)
}
fn filter_options(mut options: Vec<Value>, ready: &BTreeSet<String>) -> Vec<Value> {
    for option in &mut options {
        if option["id"] == "provider" {
            if let Some(choices) = option["options"].as_array_mut() {
                filter_choices(choices, ready);
            }
        }
    }
    options
}
fn filter_choices(choices: &mut Vec<Value>, ready: &BTreeSet<String>) {
    choices.retain_mut(|choice| {
        if let Some(group) = choice["options"].as_array_mut() {
            filter_choices(group, ready);
            !group.is_empty()
        } else {
            choice["value"]
                .as_str()
                .is_some_and(|name| ready.contains(name))
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn v57_provider_scope_checks_defaults_and_explicit_selection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "GOOSE_PROVIDER: codex\nGOOSE_MODEL: preserved\n").unwrap();
        let config = Config::new_with_file_secrets(&path, dir.path().join("secrets.yaml")).unwrap();
        let original = std::fs::read(&path).unwrap();
        for name in [
            None,
            Some("goose"),
            Some("codex"),
            Some("claude_acp"),
            Some("ollama"),
            Some("remote"),
        ] {
            assert!(resolve(&config, name).is_err());
        }
        for name in super::super::setup::PROVIDERS {
            assert_eq!(resolve(&config, Some(name)).unwrap(), name);
        }
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(!dir.path().join("secrets.yaml").exists());
        config.set_goose_provider("aws_bedrock").unwrap();
        assert_eq!(resolve(&config, Some("goose")).unwrap(), "aws_bedrock");
    }
    #[test]
    fn v57_provider_choices_filter_groups_and_preserve_other_options() {
        let options = vec![
            json!({"id":"provider","currentValue":"aws_bedrock","options":[
                {"value":"goose"}, {"name":"cloud","options":[{"value":"aws_bedrock"},{"value":"openai"},{"value":"anthropic"},{"value":"codex"}]},
                {"name":"agents","options":[{"value":"claude_acp"}]}
            ]}),
            json!({"id":"thinking_effort","options":[{"value":"high"}]}),
        ];
        let filtered = filter_options(options.clone(), &BTreeSet::from(["aws_bedrock".into()]));
        assert_eq!(filtered[0]["options"].as_array().unwrap().len(), 1);
        assert_eq!(
            filtered[0]["options"][0]["options"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(filtered[1], options[1]);
        assert_eq!(filtered[0]["currentValue"], "aws_bedrock");
    }
}
