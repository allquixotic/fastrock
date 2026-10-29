//! Preserve vwork's command-hook policy without requiring its executable.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(super) async fn prepare(cwd: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("Hook setup unavailable"))?;
        let executable = std::env::current_exe()?;
        let command = helper_command(&executable);
        let root = goose_runtime::plugins::plugin_install_dir().join("fastrock-vwork-policy");
        std::fs::create_dir_all(root.join("hooks"))?;
        let manifest = root.join("plugin.json");
        if !manifest.exists() {
            crate::rally::store::atomic_write(
                &manifest,
                &serde_json::to_vec_pretty(&json!({
                    "name":"fastrock-vwork-policy", "version":"0.4.0",
                    "description":"Fastrock embedded vwork policy"
                }))?,
            )?;
        }
        let hooks = root.join("hooks/hooks.json");
        if !hooks.exists() {
            crate::rally::store::atomic_write(
                &hooks,
                &serde_json::to_vec_pretty(&json!({
                    "hooks":{"PreToolUse":[{"matcher":fastrock_workspaces::embedded::hook_matcher(),
                        "hooks":[{"type":"command","command":command,"on_failure":"allow"}]}]}
                }))?,
            )?;
        }
        // Goose remains responsible for plugin enable/disable settings. Only
        // recognized vwork helper commands change; other hook content stays intact.
        for plugin in goose_runtime::plugins::discovery::discover_enabled_plugins(Some(&cwd)) {
            migrate_file(
                &plugin.root.join("hooks/hooks.json"),
                &command,
                plugin.root == root,
            )?;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

fn helper_command(executable: &Path) -> String {
    let path = executable.to_string_lossy();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    format!(
        "'{}' --vwork-hook",
        path.replace('\\', "/").replace('\'', "'\"'\"'")
    )
}

fn legacy_hook(command: &str) -> bool {
    let Some(executable) = command.trim().strip_suffix(" hook") else {
        return false;
    };
    let executable = executable.trim();
    let executable = if let Some(path) = executable
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
    {
        if path.contains('\'') {
            return false;
        }
        path
    } else if let Some(path) = executable
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
    {
        if path.contains(['"', '$', '`']) {
            return false;
        }
        path
    } else {
        if executable.chars().any(char::is_whitespace)
            || executable.contains(['|', ';', '&', '>', '<', '$', '`', '(', ')', '\'', '"'])
        {
            return false;
        }
        executable
    };
    executable.rsplit(['/', '\\']).next().is_some_and(|name| {
        name.eq_ignore_ascii_case("vwork") || name.eq_ignore_ascii_case("vwork.exe")
    })
}

fn migrate_file(path: &Path, command: &str, owned: bool) -> Result<()> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    ensure!(bytes.len() <= 1024 * 1024, "Hook definition exceeds 1 MiB");
    let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
        // Preserve Goose's handling of malformed unrelated plugin files.
        return Ok(());
    };
    let mut changed = false;
    for rules in value["hooks"]
        .as_object_mut()
        .into_iter()
        .flat_map(|hooks| hooks.values_mut())
    {
        for rule in rules.as_array_mut().into_iter().flatten() {
            for action in rule["hooks"].as_array_mut().into_iter().flatten() {
                if action["type"] != "command" {
                    continue;
                }
                let Some(old) = action["command"].as_str() else {
                    continue;
                };
                if old != command && (legacy_hook(old) || (owned && old.ends_with(" --vwork-hook")))
                {
                    action["command"] = json!(command);
                    changed = true;
                }
            }
        }
    }
    if changed {
        let backup = path.with_extension("json.pre-fastrock-040");
        // A first migration backup is never replaced, even after an app update.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).context("Save original vwork hook"),
        }
        crate::rally::store::atomic_write(path, &serde_json::to_vec_pretty(&value)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v38_hook_migration_preserves_custom_actions_and_original_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        let original = json!({"hooks":{"PreToolUse":[{"matcher":"custom", "hooks":[
            {"type":"command","command":"'C:/Tools/vwork.exe' hook","on_failure":"block","timeout":7},
            {"type":"command","command":"custom-hook --flag"}]}]}});
        std::fs::write(&path, original.to_string()).unwrap();
        let command = helper_command(Path::new("C:/Fastrock's folder/fastrock.exe"));
        assert_eq!(
            command,
            "'C:/Fastrock'\"'\"'s folder/fastrock.exe' --vwork-hook"
        );
        migrate_file(&path, &command, false).unwrap();
        migrate_file(&path, &command, false).unwrap();
        let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let actions = &value["hooks"]["PreToolUse"][0]["hooks"];
        assert_eq!(actions[0]["command"], command);
        assert_eq!(actions[0]["on_failure"], "block");
        assert_eq!(actions[0]["timeout"], 7);
        assert_eq!(actions[1]["command"], "custom-hook --flag");
        let backup: Value = serde_json::from_slice(
            &std::fs::read(path.with_extension("json.pre-fastrock-040")).unwrap(),
        )
        .unwrap();
        assert_eq!(backup, original);
        assert!(!legacy_hook("wrapper vwork hook --extra"));
        assert!(!legacy_hook("wrapper /path/vwork hook"));
        assert!(!legacy_hook("echo>/path/vwork hook"));
    }
}
