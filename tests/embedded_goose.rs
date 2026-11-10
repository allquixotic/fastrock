//! Standalone fixture process: environment isolation precedes any runtime threads.
#[path = "fixtures/benchmark_seed.rs"]
mod benchmark_seed;
#[path = "../src/goose/cheap_profile.rs"]
mod cheap_profile;
#[path = "../src/goose/config_guard.rs"]
mod config_guard;
#[path = "../src/goose/embedded.rs"]
mod embedded;
#[path = "../src/goose/embedded_router.rs"]
mod embedded_router;
#[path = "../src/goose/memory.rs"]
mod memory;
#[path = "../src/goose/memory_tool.rs"]
mod memory_tool;
#[path = "../src/goose/summary_service.rs"]
mod summary_service;
#[path = "../src/goose/summary_worker.rs"]
mod summary_worker;
mod model {
    pub fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }
    pub fn ui_id(id: &str) -> String {
        format!("goose:{id}")
    }
}

use anyhow::{Context, Result, ensure};
use goose_provider_types::{base::Provider, conversation::message::Message, model::ModelConfig};
use goose_runtime::providers::bedrock::BedrockProvider;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--seed-benchmark") {
        return benchmark_seed::run();
    }
    // The CI runner separately invokes window-only tests on every executable.
    if std::env::args().nth(1).as_deref() == Some("window_runtime::tests") {
        return Ok(());
    }
    let temp = tempfile::tempdir()?;
    std::fs::create_dir_all(temp.path().join("config"))?;
    std::fs::write(
        temp.path().join("config/config.yaml"),
        "GOOSE_PROVIDER: aws_bedrock\nGOOSE_MODEL: us.openai.gpt-6.1-sol\nBEDROCK_DISABLE_STREAMING: true\nBEDROCK_MAX_RETRIES: 0\nGOOSE_DISABLE_SESSION_NAMING: true\nextensions: {}\n",
    )?;
    std::fs::write(temp.path().join("aws-empty"), "")?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    // SAFETY: harness=false enters this main on its only thread. No libraries
    // that start threads or inspect provider configuration have run yet.
    unsafe {
        std::env::set_var("GOOSE_PATH_ROOT", temp.path());
        std::env::set_var("GOOSE_DISABLE_KEYRING", "1");
        std::env::set_var("AWS_CONFIG_FILE", temp.path().join("aws-empty"));
        std::env::set_var("AWS_SHARED_CREDENTIALS_FILE", temp.path().join("aws-empty"));
        for key in [
            "AWS_PROFILE",
            "AWS_SESSION_TOKEN",
            "AWS_BEARER_TOKEN_BEDROCK",
            "GOOSE_PROVIDER",
            "GOOSE_MODEL",
        ] {
            std::env::remove_var(key);
        }
        std::env::set_var("AWS_REGION", "us-east-1");
        std::env::set_var("AWS_ACCESS_KEY_ID", "fixture-access");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "fixture-secret");
        std::env::set_var("AWS_EC2_METADATA_DISABLED", "true");
        std::env::set_var("AWS_ENDPOINT_URL_BEDROCK_RUNTIME", &endpoint);
        std::env::set_var("AWS_ENDPOINT_URL_BEDROCK", &endpoint);
        std::env::set_var("AWS_ENDPOINT_URL", &endpoint);
    }
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(60), fixture(listener)).await?
        })
}

