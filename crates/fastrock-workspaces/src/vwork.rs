//! Existing vwork configuration and file operations, backed by the embedded owner.
use super::*;
use base64::Engine;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installation {
    pub id: String,
    pub label: String,
    pub home: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DiscoveryPaths {
    pub user_home: PathBuf,
    pub configs: Vec<PathBuf>,
    pub search: Vec<PathBuf>,
    pub default_vwork_home: PathBuf,
}
impl DiscoveryPaths {
    pub fn current() -> Self {
        let user_home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .unwrap_or_default();
        let mut configs = Vec::new();
        if let Some(path) = std::env::var_os("FASTROCK_GOOSE_CONFIG")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            configs.push(path);
        } else if let Some(root) = std::env::var_os("GOOSE_PATH_ROOT")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            configs.push(root.join("config/config.yaml"));
        } else {
            #[cfg(windows)]
            if let Some(appdata) = std::env::var_os("APPDATA") {
                configs.push(PathBuf::from(appdata).join("Block/goose/config/config.yaml"));
            }
            #[cfg(target_os = "macos")]
            configs.push(user_home.join("Library/Application Support/Block.goose/config.yaml"));
            #[cfg(not(windows))]
            {
                let xdg = std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .unwrap_or_else(|| user_home.join(".config"));
                configs.push(xdg.join("goose/config.yaml"));
            }
        }
        let default_vwork_home = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| user_home.join("AppData/Local"))
        } else if cfg!(target_os = "macos") {
            user_home.join("Library/Application Support")
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| user_home.join(".local/share"))
        }
        .join("vwork");
        let mut search: Vec<_> = std::env::var_os("GOOSE_SEARCH_PATHS")
            .map(|v| std::env::split_paths(&v).collect())
            .unwrap_or_default();
        if let Some(path) = std::env::var_os("PATH") {
            search.extend(std::env::split_paths(&path));
        }
        Self {
            user_home,
            configs,
            search,
            default_vwork_home,
        }
    }
}

#[derive(Deserialize, Default)]
struct GooseConfig {
    #[serde(default)]
    extensions: BTreeMap<String, Extension>,
}
#[derive(Deserialize, Default)]
struct Extension {
    #[serde(default)]
    enabled: bool,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    cmd: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    envs: Value,
    #[serde(default)]
    env_keys: Vec<String>,
    #[serde(default)]
    available_tools: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
}

/// A vwork MCP binding mapped onto the embedded database owner.
/// Credentials and arbitrary extension environment are deliberately not copied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpBinding {
    pub name: String,
    pub executable: PathBuf,
    pub command: String,
    pub args: Vec<String>,
    pub home: PathBuf,
    pub workspace: Option<String>,
    pub auto_start: bool,
    pub timeout_seconds: u64,
}

impl McpBinding {
    pub fn installation(&self) -> Installation {
        Installation {
            id: self.home.to_string_lossy().into(),
            label: self.name.clone(),
            home: self.home.clone(),
        }
    }

    pub fn matches_stdio(&self, server: &Value) -> bool {
        server["name"] == self.name
            && server["command"] == self.command
            && server["args"] == json!(self.args)
            && server["env"].as_array().is_none_or(|env| env.is_empty())
    }
}

/// Reject unsupported vwork settings rather than silently launching a daemon.
pub async fn discover_mcp(paths: &DiscoveryPaths) -> Result<Vec<McpBinding>, WorkspaceError> {
    let mut extensions = BTreeMap::new();
    for path in &paths.configs {
        if !tokio::fs::try_exists(path)
            .await
            .map_err(|_| WorkspaceError::Transport)?
        {
            continue;
        }
        let bytes = limited_file(path, 1024 * 1024).await?;
        let config: GooseConfig =
            serde_yaml::from_slice(&bytes).map_err(|_| WorkspaceError::Protocol)?;
        extensions.extend(config.extensions);
    }
    configured_bindings(paths, extensions)
}

/// Public Goose configuration/plugin snapshots use the same checked migration.
/// The caller must retain disabled entries so explicit disabling is respected.
pub fn bindings_from_extensions(
    paths: &DiscoveryPaths,
    extensions: Value,
) -> Result<Vec<McpBinding>, WorkspaceError> {
    let extensions = serde_json::from_value(extensions).map_err(|_| WorkspaceError::Protocol)?;
    configured_bindings(paths, extensions)
}

