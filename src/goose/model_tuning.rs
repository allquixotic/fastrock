//! Model-specific working instructions, through Goose's public session API.
//! Keep the prompt stable between turns; never retry a completed model turn.
use anyhow::Result;
use goose_runtime::session::SessionManager;

const KEY: &str = "fastrock-gpt-6.2-work-policy";
const INSTRUCTIONS: &str = r#"# Fastrock GPT-6.2 working defaults

Apply these defaults within the user's requested scope and the applicable project,
recipe, tool, and permission rules. Respect explicit requests for a short answer,
planning only, review only, or no changes. These defaults grant no new permissions.

For an execution request, carry the authorized work through implementation and
appropriate verification. Do not end the turn with an offer, a plan, or a suggested
next step when you can take that step yourself within scope. Make reasonable,
reversible assumptions for routine details and explain consequential assumptions.
Ask when a necessary decision cannot be inferred, a major scope change is needed,
or a destructive or irreversible action lacks authorization. When one part is
blocked, continue independent authorized work; do not bypass a denial or repeatedly
retry a failed approach without new evidence. Honor cancellation and tool limits.

For multi-step work, keep track of the requested outcomes and what would demonstrate
completion. Use an existing task list or planning tool when helpful; do not create
extra files or process for a simple request. Before finishing, check every requested
outcome against the work actually done. Perform the relevant checks that are
available and authorized, address failures within scope, and stop once the goal is
met. Do not expand the task into unrelated improvements or unnecessary testing.

During sustained work, give useful progress updates about findings, decisions,
blockers, and what you are doing next. Updates are not handoffs: continue working
when an authorized next step remains. Do not wait for an acknowledgment unless
the user's input or permission is actually required.

Make the final answer understandable without reading the progress updates. State
the result, the changes that matter, verification performed and its outcome, and
any remaining limitation or unfinished work. Include enough explanation to make
the result usable; brevity must not hide missing work, failed tools, skipped checks,
or uncertainty. Distinguish verified results from assumptions and untested behavior.
Never claim completion or successful checks without supporting evidence.
"#;

fn targeted_model(model: &str) -> bool {
    // Exact identities only: no substring matching of deployment aliases, ARNs,
    // future models, or names which merely mention GPT-6.2.
    if matches!(model, "gpt-6.2-sol" | "gpt-6.2-astra") {
        return true;
    }
    let model = ["us.", "global.", "eu.", "apac."]
        .into_iter()
        .find_map(|prefix| model.strip_prefix(prefix))
        .unwrap_or(model);
    matches!(model, "openai.gpt-6.2-sol" | "openai.gpt-6.2-astra")
}

fn targeted_provider(provider: &str) -> bool {
    use goose_runtime::config::declarative_providers::{ProviderEngine, load_provider};
    match provider {
        "openai" | "aws_bedrock" => true,
        "anthropic" => false,
        _ => load_provider(provider)
            .is_ok_and(|p| p.is_editable && p.config.engine == ProviderEngine::OpenAI),
    }
}

/// Run immediately before dispatch, after model selection. This also reconciles
/// restored sessions, recipe models and inherited fork prompts. Only our named
/// extra is changed; Goose's base prompt, user extras and model settings survive.
pub(super) async fn sync(manager: &SessionManager, id: &str) -> Result<()> {
    let session = manager.get_session(id, false).await?;
    let targeted = if session
        .model_config
        .as_ref()
        .is_some_and(|model| targeted_model(&model.model_name))
    {
        let provider = session.provider_name.unwrap_or_default();
        tokio::task::spawn_blocking(move || targeted_provider(&provider)).await?
    } else {
        false
    };
    let desired = targeted.then_some(INSTRUCTIONS);
    if session.system_prompt_extras.get(KEY).map(String::as_str) != desired {
        manager
            .set_system_prompt_extra(id, KEY, desired.map(str::to_owned))
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn v118_only_known_gpt_62_identities_receive_working_defaults() {
        use super::{targeted_model, targeted_provider};

        for model in ["gpt-6.2-sol", "gpt-6.2-astra"] {
            assert!(targeted_model(model));
            for prefix in [
                "openai.",
                "us.openai.",
                "global.openai.",
                "eu.openai.",
                "apac.openai.",
            ] {
                assert!(targeted_model(&format!("{prefix}{model}")));
            }
        }
        for model in [
            "gpt-6.1-sol",
            "us.openai.gpt-6.1-sol",
            "openai.gpt-6-luna",
            "us.anthropic.claude-opus-5-5-v1:0",
            "MAI-Code-2-Flash",
            "gpt-6.2-sol-review",
            "gpt-6.2-astra-custom",
            "gpt-6.2-sol-unknown",
            "gpt-6.20-sol",
            "alias-gpt-6.2-sol",
            "Gpt-6.2-sol",
            " gpt-6.2-sol",
            "us.gpt-6.2-sol",
            "us.global.openai.gpt-6.2-sol",
            "arn:aws:bedrock:us-east-1:123456789012:inference-profile/gpt-6.2-sol",
        ] {
            assert!(!targeted_model(model), "Unexpected tuning for {model}");
        }
        assert!(targeted_provider("openai"));
        assert!(targeted_provider("aws_bedrock"));
        assert!(!targeted_provider("anthropic"));
    }
}
