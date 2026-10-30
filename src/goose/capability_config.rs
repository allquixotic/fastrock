//! Native capability settings; no secret setter or private Goose storage API.
use anyhow::{Context, Result, bail, ensure};
use goose_runtime::{
    agents::ExtensionConfig,
    config::{Config, ConfigError},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub(super) const LIMIT: usize = 512;
pub(super) fn revision(value: &Value) -> String {
    let mut value = value.clone();
    value.sort_all_objects();
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&value).unwrap_or_default())
    )
}
fn read(config: &Config, key: &str) -> Result<Value> {
    super::config_guard::validate(config)?;
    match config.get_param::<Value>(key) {
        Ok(value) => {
            ensure!(
                value.is_object(),
                "Goose {key} configuration must be a mapping"
            );
            Ok(value)
        }
        Err(ConfigError::NotFound(_)) => Ok(json!({})),
        Err(e) => Err(e.into()),
    }
}
pub(super) fn protected(name: &str) -> bool {
    let key = name.to_ascii_lowercase();
    key.contains("vwork") || key.starts_with("fastrock")
}
pub(super) fn managed_extension(key: &str, value: &Value) -> bool {
    protected(key)
        || protected(value["name"].as_str().unwrap_or(""))
        || (value["type"] == "stdio"
            && value["cmd"].as_str().is_some_and(|cmd| {
                cmd.rsplit(['/', '\\']).next().is_some_and(|binary| {
                    binary.eq_ignore_ascii_case("vwork") || binary.eq_ignore_ascii_case("vwork.exe")
                })
            }))
}
pub(super) fn managed_session_name(
    name: &str,
    bindings: &[fastrock_workspaces::vwork::McpBinding],
    tool: bool,
) -> bool {
    protected(name)
        || bindings.iter().any(|b| {
            name == b.name
                || (tool
                    && name
                        .strip_prefix(&goose_runtime::config::extensions::name_to_key(&b.name))
                        .is_some_and(|rest| rest.starts_with("__")))
        })
}
fn bounded(value: &str, max: usize, label: &str) -> Result<()> {
    ensure!(
        value.len() <= max && !value.contains('\0'),
        "Invalid or oversized {label}"
    );
    Ok(())
}

