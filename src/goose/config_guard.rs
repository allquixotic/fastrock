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
    stage("edit", edit_inner(config, change))
}

fn edit_inner<T>(config: &Config, change: impl FnOnce(&Config) -> Result<T>) -> Result<T> {
    let _guard = EDIT
        .lock()
        .map_err(|_| anyhow::anyhow!("Provider settings unavailable"))?;
    validate(config)?;
    let path = config.path();
    let path = Path::new(&path);
    let original = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => anyhow::bail!("Cannot read provider configuration"),
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
    stage(
        "draft normalization",
        draft.delete(&marker).map_err(Into::into),
    )?;
    let baseline: serde_yaml::Mapping = serde_yaml::from_slice(&std::fs::read(&draft_path)?)?;
    let result = stage("parameter update", change(&draft))?;
    let desired: serde_yaml::Mapping = serde_yaml::from_slice(&std::fs::read(&draft_path)?)?;
    let mut planned: serde_yaml::Value = serde_yaml::from_slice(source)?;
    apply_delta(
        &mut planned,
        &baseline.clone().into(),
        &desired.clone().into(),
    );
    let rendered = stage(
        "format preservation",
        merge_format(source, serde_yaml::to_string(&planned)?.as_bytes()),
    )?;
    if rendered == source {
        return Ok(result);
    }
    // Verify effective values using Goose itself before replacing anything.
    std::fs::write(&draft_path, &rendered)?;
    stage(
        "draft verification",
        draft.delete(&marker).map_err(Into::into),
    )?;
    let effective: serde_yaml::Mapping = serde_yaml::from_slice(&std::fs::read(&draft_path)?)?;
    ensure!(
        effective == desired,
        "Cannot preserve provider configuration semantics; original file kept"
    );
    ensure!(
        read_original(path)? == original,
        "Provider settings changed during editing; reload before saving"
    );
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid provider configuration path"))?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    // NamedTempFile starts private. Preserve more restrictive existing modes.
    if let Ok(metadata) = std::fs::metadata(path) {
        file.as_file().set_permissions(metadata.permissions())?;
    }
    file.write_all(&rendered)?;
    file.as_file().sync_all()?;
    stage(
        "atomic replacement",
        persist_unchanged(file, path, &original),
    )?;
    // Public invalidation also advances Goose's config generation. Ordinary
    // parameter reads already reload the file; provider caches observe this.
    config.invalidate_secrets_cache();
    Ok(result)
}

/// Public Goose errors may contain configuration values. Report the failed
/// stage and numeric OS error only; never forward parser lines or credentials.
#[derive(Debug)]
struct EditFailure {
    stage: &'static str,
    code: Option<i32>,
}

impl std::fmt::Display for EditFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Provider configuration {} failed", self.stage)?;
        if let Some(code) = self.code {
            write!(formatter, " (OS error {code})")?;
        }
        write!(formatter, "; original file kept. Reload before retrying")
    }
}

impl std::error::Error for EditFailure {}

fn stage<T>(name: &'static str, result: Result<T>) -> Result<T> {
    result.map_err(|error| {
        if error.is::<EditFailure>() {
            return error;
        }
        let code = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<std::io::Error>())
            .and_then(std::io::Error::raw_os_error);
        EditFailure { stage: name, code }.into()
    })
}

fn read_original(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => anyhow::bail!("Cannot recheck provider configuration; original file kept"),
    }
}

