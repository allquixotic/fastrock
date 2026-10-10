//! External CLI startup inputs. This module contains no agent/inference runtime.
use codex_model_provider_info::ModelProviderInfo;
use codex_protocol::openai_models::ReasoningEffort;
use std::collections::HashMap;
use std::path::PathBuf;
pub(crate) const GUI_CLIENT_NAME: &str = "fastrock";
#[derive(Clone, Debug, Default)]
pub struct Arg0DispatchPaths;
#[derive(Clone, Debug)]
pub(crate) struct StartupOptions {
    pub connection: Result<crate::connection::ConnectionTarget, String>,
    pub cwd: Option<PathBuf>,
    pub cli_overrides: Vec<(String, toml::Value)>,
}
/// Read-only configuration snapshot used by upstream presentation code.
/// Authoritative configuration always belongs to the installed external Codex.
pub(crate) struct Config {
    pub model_provider_id: String,
    pub model_provider: ModelProviderInfo,
    pub model_providers: HashMap<String, ModelProviderInfo>,
    pub model: Option<String>,
    pub plan_mode_reasoning_effort: Option<ReasoningEffort>,
    pub feedback_enabled: bool,
    pub startup_warnings: Vec<String>,
    pub log_dir: PathBuf,
    pub cwd: PathBuf,
    pub codex_home: PathBuf,
    pub developer_instructions: Option<String>,
    pub show_raw_agent_reasoning: bool,
    pub hide_agent_reasoning: bool,
}
impl Config {
    pub fn http_client_factory(&self) -> codex_http_client::HttpClientFactory {
        codex_http_client::HttpClientFactory::new(
            codex_http_client::OutboundProxyPolicy::RespectSystemProxy,
        )
    }
}
pub(crate) fn log_startup_error(message: &str) {
    tracing::error!(error = %message, "external Codex startup failed");
}

/// Read the external server's effective config; no second config/inference loader.
pub(crate) async fn read_snapshot(
    client: &crate::transport::AppServerClient,
    options: &StartupOptions,
) -> Option<Config> {
    let response=client.request_handle().raw(serde_json::json!({"id":"fastrock-config-snapshot","method":"config/read","params":{"includeLayers":false,"cwd":options.cwd.as_ref().map(|p|p.to_string_lossy())}})).await.ok()?.ok()?;
    let value = &response["config"];
    let model_provider_id = value["model_provider"]
        .as_str()
        .unwrap_or("openai")
        .to_string();
    let mut model_providers = codex_model_provider_info::built_in_model_providers(None);
    if let Some(custom) = value["model_providers"].as_object() {
        for (id, value) in custom {
            if let Ok(provider) = serde_json::from_value(value.clone()) {
                model_providers.insert(id.clone(), provider);
            }
        }
    }
    let model_provider = model_providers.get(&model_provider_id)?.clone();
    let codex_home = client
        .codex_home()
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CODEX_HOME").map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|p| p.join(".codex")))?;
    Some(Config {
        model_provider_id,
        model_provider,
        model_providers,
        model: value["model"].as_str().map(str::to_owned),
        plan_mode_reasoning_effort: serde_json::from_value(
            value["plan_mode_reasoning_effort"].clone(),
        )
        .ok(),
        feedback_enabled: value["feedback"]["enabled"].as_bool().unwrap_or(true),
        startup_warnings: vec![],
        log_dir: codex_home.join("log"),
        cwd: options
            .cwd
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default(),
        codex_home,
        developer_instructions: value["developer_instructions"].as_str().map(str::to_owned),
        show_raw_agent_reasoning: value["show_raw_agent_reasoning"].as_bool().unwrap_or(false),
        hide_agent_reasoning: value["hide_agent_reasoning"].as_bool().unwrap_or(false),
    })
}