async fn fixture(listener: std::net::TcpListener) -> Result<()> {
    use axum::{Json, Router, extract::State, routing::post};
    use std::sync::Arc;
    let captured = Arc::new(tokio::sync::Mutex::new(Vec::<(String, Value)>::new()));
    async fn respond(
        State(captured): State<Arc<tokio::sync::Mutex<Vec<(String, Value)>>>>,
        uri: axum::http::Uri,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let model = body["model"].clone();
        captured.lock().await.push((uri.to_string(), body));
        if uri.path() == "/openai/v1/responses" {
            return Json(
                json!({"id":"fixture-response","object":"response","created_at":0,
                "status":"completed","model":model,"output":[{"id":"fixture-message","type":"message","role":"assistant","status":"completed",
                "content":[{"type":"output_text","text":"speed fixture reply","annotations":[]}]}],
                "usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}),
            );
        }
        Json(
            json!({"output":{"message":{"role":"assistant","content":[{"text":"fixture reply"}]}},"stopReason":"end_turn","usage":{"inputTokens":2,"outputTokens":2,"totalTokens":4},"metrics":{"latencyMs":1}}),
        )
    }
    let router = Router::new()
        .route("/{*path}", post(respond))
        .with_state(captured.clone());
    let listener = tokio::net::TcpListener::from_std(listener)?;
    let http = tokio::spawn(async move { axum::serve(listener, router).await });
    let provider = BedrockProvider::from_env(None).await?;
    for model in [
        "us.openai.gpt-6.1-sol",
        "us.openai.gpt-6-luna",
        "us.anthropic.claude-opus-5-5",
    ] {
        let config = ModelConfig::new(model);
        let (reply, _) = provider
            .complete(
                &config,
                "Fixture only",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await?;
        ensure!(
            reply.as_concat_text().contains("fixture reply"),
            "Missing fixture response for {model}"
        );
    }
    let requests = captured.lock().await;
    ensure!(
        requests.len() == 3,
        "Unexpected retry or missing model request"
    );
    for ((path, body), model) in requests.iter().zip([
        "us.openai.gpt-6.1-sol",
        "us.openai.gpt-6-luna",
        "us.anthropic.claude-opus-5-5",
    ]) {
        ensure!(
            path.starts_with("/model/") && path.ends_with("/converse") && path.contains(model),
            "Unexpected Bedrock Runtime route: {path}"
        );
        ensure!(
            body["messages"][0]["content"][0]["text"] == "hello",
            "Input lost"
        );
    }
    drop(requests);
    let connection = embedded::connect().await?;
    let mut writer = connection.writer;
    let mut reader = BufReader::new(connection.reader);
    writer.write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"fastrock-fixture","version":"0.4.0"}}})).as_bytes()).await?;
    let mut line = String::new();
    loop {
        line.clear();
        ensure!(
            reader.read_line(&mut line).await? > 0,
            "Embedded Goose closed before initialization"
        );
        let response: Value = serde_json::from_str(&line)?;
        if response["id"] == 1 {
            ensure!(
                response.get("error").is_none(),
                "Embedded initialization failed: {}",
                response["error"]
            );
            response["result"]
                .as_object()
                .context("Missing initialization result")?;
            break;
        }
    }
    let cwd = std::env::var("GOOSE_PATH_ROOT")?;
    writer.write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":cwd,"mcpServers":[],"_meta":{"enabledExtensions":[]}}})).as_bytes()).await?;
    let session = loop {
        line.clear();
        ensure!(
            reader.read_line(&mut line).await? > 0,
            "Session creation disconnected"
        );
        let response: Value = serde_json::from_str(&line)?;
        if response["id"] == 2 {
            ensure!(
                response.get("error").is_none(),
                "Session creation failed: {}",
                response["error"]
            );
            break response["result"]["sessionId"]
                .as_str()
                .context("Missing session ID")?
                .to_owned();
        }
    };
    writer.write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":session,"prompt":[{"type":"text","text":"embedded turn fixture"}]}})).as_bytes()).await?;
    let mut streamed = false;
    loop {
        line.clear();
        ensure!(
            reader.read_line(&mut line).await? > 0,
            "Prompt disconnected"
        );
        let response: Value = serde_json::from_str(&line)?;
        if response["method"] == "session/update"
            && response["params"]["update"]["sessionUpdate"] == "agent_message_chunk"
        {
            streamed |= response["params"]["update"]["content"]["text"]
                .as_str()
                .is_some_and(|text| text.contains("fixture reply"));
        }
        if response["id"] == 3 {
            ensure!(
                response.get("error").is_none(),
                "Prompt failed: {}",
                response["error"]
            );
            ensure!(
                response["result"]["stopReason"] == "end_turn",
                "Prompt did not finish"
            );
            break;
        }
    }
    ensure!(
        streamed,
        "No assistant text notification from embedded turn"
    );
    ensure!(
        captured.lock().await.len() == 4,
        "Embedded turn did not use the fixture exactly once"
    );
    writer.write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":4,"method":"session/close","params":{"sessionId":session}})).as_bytes()).await?;
    loop {
        line.clear();
        ensure!(reader.read_line(&mut line).await? > 0, "Close disconnected");
        let response: Value = serde_json::from_str(&line)?;
        if response["id"] == 4 {
            ensure!(response.get("error").is_none(), "Session close failed");
            break;
        }
    }
    drop(writer);
    drop(reader);
    connection.task.abort();

    // Backfill the actual persisted Goose session through the public compaction
    // provider API, then replace it on another completed turn. No agent tools.
    let home = std::path::PathBuf::from(std::env::var("GOOSE_PATH_ROOT")?).join("vwork");
    let summaries = fastrock_workspaces::summaries::Summaries(
        fastrock_workspaces::embedded::Connection::new(home.clone()),
    );
    let worker = summary_service::Service::start(home.clone());
    worker.wait_idle().await;
    let key = model::ui_id(&session);
    let first = summaries
        .get(key.clone())
        .await?
        .context("Missing backfilled summary")?;
    ensure!(
        first.text.contains("fixture reply"),
        "Invalid backfill response"
    );
    ensure!(
        captured.lock().await.len() == 5,
        "Backfill must call the cheap model exactly once"
    );
    let manager = goose_runtime::session::SessionManager::new(
        goose_runtime::config::paths::Paths::data_dir(),
    );
    let original = manager
        .get_session(&session, true)
        .await?
        .conversation
        .context("Missing source")?;
    manager
        .add_message(
            &session,
            &Message::user().with_text("A later unique search keyword: ZirconBridge"),
        )
        .await?;
    manager
        .add_message(
            &session,
            &Message::assistant().with_text("New completed answer"),
        )
        .await?;
    worker.completed(session.clone()).await;
    worker.wait_idle().await;
    let latest = summaries
        .get(key.clone())
        .await?
        .context("Missing replacement")?;
    ensure!(
        latest.revision > first.revision && latest.source_messages > first.source_messages,
        "Summary did not advance"
    );
    ensure!(
        summaries.page(None, 10).await?.len() == 1,
        "Historical summaries retained"
    );
    let unchanged = manager
        .get_session(&session, true)
        .await?
        .conversation
        .context("Missing source")?;
    ensure!(
        serde_json::to_value(&unchanged.messages()[..original.messages().len()])?
            == serde_json::to_value(original.messages())?,
        "Summarization changed the source conversation"
    );
    let requests = captured.lock().await;
    ensure!(
        requests.len() == 6,
        "Completed turn must make one summary call"
    );
    for (path, body) in &requests[4..] {
        ensure!(
            path.contains("us.openai.gpt-6-luna"),
            "Summary used an expensive fallback"
        );
        ensure!(
            body.get("toolConfig").is_none(),
            "Summary request exposed agent tools"
        );
    }
    drop(requests);
    drop(worker);
    let restarted = summary_service::Service::start(home);
    restarted.wait_idle().await;
    ensure!(
        captured.lock().await.len() == 6,
        "Restart repeated an up-to-date summary"
    );
    drop(restarted);
    speed_fixture(captured.clone()).await?;
    http.abort();
    println!(
        "V38/V39/V40/V44: embedded Goose routes and sessions; cheap-model backfill, replacement, source preservation and restart passed (fixtures only)."
    );
    Ok(())
}

