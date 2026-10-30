//! Exact Goose message boundaries, using only the public session storage API.
use anyhow::{Result, ensure};
use goose_provider_types::conversation::Conversation;
use goose_runtime::session::SessionManager;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{self, Write};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Boundary {
    pub count: usize,
    pub last: Option<String>,
    pub digest: String,
}
struct HashWriter(Sha256);
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn boundary(conversation: &Conversation) -> Result<Boundary> {
    let mut hash = HashWriter(Sha256::new());
    serde_json::to_writer(&mut hash, conversation)?;
    Ok(Boundary {
        count: conversation.len(),
        last: conversation.last().and_then(|m| m.id.clone()),
        digest: format!("{:x}", hash.0.finalize()),
    })
}
pub(super) fn prefix(mut conversation: Conversation, point: &Boundary) -> Result<Conversation> {
    ensure!(
        conversation.len() >= point.count,
        "This conversation no longer contains the checkpoint's messages"
    );
    conversation.truncate(point.count);
    ensure!(
        boundary(&conversation)? == *point,
        "Conversation history changed or was compacted; this checkpoint can only restore files"
    );
    Ok(conversation)
}
pub(super) async fn read(manager: &SessionManager, id: &str) -> Result<Conversation> {
    Ok(manager
        .get_session(id, true)
        .await?
        .conversation
        .unwrap_or_default())
}
pub(super) async fn replace(
    manager: &SessionManager,
    id: &str,
    expected: &Boundary,
    target: &Conversation,
) -> Result<()> {
    ensure!(
        boundary(&read(manager, id).await?)? == *expected,
        "Conversation changed after review; review it again"
    );
    manager.replace_conversation(id, target).await
}
#[cfg(test)]
mod tests {
    use super::*;
    use goose_provider_types::conversation::message::Message;
    use goose_runtime::session::SessionType;
    #[tokio::test]
    async fn v63_rewind_exact_boundary_preserves_config_and_rejects_changed_history() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(temp.path().join("data"));
        let session = manager
            .create_session(
                temp.path().into(),
                "Checkpoint".into(),
                SessionType::User,
                Default::default(),
            )
            .await
            .unwrap();
        manager
            .update(&session.id)
            .provider_name("aws_bedrock")
            .apply()
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
            manager.add_message(&session.id, message).await.unwrap();
        }
        let all = read(&manager, &session.id).await.unwrap();
        let current = boundary(&all).unwrap();
        let mut first = all.clone();
        first.truncate(2);
        let point = boundary(&first).unwrap();
        let target = prefix(all.clone(), &point).unwrap();
        assert_eq!(target.len(), 2);
        replace(&manager, &session.id, &current, &target)
            .await
            .unwrap();
        assert_eq!(read(&manager, &session.id).await.unwrap(), first);
        assert_eq!(
            manager
                .get_session(&session.id, false)
                .await
                .unwrap()
                .provider_name
                .as_deref(),
            Some("aws_bedrock")
        );
        assert!(
            replace(&manager, &session.id, &current, &all)
                .await
                .is_err()
        );
        let mut changed = all.clone();
        changed.messages_mut()[0] = Message::user().with_text("different").with_id("u1");
        assert!(prefix(changed, &point).is_err());
        assert!(prefix(first.clone(), &current).is_err());
        let empty = prefix(all.clone(), &boundary(&Conversation::empty()).unwrap()).unwrap();
        assert!(empty.is_empty());
        replace(&manager, &session.id, &boundary(&first).unwrap(), &all)
            .await
            .unwrap();
        assert_eq!(read(&manager, &session.id).await.unwrap(), all);
    }
}
