//! Goose's ordinary read/write APIs can recover from malformed YAML by treating
//! it as empty. Native edits must preserve that evidence instead of replacing it.
use anyhow::{Result, ensure};
use goose_runtime::config::Config;
use std::{
    io::{Read, Write},
    path::Path,
    str::FromStr,
    sync::Mutex,
};

static EDIT: Mutex<()> = Mutex::new(());

/// JSON's arbitrary_precision feature uses a serializer-private number marker.
/// Goose's generic YAML setter does not understand that marker. Reparse JSON
/// text as YAML before passing JSON-valued parameters to the public setter.
pub(super) fn parameter(value: &impl serde::Serialize) -> Result<serde_yaml::Value> {
    Ok(serde_yaml::from_str(&serde_json::to_string(value)?)?)
}

/// Compute changes with Goose's public parameter APIs on a private draft, then
/// apply only the semantic differences to the original syntax tree. Goose's
/// ordinary setters serialize the entire file and discard users' comments.
/// This function is for non-secret parameters only; credential APIs still use
/// the user's actual Config/vault and must never be called on the draft.
pub(super) fn edit<T>(config: &Config, change: impl FnOnce(&Config) -> Result<T>) -> Result<T> {
    let _guard = EDIT
        .lock()
        .map_err(|_| anyhow::anyhow!("Goose settings unavailable"))?;
    validate(config)?;
    let path = config.path();
    let path = Path::new(&path);
    let original = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => anyhow::bail!("Cannot read Goose configuration"),
    };
    let source = original.as_deref().unwrap_or(b"{}\n");
    // A private temporary directory also isolates Goose's intermediate .tmp
    // files. No draft uses, reads or writes the real credential vault.
    let directory = tempfile::tempdir()?;
    let draft_path = directory.path().join("config.yaml");
    std::fs::write(&draft_path, source)?;
    let draft =
        Config::new_with_file_secrets(&draft_path, directory.path().join("unused-secrets"))?;
    // Goose setters migrate legacy provider keys too. Normalize only the draft
    // first, so unrelated edits do not remove commented provider settings.
    let marker = format!("fastrock-draft-{}", uuid::Uuid::new_v4());
    draft.delete(&marker)?;
    let baseline: serde_yaml::Mapping = serde_yaml::from_slice(&std::fs::read(&draft_path)?)?;
    let result = change(&draft)?;
    let desired: serde_yaml::Mapping = serde_yaml::from_slice(&std::fs::read(&draft_path)?)?;
    let mut planned: serde_yaml::Value = serde_yaml::from_slice(source)?;
    apply_delta(
        &mut planned,
        &baseline.clone().into(),
        &desired.clone().into(),
    );
    let rendered = merge_format(source, serde_yaml::to_string(&planned)?.as_bytes())?;
    if rendered == source {
        return Ok(result);
    }
    // Verify effective values using Goose itself before replacing anything.
    std::fs::write(&draft_path, &rendered)?;
    draft.delete(&marker)?;
    let effective: serde_yaml::Mapping = serde_yaml::from_slice(&std::fs::read(&draft_path)?)?;
    ensure!(
        effective == desired,
        "Cannot preserve Goose configuration semantics; original file kept"
    );
    ensure!(
        std::fs::read(path).ok() == original,
        "Goose settings changed during editing; reload before saving"
    );
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid Goose configuration path"))?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    // NamedTempFile starts private. Preserve more restrictive existing modes.
    if let Ok(metadata) = std::fs::metadata(path) {
        file.as_file().set_permissions(metadata.permissions())?;
    }
    file.write_all(&rendered)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|_| anyhow::anyhow!("Cannot save Goose configuration"))?;
    // Public invalidation also advances Goose's config generation. Ordinary
    // parameter reads already reload the file; provider caches observe this.
    config.invalidate_secrets_cache();
    Ok(result)
}