#[path = "../src/goose/provider_policy.rs"]
mod provider_policy;
#[path = "../src/goose/setup.rs"]
mod setup;

#[path = "../src/goose/provider_readiness.rs"]
mod provider_readiness;
#[path = "../src/goose/speed.rs"]
mod speed;
#[path = "../src/vault.rs"]
mod vault;

async fn speed_fixture(
    captured: std::sync::Arc<tokio::sync::Mutex<Vec<(String, Value)>>>,
) -> Result<()> {
    use goose_runtime::config::{declarative_providers as api, paths::Paths};
    use goose_runtime::{
        agents::{StateMachineServices, StateMachineServicesConfig},
        session::{SessionManager, SessionType},
    };
    let model = "us.openai.gpt-6.1-sol";
    let profile: api::DeclarativeProviderConfig = serde_json::from_value(json!({
        "name":"custom_speed_fixture", "display_name":"Speed fixture", "engine":"openai",
        "base_url":std::env::var("AWS_ENDPOINT_URL_BEDROCK_RUNTIME")?, "base_path":"openai/v1/responses",
        "requires_auth":false,"supports_streaming":false,"dynamic_models":false,"models":[{"name":model}]
    }))?;
    tokio::fs::create_dir_all(api::custom_providers_dir()).await?;
    tokio::fs::write(
        api::custom_providers_dir().join("custom_speed_fixture.json"),
        serde_json::to_vec(&profile)?,
    )
    .await?;
    goose_runtime::providers::refresh_custom_providers().await?;
    let manager = SessionManager::new(Paths::data_dir());
    let session = manager
        .create_session(
            std::env::current_dir()?,
            "speed fixture".into(),
            SessionType::User,
            goose_runtime::config::GooseMode::Auto,
        )
        .await?;
    let agent = StateMachineServices::with_config(StateMachineServicesConfig::cli());
    let provider = goose_runtime::providers::create("custom_speed_fixture", vec![]).await?;
    agent
        .update_provider(provider, ModelConfig::new(model), &session.id)
        .await?;
    manager
        .update(&session.id)
        .provider_name("custom_speed_fixture")
        .model_config(ModelConfig::new(model))
        .apply()
        .await?;
    let before = captured.lock().await.len();
    for value in ["ultrafast", "default"] {
        let mut config = manager
            .get_session(&session.id, false)
            .await?
            .model_config
            .context("Missing model config")?;
        speed::change(&mut config, value, true)?;
        manager
            .update(&session.id)
            .model_config(config)
            .apply()
            .await?;
        // Agent resolves its model config from the public store for each call.
        let config = manager
            .get_session(&session.id, false)
            .await?
            .model_config
            .context("Missing model config")?;
        let provider = agent.provider(&session.id).await?;
        provider
            .complete(
                &config,
                "fixture",
                &[Message::user().with_text("hello speed")],
                &[],
            )
            .await?;
    }
    let rows = captured.lock().await;
    ensure!(
        rows.len() == before + 2,
        "Speed must make exactly two local calls"
    );
    for ((path, body), expected) in rows[before..].iter().zip(["ultrafast", "default"]) {
        ensure!(
            path == "/openai/v1/responses"
                && body["model"] == model
                && body["service_tier"] == expected,
            "Speed/model/route lost before the actual HTTP request"
        );
    }
    println!("V90: public session settings reached actual Responses HTTP payloads");
    Ok(())
}
