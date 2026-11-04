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
        .rposition(|m| user(m) && m.is_user_visible() && !m.is_tool_response() && !m.metadata.steer)
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
    let action = if session.storage_pending {
        "Saved history is available read-only. When workspace storage is available, choose Resume to retry loading. No saved tools have been resumed."
    } else {
        "Open Checkpoints to restore a completed boundary, or fork before the interrupted turn. No saved tools have been resumed."
    };
    session.update(&json!({"sessionUpdate":"agent_message_chunk","messageId":"fastrock-recovery-notice","content":{"type":"text","text":format!("\n**Recovery review required**\n\n{reason}\n\n{action}")}}));
    session.replaying = false;
    session.loaded = true;
    session.recovery_only = true;
}
impl Adapter {
    pub(super) async fn recovery_history(&self, id: &str) -> Result<bool, JSONRPCErrorError> {
        if !self.embedded_runtime {
            return Ok(false);
        }
        let manager = goose_runtime::session::SessionManager::instance();
        let conversation = super::checkpoint_storage::read(&manager, id)
            .await
            .map_err(|e| error(e.to_string()))?;
        let (reason, storage_pending) = if uncertain(&conversation) {
            (Some("This conversation contains tool calls whose outcomes are uncertain. Opening it cannot replay those calls.".to_string()), false)
        } else {
            // Browsing is already independent of activation. Explicit Resume
            // waits for the journal without imposing an IO speed requirement;
            // never interpret slow storage as an empty recovery journal.
            journal_status(self.checkpoint_pending(id)).await
        };
        if let Some(reason) = reason {
            let mut sessions = self.sessions.lock().await;
            if let Some(s) = sessions.get_mut(id) {
                s.storage_pending = storage_pending;
                project(s, &conversation, &reason);
            }
            drop(sessions);
            self.pause_agent_queue(id).await?;
            Ok(true)
        } else {
            if let Some(s) = self.sessions.lock().await.get_mut(id) {
                // A provisional view stays protected until ACP activation also
                // succeeds. A provider/extension error must remain retryable.
                if !s.storage_pending {
                    s.recovery_only = false;
                }
            }
            Ok(false)
        }
    }

    /// Only an explicit Resume can leave a provisional read-only history view.
    pub(super) async fn retry_storage_history(
        self: &Arc<Self>,
        id: &str,
    ) -> Result<bool, JSONRPCErrorError> {
        let pending = self
            .sessions
            .lock()
            .await
            .get(id)
            .is_some_and(|s| s.storage_pending || s.activation_pending);
        if pending {
            self.load_session(&json!({"threadId":super::model::ui_id(id)}), false)
                .await?;
            if self
                .sessions
                .lock()
                .await
                .get(id)
                .is_some_and(|s| s.storage_pending)
            {
                return Err(error(
                    "Workspace storage is still unavailable. Saved history remains read-only; retry Resume once the other instance has quit or storage has finished opening.",
                ));
            }
        }
        Ok(pending)
    }
}

async fn journal_status(
    read: impl std::future::Future<Output = anyhow::Result<bool>>,
) -> (Option<String>, bool) {
    match read.await {
        Ok(false) => (None, false),
        Ok(true) => (
            Some(
                "A file/conversation restore was interrupted. Its retained backup needs review."
                    .into(),
            ),
            false,
        ),
        Err(error) => (
            Some(format!(
                "Workspace recovery could not be checked: {error:#}"
            )),
            true,
        ),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn v94_unavailable_journal_keeps_history_read_only_and_retryable() {
        let (reason, retry) = journal_status(async { anyhow::bail!("store locked") }).await;
        let conversation = Conversation::new_unvalidated([
            Message::user().with_text("Existing question"),
            Message::assistant().with_text("Existing answer"),
        ]);
        let mut session = Session::new("old", "C:\\fixture");
        session.storage_pending = retry;
        project(&mut session, &conversation, &reason.unwrap());
        let history = json!(session.turns.all_turns().unwrap()).to_string();
        assert!(history.contains("Existing question") && history.contains("Existing answer"));
        assert!(history.contains("choose Resume"));
        assert!(
            session.loaded
                && session.recovery_only
                && session.storage_pending
                && session.active.is_none()
        );
        assert_eq!(journal_status(async { Ok(false) }).await, (None, false));
        let (reason, retry) = journal_status(async { Ok(true) }).await;
        assert!(reason.unwrap().contains("interrupted") && !retry);
        let (reason, retry) = journal_status(async { anyhow::bail!("locked") }).await;
        assert!(reason.unwrap().contains("locked") && retry);
    }
    #[tokio::test(start_paused = true)]
    async fn v100_resume_waits_for_slow_recovery_check() {
        let (send, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(journal_status(async { receive.await.unwrap() }));
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(3600)).await;
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        send.send(Ok(true)).unwrap();
        let (reason, retry) = task.await.unwrap();
        assert!(reason.unwrap().contains("interrupted") && !retry);
    }
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
        conversation.push(
            Message::user()
                .with_text("Steering the interrupted turn")
                .with_steer(),
        );
        assert!(
            uncertain(&conversation),
            "Steering must not hide an earlier uncertain tool"
        );
        conversation.push(Message::user().with_text("A later explicit question"));
        assert!(!uncertain(&conversation));
    }
}