fn configured_bindings(
    paths: &DiscoveryPaths,
    extensions: BTreeMap<String, Extension>,
) -> Result<Vec<McpBinding>, WorkspaceError> {
    let mut bindings = Vec::new();
    let mut configured_vwork = false;
    for (key, extension) in extensions {
        if extension.kind != "stdio" {
            continue;
        }
        let name = if extension.name.is_empty() {
            key
        } else {
            extension.name
        };
        let command = expand(&extension.cmd, &paths.user_home);
        let Some(binary) = command.file_name().and_then(|p| p.to_str()) else {
            continue;
        };
        if !binary.eq_ignore_ascii_case("vwork") && !binary.eq_ignore_ascii_case("vwork.exe") {
            continue;
        }
        configured_vwork = true;
        if !extension.enabled {
            continue;
        }
        let unsupported = || {
            WorkspaceError::Message(format!(
                "vwork extension {name:?} cannot be embedded: use mcp with an absolute --home, optional --workspace, and no custom environment, cwd or tool filter"
            ))
        };
        if name == "fastrock"
            || !extension.env_keys.is_empty()
            || !extension.available_tools.is_empty()
            || extension.cwd.is_some()
            || !(extension.envs.is_null()
                || extension.envs.as_object().is_some_and(|v| v.is_empty()))
        {
            return Err(unsupported());
        }
        // The executable path is legacy metadata, never launched. Existing
        // configuration continues to work after uninstalling standalone vwork.
        let executable = command.clone();
        let mut home = paths.default_vwork_home.clone();
        let mut workspace = None;
        let mut auto_start = true;
        let mut mcp = false;
        let mut valid = true;
        let mut args = extension.args.iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "mcp" if !mcp => mcp = true,
                "--no-auto-start" => auto_start = false,
                "--home" => match args.next() {
                    Some(v) => home = expand(v, &paths.user_home),
                    None => valid = false,
                },
                "--workspace" => match args.next() {
                    Some(v) => workspace = Some(v.clone()),
                    None => valid = false,
                },
                _ if arg.starts_with("--home=") => home = expand(&arg[7..], &paths.user_home),
                _ if arg.starts_with("--workspace=") => workspace = Some(arg[12..].into()),
                _ => valid = false,
            }
        }
        if !valid || !mcp || !home.is_absolute() {
            return Err(unsupported());
        }
        {
            bindings.push(McpBinding {
                name,
                executable,
                command: extension.cmd,
                args: extension.args,
                home,
                workspace,
                auto_start,
                timeout_seconds: extension.timeout.unwrap_or(300).clamp(1, 86400),
            });
        }
    }
    if !configured_vwork {
        bindings.push(McpBinding::default_for(paths));
    }
    Ok(bindings)
}

impl McpBinding {
    pub fn default_for(paths: &DiscoveryPaths) -> Self {
        Self {
            name: "vwork".into(),
            executable: PathBuf::new(),
            command: String::new(),
            args: Vec::new(),
            home: paths.default_vwork_home.clone(),
            workspace: None,
            auto_start: false,
            timeout_seconds: 900,
        }
    }
}

fn expand(path: &str, user_home: &Path) -> PathBuf {
    if path == "~" {
        user_home.to_owned()
    } else if let Some(tail) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        user_home.join(tail)
    } else {
        PathBuf::from(path)
    }
}
/// Read existing vwork configuration without requiring an installed executable.
pub async fn discover(paths: &DiscoveryPaths) -> Result<Vec<Installation>, WorkspaceError> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for config in &paths.configs {
        let bytes = match limited_file(config, 1024 * 1024).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Ok(config): Result<GooseConfig, _> = serde_yaml::from_slice(&bytes) else {
            continue;
        };
        for (name, extension) in config.extensions {
            if !extension.enabled
                || extension.kind != "stdio"
                || !extension.args.iter().any(|a| a == "mcp")
            {
                continue;
            }
            let command = expand(&extension.cmd, &paths.user_home);
            let Some(binary) = command.file_name().and_then(|p| p.to_str()) else {
                continue;
            };
            if !binary.eq_ignore_ascii_case("vwork") && !binary.eq_ignore_ascii_case("vwork.exe") {
                continue;
            }
            let mut home = paths.default_vwork_home.clone();
            for (i, arg) in extension.args.iter().enumerate() {
                if arg == "--home" {
                    if let Some(value) = extension.args.get(i + 1) {
                        home = expand(value, &paths.user_home);
                    } else {
                        home = PathBuf::new();
                    }
                } else if let Some(value) = arg.strip_prefix("--home=") {
                    home = expand(value, &paths.user_home);
                }
            }
            if !home.is_absolute() {
                continue;
            }
            let home = tokio::fs::canonicalize(&home).await.unwrap_or(home);
            if !seen.insert(home.clone()) {
                continue;
            }
            // Identifier is configuration-local, never a token or connection URL.
            result.push(Installation {
                id: home.to_string_lossy().into(),
                label: format!("vwork · {name}"),
                home,
            });
        }
    }
    if result.is_empty() {
        result.push(McpBinding::default_for(paths).installation());
    }
    Ok(result)
}