pub(super) fn snapshot(cwd: &Path) -> Result<Value> {
    let config = Config::global();
    let raw = read(config, "extensions")?;
    let mut entries = Vec::new();
    for (key, value) in raw.as_object().unwrap() {
        entries.push(extension_row(key, value, true));
    }
    for extension in goose_runtime::config::extensions::get_available_extensions() {
        let key = extension.key();
        if !raw.as_object().unwrap().contains_key(&key) {
            let mut value = serde_json::to_value(&extension)?;
            value["enabled"] = json!(false);
            entries.push(extension_row(&key, &value, false));
        }
    }
    entries.sort_by_key(|e| e["name"].as_str().unwrap_or("").to_ascii_lowercase());
    ensure!(
        entries.len() <= LIMIT,
        "Extension catalog exceeds {LIMIT} entries"
    );
    // Discovery respects project/user precedence and registers new plugins with
    // Goose's public config. Disabled plugins remain visible in that config.
    let active = goose_runtime::plugins::discovery::discover_enabled_plugins(Some(cwd));
    let plugins = read(config, "plugins")?;
    ensure!(
        plugins.as_object().unwrap().len() <= LIMIT,
        "Plugin catalog exceeds {LIMIT} entries"
    );
    let hooks = plugins
        .as_object()
        .unwrap()
        .iter()
        .map(|(path, value)| {
            let root = Path::new(path);
            let name = root.file_name().unwrap_or_default().to_string_lossy();
            json!({"key":path,"name":name,"enabled":value["enabled"]==true,
            "active":active.iter().any(|p|p.root==root), "protected":protected(&name),
            "hasHooks":root.join("hooks/hooks.json").is_file()})
        })
        .collect::<Vec<_>>();
    Ok(
        json!({"extensions":entries,"revision":revision(&raw),"hooks":hooks,"pluginRevision":revision(&plugins)}),
    )
}
fn extension_row(key: &str, value: &Value, configured: bool) -> Value {
    // Deliberately never publish commands, arguments, headers or env values.
    json!({"key":key,"name":value["display_name"].as_str().or(value["name"].as_str()).unwrap_or(key),
        "kind":value["type"].as_str().unwrap_or("unknown"),"enabled":value["enabled"]==true,
        "configured":configured,"protected":managed_extension(key, value)})
}
fn mutate(
    config: &Config,
    key: &str,
    expected: &str,
    edit: impl FnOnce(&mut Value) -> Result<()>,
) -> Result<()> {
    super::config_guard::validate(config)?;
    ensure!(
        std::env::var_os(key.to_uppercase()).is_none(),
        "An environment override controls {key}; change it before editing defaults"
    );
    // Value preserves unfamiliar fields and malformed entries instead of
    // round-tripping the whole mapping through ExtensionEntry.
    let mut outcome = Ok(());
    config.update_param::<Value, Value, _>(key, |raw| {
        let original = if raw.is_null() { json!({}) } else { raw };
        let mut next = original.clone();
        outcome = (|| {
            ensure!(
                next.is_object() && revision(&next) == expected,
                "Goose settings changed; reload before saving"
            );
            edit(&mut next)
        })();
        if outcome.is_ok() { next } else { original }
    })?;
    outcome
}
pub(super) fn toggle(
    config: &Config,
    group: &str,
    key: &str,
    expected: &str,
    enabled: bool,
) -> Result<()> {
    ensure!(
        matches!(group, "extensions" | "plugins"),
        "Unknown capability group"
    );
    let name = if group == "plugins" {
        Path::new(key)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    } else {
        key.into()
    };
    ensure!(
        !protected(&name),
        "Fastrock's workspace policy and tool bindings are managed by Fastrock"
    );
    mutate(config, group, expected, |raw| {
        if raw.get(key).is_none() && group == "extensions" {
            let extension = goose_runtime::config::extensions::get_available_extensions()
                .into_iter()
                .find(|e| e.key() == key)
                .context("Extension no longer available")?;
            raw[key] = serde_json::to_value(extension)?;
        }
        ensure!(
            raw.get(key).is_some_and(Value::is_object),
            "Capability no longer exists"
        );
        ensure!(
            group != "extensions" || !managed_extension(key, &raw[key]),
            "Fastrock's workspace policy and tool bindings are managed by Fastrock"
        );
        raw[key]["enabled"] = json!(enabled);
        Ok(())
    })
}
pub(super) fn new_extension(form: &Value) -> Result<Value> {
    let name = form["name"].as_str().unwrap_or("").trim();
    ensure!(
        !name.is_empty()
            && name.len() <= 80
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
        "Use 1–80 letters, numbers, underscores or hyphens for the extension name"
    );
    ensure!(
        !protected(name),
        "This name is reserved for Fastrock's workspace tools"
    );
    let address = form["address"].as_str().unwrap_or("").trim();
    bounded(address, 4096, "extension address")?;
    ensure!(!address.is_empty(), "Enter a command or URL");
    let raw_args = form["args"].as_str().unwrap_or("[]");
    bounded(raw_args, 256 * 1024, "argument list")?;
    let args: Vec<String> =
        serde_json::from_str(raw_args).context("Arguments must be a JSON array of strings")?;
    ensure!(args.len() <= 64, "At most 64 arguments");
    for arg in &args {
        bounded(arg, 4096, "argument")?;
    }
    let env_keys: Vec<_> = form["keys"]
        .as_str()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    ensure!(
        env_keys.len() <= 64
            && env_keys.iter().all(
                |s| s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            ),
        "Enter comma-separated environment or saved Goose credential key names"
    );
    let mut value = match form["kind"].as_str() {
        Some("stdio") => {
            json!({"type":"stdio","name":name,"cmd":address,"args":args,"env_keys":env_keys,"timeout":300})
        }
        Some("http") => {
            let url = url::Url::parse(address).context("Enter an HTTP or HTTPS URL")?;
            ensure!(
                matches!(url.scheme(), "http" | "https")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none(),
                "URL must use HTTP(S) without credentials, query or fragment"
            );
            json!({"type":"streamable_http","name":name,"uri":address,"env_keys":env_keys,"timeout":300})
        }
        _ => bail!("Unknown extension transport"),
    };
    let _: ExtensionConfig = serde_json::from_value(value.clone())?;
    value["enabled"] = json!(false);
    Ok(value)
}
pub(super) fn add(config: &Config, expected: &str, form: &Value) -> Result<()> {
    let value = new_extension(form)?;
    let key = goose_runtime::config::extensions::name_to_key(value["name"].as_str().unwrap());
    mutate(config, "extensions", expected, |raw| {
        ensure!(raw.get(&key).is_none(), "That extension already exists");
        ensure!(
            raw.as_object().unwrap().len() < LIMIT,
            "Extension catalog is full"
        );
        raw[&key] = value;
        Ok(())
    })
}

