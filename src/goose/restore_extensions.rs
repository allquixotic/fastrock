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
        .context("Saved tool selection is invalid; preserve it for recovery")?;
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
    let session = manager.get_session(id, false).await?;
    let mut defaults = if has_selection(&session.extension_data)? {
        EnabledExtensionsState::from_extension_data(&session.extension_data)
            .context("Saved tool selection is invalid")?
            .extensions
    } else {
        tokio::task::spawn_blocking(|| {
            goose_runtime::config::extensions::get_enabled_extensions_with_config(Config::global())
        })
        .await?
    };
    // Old ordinary coding sessions acquire the local read-only recall tool.
    // Explicit empty/restricted tool sets and temporary/subagent sessions stay
    // untouched. Saved memory settings still control what the tool can return.
    let add_memory = !matches!(
        session.session_type,
        goose_runtime::session::SessionType::Hidden | goose_runtime::session::SessionType::SubAgent
    ) && defaults.iter().any(unrestricted_developer)
        && !defaults
            .iter()
            .any(|e| e.name() == super::memory_tool::NAME);
    if add_memory {
        defaults.push(ExtensionConfig::Builtin {
            name: super::memory_tool::NAME.into(),
            description: "Bounded local memory recall".into(),
            display_name: Some("Memories".into()),
            timeout: Some(5),
            bundled: Some(true),
            available_tools: vec![],
        });
        manager
            .set_extension_state(id, &EnabledExtensionsState::new(defaults))
            .await?;
        return Ok(());
    }
    persist(&manager, id, defaults).await
}

fn unrestricted_developer(extension: &ExtensionConfig) -> bool {
    match extension {
        ExtensionConfig::Platform {
            name,
            available_tools,
            ..
        }
        | ExtensionConfig::Builtin {
            name,
            available_tools,
            ..
        } => name == "developer" && available_tools.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use goose_runtime::{config::GooseMode, session::SessionType};
    use serde_json::json;

    #[test]
    fn v92_memory_migration_preserves_restricted_developer_tools() {
        let extension = |kind, tools| {
            serde_json::from_value::<ExtensionConfig>(json!({
                "type":kind,"name":"developer","description":"","available_tools":tools
            }))
            .unwrap()
        };
        for kind in ["builtin", "platform"] {
            assert!(unrestricted_developer(&extension(kind, json!([]))));
            assert!(!unrestricted_developer(&extension(
                kind,
                json!(["text_editor"])
            )));
        }
        let remote: ExtensionConfig = serde_json::from_value(json!({
            "type":"streamable_http","name":"developer","description":"","uri":"http://127.0.0.1:1"
        }))
        .unwrap();
        assert!(!unrestricted_developer(&remote));
    }

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
