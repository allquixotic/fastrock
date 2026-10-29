//! Standalone fixture process: environment isolation precedes any runtime threads.
#[path = "../src/goose/embedded.rs"]
mod embedded;

use anyhow::{Context, Result, ensure};
use goose_provider_types::{base::Provider, conversation::message::Message, model::ModelConfig};
use goose_runtime::providers::bedrock::BedrockProvider;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn main() -> Result<()> {
    // The CI runner separately invokes window-only tests on every executable.
    if std::env::args().nth(1).as_deref() == Some("window_runtime::tests") {
        return Ok(());
    }
    let temp = tempfile::tempdir()?;
    std::fs::create_dir_all(temp.path().join("config"))?;
    std::fs::write(
        temp.path().join("config/config.yaml"),
        "GOOSE_PROVIDER: aws_bedrock\nGOOSE_MODEL: openai.gpt-6.1-sol\nBEDROCK_DISABLE_STREAMING: true\nBEDROCK_MAX_RETRIES: 0\nGOOSE_DISABLE_SESSION_NAMING: true\nextensions: {}\n",
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
        captured.lock().await.push((uri.to_string(), body));
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
        "openai.gpt-6.1-sol",
        "openai.gpt-6-luna",
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
        "openai.gpt-6.1-sol",
        "openai.gpt-6-luna",
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
    http.abort();
    println!(
        "V38/V39: embedded Goose initialize/session/prompt/text/close and three Bedrock Runtime model routes passed (fixtures only)."
    );
    Ok(())
}
