use serde_json::Value;
pub(super) fn context(
    session: &super::model::Session,
    input: &Value,
    turn: &str,
) -> Option<super::memory::Context> {
    use sha2::{Digest, Sha256};
    const MAX_INPUT: usize = 8192;
    if session.ephemeral || session.binding.delegation.is_some() || session.binding.side {
        return None;
    }
    let input = input
        .as_array()?
        .iter()
        .filter(|p| p["type"] == "text")
        .filter_map(|p| p["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if input.len() > MAX_INPUT || input.trim_start().starts_with('/') {
        return None;
    }
    let (scope, name) = if let Some(workspace) = &session.binding.workspace {
        (
            fastrock_workspaces::projects::id(&workspace.selection.home, &workspace.repo),
            workspace.source_root.to_string_lossy().into_owned(),
        )
    } else {
        let path = session.info["cwd"].as_str()?;
        let normalized = path.replace('\\', "/").trim_end_matches('/').to_owned();
        let normalized =
            if normalized.as_bytes().get(1) == Some(&b':') || normalized.starts_with("//") {
                normalized.to_lowercase()
            } else {
                normalized
            };
        (
            format!(
                "directory:{}",
                hex::encode(Sha256::digest(normalized.as_bytes()))
            ),
            path.to_owned(),
        )
    };
    let mut name = name;
    while name.len() > 256 {
        name.pop();
    }
    Some(super::memory::Context {
        scope,
        name,
        input,
        turn: turn.into(),
        learn: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn v92_temporary_and_commands_do_not_contribute() {
        let mut session = super::super::model::Session::new("s", "C:\\Repository");
        let input = json!([{"type":"text","text":"Quartz means our release service"}]);
        let first = context(&session, &input, "turn").unwrap();
        session.info["cwd"] = json!("c:/repository/");
        assert_eq!(
            first.scope,
            context(&session, &input, "turn").unwrap().scope
        );
        session.ephemeral = true;
        assert!(context(&session, &input, "turn").is_none());
        session.ephemeral = false;
        assert!(
            context(
                &session,
                &json!([{"type":"text","text":"/compact"}]),
                "turn"
            )
            .is_none()
        );
    }
}
