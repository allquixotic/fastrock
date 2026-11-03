//! Local configuration checks only: never probe a model or execute credentials.
use goose_runtime::config::{Config, declarative_providers as api, providers};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(super) fn nonblank(value: Option<String>) -> bool {
    value.is_some_and(|s| !s.trim().is_empty())
}
fn value(config: &Config, key: &str) -> Option<String> {
    config.get_param::<String>(key).ok()
}
fn secret(config: &Config, key: &str) -> bool {
    nonblank(config.get_secret::<String>(key).ok())
}
pub(super) fn model_configured(config: &Config, name: &str, declared: bool) -> bool {
    if let Some(entry) = providers::get_provider_entry(config, name) {
        if !entry.enabled {
            return false;
        }
        if !entry.model.trim().is_empty() {
            return true;
        }
    }
    declared
        || (config.get_goose_provider().ok().as_deref() == Some(name)
            && nonblank(config.get_goose_model().ok()))
}
fn endpoint(config: &Config, key: &str) -> bool {
    value(config, key).is_none_or(|s| {
        url::Url::parse(&s)
            .is_ok_and(|u| matches!(u.scheme(), "https" | "http") && u.host_str().is_some())
    })
}
pub(super) fn builtin(config: &Config, name: &str) -> bool {
    model_configured(config, name, false)
        && match name {
            "openai" => secret(config, "OPENAI_API_KEY") && endpoint(config, "OPENAI_BASE_URL"),
            "anthropic" => {
                secret(config, "ANTHROPIC_API_KEY") && endpoint(config, "ANTHROPIC_HOST")
            }
            "aws_bedrock" => aws_configured(config),
            _ => false,
        }
}
pub(super) fn credential_id(config: &api::DeclarativeProviderConfig) -> Option<&str> {
    let auth = config.auth.as_ref()?;
    (matches!(
        config.engine,
        api::ProviderEngine::OpenAI | api::ProviderEngine::Anthropic
    ) && config.api_key_env.is_empty()
        && auth.args.len() == 2
        && auth.args[0] == "--fastrock-provider-credential"
        && uuid::Uuid::parse_str(&auth.args[1]).is_ok())
    .then(|| auth.args[1].as_str())
}
pub(super) fn custom(config: &Config, p: &api::DeclarativeProviderConfig) -> bool {
    if !model_configured(
        config,
        &p.name,
        p.models.iter().any(|m| !m.name.trim().is_empty()),
    ) || p.validate_auth().is_err()
    {
        return false;
    }
    let env = p.env_vars.as_deref().unwrap_or_default();
    for var in env.iter().filter(|v| v.required) {
        let supplied = if var.secret {
            config.get_secret::<String>(&var.name).ok()
        } else {
            value(config, &var.name)
        };
        if !nonblank(supplied.or_else(|| var.default.clone())) {
            return false;
        }
    }
    let Ok(url) = api::expand_env_vars(&p.base_url, env) else {
        return false;
    };
    if url.contains("${")
        || url::Url::parse(&url)
            .ok()
            .is_none_or(|u| !matches!(u.scheme(), "http" | "https") || u.host_str().is_none())
    {
        return false;
    }
    if let Some(id) = credential_id(p) {
        return nonblank(crate::vault::get("provider", id).ok());
    }
    if let Some(auth) = &p.auth {
        return command_exists(&auth.command, auth.cwd.as_deref());
    }
    !p.requires_auth || (!p.api_key_env.trim().is_empty() && secret(config, &p.api_key_env))
}
fn command_exists(command: &str, cwd: Option<&str>) -> bool {
    if command.trim().is_empty() || cwd.is_some_and(|d| !Path::new(d).is_dir()) {
        return false;
    }
    let path = Path::new(command);
    if path.is_absolute() || command.contains(['/', '\\']) {
        return cwd
            .map_or_else(|| path.to_path_buf(), |dir| Path::new(dir).join(path))
            .is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            if dir.join(command).is_file() {
                return true;
            }
            #[cfg(windows)]
            if std::env::var("PATHEXT")
                .unwrap_or_else(|_| ".EXE;.COM;.BAT;.CMD".into())
                .split(';')
                .any(|ext| dir.join(format!("{command}{ext}")).is_file())
            {
                return true;
            }
            false
        })
    })
}
// AWS SDK sources may be IAM/SSO/command credentials rather than an API key.
// Inspect bounded local configuration; validity/expiry is resolved on actual use.
fn aws_configured(config: &Config) -> bool {
    if secret(config, "AWS_BEARER_TOKEN_BEDROCK")
        || (secret(config, "AWS_ACCESS_KEY_ID") && secret(config, "AWS_SECRET_ACCESS_KEY"))
    {
        return true;
    }
    if [
        "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
        "AWS_CONTAINER_CREDENTIALS_FULL_URI",
    ]
    .iter()
    .any(|k| nonblank(value(config, k)))
    {
        return true;
    }
    if nonblank(value(config, "AWS_ROLE_ARN"))
        && value(config, "AWS_WEB_IDENTITY_TOKEN_FILE").is_some_and(|p| Path::new(&p).is_file())
    {
        return true;
    }
    let home = dirs::home_dir().unwrap_or_default().join(".aws");
    let path = |key, fallback: &str| {
        value(config, key)
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(fallback))
    };
    let mut profiles = ini(&path("AWS_CONFIG_FILE", "config"));
    for (name, fields) in ini(&path("AWS_SHARED_CREDENTIALS_FILE", "credentials")) {
        profiles.entry(name).or_default().extend(fields);
    }
    let name = value(config, "AWS_PROFILE")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "default".into());
    aws_profile(&profiles, &name, 0)
}
type Profiles = BTreeMap<String, BTreeMap<String, String>>;
fn ini(path: &Path) -> Profiles {
    use std::io::Read;
    let mut bytes = Vec::new();
    let Ok(file) = std::fs::File::open(path) else {
        return Profiles::new();
    };
    if file.take(1_048_577).read_to_end(&mut bytes).is_err() || bytes.len() > 1_048_576 {
        return Profiles::new();
    }
    let mut result = Profiles::new();
    let mut section = String::new();
    for line in String::from_utf8_lossy(&bytes).lines().map(str::trim) {
        if line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.strip_prefix("profile ").unwrap_or(name).trim().into();
        } else if let Some((key, val)) = line.split_once('=') {
            result
                .entry(section.clone())
                .or_default()
                .insert(key.trim().into(), val.trim().into());
        }
    }
    result
}
fn aws_profile(profiles: &Profiles, name: &str, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    let Some(p) = profiles.get(name) else {
        return false;
    };
    let has = |key: &str| p.get(key).is_some_and(|s| !s.trim().is_empty());
    (has("aws_access_key_id") && has("aws_secret_access_key"))
        || has("credential_process")
        || (has("sso_account_id")
            && has("sso_role_name")
            && (has("sso_start_url")
                || p.get("sso_session").is_some_and(|s| {
                    profiles.get(&format!("sso-session {s}")).is_some_and(|s| {
                        s.get("sso_start_url").is_some_and(|v| !v.trim().is_empty())
                    })
                })))
        || (has("role_arn")
            && (has("web_identity_token_file")
                || p.get("source_profile")
                    .is_some_and(|s| aws_profile(profiles, s, depth + 1))
                || p.get("credential_source")
                    .is_some_and(|s| matches!(s.as_str(), "Ec2InstanceMetadata" | "EcsContainer"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v89_blank_credentials_models_and_disabled_providers_are_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "GOOSE_PROVIDER: custom_test\nGOOSE_MODEL: model\nproviders:\n  custom_disabled: {enabled: false, configured: true, model: model}\n").unwrap();
        let config = Config::new_with_file_secrets(&path, dir.path().join("secrets.yaml")).unwrap();
        assert!(!nonblank(Some(" \n\t".into())));
        assert!(!model_configured(&config, "custom_empty", false));
        assert!(!model_configured(&config, "custom_disabled", true));
        assert!(model_configured(&config, "custom_test", false));
        let mut p: api::DeclarativeProviderConfig = serde_json::from_value(serde_json::json!({
            "name":"custom_test", "display_name":"Test", "engine":"openai", "base_url":"http://127.0.0.1/v1",
            "models":[{"name":"model"}], "api_key_env":"FASTROCK_V89_TEST_KEY", "requires_auth":true
        })).unwrap();
        assert!(!custom(&config, &p));
        config.set_secret("FASTROCK_V89_TEST_KEY", &"  ").unwrap();
        assert!(!custom(&config, &p));
        config
            .set_secret("FASTROCK_V89_TEST_KEY", &"fixture")
            .unwrap();
        assert!(custom(&config, &p));
        p.base_url.clear();
        assert!(!custom(&config, &p));
    }
    #[test]
    fn v89_aws_profiles_support_sso_helpers_roles_and_reject_cycles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::write(&path, "[profile work]\ncredential_process = helper\n[profile role]\nrole_arn = arn\nsource_profile = work\n[profile sso]\nsso_session = work\nsso_account_id = account\nsso_role_name = role\n[sso-session work]\nsso_start_url = https://example.com\n[profile empty]\naws_access_key_id = \n[profile cycle]\nrole_arn = arn\nsource_profile = cycle\n").unwrap();
        let profiles = ini(&path);
        for name in ["work", "role", "sso"] {
            assert!(aws_profile(&profiles, name, 0));
        }
        for name in ["empty", "cycle", "missing"] {
            assert!(!aws_profile(&profiles, name, 0));
        }
    }
}