async fn limited_file(path: &Path, limit: usize) -> Result<Vec<u8>, WorkspaceError> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| WorkspaceError::Transport)?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| WorkspaceError::Transport)?;
    if bytes.len() > limit {
        return Err(WorkspaceError::Protocol);
    }
    Ok(bytes)
}

pub struct VworkProvider {
    embedded: crate::embedded::Connection,
    store_home: PathBuf,
    #[cfg(feature = "http-fixtures")]
    fixture: bool,
    #[cfg(feature = "http-fixtures")]
    home: PathBuf,
    #[cfg(feature = "http-fixtures")]
    client: reqwest::Client,
}
impl VworkProvider {
    pub fn new(installation: &Installation) -> Result<Self, WorkspaceError> {
        #[cfg(feature = "http-fixtures")]
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| WorkspaceError::Transport)?;
        Ok(Self {
            embedded: crate::embedded::Connection::new(installation.home.clone()),
            store_home: installation.home.clone(),
            #[cfg(feature = "http-fixtures")]
            fixture: false,
            #[cfg(feature = "http-fixtures")]
            home: installation.home.clone(),
            #[cfg(feature = "http-fixtures")]
            client,
        })
    }

    /// Historical wire fixtures only. Release builds have no daemon transport.
    #[cfg(feature = "http-fixtures")]
    pub fn fixture(installation: &Installation) -> Result<Self, WorkspaceError> {
        let mut provider = Self::new(installation)?;
        provider.fixture = true;
        Ok(provider)
    }

    async fn call(&self, op: &str, arguments: Value) -> Result<Value, WorkspaceError> {
        self.call_with_timeout(
            op,
            arguments,
            if op == "database_compact" {
                Duration::from_secs(3600)
            } else {
                Duration::from_secs(20)
            },
        )
        .await
    }

    pub async fn shared_mcp_supported(&self) -> Result<bool, WorkspaceError> {
        Ok(self
            .call_with_timeout("health", json!({}), Duration::from_secs(3))
            .await?["capabilities"]["shared_mcp"]
            == true)
    }

    pub async fn mcp_request(
        &self,
        workspace: Option<&str>,
        request: Value,
        timeout: Duration,
    ) -> Result<Value, WorkspaceError> {
        self.call_with_timeout(
            "mcp",
            json!({"workspace":workspace,"request":request}),
            timeout,
        )
        .await
    }

    async fn call_with_timeout(
        &self,
        op: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<Value, WorkspaceError> {
        #[cfg(feature = "http-fixtures")]
        if self.fixture {
            return self.fixture_call(op, arguments, timeout).await;
        }
        // Timeouts do not imply rollback. The owner retains its concurrency
        // permit until a started mutation finishes, and never retries it.
        tokio::time::timeout(timeout, self.embedded.call(op.into(), arguments))
            .await
            .map_err(|_| {
                WorkspaceError::Message(
                    "Workspace operation is still running; check its state before retrying".into(),
                )
            })?
            .map_err(|e| WorkspaceError::Message(format!("{e:#}")))
    }

    #[cfg(feature = "http-fixtures")]
    async fn fixture_call(
        &self,
        op: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<Value, WorkspaceError> {
        #[derive(Deserialize)]
        struct Connection {
            url: String,
            token: String,
        }
        let connection: Connection =
            serde_json::from_slice(&limited_file(&self.home.join("connection.json"), 65536).await?)
                .map_err(|_| WorkspaceError::Protocol)?;
        validate_loopback(&connection.url)?;
        let mut response = self
            .client
            .post(&connection.url)
            .timeout(timeout)
            .bearer_auth(&connection.token)
            .json(&json!({"operation":op,"arguments":arguments}))
            .send()
            .await
            .map_err(|_| WorkspaceError::Transport)?;
        if !response.status().is_success() {
            return Err(WorkspaceError::Transport);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| WorkspaceError::Transport)?
        {
            if bytes.len() + chunk.len() > (if op == "mcp" { 32 } else { 4 }) * 1024 * 1024 {
                return Err(WorkspaceError::Protocol);
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| WorkspaceError::Protocol)?;
        if let Some(error) = value.get("error") {
            return Err(WorkspaceError::Message(
                error
                    .as_str()
                    .unwrap_or("Workspace operation failed")
                    .chars()
                    .take(2000)
                    .collect(),
            ));
        }
        value.get("result").cloned().ok_or(WorkspaceError::Protocol)
    }
    async fn run(&self, request: Request) -> Result<Reply, WorkspaceError> {
        match request {
            Request::OpenConversation { workspace } => crate::session::Selection {
                home: self.store_home.clone(),
                workspace,
            }
            .resolve()
            .await
            .map(Reply::Conversation)
            .map_err(|error| WorkspaceError::Message(format!("{error:#}"))),
            Request::Compact => {
                let value = self.call("database_compact", json!({})).await?;
                let reason = string(&value, "reason")?;
                let message = match reason.as_str() {
                    "busy" => {
                        "A vwork operation is active. Try compacting again after it finishes."
                            .to_owned()
                    }
                    "below_threshold" => format!(
                        "No compaction needed. Estimated unused space is below the configured threshold ({} GB).",
                        value["threshold_bytes"].as_u64().unwrap_or(1_000_000_000) as f64
                            / 1_000_000_000.0
                    ),
                    "compacted" | "already_compact" => format!(
                        "Compaction finished. Reclaimed {:.2} GB; database is now {:.2} GB.",
                        value["reclaimed_bytes"]
                            .as_u64()
                            .ok_or(WorkspaceError::Protocol)? as f64
                            / 1_000_000_000.0,
                        value["after_bytes"]
                            .as_u64()
                            .ok_or(WorkspaceError::Protocol)? as f64
                            / 1_000_000_000.0
                    ),
                    _ => return Err(WorkspaceError::Protocol),
                };
                Ok(Reply::Report(message))
            }
            Request::Catalog => {
                let repos = self.call("repo_list", json!({})).await?;
                let workspaces = self.call("workspace_list", json!({})).await?;
                let repositories = repos
                    .as_array()
                    .ok_or(WorkspaceError::Protocol)?
                    .iter()
                    .map(|r| {
                        Ok(Repository {
                            id: string(r, "id")?,
                            label: string(r, "id")?,
                        })
                    })
                    .collect::<Result<_, WorkspaceError>>()?;
                let workspaces = workspaces
                    .as_array()
                    .ok_or(WorkspaceError::Protocol)?
                    .iter()
                    .map(workspace)
                    .collect::<Result<_, _>>()?;
                Ok(Reply::Catalog {
                    repositories,
                    workspaces,
                })
            }
            Request::Files {
                workspace,
                prefix,
                offset,
            } => {
                validate_path(&prefix, true)?;
                let v = self.call("files_list", json!({"workspace":workspace,"path":prefix,"offset":offset,"limit":PAGE_SIZE})).await?;
                let entries: Vec<FileEntry> = serde_json::from_value(v["entries"].clone())
                    .map_err(|_| WorkspaceError::Protocol)?;
                if entries.len() > PAGE_SIZE {
                    return Err(WorkspaceError::Protocol);
                }
                Ok(Reply::Files(FilePage {
                    revision: string(&v, "revision")?,
                    entries,
                    total: v["total"].as_u64().ok_or(WorkspaceError::Protocol)? as usize,
                    next: v["next_offset"].as_u64().map(|n| n as usize),
                }))
            }
            Request::Read { workspace, file } => {
                validate_path(&file.path, false)?;
                let stat = self
                    .call(
                        "files_stat",
                        json!({"workspace":workspace,"path":file.path,"limit":1}),
                    )
                    .await?;
                let current: FileEntry = serde_json::from_value(stat["entries"][0].clone())
                    .map_err(|_| WorkspaceError::Protocol)?;
                if current.path != file.path
                    || current.size > MAX_TEXT_BYTES as u64
                    || !["100644", "100755"].contains(&current.mode.as_str())
                {
                    return Err(WorkspaceError::NotEditable);
                }
                let v = self
                    .call(
                        "files_read",
                        json!({"workspace":workspace,"path":file.path,"binary":true}),
                    )
                    .await?;
                if v["revision"] != stat["revision"] {
                    return Err(WorkspaceError::Message(
                        "Workspace changed while opening the file; retry".into(),
                    ));
                }
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(string(&v, "base64")?)
                    .map_err(|_| WorkspaceError::Protocol)?;
                if raw.len() > MAX_TEXT_BYTES || raw.contains(&0) {
                    return Err(WorkspaceError::NotEditable);
                }
                let text = String::from_utf8(raw).map_err(|_| WorkspaceError::NotEditable)?;
                Ok(Reply::Document(Document {
                    path: file.path,
                    text,
                    mode: current.mode,
                    revision: string(&v, "revision")?,
                }))
            }
            Request::Write {
                workspace,
                document,
            } => {
                validate_path(&document.path, false)?;
                if document.text.len() > MAX_TEXT_BYTES
                    || document.text.contains('\0')
                    || !["100644", "100755"].contains(&document.mode.as_str())
                {
                    return Err(WorkspaceError::NotEditable);
                }
                let value = self.call("files_write", json!({"workspace":workspace,"path":document.path,"expected_revision":document.revision,"mode":document.mode,"base64":base64::engine::general_purpose::STANDARD.encode(document.text.as_bytes())})).await?;
                Ok(Reply::Saved {
                    revision: string(&value, "revision")?,
                })
            }
            Request::Create { repo, name } => Ok(Reply::Created(workspace(
                &self
                    .call("workspace_create", json!({"repo":repo,"workspace":name}))
                    .await?,
            )?)),
            Request::Fork {
                workspace: source,
                name,
            } => Ok(Reply::Created(workspace(
                &self
                    .call("workspace_fork", json!({"workspace":source,"name":name}))
                    .await?,
            )?)),
            Request::Diff { workspace } => self.report("workspace_diff", workspace).await,
            Request::Size { workspace } => self.report("workspace_size", workspace).await,
            Request::ReviewMerge { workspace, source } => {
                crate::integration::review(&self.embedded, workspace, source)
                    .await
                    .map(Reply::MergeReview)
                    .map_err(|error| WorkspaceError::Message(format!("{error:#}")))
            }
            Request::Merge(review) => crate::integration::apply(&self.embedded, review)
                .await
                .map(Reply::MergeResult)
                .map_err(|error| WorkspaceError::Message(format!("{error:#}"))),
        }
    }
    async fn report(&self, op: &str, workspace: String) -> Result<Reply, WorkspaceError> {
        let value = self
            .call(op, json!({"workspace":workspace,"limit":PAGE_SIZE}))
            .await?;
        let report = value["diff"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| serde_json::to_string_pretty(&value).unwrap_or_default());
        Ok(Reply::Report(report.chars().take(64000).collect()))
    }
}
impl WorkspaceProvider for VworkProvider {
    fn execute(&self, request: Request) -> ProviderFuture<'_> {
        Box::pin(self.run(request))
    }
}
fn string(value: &Value, name: &str) -> Result<String, WorkspaceError> {
    value[name]
        .as_str()
        .map(str::to_owned)
        .ok_or(WorkspaceError::Protocol)
}
fn workspace(value: &Value) -> Result<Workspace, WorkspaceError> {
    Ok(Workspace {
        id: string(value, "id")?,
        repo: string(value, "repo")?,
        revision: string(value, "head")?,
    })
}
pub fn validate_loopback(url: &str) -> Result<(), WorkspaceError> {
    let address: std::net::SocketAddr = url
        .strip_prefix("http://")
        .and_then(|v| v.strip_suffix('/'))
        .and_then(|v| v.parse().ok())
        .ok_or(WorkspaceError::Protocol)?;
    if address.ip() != std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) || address.port() == 0 {
        return Err(WorkspaceError::Protocol);
    }
    Ok(())
}