fn persist_unchanged(
    mut file: tempfile::NamedTempFile,
    path: &Path,
    original: &Option<Vec<u8>>,
) -> Result<()> {
    // Windows scanners/readers can briefly deny replacement even though our
    // private draft is complete. Retry only this atomic filesystem operation,
    // never the public Goose mutation callback or credential/tool operations.
    for attempt in 0..=6 {
        ensure!(
            &read_original(path)? == original,
            "Provider settings changed during editing; reload before saving"
        );
        match file.persist(path) {
            Ok(_) => return Ok(()),
            Err(error)
                if cfg!(windows)
                    && attempt < 6
                    && matches!(error.error.raw_os_error(), Some(5 | 32 | 33)) =>
            {
                file = error.file;
                std::thread::sleep(std::time::Duration::from_millis(
                    (50_u64 << attempt).min(200),
                ));
            }
            Err(error) => return Err(error.error.into()),
        }
    }
    unreachable!()
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
            .map_err(|_| anyhow::anyhow!("Cannot safely edit provider configuration"))
    };
    let before = parse(original)?;
    let after = parse(desired)?;
    if before == after {
        return Ok(original.to_vec());
    }
    let source = std::str::from_utf8(original)
        .map_err(|_| anyhow::anyhow!("Provider configuration must be UTF-8"))?;
    let file = yaml_edit::YamlFile::from_str(source)
        .map_err(|_| anyhow::anyhow!("Cannot preserve provider configuration formatting"))?;
    let document = file
        .document()
        .ok_or_else(|| anyhow::anyhow!("Provider configuration must be a mapping"))?;
    let mapping = document
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("Provider configuration must be a mapping"))?;
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
            .ok_or_else(|| anyhow::anyhow!("Unsupported setting key"))?;
        target.remove(key);
    }
    for (key, value) in after {
        if before.get(key) == Some(value) {
            continue;
        }
        let name = key
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Unsupported setting key"))?;
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
            .ok_or_else(|| anyhow::anyhow!("Cannot encode setting"))?;
        target.set(name, node);
    }
    Ok(())
}

pub(super) fn validate(config: &Config) -> Result<()> {
    let file = match std::fs::File::open(config.path()) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => anyhow::bail!("Cannot read provider configuration; no settings were saved"),
    };
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 8 * 1024 * 1024,
        "Provider configuration exceeds 8 MiB; no settings were saved"
    );
    // Do not include parser diagnostics: a bad line may contain a credential.
    ensure!(
        serde_yaml::from_slice::<serde_yaml::Mapping>(&bytes).is_ok(),
        "Provider configuration is malformed; repair config.yaml before saving settings. Its original contents are preserved."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v97_config_edit_errors_expose_stage_without_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.yaml");
        let original = b"GOOSE_MODEL: preserved\n";
        std::fs::write(&path, original).unwrap();
        let config =
            Config::new_with_file_secrets(&path, directory.path().join("secrets")).unwrap();
        let error = edit(&config, |draft| -> Result<()> {
            draft.set_param("intermediate", true)?;
            anyhow::bail!("parser line containing fixture-secret-value")
        })
        .unwrap_err();
        assert!(error.to_string().contains("parameter update"));
        assert!(!format!("{error:#}").contains("fixture-secret-value"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let error = stage::<()>(
            "draft normalization",
            Err(std::io::Error::from_raw_os_error(32).into()),
        )
        .unwrap_err();
        assert!(error.to_string().contains("OS error 32"));
    }

    #[cfg(windows)]
    fn locked_target() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        tempfile::NamedTempFile,
        std::fs::File,
    ) {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.yaml");
        std::fs::write(&path, b"original").unwrap();
        // Permit reads/writes but deny DELETE sharing, as a Windows scanner can.
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .open(&path)
            .unwrap();
        let mut replacement = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        replacement.write_all(b"replacement").unwrap();
        replacement.as_file().sync_all().unwrap();
        (directory, path, replacement, reader)
    }

    #[cfg(windows)]
    #[test]
    fn v97_windows_temporary_reader_allows_bounded_atomic_save() {
        let (_directory, path, replacement, reader) = locked_target();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(reader);
        });
        let result = persist_unchanged(replacement, &path, &Some(b"original".to_vec()));
        release.join().unwrap();
        result.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"replacement");
    }

    #[cfg(windows)]
    #[test]
    fn v97_windows_persistent_reader_retains_original() {
        let (_directory, path, replacement, _reader) = locked_target();
        assert!(persist_unchanged(replacement, &path, &Some(b"original".to_vec())).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"original");
    }

    #[cfg(windows)]
    #[test]
    fn v97_windows_retry_preserves_concurrent_external_edit() {
        let (_directory, path, replacement, reader) = locked_target();
        let changed_path = path.clone();
        let external = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            std::fs::write(changed_path, b"external edit").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(reader);
        });
        let result = persist_unchanged(replacement, &path, &Some(b"original".to_vec()));
        external.join().unwrap();
        assert!(result.unwrap_err().to_string().contains("changed"));
        assert_eq!(std::fs::read(path).unwrap(), b"external edit");
    }

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
