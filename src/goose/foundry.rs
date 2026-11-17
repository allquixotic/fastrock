use anyhow::{Context, Result, ensure};
use goose_runtime::config::declarative_providers as api;
use std::path::PathBuf;
use uuid::Uuid;

pub(super) const ENGINE: &str = "azure-foundry";
pub(crate) const HELPER: &str = "--fastrock-foundry-credential";
pub(super) const FLASH: &str = "MAI-Code-2-Flash";
const SCOPE: &str = "https://ai.azure.com/.default";

pub(super) fn subscription(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value).context("Enter the Azure subscription ID (a UUID)")
}

pub(super) fn project_route(endpoint: &str) -> Result<String> {
    let invalid = "Enter the HTTPS Foundry project endpoint from the project overview";
    let url = url::Url::parse(endpoint).context(invalid)?;
    let host = url
        .host_str()
        .and_then(|host| host.strip_suffix(".services.ai.azure.com"));
    let path = url.path().trim_end_matches('/');
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && host.is_some_and(azure_name)
            && path.strip_prefix("/api/projects/").is_some_and(azure_name),
        "{invalid}"
    );
    Ok(format!(
        "{}/openai/v1/responses",
        path.trim_start_matches('/')
    ))
}

pub(super) fn azure_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

pub(super) fn profile_args(config: &api::DeclarativeProviderConfig) -> Option<(&str, &str)> {
    let auth = config.auth.as_ref()?;
    let [flag, id, subscription_id] = auth.args.as_slice() else {
        return None;
    };
    (flag == HELPER
        && config.engine == api::ProviderEngine::OpenAI
        && config.api_key_env.is_empty()
        && Uuid::parse_str(id).is_ok()
        && subscription(subscription_id).is_ok()
        && project_route(&config.base_url)
            .is_ok_and(|route| config.base_path.as_ref() == Some(&route)))
    .then_some((id.as_str(), subscription_id.as_str()))
}

pub(super) fn cli_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: &[&str] = if cfg!(windows) {
        &["az.exe", "az.cmd"]
    } else {
        &["az"]
    };
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file())
}

fn cli_args(subscription_id: &str) -> [&str; 11] {
    [
        "account",
        "get-access-token",
        "--subscription",
        subscription_id,
        "--scope",
        SCOPE,
        "--query",
        "accessToken",
        "--output",
        "tsv",
        "--only-show-errors",
    ]
}

fn token(stdout: &[u8]) -> Result<&str> {
    let value = std::str::from_utf8(stdout)
        .context("Azure CLI returned invalid token output")?
        .trim();
    ensure!(
        !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')),
        "Azure CLI returned invalid token output"
    );
    Ok(value)
}

// Goose reads stdout as a bearer credential. Keep CLI diagnostics and credentials
// out of Fastrock errors, and use Azure CLI's own cache for the work sign-in.
pub(crate) fn credential_helper(profile_id: &str, subscription_id: &str) -> Result<()> {
    use std::io::Write;
    ensure!(
        Uuid::parse_str(profile_id).is_ok(),
        "Invalid Foundry profile identity"
    );
    let subscription_id = subscription(subscription_id)?.to_string();
    let program = cli_path().context("Install Azure CLI and restart Fastrock so it can find az")?;
    let mut command = std::process::Command::new(program);
    command
        .args(cli_args(&subscription_id))
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().context("Could not run Azure CLI")?;
    ensure!(
        output.status.success(),
        "Azure CLI could not get a Foundry token. Run az login with your work account and check the profile's subscription ID"
    );
    writeln!(std::io::stdout().lock(), "{}", token(&output.stdout)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v114_foundry_project_route_restricts_credential_destination() {
        let endpoint = "https://work.services.ai.azure.com/api/projects/coding/";
        assert_eq!(
            project_route(endpoint).unwrap(),
            "api/projects/coding/openai/v1/responses"
        );
        for bad in [
            "http://work.services.ai.azure.com/api/projects/coding",
            "https://work.services.ai.azure.com.attacker.test/api/projects/coding",
            "https://work.services.ai.azure.com@elsewhere.test/api/projects/coding",
            "https://work.services.ai.azure.com:8080/api/projects/coding",
            "https://work.services.ai.azure.com/api/projects/coding?secret=1",
            "https://work.services.ai.azure.com/api/projects/coding/openai/v1",
            "https://work.services.ai.azure.com/api/projects/a%2Fb",
        ] {
            assert!(project_route(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn v114_azure_cli_uses_subscription_and_foundry_audience() {
        let id = "12345678-1234-1234-1234-123456789abc";
        assert_eq!(subscription(id).unwrap().to_string(), id);
        assert!(subscription("work").is_err());
        assert_eq!(
            cli_args(id),
            [
                "account",
                "get-access-token",
                "--subscription",
                id,
                "--scope",
                "https://ai.azure.com/.default",
                "--query",
                "accessToken",
                "--output",
                "tsv",
                "--only-show-errors"
            ]
        );
        assert_eq!(
            token(b"fixture.token-value_1\r\n").unwrap(),
            "fixture.token-value_1"
        );
        assert!(token(b"extra line\nfixture.token").is_err());
        assert!(token(b" \r\n").is_err());
    }
}
