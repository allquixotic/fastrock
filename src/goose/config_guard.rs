//! Goose's ordinary read/write APIs can recover from malformed YAML by treating
//! it as empty. Native edits must preserve that evidence instead of replacing it.
use anyhow::{Result, ensure};
use goose_runtime::config::Config;
use std::io::Read;

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
