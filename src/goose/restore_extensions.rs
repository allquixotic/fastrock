//! Older Goose sessions inherit global tools until a selection is persisted.
//! Goose's public selection mutators start from empty when that state is absent,
//! so materialize inheritance before activation or adding/restricting tools.
use anyhow::{Context, Result};
use goose_runtime::{
    config::{Config, ExtensionConfig},
    session::{
        SessionManager,
        extension_data::{EnabledExtensionsState, ExtensionData, ExtensionState},
    },
};

fn has_selection(data: &ExtensionData) -> Result<bool> {
    let Some(value) = data.get_extension_state(
        EnabledExtensionsState::EXTENSION_NAME,
        EnabledExtensionsState::VERSION,
    ) else {
        return Ok(false);
    };
    // Malformed explicit state is not permission to enable inherited tools.
    let state: EnabledExtensionsState = serde_json::from_value(value.clone())
        .context("Saved Goose tool selection is invalid; preserve it for recovery")?;
    EnabledExtensionsState::check_unique_keys(&state.extensions)?;
    Ok(true)
}

async fn persist(manager: &SessionManager, id: &str, defaults: Vec<ExtensionConfig>) -> Result<()> {
    // Caller holds the session gate and has not activated its agent. Preserve
    // explicit empty/restricted choices and all unrelated extension state.
    let session = manager.get_session(id, false).await?;
    if !has_selection(&session.extension_data)? {
        EnabledExtensionsState::check_unique_keys(&defaults)?;
        manager
            .set_extension_state(id, &EnabledExtensionsState::new(defaults))
            .await?;
    }
    Ok(())
}

pub(super) async fn prepare(id: &str) -> Result<()> {
    let manager = SessionManager::new(goose_runtime::config::paths::Paths::data_dir());
    if has_selection(&manager.get_session(id, false).await?.extension_data)? {
        return Ok(());
    }
    let defaults = tokio::task::spawn_blocking(|| {
        goose_runtime::config::extensions::get_enabled_extensions_with_config(Config::global())
    })
    .await?;
    persist(&manager, id, defaults).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use goose_runtime::{config::GooseMode, session::SessionType};
    use serde_json::json;

    #[tokio::test]
    async fn v81_legacy_tools_materialize_without_replacing_explicit_choices() {
        let root = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(root.path().into());
        let summon: ExtensionConfig = serde_json::from_value(json!({
            "type":"platform", "name":"summon", "description":"", "available_tools":["load"]
        }))
        .unwrap();
        for id in ["legacy", "empty", "restricted", "corrupt"] {
            manager
                .create_session_with_id(
                    id,
                    root.path().into(),
                    id.into(),
                    SessionType::Acp,
                    GooseMode::Auto,
                )
                .await
                .unwrap();
            manager
                .set_extension_value(id, "fixture", "v0", json!({"preserved":true}))
                .await
                .unwrap();
        }
        manager
            .set_extension_state("empty", &EnabledExtensionsState::new(vec![]))
            .await
            .unwrap();
        manager
            .set_extension_state(
                "restricted",
                &EnabledExtensionsState::new(vec![summon.clone()]),
            )
            .await
            .unwrap();
        manager
            .set_extension_value(
                "corrupt",
                "enabled_extensions",
                "v0",
                json!({"extensions":"invalid"}),
            )
            .await
            .unwrap();
        for id in ["legacy", "empty", "restricted"] {
            persist(&manager, id, vec![summon.clone()]).await.unwrap();
            let session = manager.get_session(id, false).await.unwrap();
            let selected =
                EnabledExtensionsState::from_extension_data(&session.extension_data).unwrap();
            assert_eq!(selected.extensions.len(), usize::from(id != "empty"));
            assert_eq!(
                session.extension_data.get_extension_state("fixture", "v0"),
                Some(&json!({"preserved":true}))
            );
        }
        // A later global change cannot erase an explicit or materialized choice.
        persist(&manager, "restricted", vec![]).await.unwrap();
        assert_eq!(
            EnabledExtensionsState::from_extension_data(
                &manager
                    .get_session("restricted", false)
                    .await
                    .unwrap()
                    .extension_data
            )
            .unwrap()
            .extensions
            .len(),
            1
        );
        assert!(persist(&manager, "corrupt", vec![summon]).await.is_err());
        assert_eq!(
            manager
                .get_session("corrupt", false)
                .await
                .unwrap()
                .extension_data
                .get_extension_state("enabled_extensions", "v0"),
            Some(&json!({"extensions":"invalid"}))
        );
    }
}
