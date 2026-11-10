//! Unmodified Goose runs on our Tokio runtime through bounded memory streams.
//! These are ordinary allocations: no listener, subprocess, stdin or stdout.
use goose_runtime::acp::server::{
    AcpBuiltinSelection, AcpConnection, AcpConnectionOptions, AcpProviderFactory, LiveVoiceService,
    acp_services_config,
};
use goose_runtime::agents::{GoosePlatform, StateMachineServices};
use goose_runtime::config::paths::Paths;
use std::sync::Arc;
use tokio::io::{DuplexStream, ReadHalf, WriteHalf};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

pub(super) struct Connection {
    pub reader: ReadHalf<DuplexStream>,
    pub writer: WriteHalf<DuplexStream>,
    pub task: tokio::task::JoinHandle<()>,
}

pub(super) async fn connect() -> anyhow::Result<Connection> {
    goose_runtime::builtin_extension::register_builtin_extensions(
        goose_mcp::BUILTIN_EXTENSIONS.clone(),
    );
    super::memory_tool::register();
    let (client, embedded) = tokio::io::duplex(256 * 1024);
    let (reader, writer) = tokio::io::split(client);
    let task = tokio::spawn(super::embedded_router::serve(embedded));
    Ok(Connection {
        reader,
        writer,
        task,
    })
}

/// Goose 1.58 snapshots hooks in services. Give each activated conversation its
/// own services, retaining the shared public session store and unchanged runtime.
pub(super) async fn connect_session(cwd: Option<std::path::PathBuf>) -> anyhow::Result<Connection> {
    let mut config = acp_services_config(
        Arc::new(goose_runtime::session::SessionManager::instance()),
        Paths::config_dir(),
        goose_runtime::config::Config::global()
            .get_goose_disable_session_naming()
            .unwrap_or(false),
        GoosePlatform::GooseDesktop,
    );
    // Permissions are global settings, while hooks are a conversation snapshot.
    // Every connection must observe edits made through the control connection.
    config.permission_manager = goose_runtime::config::permission::PermissionManager::instance();
    config.hooks = Some(
        tokio::task::spawn_blocking(move || {
            goose_runtime::hooks::HookManager::load(cwd.as_deref())
        })
        .await?,
    );
    let services = StateMachineServices::with_config(config);
    let live_voice = Arc::new(LiveVoiceService::from_config(services.runtime.runs.clone()));
    let provider_factory: AcpProviderFactory = Arc::new(|name| {
        Box::pin(async move {
            anyhow::ensure!(
                tokio::task::spawn_blocking({
                    let name = name.clone();
                    move || supported_provider(&name)
                })
                .await?,
                "Fastrock supports Bedrock, OpenAI and Anthropic providers; external agent providers are disabled"
            );
            goose_runtime::providers::create_with_default_model(&name, Vec::new()).await
        })
    });
    let agent = Arc::new(
        AcpConnection::new(AcpConnectionOptions {
            provider_factory,
            builtin_selection: AcpBuiltinSelection {
                defaults: vec!["developer".into(), super::memory_tool::NAME.into()],
                explicit: Vec::new(),
            },
            config_dir: Paths::config_dir(),
            additional_source_roots: Vec::new(),
            session_cwd: None,
            // Fastrock owns orchestration and restart/resume policy. Do not start
            // Goose's separate scheduler implicitly when opening a connection.
            services,
            live_voice,
        })
        .await?,
    );
    let (client, embedded) = tokio::io::duplex(256 * 1024);
    let (reader, writer) = tokio::io::split(client);
    let (agent_reader, agent_writer) = tokio::io::split(embedded);
    let task = tokio::spawn(async move {
        // Closing the memory stream reports failure through the existing
        // bounded adapter channel. Provider errors may contain secrets, so do
        // not persist the underlying error here.
        let _ = goose_runtime::acp::server::serve(
            Arc::clone(&agent),
            agent_reader.compat(),
            agent_writer.compat_write(),
        )
        .await;
    });
    Ok(Connection {
        reader,
        writer,
        task,
    })
}

// Public declarative APIs describe the actual engine; never infer provider
// capabilities from a user-selected display name or launch an agent backend.
pub(super) fn supported_provider(name: &str) -> bool {
    if matches!(name, "aws_bedrock" | "openai" | "anthropic") {
        return true;
    }
    use goose_runtime::config::declarative_providers::{ProviderEngine, load_provider};
    load_provider(name).is_ok_and(|p| {
        p.is_editable
            && matches!(
                p.config.engine,
                ProviderEngine::OpenAI | ProviderEngine::Anthropic
            )
    })
}