/// Preserve original fields that Goose's typed normalization does not know
/// about, including fields inside an otherwise edited plugin/provider mapping.
fn apply_delta(
    original: &mut serde_yaml::Value,
    before: &serde_yaml::Value,
    after: &serde_yaml::Value,
) {
    if before == after {
        return;
    }
    if let (Some(target), Some(before), Some(after)) = (
        original.as_mapping_mut(),
        before.as_mapping(),
        after.as_mapping(),
    ) {
        for key in before.keys().filter(|key| !after.contains_key(*key)) {
            target.remove(key);
        }
        for (key, value) in after {
            if let Some(previous) = before.get(key) {
                if previous != value {
                    apply_delta(
                        target.entry(key.clone()).or_insert(serde_yaml::Value::Null),
                        previous,
                        value,
                    );
                }
            } else {
                target.insert(key.clone(), value.clone());
            }
        }
    } else {
        *original = after.clone();
    }
}

fn merge_format(original: &[u8], desired: &[u8]) -> Result<Vec<u8>> {
    let parse = |bytes: &[u8]| {
        serde_yaml::from_slice::<serde_yaml::Mapping>(bytes)
            .map_err(|_| anyhow::anyhow!("Cannot safely edit Goose configuration"))
    };
    let before = parse(original)?;
    let after = parse(desired)?;
    if before == after {
        return Ok(original.to_vec());
    }
    let source = std::str::from_utf8(original)
        .map_err(|_| anyhow::anyhow!("Goose configuration must be UTF-8"))?;
    let file = yaml_edit::YamlFile::from_str(source)
        .map_err(|_| anyhow::anyhow!("Cannot preserve Goose configuration formatting"))?;
    let document = file
        .document()
        .ok_or_else(|| anyhow::anyhow!("Goose configuration must be a mapping"))?;
    let mapping = document
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("Goose configuration must be a mapping"))?;
    patch_mapping(&mapping, &before, &after)?;
    let rendered = file.to_string().into_bytes();
    ensure!(
        parse(&rendered)? == after,
        "Cannot safely preserve this YAML structure; original configuration was kept"
    );
    Ok(rendered)
}

fn patch_mapping(
    target: &yaml_edit::Mapping,
    before: &serde_yaml::Mapping,
    after: &serde_yaml::Mapping,
) -> Result<()> {
    for key in before.keys().filter(|key| !after.contains_key(*key)) {
        let key = key
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Unsupported Goose setting key"))?;
        target.remove(key);
    }
    for (key, value) in after {
        if before.get(key) == Some(value) {
            continue;
        }
        let name = key
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Unsupported Goose setting key"))?;
        if let (Some(old), Some(new), Some(node)) = (
            before.get(key).and_then(|v| v.as_mapping()),
            value.as_mapping(),
            target.get(name),
        ) {
            if let Some(mapping) = node.as_mapping() {
                patch_mapping(mapping, old, new)?;
                continue;
            }
        }
        // Goose parameters are JSON-compatible. Parse new values in flow style
        // so their nodes are valid in either block or flow destinations. The
        // library's data-map builder emits block syntax even in flow context.
        let replacement = yaml_edit::YamlFile::from_str(&format!(
            "{{\"value\":{}}}",
            serde_json::to_string(value)?
        ))?;
        let node = replacement
            .document()
            .and_then(|doc| doc.as_mapping())
            .and_then(|map| map.get("value"))
            .ok_or_else(|| anyhow::anyhow!("Cannot encode Goose setting"))?;
        target.set(name, node);
    }
    Ok(())
}

