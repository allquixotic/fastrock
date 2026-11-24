//! Verify image input on the actual provider wire using only generated pixels.
use super::acp_client::Client;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use goose_runtime::{config::declarative_providers as api, session::SessionManager};
use image::{ImageFormat, Rgba, RgbaImage};
use serde_json::{Value, json};
use std::{io::Cursor, path::PathBuf, sync::Arc};

fn png(pixel: [u8; 4]) -> Result<String> {
    let mut bytes = Cursor::new(Vec::new());
    RgbaImage::from_pixel(2, 3, Rgba(pixel)).write_to(&mut bytes, ImageFormat::Png)?;
    Ok(STANDARD.encode(bytes.into_inner()))
}

fn user_blocks(body: &Value) -> Result<&[Value]> {
    let messages = body["input"]
        .as_array()
        .or_else(|| body["messages"].as_array())
        .context("Missing provider messages")?;
    messages
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_array())
        .map(Vec::as_slice)
        .context("Missing user content blocks")
}

// Compare the image MIME type and bytes from the provider-specific image fields.
// Other fields (e.g. Anthropic cache hints) may legitimately vary.
fn wire_image(path: &str, block: &Value) -> Option<String> {
    if path.ends_with("/converse") {
        let image = block.get("image")?;
        return Some(format!(
            "data:image/{};base64,{}",
            image["format"].as_str()?,
            image["source"]["bytes"].as_str()?
        ));
    }
    match (path, block["type"].as_str()?) {
        ("/openai/v1/responses", "input_image") => Some(block["image_url"].as_str()?.to_owned()),
        ("/v1/chat/completions", "image_url") => {
            Some(block["image_url"]["url"].as_str()?.to_owned())
        }
        ("/v1/messages", "image") if block["source"]["type"] == "base64" => {
            let source = &block["source"];
            Some(format!(
                "data:{};base64,{}",
                source["media_type"].as_str()?,
                source["data"].as_str()?
            ))
        }
        _ => None,
    }
}

pub(super) async fn v119_image_prompts(
    captured: Arc<tokio::sync::Mutex<Vec<(String, Value)>>>,
) -> Result<()> {
    let cwd = PathBuf::from(std::env::var("GOOSE_PATH_ROOT")?);
    let skill = cwd.join(".agents/skills/color-card");
    std::fs::create_dir_all(&skill)?;
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: color-card\ndescription: Inspect a color card\n---\nColor card request = $ARGUMENTS",
    )?;
    for (name, engine, path) in [
        ("custom_image_chat", "openai", "v1/chat/completions"),
        ("custom_image_anthropic", "anthropic", "v1/messages"),
    ] {
        let config = json!({"name":name,"display_name":"Image fixture","engine":engine,
            "base_url":std::env::var("AWS_ENDPOINT_URL_BEDROCK_RUNTIME")?,"base_path":path,
            "requires_auth":false,"dynamic_models":false,"supports_streaming":engine == "anthropic","models":[{"name":"card-deployment"},
            {"name":"fixture-text-only","supports_vision":false}]});
        std::fs::write(
            api::custom_providers_dir().join(format!("{name}.json")),
            serde_json::to_vec(&config)?,
        )?;
    }
    goose_runtime::providers::refresh_custom_providers().await?;
    let images = [png([12, 45, 67, 255])?, png([22, 99, 64, 255])?];
    let mut client = Client::connect().await?;
    for (provider, model, path) in [
        (
            "aws_bedrock",
            "us.openai.gpt-6.2-sol",
            "/model/us.openai.gpt-6.2-sol/converse",
        ),
        (
            "aws_bedrock",
            "us.openai.gpt-6.2-astra",
            "/model/us.openai.gpt-6.2-astra/converse",
        ),
        (
            "aws_bedrock",
            "us.openai.gpt-6.1-sol",
            "/model/us.openai.gpt-6.1-sol/converse",
        ),
        (
            "aws_bedrock",
            "us.openai.gpt-6-luna",
            "/model/us.openai.gpt-6-luna/converse",
        ),
        (
            "aws_bedrock",
            "us.anthropic.claude-opus-5-5",
            "/model/us.anthropic.claude-opus-5-5/converse",
        ),
        (
            "custom_speed_fixture",
            "gpt-6.2-sol",
            "/openai/v1/responses",
        ),
        (
            "custom_speed_fixture",
            "card-deployment",
            "/openai/v1/responses",
        ),
        (
            "custom_image_chat",
            "card-deployment",
            "/v1/chat/completions",
        ),
        ("custom_image_anthropic", "card-deployment", "/v1/messages"),
    ] {
        // Each scenario gets a fresh conversation so history cannot satisfy an assertion.
        for command in [true, false] {
            let session = client
                .call(
                    "session/new",
                    json!({"cwd":cwd,"mcpServers":[],
                "_meta":{"provider":provider,"enabledExtensions":[]}}),
                )
                .await?;
            let id = session["sessionId"]
                .as_str()
                .context("Missing session ID")?;
            client.model(id, model).await?;
            let mut prompt: Vec<_> = images
                .iter()
                .map(|data| json!({"type":"image","mimeType":"image/png","data":data}))
                .collect();
            if command {
                prompt.push(json!({"type":"text","text":"/color-card compare"}));
            }
            let before = captured.lock().await.len();
            client
                .call("session/prompt", json!({"sessionId":id,"prompt":prompt}))
                .await?;
            let rows = captured.lock().await;
            ensure!(
                rows.len() == before + 1 && rows[before].0 == path,
                "Unexpected route or request count for {provider}"
            );
            let blocks = user_blocks(&rows[before].1)?;
            let received: Vec<_> = blocks
                .iter()
                .filter_map(|block| wire_image(path, block))
                .collect();
            let expected = images
                .each_ref()
                .map(|data| format!("data:image/png;base64,{data}"));
            ensure!(
                received == expected,
                "{provider} did not receive the original images in order (skill={command})"
            );
            if command {
                ensure!(
                    blocks.iter().any(|b| b["text"]
                        .as_str()
                        .is_some_and(|s| s.contains("Color card request = compare"))),
                    "Expanded skill arguments missing for {provider}"
                );
                let stored = SessionManager::instance().get_session(id, true).await?;
                let conversation = stored.conversation.context("Missing saved history")?;
                ensure!(
                    conversation.messages().iter().any(|m| m.is_user_visible()
                        && m.user_visible_content().as_concat_text() == "/color-card compare"),
                    "Original skill invocation lost from history"
                );
            }
            drop(rows);
            if provider == "custom_image_chat" {
                client.model(id, "fixture-text-only").await?;
                let before = captured.lock().await.len();
                let error = client
                    .call(
                        "session/prompt",
                        json!({"sessionId":id,
                    "prompt":[{"type":"image","mimeType":"image/png","data":images[0]}]}),
                    )
                    .await
                    .expect_err("text-only model");
                ensure!(
                    error.to_string().contains("does not accept images")
                        && captured.lock().await.len() == before
                );
            }
            client
                .call("session/close", json!({"sessionId":id}))
                .await?;
        }
    }
    println!(
        "V119: generated images reached all four provider formats in image-only and slash-skill turns"
    );
    Ok(())
}
