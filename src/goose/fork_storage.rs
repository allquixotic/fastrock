//! Fork through unchanged Goose's public storage API. A partial fork remains
//! hidden until Fastrock has persisted the workspace/tool bindings and lineage.
use anyhow::{Result, ensure};
use goose_runtime::session::{Session, SessionManager, SessionType};
use std::path::PathBuf;

pub(super) async fn copy_hidden(
    manager: &SessionManager,
    source: Session,
    cwd: PathBuf,
    before_message: Option<&str>,
) -> Result<String> {
    let conversation = source.conversation.as_ref();
    if let Some(boundary) = before_message {
        ensure!(
            conversation.is_some_and(|c| c
                .messages()
                .iter()
                .any(|m| m.id.as_deref() == Some(boundary))),
            "The selected turn no longer has a stored message; reload the conversation before forking"
        );
    }
    let child = manager
        .create_session(
            cwd,
            format!("{} (fork)", source.name),
            SessionType::Hidden,
            source.goose_mode,
        )
        .await?;
    let mut builder = manager
        .update(&child.id)
        .extension_data(source.extension_data)
        .recipe(source.recipe)
        .user_recipe_values(source.user_recipe_values)
        .system_prompt_override(source.system_prompt_override)
        .system_prompt_extras(source.system_prompt_extras)
        .container(source.container);
    if let Some(provider) = source.provider_name {
        builder = builder.provider_name(provider);
    }
    if let Some(model) = source.model_config {
        builder = builder.model_config(model);
    }
    // A manual fork never inherits an automatic schedule or a pending run.
    builder.apply().await?;
    if let Some(conversation) = conversation {
        manager
            .replace_conversation(&child.id, conversation)
            .await?;
    }
    if let Some(message) = before_message {
        manager
            .truncate_conversation_from_message(&child.id, message)
            .await?;
    }
    Ok(child.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use goose_provider_types::conversation::message::Message;

    #[tokio::test]
    async fn v50_fork_is_hidden_preserves_source_and_rewinds_exact_message() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(temp.path().join("data"));
        let source = manager
            .create_session(
                temp.path().join("parent"),
                "parent".into(),
                SessionType::User,
                Default::default(),
            )
            .await
            .unwrap();
        let mut messages = vec![
            Message::user().with_text("first").with_id("u1"),
            Message::assistant().with_text("answer").with_id("a1"),
            Message::user().with_text("second").with_id("u2"),
            Message::assistant().with_text("last").with_id("a2"),
        ];
        let timestamp = messages[0].created;
        for message in &mut messages {
            message.created = timestamp;
            manager.add_message(&source.id, message).await.unwrap();
        }
        manager
            .update(&source.id)
            .schedule_id(Some("must-not-repeat".into()))
            .provider_name("aws_bedrock")
            .apply()
            .await
            .unwrap();
        let before = manager.get_session(&source.id, true).await.unwrap();
        let expected = serde_json::to_value(before.conversation.as_ref().unwrap()).unwrap();
        let child = copy_hidden(&manager, before, temp.path().join("child"), Some("u2"))
            .await
            .unwrap();
        let fork = manager.get_session(&child, true).await.unwrap();
        assert_eq!(fork.session_type, SessionType::Hidden);
        assert!(fork.schedule_id.is_none());
        assert_eq!(fork.provider_name.as_deref(), Some("aws_bedrock"));
        assert_eq!(fork.working_dir, temp.path().join("child"));
        let conversation = fork.conversation.unwrap();
        assert_eq!(conversation.messages().len(), 2);
        assert_eq!(conversation.messages()[1].id.as_deref(), Some("a1"));
        assert_eq!(
            serde_json::to_value(
                manager
                    .get_session(&source.id, true)
                    .await
                    .unwrap()
                    .conversation
                    .unwrap()
            )
            .unwrap(),
            expected
        );
        assert!(
            copy_hidden(
                &manager,
                manager.get_session(&source.id, true).await.unwrap(),
                temp.path().join("invalid"),
                Some("missing")
            )
            .await
            .is_err()
        );
        let full = copy_hidden(
            &manager,
            manager.get_session(&source.id, true).await.unwrap(),
            temp.path().join("full"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(
                manager
                    .get_session(&full, true)
                    .await
                    .unwrap()
                    .conversation
                    .unwrap()
            )
            .unwrap(),
            expected
        );
    }
}
