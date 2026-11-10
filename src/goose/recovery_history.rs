//! Settle interrupted tools through public Goose storage before activation.
use super::*;
use goose_provider_types::conversation::{
    Conversation,
    message::{ActionRequiredData, Message, MessageContentBlock as Block},
};
use std::collections::BTreeSet;

pub(super) const INTERRUPTED: &str = "Interrupted when Fastrock stopped. This tool call failed without a saved result and will not be retried automatically. It may have made changes before stopping.";

pub(super) fn unfinished(conversation: &Conversation) -> BTreeSet<String> {
    let mut requested = BTreeSet::new();
    let mut answered = BTreeSet::new();
    for message in conversation.messages() {
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
    requested.retain(|id| !answered.contains(id));
    requested
}

pub(super) fn uncertain(conversation: &Conversation) -> bool {
    !unfinished(conversation).is_empty()
}

pub(super) fn settle(conversation: &mut Conversation) -> anyhow::Result<bool> {
    let ids = unfinished(conversation);
    let changed = !ids.is_empty();
    for id in ids {
        // Deserialize the public MCP result type without depending on Goose's
        // private state machine or changing any request/approval/history bytes.
        let result = serde_json::from_value(
            json!({"content":[{"type":"text","text":INTERRUPTED}],"isError":true}),
        )?;
        conversation.push(Message::user().with_tool_response(id, Ok(result)));
    }
    Ok(changed)
}

impl Adapter {
    pub(super) async fn recovery_history(&self, id: &str) -> Result<bool, JSONRPCErrorError> {
        if !self.embedded_runtime {
            return Ok(false);
        }
        self.checkpoint_ready(id)
            .await
            .map_err(|e| error(format!("Open workspace history: {e:#}")))?;
        let manager = goose_runtime::session::SessionManager::instance();
        let mut conversation = super::checkpoint_storage::read(&manager, id)
            .await
            .map_err(|e| error(e.to_string()))?;
        let original_count = conversation.len();
        if settle(&mut conversation).map_err(|e| error(e.to_string()))? {
            // Append only: retain original message IDs, timestamps, visibility
            // and encoded content. A crash partway through is idempotent: the
            // next scan finds only requests whose failure was not yet saved.
            for message in &conversation.messages()[original_count..] {
                manager
                    .add_message(id, message)
                    .await
                    .map_err(|e| error(e.to_string()))?;
            }
        }
        if let Some(s) = self.sessions.lock().await.get_mut(id) {
            s.recovery_only = false;
        }
        Ok(false)
    }

    /// Activate lazily for the next user action; opening history starts no work.
    pub(super) async fn retry_storage_history(
        self: &Arc<Self>,
        id: &str,
    ) -> Result<bool, JSONRPCErrorError> {
        let pending = self
            .sessions
            .lock()
            .await
            .get(id)
            .is_some_and(|s| s.storage_pending || s.activation_pending || !s.loaded);
        if pending {
            self.load_session(&json!({"threadId":super::model::ui_id(id)}), false)
                .await?;
        }
        Ok(pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v107_interrupted_approvals_and_tools_fail_once_without_changing_history() {
        let request: Message = serde_json::from_value(json!({"role":"assistant","created":1,"content":[
            {"type":"toolRequest","id":"tool-1","toolCall":{"status":"success","value":{"name":"write","arguments":{"path":"file"}}}},
            {"type":"actionRequired","data":{"actionType":"toolConfirmation","id":"tool-1","toolName":"write","arguments":{"path":"file"}}}
        ],"metadata":{"userVisible":true,"agentVisible":true}})).unwrap();
        let response: Message = serde_json::from_value(json!({"role":"user","created":2,"content":[
            {"type":"actionRequired","data":{"actionType":"toolConfirmationResponse","id":"tool-1","permission":"allow_once"}}
        ],"metadata":{"userVisible":false,"agentVisible":false}})).unwrap();
        let mut conversation = Conversation::new_unvalidated([
            Message::user().with_text("Do work"),
            request,
            response,
            Message::user().with_text("Steer").with_steer(),
        ]);
        let before = conversation.clone();
        assert!(settle(&mut conversation).unwrap());
        assert_eq!(&conversation.messages()[..before.len()], before.messages());
        assert!(unfinished(&conversation).is_empty());
        assert!(!settle(&mut conversation).unwrap());
        assert_eq!(conversation.len(), before.len() + 1);
        assert!(conversation.last().unwrap().is_tool_response());
        let Block::ToolResponse(r) = &conversation.last().unwrap().content[0] else {
            panic!()
        };
        assert_eq!(r.tool_result.as_ref().unwrap().is_error, Some(true));
    }
}
