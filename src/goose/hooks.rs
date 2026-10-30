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
        prepare_at(&cwd, &root, &command)?;
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

/// Pre-register discovered directories without round-tripping config.yaml.
/// Goose remains authoritative for enable/disable settings and scope precedence.
pub(super) fn discover(
    cwd: &Path,
) -> Result<Vec<goose_runtime::plugins::discovery::DiscoveredPlugin>> {
    let roots = [
        goose_runtime::plugins::plugin_install_dir(),
        cwd.join(".agents/plugins"),
    ];
    register(goose_runtime::config::Config::global(), &roots)?;
    Ok(goose_runtime::plugins::discovery::discover_enabled_plugins(
        Some(cwd),
    ))
}

fn register(config: &goose_runtime::config::Config, roots: &[PathBuf]) -> Result<()> {
    let mut paths = Vec::new();
    for root in roots {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let path = entry?.path();
            if path.is_dir() && path.file_name().and_then(|s| s.to_str()).is_some() {
                paths.push(path.to_string_lossy().into_owned());
            }
        }
    }
    if paths.is_empty() {
        return Ok(());
    }
    super::config_guard::edit(config, |config| {
        let mut plugins: Value = match config.get_param("plugins") {
            Ok(value) => value,
            Err(goose_runtime::config::ConfigError::NotFound(_)) => json!({}),
            Err(_) => anyhow::bail!("Cannot read Goose plugin settings"),
        };
        let entries = plugins
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Goose plugins must be a mapping"))?;
        ensure!(
            entries.values().all(|entry| entry["enabled"].is_boolean()),
            "Invalid Goose plugin setting; original configuration preserved"
        );
        let mut changed = false;
        for path in paths {
            if !entries.contains_key(&path) {
                entries.insert(path, json!({"enabled":true}));
                changed = true;
            }
        }
        if changed {
            config.set_param("plugins", super::config_guard::parameter(&plugins)?)?;
        }
        Ok(())
    })
}

fn prepare_at(cwd: &Path, root: &Path, command: &str) -> Result<()> {
    let plugins = discover(cwd)?;
    let mut inherited = false;
    for plugin in plugins {
        let hooks = plugin.root.join("hooks/hooks.json");
        migrate_file(&hooks, command, plugin.root == root)?;
        // Reuse user-wide policy. A project-only hook cannot replace a global
        // policy needed by other concurrently open repositories.
        if plugin.root != root
            && plugin.scope == goose_runtime::plugins::discovery::PluginScope::User
            && has_policy(&hooks, command)?
        {
            inherited = true;
        }
    }
    if !inherited || root.exists() {
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
        managed_policy(&root.join("hooks/hooks.json"), command, !inherited)?;
        // Do this before Goose initializes its hooks, avoiding its lossy setter.
        discover(cwd)?;
    }
    Ok(())
}

fn has_policy(path: &Path, command: &str) -> Result<bool> {
    let Some((_, value)) = read_hooks(path)? else {
        return Ok(false);
    };
    Ok(value["hooks"]["PreToolUse"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|rule| {
            rule["hooks"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|action| {
                    action["type"] == "command" && action["command"].as_str() == Some(command)
                })
        }))
}

fn managed_policy(path: &Path, command: &str, needed: bool) -> Result<()> {
    let existing = read_hooks(path)?;
    ensure!(
        existing.is_some() || !path.exists(),
        "Managed policy hooks are malformed; original file preserved"
    );
    let (original, mut value) = existing.unwrap_or_else(|| (Vec::new(), json!({"hooks":{}})));
    ensure!(
        value.is_object() && value["hooks"].is_object(),
        "Managed policy hooks are malformed; original file preserved"
    );
    let mut present = false;
    if let Some(rules) = value["hooks"]["PreToolUse"].as_array_mut() {
        for rule in rules {
            if let Some(actions) = rule.get_mut("hooks").and_then(Value::as_array_mut) {
                actions.retain(|action| {
                    let policy = action["type"] == "command"
                        && action["command"].as_str().is_some_and(|old| {
                            old == command || legacy_hook(old) || embedded_hook(old)
                        });
                    present |= policy;
                    !policy || needed
                });
            }
        }
    }
    if needed && !present {
        if value["hooks"]["PreToolUse"].is_null() {
            value["hooks"]["PreToolUse"] = json!([]);
        }
        let rules = value["hooks"]["PreToolUse"]
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid managed policy hooks"))?;
        rules.push(
            json!({"matcher":fastrock_workspaces::embedded::hook_matcher(),
            "hooks":[{"type":"command","command":command,"on_failure":"allow"}]}),
        );
    }
    if original.is_empty() || serde_json::from_slice::<Value>(&original)? != value {
        save_hooks(path, &original, &value)?;
    }
    Ok(())
}

