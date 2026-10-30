//! Goose allocates date/counter IDs from MAX(existing IDs), including hidden
//! sessions. Reserve a higher empty ID before deletion so durable Fastrock
//! bindings can never attach to a newly created conversation with a reused ID.
use anyhow::Result;
use goose_runtime::session::{SessionManager, SessionType};

pub(super) async fn before_delete(manager: &SessionManager, id: &str) -> Result<()> {
    let session = manager.get_session(id, false).await?;
    manager
        .create_session(
            session.working_dir,
            "Fastrock ID reservation".into(),
            SessionType::Hidden,
            session.goose_mode,
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use goose_provider_types::conversation::message::Message;
    #[tokio::test]
    async fn v74_deleted_highest_id_cannot_reuse_retained_bindings() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(temp.path().join("data"));
        let old = manager
            .create_session(
                temp.path().into(),
                "Delete me".into(),
                SessionType::User,
                Default::default(),
            )
            .await
            .unwrap();
        manager
            .add_message(&old.id, &Message::user().with_text("History to delete"))
            .await
            .unwrap();
        before_delete(&manager, &old.id).await.unwrap();
        manager.delete_session(&old.id).await.unwrap();
        let new = manager
            .create_session(
                temp.path().into(),
                "New conversation".into(),
                SessionType::User,
                Default::default(),
            )
            .await
            .unwrap();
        assert_ne!(new.id, old.id);
        let visible = manager.list_sessions().await.unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, new.id);
        let all = manager.list_all_sessions().await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(
            all.iter()
                .filter(|s| s.session_type == SessionType::Hidden)
                .all(|s| s.message_count == 0)
        );
        assert!(manager.get_session(&old.id, true).await.is_err());
    }
}