pub(super) fn create_plugin(
    config: &Config,
    root: &Path,
    expected: &str,
    name: &str,
) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 80
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            && !protected(name),
        "Use 1–80 letters, numbers, underscores or hyphens; Fastrock names are reserved"
    );
    let path = root.join(name);
    ensure!(!path.exists(), "That plugin directory already exists");
    let key = path.to_str().context("Plugin path is not valid UTF-8")?;
    // Save disabled before exposing a discoverable plugin directory. Failure
    // leaves a disabled entry, never a partly created automatically active hook.
    mutate(config, "plugins", expected, |raw| {
        ensure!(raw.get(key).is_none(), "That plugin is already registered");
        ensure!(
            raw.as_object().unwrap().len() < LIMIT,
            "Plugin catalog is full"
        );
        raw[key] = json!({"enabled":false});
        Ok(())
    })?;
    std::fs::create_dir_all(root)?;
    std::fs::create_dir(&path)?;
    std::fs::create_dir(path.join("hooks"))?;
    crate::rally::store::atomic_write(&path.join("hooks/hooks.json"), b"{\n  \"hooks\": {}\n}\n")?;
    crate::rally::store::atomic_write(
        &path.join("plugin.json"),
        &serde_json::to_vec_pretty(
            &json!({"name":name,"version":"1.0.0","description":"User-configured Goose hooks"}),
        )?,
    )?;
    Ok(())
}
fn hook_path(config: &Config, key: &str) -> Result<PathBuf> {
    let plugins = read(config, "plugins")?;
    ensure!(
        plugins.get(key).is_some(),
        "Reload the plugin catalog before opening hooks"
    );
    let path = PathBuf::from(key);
    ensure!(path.is_absolute(), "Plugin path must be absolute");
    Ok(path.join("hooks/hooks.json"))
}
fn hook_bytes(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    match std::fs::File::open(path) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(256 * 1024 + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 256 * 1024, "Hook file exceeds 256 KiB");
            Ok(bytes)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}
