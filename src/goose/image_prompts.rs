//! Keep image blocks visible across Goose's command and provider boundaries.
use goose_runtime::{
    agents::{
        execute_commands::{parse_slash_command, slash_commands_enabled},
        provider_manager::{model_config_for, provider_name_for},
    },
    providers::get_from_registry,
    session::{Session, SessionManager},
    slash_commands::{recipe_slash_command::runnable_recipes, slash_command::is_builtin_command},
};
use serde_json::{Value, json};
use std::path::Path;

pub(super) async fn prepare(
    manager: &SessionManager,
    session: &Session,
    prompt: &mut Vec<Value>,
    steer: bool,
) -> Result<(), &'static str> {
    let images = prompt.iter().any(|block| block["type"] == "image");
    if !images
        && (steer
            || session
                .model_config
                .as_ref()
                .is_some_and(|m| m.supports_vision.is_some()))
    {
        return Ok(());
    }

    // Goose snapshots the model config at run start, including for subsequent
    // steers. Resolve the same declared/canonical metadata its formatter uses.
    // For unlisted deployments let the endpoint decide whether it accepts pixels.
    const CONFIG_ERROR: &str = "Could not read the model's image settings.";
    let model = model_config_for(session).map_err(|_| CONFIG_ERROR)?;
    let provider = provider_name_for(session).map_err(|_| CONFIG_ERROR)?;
    let resolved = get_from_registry(&provider)
        .await
        .map_err(|_| CONFIG_ERROR)?
        .normalize_model_config(model.clone())
        .map_err(|_| CONFIG_ERROR)?;
    match resolved.supports_vision {
        Some(false) if images => {
            return Err(
                "This model does not accept images. Choose a vision-capable model or remove the attachments.",
            );
        }
        None => {
            manager
                .update(&session.id)
                .model_config(model.with_vision_support(true))
                .apply()
                .await
                .map_err(|_| "Could not enable image input for this conversation.")?;
        }
        _ => {}
    }
    if !images {
        return Ok(());
    }

    // The composer sends its text in a single block. A slash skill normally
    // hides the entire kickoff message and appends only the rendered skill text.
    // Render the same skill here so its prompt and the original images share
    // one agent-visible message. The user's original text stays in history.
    let Some(index) = prompt.iter().position(|part| part["type"] == "text") else {
        return Ok(());
    };
    let Some(command) = prompt[index]["text"].as_str().and_then(parse_slash_command) else {
        return Ok(());
    };
    let cwd = session.working_dir.clone();
    let name = command.command.to_lowercase();
    let args = command.params_str.to_owned();
    let expanded = tokio::task::spawn_blocking(move || image_skill(&cwd, &name, &args))
        .await
        .map_err(|_| "Could not prepare the image command.")??;
    if let Some(expanded) = expanded {
        prompt[index]["annotations"] = json!({"audience":["user"]});
        prompt
            .push(json!({"type":"text","text":expanded,"annotations":{"audience":["assistant"]}}));
    }
    Ok(())
}

fn image_skill(cwd: &Path, name: &str, args: &str) -> Result<Option<String>, &'static str> {
    if !slash_commands_enabled() {
        return Ok(None);
    }
    if is_builtin_command(name) || runnable_recipes().iter().any(|(recipe, _)| recipe == name) {
        return Err(
            "This command cannot receive images. Send the images in a regular message or with a skill.",
        );
    }
    let skills = goose_runtime::skills::skills_for_session(cwd)
        .map_err(|_| "Could not read skills for this conversation.")?;
    let Some(skill) = skills
        .iter()
        .filter(|s| s.name.to_lowercase() == name)
        .min_by_key(|s| (&s.name, &s.path))
    else {
        return Ok(None);
    };
    let content =
        goose_agent::skills::skill_content_with_args(skill, (!args.is_empty()).then_some(args))
            .map_err(
                |_| "Could not expand the skill arguments. Check their quoting and try again.",
            )?;
    Ok(Some(goose_agent::skills::loaded_skill_context(
        skill, &content,
    )))
}
