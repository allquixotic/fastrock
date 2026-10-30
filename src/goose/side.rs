//! Temporary side conversations use public Goose storage, never a second agent service.
use super::*;
use goose_provider_types::conversation::message::Message;
use goose_runtime::session::{Session as StoredSession, SessionManager};

pub(super) fn filter_tools(tools: &mut Vec<Value>) {
    tools.retain(|t| {
        let name = text(t, "name");
        !["fastrock_agents", "codex_gui"].iter().any(|namespace| {
            name == *namespace
                || name.starts_with(&format!("{namespace}."))
                || name.starts_with(&format!("{namespace}__"))
        })
    });
}

/// The active parent's latest request may contain pending tools or unfinished
/// instructions. Exclude that entire turn rather than copying executable state.
pub(super) fn reference_history(source: &mut StoredSession, active: bool) {
    let Some(conversation) = source.conversation.as_mut() else {
        return;
    };
    if active || super::recovery_history::uncertain(conversation) {
        let start = conversation
            .messages()
            .iter()
            .rposition(|m| {
                serde_json::to_value(&m.role).ok() == Some(json!("user"))
                    && m.is_user_visible()
                    && !m.is_tool_response()
            })
            .unwrap_or(0);
        conversation.truncate(start);
    }
}

/// Copy extension configuration, not the parent's active goal or app tool route.
pub(super) fn reference_configuration(
    source: &mut StoredSession,
    excluded: &[String],
) -> anyhow::Result<()> {
    use goose_runtime::session::extension_data::{
        EnabledExtensionsState, ExtensionState, GoalState, TodoState,
    };
    let mut enabled = EnabledExtensionsState::from_extension_data(&source.extension_data)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Source has no saved extension selection; reopen it before starting a side chat"
            )
        })?;
    enabled
        .extensions
        .retain(|e| e.name() != "fastrock" && !excluded.contains(&e.name()));
    enabled.to_extension_data(&mut source.extension_data)?;
    GoalState::default().to_extension_data(&mut source.extension_data)?;
    TodoState::new(String::new()).to_extension_data(&mut source.extension_data)?;
    Ok(())
}

pub(super) async fn prepare(manager: &SessionManager, id: &str) -> anyhow::Result<()> {
    manager
        .set_system_prompt_extra(
            id,
            "fastrock-side",
            Some(crate::threads::side::SIDE_DEVELOPER_INSTRUCTIONS.into()),
        )
        .await?;
    manager
        .add_message(
            id,
            &Message::user()
                .with_id(format!("fastrock-side-boundary-{id}"))
                .with_text(crate::threads::side::SIDE_BOUNDARY_PROMPT)
                .agent_only(),
        )
        .await?;
    Ok(())
}

impl Adapter {
    /// The boundary was stored before session/load. This canonical UI call only
    /// acknowledges that exact control message, so retries cannot append it.
    pub(super) async fn inject_side_boundary(&self, params: &Value) -> Reply {
        let id = session_id(text(params, "threadId"))?;
        if params["items"] != json!([crate::threads::side::side_boundary_item()]) {
            return Err(error(
                "Only the fixed side-conversation boundary can be injected",
            ));
        }
        let gate = self.gate(id).await;
        let _lock = gate.lock().await;
        if !self.embedded_runtime
            || !self
                .sessions
                .lock()
                .await
                .get(id)
                .is_some_and(|s| s.loaded && s.binding.side && s.ephemeral && s.active.is_none())
        {
            return Err(error(
                "Open an idle side conversation before preparing its boundary",
            ));
        }
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use goose_runtime::session::SessionType;
    #[tokio::test]
    async fn v75_side_reference_excludes_active_turn_and_keeps_hidden_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(temp.path().join("data"));
        let parent = manager
            .create_session(
                temp.path().join("parent"),
                "Parent".into(),
                SessionType::User,
                Default::default(),
            )
            .await
            .unwrap();
        for message in [
            Message::user().with_text("completed question"),
            Message::assistant().with_text("completed answer"),
            Message::user().with_text("unfinished mutation"),
        ] {
            manager.add_message(&parent.id, &message).await.unwrap();
        }
        let mut source = manager.get_session(&parent.id, true).await.unwrap();
        use goose_runtime::session::extension_data::{
            EnabledExtensionsState, ExtensionState, GoalState, TodoState,
        };
        EnabledExtensionsState::new(vec![])
            .to_extension_data(&mut source.extension_data)
            .unwrap();
        GoalState {
            goal: Some("continue parent mutation".into()),
            grind: None,
        }
        .to_extension_data(&mut source.extension_data)
        .unwrap();
        TodoState::new("pending parent work".into())
            .to_extension_data(&mut source.extension_data)
            .unwrap();
        reference_configuration(&mut source, &[]).unwrap();
        assert!(GoalState::of(&source).goal.is_none());
        assert!(
            TodoState::from_extension_data(&source.extension_data)
                .unwrap()
                .content
                .is_empty()
        );
        reference_history(&mut source, true);
        assert_eq!(source.conversation.as_ref().unwrap().messages().len(), 2);
        let child = super::super::fork_storage::copy_hidden(
            &manager,
            source,
            temp.path().join("isolated"),
            None,
        )
        .await
        .unwrap();
        prepare(&manager, &child).await.unwrap();
        let stored = manager.get_session(&child, true).await.unwrap();
        assert_eq!(stored.session_type, SessionType::Hidden);
        assert_eq!(stored.working_dir, temp.path().join("isolated"));
        let messages = stored.conversation.unwrap();
        assert_eq!(messages.messages().len(), 3);
        let boundary = messages.messages().last().unwrap();
        assert!(!boundary.is_user_visible() && boundary.is_agent_visible());
        assert_eq!(
            boundary.id.as_deref(),
            Some(format!("fastrock-side-boundary-{child}").as_str())
        );
        assert!(!super::super::recovery_history::uncertain(&messages));
        assert_eq!(manager.list_sessions().await.unwrap().len(), 1);
        assert_eq!(
            manager
                .get_session(&parent.id, true)
                .await
                .unwrap()
                .conversation
                .unwrap()
                .messages()
                .len(),
            3
        );
        let mut tools = vec![
            json!({"name":"fastrock_agents"}),
            json!({"name":"codex_gui"}),
            json!({"name":"read_source"}),
            json!({"name":"fastrock_agents.delegate_task"}),
        ];
        filter_tools(&mut tools);
        assert_eq!(tools, vec![json!({"name":"read_source"})]);
    }
}