pub(super) fn validate(config: &Config) -> Result<()> {
    let file = match std::fs::File::open(config.path()) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => anyhow::bail!("Cannot read Goose configuration; no settings were saved"),
    };
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 8 * 1024 * 1024,
        "Goose configuration exceeds 8 MiB; no settings were saved"
    );
    // Do not include parser diagnostics: a bad line may contain a credential.
    ensure!(
        serde_yaml::from_slice::<serde_yaml::Mapping>(&bytes).is_ok(),
        "Goose configuration is malformed; repair config.yaml before saving settings. Its original contents are preserved."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v83_goose_edits_preserve_comments_scalars_and_unknown_settings() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.yaml");
        let original = "# providers: keep this explanation\nGOOSE_PROVIDER: 'custom_bedrock_claude' # working route\n# vwork stays in process\nextensions:\n  vwork:\n    enabled: true # keep this switch\n    args: [mcp] # unbound is intentional\n# Exa is intentionally off\nplugins:\n  'C:/Tools/old':\n    enabled: false # leave disabled\n    future_setting: 17\nnotes: |\n  literal: text # not a comment\n# final comment\n";
        std::fs::write(&path, original).unwrap();
        let config = Config::new_with_file_secrets(&path, temp.path().join("secrets")).unwrap();
        let generation = config.generation();
        edit(&config, |draft| {
            let mut plugins: serde_json::Value = draft.get_param("plugins")?;
            plugins["C:/Tools/new"] = serde_json::json!({"enabled":true});
            draft.set_param("plugins", parameter(&plugins)?)?;
            draft.set_param("FASTROCK_CHEAP_PROFILE", serde_json::json!({"name":"Cheap","provider":"aws_bedrock","model":"openai.gpt-6-luna"}))?;
            Ok(())
        }).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        for retained in [
            "# providers: keep this explanation",
            "'custom_bedrock_claude' # working route",
            "# vwork stays in process",
            "true # keep this switch",
            "[mcp] # unbound is intentional",
            "# Exa is intentionally off",
            "false # leave disabled",
            "future_setting: 17",
            "literal: text # not a comment",
            "# final comment",
        ] {
            assert!(
                after.contains(retained),
                "Lost {retained}; fixture:\n{after}"
            );
        }
        assert!(config.generation() > generation);
        let bytes = std::fs::read(&path).unwrap();
        edit(&config, |_| Ok(())).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(
            edit(&config, |draft| -> Result<()> {
                draft.set_param("SHOULD_NOT_COMMIT", true)?;
                anyhow::bail!("simulated later validation failure")
            })
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(!temp.path().join("secrets").exists());
        edit(&config, |draft| {
            let mut raw: serde_json::Value = draft.get_param("plugins")?;
            raw["C:/Tools/old"]["enabled"] = serde_json::json!(true);
            Ok(draft.set_param("plugins", parameter(&raw)?)?)
        })
        .unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("true # leave disabled")
        );
    }

    #[test]
    fn v83_flow_mapping_and_anchors_preserve_semantics_or_fail_without_writing() {
        for original in [
            "{GOOSE_PROVIDER: aws_bedrock, extensions: {}}\n",
            "# anchor\nbase: &base {enabled: true}\ncopy: *base\nplugins: {}\n",
        ] {
            let after = merge_format(original.as_bytes(), &{
                let mut value: serde_yaml::Mapping = serde_yaml::from_str(original).unwrap();
                value.insert(
                    "plugins".into(),
                    serde_yaml::to_value(serde_json::json!({"new":{"enabled":true}})).unwrap(),
                );
                serde_yaml::to_string(&value).unwrap().into_bytes()
            })
            .unwrap();
            if original.contains("&base") {
                assert!(String::from_utf8(after).unwrap().contains("copy: *base"));
            }
        }
    }
    #[test]
    fn v65_malformed_goose_config_is_preserved_before_capability_edits() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.yaml");
        let config = Config::new(&path, "fixture").unwrap();
        for bytes in [
            b"extensions: [broken: {\n".as_slice(),
            b"- unexpected sequence\n",
            b"provider: secret\nprovider: another\n",
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(validate(&config).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        std::fs::write(&path, "extensions: {}\ncustom: preserved\n").unwrap();
        validate(&config).unwrap();
    }
}
