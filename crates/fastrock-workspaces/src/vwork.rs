//! Optional Goose/vwork adapter. Never opens redb, starts a process, or exports files.
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
    #[serde(default, rename = "GOOSE_SEARCH_PATHS")]
    search_paths: Vec<String>,
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
fn installed(command: &Path) -> bool {
    if !command.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        command
            .metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Read only configuration, and only accept a real vwork executable + `mcp` invocation.
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
            let search = config
                .search_paths
                .iter()
                .map(|p| expand(p, &paths.user_home))
                .chain(paths.search.iter().cloned());
            let available = if command.is_absolute() {
                installed(&command)
            } else {
                search.into_iter().any(|p| {
                    installed(&p.join(&command))
                        || (cfg!(windows) && installed(&p.join(format!("{binary}.exe"))))
                })
            };
            if !available {
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
    home: PathBuf,
    client: reqwest::Client,
}
impl VworkProvider {
    pub fn new(installation: &Installation) -> Result<Self, WorkspaceError> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| WorkspaceError::Transport)?;
        Ok(Self {
            home: installation.home.clone(),
            client,
        })
    }

    async fn call(&self, op: &str, arguments: Value) -> Result<Value, WorkspaceError> {
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
            .timeout(if op == "database_compact" {
                Duration::from_secs(3600)
            } else {
                Duration::from_secs(20)
            })
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
            if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
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