fn embedded_hook(command: &str) -> bool {
    shlex::split(command).is_some_and(|args| {
        args.len() == 2
            && args[1] == "--vwork-hook"
            && args[0].rsplit(['/', '\\']).next().is_some_and(|name| {
                name.eq_ignore_ascii_case("fastrock") || name.eq_ignore_ascii_case("fastrock.exe")
            })
    })
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

fn read_hooks(path: &Path) -> Result<Option<(Vec<u8>, Value)>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(bytes.len() <= 1024 * 1024, "Hook definition exceeds 1 MiB");
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(None);
    };
    Ok(Some((bytes, value)))
}

fn migrate_file(path: &Path, command: &str, owned: bool) -> Result<()> {
    let Some((bytes, mut value)) = read_hooks(path)? else {
        return Ok(());
    };
    let mut changed = false;
    for rules in value
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .into_iter()
        .flat_map(|hooks| hooks.values_mut())
    {
        for rule in rules.as_array_mut().into_iter().flatten() {
            for action in rule
                .get_mut("hooks")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                if action["type"] != "command" {
                    continue;
                }
                let Some(old) = action["command"].as_str() else {
                    continue;
                };
                if old != command
                    && (legacy_hook(old)
                        || embedded_hook(old)
                        || (owned && old.ends_with(" --vwork-hook")))
                {
                    action["command"] = json!(command);
                    changed = true;
                }
            }
        }
    }
    if changed {
        save_hooks(path, &bytes, &value)?;
    }
    Ok(())
}

fn save_hooks(path: &Path, bytes: &[u8], value: &Value) -> Result<()> {
    if !bytes.is_empty() {
        let backup = path.with_extension("json.pre-fastrock-040");
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(bytes)?;
                file.sync_all()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).context("Save original vwork hook"),
        }
    }
    crate::rally::store::atomic_write(path, &serde_json::to_vec_pretty(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v83_plugin_registration_preserves_comments_disabled_entries_and_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "# provider explanation\nGOOSE_PROVIDER: custom_bedrock_claude # preserved\nplugins: {}\n").unwrap();
        let config = goose_runtime::config::Config::new(&path, "fixture").unwrap();
        let root = dir.path().join("plugins");
        let plugin = root.join("old");
        std::fs::create_dir_all(&plugin).unwrap();
        register(&config, &[root.clone()]).unwrap();
        super::super::config_guard::edit(&config, |draft| {
            let mut raw: Value = draft.get_param("plugins")?;
            raw[plugin.to_str().unwrap()] = json!({"enabled":false,"unfamiliar":7});
            Ok(draft.set_param("plugins", super::super::config_guard::parameter(&raw)?)?)
        })
        .unwrap();
        std::fs::create_dir(root.join("new")).unwrap();
        register(&config, &[root.clone()]).unwrap();
        let raw: Value = config.get_param("plugins").unwrap();
        assert_eq!(
            raw[plugin.to_str().unwrap()],
            json!({"enabled":false,"unfamiliar":7})
        );
        let bytes = std::fs::read_to_string(&path).unwrap();
        assert!(bytes.contains("# provider explanation"));
        assert!(bytes.contains("custom_bedrock_claude # preserved"));
        register(&config, &[root]).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
    }

    #[test]
    fn v84_managed_duplicate_is_removed_without_touching_other_actions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        let command = helper_command(Path::new("C:/Fastrock/fastrock.exe"));
        managed_policy(&path, &command, true).unwrap();
        let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["hooks"]["PreToolUse"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"command","command":"custom-hook --keep"}));
        std::fs::write(&path, value.to_string()).unwrap();
        managed_policy(&path, &command, false).unwrap();
        assert!(!has_policy(&path, &command).unwrap());
        let once = std::fs::read(&path).unwrap();
        managed_policy(&path, &command, false).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), once);
        let value: Value = serde_json::from_slice(&once).unwrap();
        assert_eq!(
            value["hooks"]["PreToolUse"][0]["hooks"],
            json!([{"type":"command","command":"custom-hook --keep"}])
        );
        assert!(path.with_extension("json.pre-fastrock-040").is_file());
        managed_policy(&path, &command, true).unwrap();
        assert!(has_policy(&path, &command).unwrap());
        for malformed in ["{broken", "null", "{\"hooks\":17}"] {
            std::fs::write(&path, malformed).unwrap();
            migrate_file(&path, &command, true).unwrap();
            assert!(managed_policy(&path, &command, true).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), malformed);
        }
    }
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
        let updated = helper_command(Path::new("C:/New version/fastrock.exe"));
        migrate_file(&path, &updated, false).unwrap();
        assert!(has_policy(&path, &updated).unwrap());
    }
}
