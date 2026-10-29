//! Embedded Goose is the only runtime. Legacy preferences migrate in place.

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ConnectionTarget {
    Goose,
}

impl ConnectionTarget {
    pub(crate) fn label(&self) -> String {
        "Embedded Goose".into()
    }
    pub(crate) fn is_embedded(&self) -> bool {
        true
    }
    pub(crate) fn uses_remote_workspace(&self) -> bool {
        false
    }
}

pub(crate) fn parse_connection(
    address: &str,
    codex_home: Option<&std::path::Path>,
    auth_token: Option<String>,
) -> anyhow::Result<ConnectionTarget> {
    let _ = (codex_home, auth_token);
    let address = address.trim();
    if address.is_empty()
        || ["goose", "acp", "embedded", "stdio", "codex"]
            .iter()
            .any(|alias| address.eq_ignore_ascii_case(alias))
    {
        return Ok(ConnectionTarget::Goose);
    }
    anyhow::bail!(
        "Fastrock uses embedded Goose; external agent connections are no longer supported"
    )
}

/// Where the connection settings come from: `--remote` /
/// `--remote-auth-token-env` win over the address saved in `gui.json`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ConnectionChoice {
    pub(crate) cli_address: Option<String>,
    pub(crate) cli_token_env: Option<String>,
    pub(crate) saved_address: String,
    pub(crate) saved_token_env: Option<String>,
}

impl ConnectionChoice {
    /// Whether the address comes from `gui.json` rather than the command line.
    pub(crate) fn uses_saved_address(&self) -> bool {
        self.cli_address.is_none()
    }

    /// Resolves the target, reading the token variable through `env` only
    /// for WebSocket endpoints. A token variable saved with a remote address
    /// is ignored when `--remote` names another server.
    pub(crate) fn resolve(
        &self,
        codex_home: Option<&std::path::Path>,
        env: impl Fn(&str) -> Option<String>,
    ) -> anyhow::Result<ConnectionTarget> {
        let _ = env;
        // Preferences from 0.3.x migrate without touching either agent's files.
        match &self.cli_address {
            None => Ok(ConnectionTarget::Goose),
            Some(address) => parse_connection(address, codex_home, None),
        }
    }

    /// Status-overlay text for a connection that cannot be resolved.
    pub(crate) fn error_message(&self, err: &anyhow::Error) -> String {
        let source = if self.uses_saved_address() {
            "The app-server connection saved under Settings › Connection"
        } else {
            "The --remote connection"
        };
        format!(
            "{source} cannot be used: {err:#}. Change it under Settings › Connection, or select embedded Goose."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v38_legacy_preferences_use_embedded_goose() {
        for address in [
            "",
            "goose",
            "codex",
            "embedded",
            "ws://127.0.0.1:1234/",
            "unix://",
        ] {
            let choice = ConnectionChoice {
                saved_address: address.into(),
                ..Default::default()
            };
            assert_eq!(
                choice
                    .resolve(None, |_| panic!("Must not read old credentials"))
                    .unwrap(),
                ConnectionTarget::Goose
            );
        }
    }
    #[test]
    fn v38_explicit_external_backend_is_rejected() {
        assert!(parse_connection("ws://127.0.0.1:1234/", None, None).is_err());
        assert_eq!(
            parse_connection("goose", None, None).unwrap(),
            ConnectionTarget::Goose
        );
    }
}
