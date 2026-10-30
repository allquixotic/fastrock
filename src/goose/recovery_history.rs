//! Opening history must never activate Goose's saved tool-continuation machine.
//! Uncertain sessions remain inspectable through public storage until the user
//! restores a safe checkpoint or forks before the interrupted turn.
use super::*;
use goose_provider_types::conversation::{
    Conversation,
    message::{ActionRequiredData, Message, MessageContentBlock as Block},
};
use std::collections::BTreeSet;
fn user(message: &Message) -> bool {
    serde_json::to_value(&message.role).ok() == Some(json!("user"))
}
pub(super) fn uncertain(conversation: &Conversation) -> bool {
    let messages = conversation.messages();
    let start = messages
        .iter()
        .rposition(|m| user(m) && m.is_user_visible() && !m.is_tool_response())
        .unwrap_or(0);
    let mut requested = BTreeSet::new();
    let mut answered = BTreeSet::new();
    for message in &messages[start..] {
        for block in &message.content {
            match block {
                Block::ToolRequest(r) => {
                    requested.insert(r.id.clone());
                }
                Block::ToolConfirmationRequest(r) => {
                    requested.insert(r.id.clone());
                }
                Block::ActionRequired(action) => match &action.data {
                    ActionRequiredData::ToolConfirmation { id, .. }
                    | ActionRequiredData::ToolConfirmationResponse { id, .. } => {
                        requested.insert(id.clone());
                    }
                    _ => {}
                },
                Block::ToolResponse(r) => {
                    answered.insert(r.id.clone());
                }
                _ => {}
            }
        }
    }
    requested.iter().any(|id| !answered.contains(id))
}
fn project(session: &mut Session, conversation: &Conversation, reason: &str) {
    session.turns.clear();
    session.last_chunk.clear();
    session.config.clear();
    session.replaying = true;
    for (index, message) in conversation.messages().iter().enumerate() {
        // Include hidden control/tool messages as inspectable text in this
        // exceptional recovery view; the stored conversation is never changed.
        let text = goose_runtime::session::message_to_markdown(message, true);
        if text.is_empty() {
            continue;
        }
        let is_user = user(message) && message.is_user_visible() && !message.is_tool_response();
        session.update(&json!({"sessionUpdate":if is_user{"user_message_chunk"}else{"agent_message_chunk"},"messageId":message.id.clone().unwrap_or_else(||format!("recovery-{index}")),"content":{"type":"text","text":text}}));
    }
    session.update(&json!({"sessionUpdate":"agent_message_chunk","messageId":"fastrock-recovery-notice","content":{"type":"text","text":format!("\n**Recovery review required**\n\n{reason}\n\nOpen Checkpoints to restore a completed boundary, or fork before the interrupted turn. No saved tools have been resumed.")}}));
    session.replaying = false;
    session.loaded = true;
    session.recovery_only = true;
}
impl Adapter {
    pub(super) async fn recovery_history(&self, id: &str) -> Result<bool, JSONRPCErrorError> {
        if !self.embedded_runtime {
            return Ok(false);
        }
        let manager = goose_runtime::session::SessionManager::new(
            goose_runtime::config::paths::Paths::data_dir(),
        );
        let conversation = super::checkpoint_storage::read(&manager, id)
            .await
            .map_err(|e| error(e.to_string()))?;
        let reason = if self
            .checkpoint_pending(id)
            .await
            .map_err(|e| error(e.to_string()))?
        {
            Some("A file/conversation restore was interrupted. Its retained backup needs review.")
        } else if uncertain(&conversation) {
            Some(
                "This conversation contains tool calls whose outcomes are uncertain. Opening it cannot replay those calls.",
            )
        } else {
            None
        };
        if let Some(reason) = reason {
            let mut sessions = self.sessions.lock().await;
            if let Some(s) = sessions.get_mut(id) {
                project(s, &conversation, reason);
            }
            drop(sessions);
            self.pause_agent_queue(id).await?;
            Ok(true)
        } else {
            if let Some(s) = self.sessions.lock().await.get_mut(id) {
                s.recovery_only = false;
            }
            Ok(false)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v63_uncertain_history_is_read_only_without_activating_tools() {
        let request = Message::assistant().with_content(Block::ActionRequired(
            goose_provider_types::conversation::message::ActionRequired {
                data: ActionRequiredData::ToolConfirmation {
                    id: "tool-1".into(),
                    tool_name: "write".into(),
                    arguments: json!({"path":"file"}).as_object().unwrap().clone(),
                    prompt: None,
                },
            },
        ));
        let mut conversation = Conversation::new_unvalidated([
            Message::user().with_text("Do work").with_id("u1"),
            request,
        ]);
        assert!(uncertain(&conversation));
        let mut session = Session::new("test", "C:\\fixture");
        project(&mut session, &conversation, "Uncertain tools");
        assert!(session.loaded && session.recovery_only && session.active.is_none());
        assert!(
            json!(session.turns.all_turns().unwrap())
                .to_string()
                .contains("No saved tools have been resumed")
        );
        conversation.push(Message::user().with_text("A later explicit question"));
        assert!(!uncertain(&conversation));
    }
}
