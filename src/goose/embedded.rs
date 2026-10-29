//! Unmodified Goose runs on our Tokio runtime through bounded memory streams.
//! These are ordinary allocations: no listener, subprocess, stdin or stdout.
use goose_runtime::acp::server::{
    AcpBuiltinSelection, AcpProviderFactory, ActiveRunRegistry, GooseAcpAgent,
    GooseAcpAgentOptions, LiveVoiceService,
};
use goose_runtime::agents::GoosePlatform;
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
    let active_runs = Arc::new(ActiveRunRegistry::default());
    let live_voice = Arc::new(LiveVoiceService::from_config(active_runs.clone()));
    let provider_factory: AcpProviderFactory = Arc::new(|name| {
        Box::pin(async move {
            anyhow::ensure!(
                matches!(name.as_str(), "aws_bedrock" | "openai" | "anthropic"),
                "Fastrock supports Goose Bedrock, OpenAI and Anthropic providers; external agent providers are disabled"
            );
            goose_runtime::providers::create_with_default_model(&name, Vec::new()).await
        })
    });
    let agent = Arc::new(
        GooseAcpAgent::new(GooseAcpAgentOptions {
            provider_factory,
            builtin_selection: AcpBuiltinSelection::from_requested(Vec::new()),
            data_dir: Paths::data_dir(),
            config_dir: Paths::config_dir(),
            disable_session_naming: goose_runtime::config::Config::global()
                .get_goose_disable_session_naming()
                .unwrap_or(false),
            goose_platform: GoosePlatform::GooseDesktop,
            additional_source_roots: Vec::new(),
            session_cwd: None,
            // Fastrock owns orchestration and restart/resume policy. Do not start
            // Goose's separate scheduler implicitly when opening a connection.
            scheduler: None,
            active_runs,
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
