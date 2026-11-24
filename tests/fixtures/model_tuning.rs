//! Exercise Fastrock's actual in-memory dispatch and unchanged Goose rendering.
use super::acp_client::Client;
use anyhow::{Context, Result, ensure};
use goose_runtime::session::SessionManager;
use serde_json::{Value, json};
use std::sync::Arc;

const KEY: &str = "fastrock-gpt-6.2-work-policy";
const MARKER: &str = "# Fastrock GPT-6.2 working defaults";
const USER_EXTRA: &str = "Fixture user instructions: preserve this exact text.";
const BASE: &str = "Fixture custom base prompt.";

fn system_text(body: &Value) -> String {
    if let Some(system) = body["system"].as_array() {
        return system
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
    }
    body["input"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| matches!(item["role"].as_str(), Some("system" | "developer")))
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn turn(
    client: &mut Client,
    manager: &SessionManager,
    captured: &Arc<tokio::sync::Mutex<Vec<(String, Value)>>>,
    id: &str,
    tuned: bool,
) -> Result<()> {
    let before = manager.get_session(id, false).await?;
    let index = captured.lock().await.len();
    let reply = client
        .call(
            "session/prompt",
            json!({"sessionId":id,
        "prompt":[{"type":"text","text":"One fixture reply, please."}]}),
        )
        .await?;
    ensure!(reply["stopReason"] == "end_turn");
    let rows = captured.lock().await;
    ensure!(
        rows.len() == index + 1,
        "A completed turn was continued or duplicated"
    );
    let system = system_text(&rows[index].1);
    ensure!(
        system.contains(BASE) && system.contains(USER_EXTRA),
        "User prompt content lost"
    );
    ensure!(
        system.matches(MARKER).count() == usize::from(tuned),
        "Wrong policy in actual HTTP request"
    );
    let saved = manager.get_session(id, false).await?;
    ensure!(saved.system_prompt_override == before.system_prompt_override);
    ensure!(saved.system_prompt_extras["fixture-user"] == USER_EXTRA);
    let mut expected = before.model_config;
    // Image-capability preparation may opt an unlisted deployment into receiving
    // pixels. Every other setting must still survive the whole dispatch.
    if let Some(model) = &mut expected
        && model.supports_vision.is_none()
        && saved.model_config.as_ref().and_then(|m| m.supports_vision) == Some(true)
    {
        model.supports_vision = Some(true);
    }
    ensure!(
        serde_json::to_value(&saved.model_config)? == serde_json::to_value(&expected)?,
        "Tuning changed model, reasoning, verbosity, token limit or other request settings"
    );
    ensure!(saved.system_prompt_extras.contains_key(KEY) == tuned);
    Ok(())
}

pub async fn run(captured: Arc<tokio::sync::Mutex<Vec<(String, Value)>>>) -> Result<()> {
    let manager = SessionManager::instance();
    let cwd = std::env::var("GOOSE_PATH_ROOT")?;
    // The named profile uses the real public OpenAI Responses engine against our
    // HTTP fixture. Native Bedrock exercises Converse through the same router.
    for (provider, sol, astra, other) in [
        (
            "aws_bedrock",
            "us.openai.gpt-6.2-sol",
            "us.openai.gpt-6.2-astra",
            "us.openai.gpt-6.1-sol",
        ),
        (
            "custom_speed_fixture",
            "gpt-6.2-sol",
            "gpt-6.2-astra",
            "MAI-Code-2-Flash",
        ),
    ] {
        let mut client = Client::connect().await?;
        let created = client
            .call(
                "session/new",
                json!({"cwd":cwd,"mcpServers":[],
            "_meta":{"provider":provider,"enabledExtensions":[]}}),
            )
            .await?;
        let id = created["sessionId"]
            .as_str()
            .context("Missing fixture session")?;
        manager
            .update(id)
            .system_prompt_override(Some(BASE.into()))
            .apply()
            .await?;
        manager
            .set_system_prompt_extra(id, "fixture-user", Some(USER_EXTRA.into()))
            .await?;
        client.model(id, sol).await?;
        let mut model = manager
            .get_session(id, false)
            .await?
            .model_config
            .context("Missing model")?;
        model.max_tokens = Some(8192);
        model = model.with_merged_request_params(std::collections::HashMap::from([
            ("thinking_effort".into(), json!("high")),
            ("text".into(), json!({"verbosity":"low"})),
        ]));
        manager.update(id).model_config(model).apply().await?;
        turn(&mut client, &manager, &captured, id, true).await?;
        let extras = manager.get_session(id, false).await?.system_prompt_extras;
        // Repeated preparation must preserve insertion order and every byte of
        // the prompt prefix; it must not accumulate instructions per turn.
        super::model_tuning::sync(&manager, &manager.get_session(id, false).await?).await?;
        ensure!(
            manager
                .get_session(id, false)
                .await?
                .system_prompt_extras
                .iter()
                .eq(extras.iter())
        );
        turn(&mut client, &manager, &captured, id, true).await?;
        {
            let rows = captured.lock().await;
            ensure!(
                system_text(&rows[rows.len() - 1].1) == system_text(&rows[rows.len() - 2].1),
                "Working instructions changed the prompt prefix between identical turns"
            );
        }

        let child = manager
            .copy_session(id, "Tuning fork fixture".into())
            .await?;
        client.model(id, other).await?;
        turn(&mut client, &manager, &captured, id, false).await?;
        client.model(id, astra).await?;
        turn(&mut client, &manager, &captured, id, true).await?;
        client
            .call("session/close", json!({"sessionId":id}))
            .await?;
        drop(client);

        // A fresh connection restores a saved targeted session; a copied child
        // may change models before its first prompt and must lose inherited tuning.
        let mut client = Client::connect().await?;
        client
            .call(
                "session/load",
                json!({"sessionId":id,"cwd":cwd,"mcpServers":[]}),
            )
            .await?;
        turn(&mut client, &manager, &captured, id, true).await?;
        client
            .call("session/close", json!({"sessionId":id}))
            .await?;
        client
            .call(
                "session/load",
                json!({"sessionId":child.id,"cwd":cwd,"mcpServers":[]}),
            )
            .await?;
        client.model(&child.id, other).await?;
        turn(&mut client, &manager, &captured, &child.id, false).await?;
        client
            .call("session/close", json!({"sessionId":child.id}))
            .await?;
    }
    // A non-OpenAI engine can use the same deployment name. Remove inherited
    // policy there too, and never infer model identity from an opaque alias.
    let session = manager
        .create_session(
            std::path::PathBuf::from(&cwd),
            "Non-target model fixture".into(),
            goose_runtime::session::SessionType::User,
            goose_runtime::config::GooseMode::Auto,
        )
        .await?;
    manager
        .set_system_prompt_extra(&session.id, "fixture-user", Some(USER_EXTRA.into()))
        .await?;
    for (provider, model) in [
        ("anthropic", "gpt-6.2-sol"),
        ("aws_bedrock", "us.anthropic.claude-opus-5-5-v1:0"),
        ("openai", "gpt-6.1-sol"),
        ("custom_speed_fixture", "gpt-6.2-sol-custom"),
        ("custom_speed_fixture", "MAI-Code-2-Flash"),
    ] {
        manager
            .update(&session.id)
            .provider_name(provider)
            .model_config(super::ModelConfig::new(model))
            .apply()
            .await?;
        manager
            .set_system_prompt_extra(&session.id, KEY, Some("Inherited policy".into()))
            .await?;
        super::model_tuning::sync(&manager, &manager.get_session(&session.id, false).await?)
            .await?;
        let saved = manager.get_session(&session.id, false).await?;
        ensure!(saved.system_prompt_extras.len() == 1);
        ensure!(saved.system_prompt_extras["fixture-user"] == USER_EXTRA);
        ensure!(saved.model_config.context("Missing model")?.model_name == model);
    }
    println!(
        "V118: GPT-6.2 prompts reached actual Converse/Responses requests; repeated turns, model switches, restore and fork preserved settings and did not continue completed turns"
    );
    Ok(())
}