pub(super) fn open_hooks(config: &Config, key: &str) -> Result<Value> {
    let path = hook_path(config, key)?;
    let bytes = hook_bytes(&path)?;
    Ok(
        json!({"key":key,"revision":format!("{:x}",Sha256::digest(&bytes)),"text":if bytes.is_empty(){"{\n  \"hooks\": {}\n}".into()}else{String::from_utf8(bytes)?}}),
    )
}
pub(super) fn validate_hooks(text: &str) -> Result<()> {
    bounded(text, 256 * 1024, "hooks")?;
    let value: Value = serde_json::from_str(text).context("Hooks must be valid JSON")?;
    let hooks = value["hooks"]
        .as_object()
        .context("Expected a hooks object")?;
    for (event, rules) in hooks {
        ensure!(
            matches!(
                event.as_str(),
                "PreToolUse"
                    | "PreToolUseResult"
                    | "PostToolUse"
                    | "PostToolUseFailure"
                    | "SessionStart"
                    | "SessionEnd"
                    | "UserPromptSubmit"
                    | "BeforeReadFile"
                    | "AfterFileEdit"
                    | "BeforeShellExecution"
                    | "AfterShellExecution"
                    | "Stop"
            ),
            "Unknown Goose hook event: {event}"
        );
        let rules = rules
            .as_array()
            .context("Hook event must contain an array of rules")?;
        ensure!(rules.len() <= 128, "Too many hook rules");
        for rule in rules {
            if let Some(matcher) = rule.get("matcher") {
                let matcher = matcher.as_str().context("Matcher must be a string")?;
                regex::Regex::new(matcher).context("Invalid hook matcher")?;
            }
            let actions = rule["hooks"]
                .as_array()
                .context("Rule must contain a hooks array")?;
            ensure!(actions.len() <= 32, "Too many hook actions");
            for action in actions {
                ensure!(
                    action["type"] == "command"
                        && action["command"]
                            .as_str()
                            .is_some_and(|s| !s.trim().is_empty()),
                    "Goose hook action requires type command and a command"
                );
                if let Some(timeout) = action.get("timeout") {
                    ensure!(
                        timeout.as_u64().is_some_and(|n| (1..=3600).contains(&n)),
                        "Hook timeout must be 1–3600 seconds"
                    );
                }
                if let Some(failure) = action.get("on_failure") {
                    ensure!(
                        failure == "allow" || failure == "block",
                        "on_failure must be allow or block"
                    );
                }
            }
        }
    }
    Ok(())
}
pub(super) fn save_hooks(config: &Config, key: &str, expected: &str, text: &str) -> Result<()> {
    ensure!(
        !protected(
            &Path::new(key)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ),
        "Fastrock's bundled workspace hook is managed by Fastrock"
    );
    validate_hooks(text)?;
    let path = hook_path(config, key)?;
    let before = hook_bytes(&path)?;
    ensure!(
        format!("{:x}", Sha256::digest(&before)) == expected,
        "Hooks changed; reload before saving"
    );
    std::fs::create_dir_all(path.parent().unwrap())?;
    if !before.is_empty() {
        let backup = path.with_file_name(format!("hooks.fastrock-backup-{expected}.json"));
        if !backup.exists() {
            crate::rally::store::atomic_write(&backup, &before)?;
        }
    }
    crate::rally::store::atomic_write(&path, text.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v65_capability_mutation_rejects_corrupt_config() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.yaml");
        let config = Config::new(&path, "fixture").unwrap();
        let bytes = b"extensions: [broken: {\n";
        std::fs::write(&path, bytes).unwrap();
        assert!(toggle(&config, "extensions", "developer", "", true).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn v64_new_hook_plugin_starts_disabled_and_preserves_existing_files() {
        let temp = tempfile::tempdir().unwrap();
        let config = Config::new(temp.path().join("config.yaml"), "fixture").unwrap();
        let root = temp.path().join("plugins");
        create_plugin(&config, &root, &revision(&json!({})), "user-hooks").unwrap();
        let key = root.join("user-hooks").to_string_lossy().into_owned();
        let saved = read(&config, "plugins").unwrap();
        assert_eq!(saved[&key]["enabled"], false);
        let bytes = std::fs::read(root.join("user-hooks/plugin.json")).unwrap();
        assert!(create_plugin(&config, &root, &revision(&saved), "user-hooks").is_err());
        assert_eq!(
            std::fs::read(root.join("user-hooks/plugin.json")).unwrap(),
            bytes
        );
        assert!(create_plugin(&config, &root, &revision(&saved), "../escape").is_err());
    }
    #[test]
    fn v64_config_edits_preserve_secrets_unknown_fields_and_reject_stale_forms() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new(tmp.path().join("config.yaml"), "fixture").unwrap();
        let original = json!({"custom":{"type":"stdio","name":"custom","cmd":"fixture","args":[],"enabled":true,"envs":{"TOKEN":"preserve"},"future":{"unknown":true}},"malformed":"preserve"});
        config.set_param("extensions", &original).unwrap();
        config.set_param("unrelated", 42).unwrap();
        let snapshot = read(&config, "extensions").unwrap();
        assert_eq!(snapshot["custom"], original["custom"]);
        assert_eq!(snapshot["malformed"], original["malformed"]);
        let original = snapshot;
        assert_eq!(
            revision(&json!({"a":{"x":1,"y":2},"b":3})),
            revision(&json!({"b":3,"a":{"y":2,"x":1}}))
        );
        toggle(&config, "extensions", "custom", &revision(&original), false).unwrap();
        let changed = read(&config, "extensions").unwrap();
        let mut expected = original.clone();
        expected["custom"]["enabled"] = json!(false);
        assert_eq!(changed, expected);
        assert!(toggle(&config, "extensions", "custom", &revision(&original), true).is_err());
        assert_eq!(read(&config, "extensions").unwrap(), changed);
        assert_eq!(config.get_param::<i32>("unrelated").unwrap(), 42);
        assert!(
            toggle(
                &config,
                "extensions",
                "fastrock_vwork",
                &revision(&changed),
                false
            )
            .is_err()
        );
        let form = json!({"name":"new_mcp","kind":"stdio","address":"fixture","args":"[\"--serve\"]","keys":"EXISTING_TOKEN"});
        add(&config, &revision(&changed), &form).unwrap();
        let added = read(&config, "extensions").unwrap();
        assert_eq!(added["custom"], changed["custom"]);
        assert_eq!(added["new_mcp"]["enabled"], false);
        assert!(added["new_mcp"].get("envs").is_none());
        assert!(add(&config, &revision(&added), &form).is_err());
    }
    #[test]
    fn v64_custom_workspace_aliases_remain_managed() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new(tmp.path().join("config.yaml"), "fixture").unwrap();
        let extension = json!({"type":"stdio","name":"source", "cmd":"C:\\tools\\vwork.exe", "args":["mcp"],"enabled":true});
        assert_eq!(extension_row("source", &extension, true)["protected"], true);
        config
            .set_param("extensions", json!({"source":extension}))
            .unwrap();
        let before = read(&config, "extensions").unwrap();
        assert!(toggle(&config, "extensions", "source", &revision(&before), false).is_err());
        assert_eq!(read(&config, "extensions").unwrap(), before);
        let binding = fastrock_workspaces::vwork::McpBinding {
            name: "source".into(),
            executable: "vwork".into(),
            command: "vwork".into(),
            args: vec![],
            home: tmp.path().into(),
            workspace: None,
            auto_start: true,
            timeout_seconds: 30,
        };
        let mut renamed = binding.clone();
        renamed.name = "My Source!".into();
        assert!(managed_session_name("mysource___list", &[renamed], true));
        let bindings = [binding];
        assert!(managed_session_name("source", &bindings, false));
        assert!(managed_session_name("source__list", &bindings, true));
        assert!(!managed_session_name("source_other__list", &bindings, true));
        assert!(!managed_session_name("source__list", &bindings, false));
    }
    #[test]
    fn v64_extension_and_hook_inputs_are_bounded_and_validated() {
        for form in [
            json!({"name":"fastrock","kind":"stdio","address":"x"}),
            json!({"name":"a","kind":"http","address":"https://user:secret@host"}),
            json!({"name":"a","kind":"http","address":"https://host?token=secret"}),
            json!({"name":"a","kind":"stdio","address":"x","keys":"TOKEN=secret"}),
            json!({"name":"a","kind":"stdio","address":"x","args":"{}"}),
        ] {
            assert!(new_extension(&form).is_err());
        }
        for text in [
            "{}",
            r#"{"hooks":{"Wrong":[]}}"#,
            r#"{"hooks":{"PreToolUse":[{"matcher":"[","hooks":[]}]}}"#,
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"x","on_failure":"deny"}]}]}}"#,
        ] {
            assert!(validate_hooks(text).is_err());
        }
        assert!(validate_hooks(r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"fixture","timeout":5}]}]}}"#).is_ok());
    }
    #[test]
    fn v64_hook_save_preserves_backup_and_rejects_stale_and_managed_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new(tmp.path().join("config.yaml"), "fixture").unwrap();
        let plugin = tmp.path().join("user-hooks");
        let key = plugin.to_str().unwrap();
        config
            .set_param("plugins", json!({key:{"enabled":true}}))
            .unwrap();
        let opened = open_hooks(&config, key).unwrap();
        save_hooks(
            &config,
            key,
            opened["revision"].as_str().unwrap(),
            "{\"hooks\":{}}",
        )
        .unwrap();
        assert!(
            save_hooks(
                &config,
                key,
                opened["revision"].as_str().unwrap(),
                "{\"hooks\":{}}"
            )
            .is_err()
        );
        let opened = open_hooks(&config, key).unwrap();
        save_hooks(
            &config,
            key,
            opened["revision"].as_str().unwrap(),
            "{\"hooks\":{\"Stop\":[]}}",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(plugin.join(format!(
                "hooks/hooks.fastrock-backup-{}.json",
                opened["revision"].as_str().unwrap()
            )))
            .unwrap(),
            "{\"hooks\":{}}"
        );
    }
}
