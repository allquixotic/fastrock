//! Fastrock-owned secrets. No plaintext fallback and no Goose storage internals.
use anyhow::{Result, ensure};

pub(crate) fn platform_entry(user: &str) -> Result<keyring::Entry> {
    #[cfg(windows)]
    {
        // Keyring 4 changed its generated Windows target. Keep the exact legacy
        // Fastrock target for Rally, provider profiles and webhook secrets.
        keyring::Entry::store_status()
            .as_ref()
            .map_err(|_| anyhow::anyhow!("OS credential vault is unavailable"))?;
        let target = format!("fastrock:{user}");
        Ok(keyring::Entry {
            inner: keyring_core::Entry::new_with_modifiers(
                "fastrock",
                user,
                &std::collections::HashMap::from([("target", target.as_str())]),
            )?,
        })
    }
    #[cfg(not(windows))]
    {
        Ok(keyring::Entry::new("fastrock", user)?)
    }
}

fn entry(kind: &str, id: &str) -> Result<keyring::Entry> {
    ensure!(
        matches!(kind, "webhook" | "provider"),
        "Unknown credential kind"
    );
    let id =
        uuid::Uuid::parse_str(id).map_err(|_| anyhow::anyhow!("Invalid credential identity"))?;
    let user = format!("{kind}/{id}");
    let entry = platform_entry(&user);
    entry.map_err(|_| anyhow::anyhow!("OS credential vault is unavailable"))
}
pub(crate) fn get(kind: &str, id: &str) -> Result<String> {
    entry(kind, id)?
        .get_password()
        .map_err(|_| anyhow::anyhow!("Credential is missing or the OS vault is unavailable"))
}
pub(crate) fn put(kind: &str, id: &str, value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control),
        "Invalid credential value"
    );
    entry(kind, id)?.set_password(value).map_err(|_| {
        anyhow::anyhow!("Cannot save credential in OS vault; no file fallback was used")
    })
}
pub(crate) fn remove(kind: &str, id: &str) -> Result<()> {
    match entry(kind, id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => anyhow::bail!("Cannot remove credential from OS vault"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn v58_vault_identity_cannot_address_legacy_credentials() {
        assert!(super::entry("rally", "fixture").is_err());
        assert!(super::entry("webhook", "../provider").is_err());
        assert!(super::put("webhook", &uuid::Uuid::new_v4().to_string(), "").is_err());
    }
}

/// Called before GUI/runtime initialization by Goose's supported command-auth
/// provider. stdout is its credential pipe; diagnostics never include the key.
pub(crate) fn credential_helper(id: &str) -> Result<()> {
    use std::io::Write;
    let value = get("provider", id)?;
    std::io::stdout().lock().write_all(value.as_bytes())?;
    Ok(())
}
