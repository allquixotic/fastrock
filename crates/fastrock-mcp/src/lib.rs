#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub scope: McpConfigScope,
    pub enabled: bool,
    pub transport: McpTransportConfig,
    #[serde(default = "default_mcp_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub always_allow_tools: BTreeSet<String>,
    #[serde(default)]
    pub disabled_tools: BTreeSet<String>,
    pub oauth: Option<McpOAuthConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpConfigScope {
    #[default]
    Global,
    Project {
        project_folder_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransportConfig {
    Stdio {
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
        env: BTreeMap<String, String>,
    },
    StreamableHttp {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpOAuthConfig {
    pub client_id: String,
    pub client_secret_ref: String,
    pub authorization_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
}

pub const MCP_OAUTH_TOKEN_SECRET_SERVICE: &str = "fastrock-mcp-oauth";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpOAuthTokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_at_ms: Option<i64>,
    pub scopes: Vec<String>,
}

impl fmt::Debug for McpOAuthTokenSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpOAuthTokenSet")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("token_type", &self.token_type)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("scopes", &self.scopes)
            .finish()
    }
}

pub fn mcp_oauth_token_secret_ref(server_id: &str) -> String {
    format!("secret:mcp-oauth:{server_id}")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct McpConfigSet {
    pub servers: Vec<McpServerConfig>,
}

impl McpConfigSet {
    pub fn validate(&self) -> Result<(), McpConfigError> {
        let mut ids = BTreeSet::new();
        for server in &self.servers {
            if server.id.trim().is_empty() {
                return Err(McpConfigError::MissingId);
            }
            if !ids.insert(server.id.clone()) {
                return Err(McpConfigError::DuplicateId(server.id.clone()));
            }
            server.validate()?;
        }
        Ok(())
    }

    pub fn diff_for_hot_reload(&self, next: &Self) -> Result<Vec<McpConfigChange>, McpConfigError> {
        self.validate()?;
        next.validate()?;

        let current = self
            .servers
            .iter()
            .map(|server| (server.id.as_str(), server))
            .collect::<BTreeMap<_, _>>();
        let next_map = next
            .servers
            .iter()
            .map(|server| (server.id.as_str(), server))
            .collect::<BTreeMap<_, _>>();
        let ids = current
            .keys()
            .chain(next_map.keys())
            .copied()
            .collect::<BTreeSet<_>>();

        Ok(ids
            .into_iter()
            .filter_map(|id| match (current.get(id), next_map.get(id)) {
                (None, Some(server)) => Some(McpConfigChange::Added(server.id.clone())),
                (Some(server), None) => Some(McpConfigChange::Removed(server.id.clone())),
                (Some(old), Some(new)) if old.enabled != new.enabled && new.enabled => {
                    Some(McpConfigChange::Enabled(new.id.clone()))
                }
                (Some(old), Some(new)) if old.enabled != new.enabled && !new.enabled => {
                    Some(McpConfigChange::Disabled(new.id.clone()))
                }
                (Some(old), Some(new)) if old != new => {
                    Some(McpConfigChange::Updated(new.id.clone()))
                }
                _ => None,
            })
            .collect())
    }

    pub fn server(&self, id: &str) -> Option<&McpServerConfig> {
        self.servers.iter().find(|server| server.id == id)
    }

    pub fn add_server(&mut self, server: McpServerConfig) -> Result<(), McpConfigError> {
        server.validate()?;
        if self.server(&server.id).is_some() {
            return Err(McpConfigError::DuplicateId(server.id));
        }
        self.servers.push(server);
        self.sort_servers();
        self.validate()
    }

    pub fn update_server(&mut self, server: McpServerConfig) -> Result<(), McpConfigError> {
        server.validate()?;
        let Some(index) = self
            .servers
            .iter()
            .position(|existing| existing.id == server.id)
        else {
            return Err(McpConfigError::ServerNotFound(server.id));
        };
        let original = self.clone();
        self.servers[index] = server;
        self.sort_servers();
        self.validate().inspect_err(|_| {
            *self = original;
        })
    }

    pub fn remove_server(&mut self, id: &str) -> Result<McpServerConfig, McpConfigError> {
        let Some(index) = self.servers.iter().position(|server| server.id == id) else {
            return Err(McpConfigError::ServerNotFound(id.to_owned()));
        };
        Ok(self.servers.remove(index))
    }

    pub fn set_server_enabled(&mut self, id: &str, enabled: bool) -> Result<(), McpConfigError> {
        self.edit_server(id, |server| {
            server.enabled = enabled;
            Ok(())
        })
    }

    pub fn set_tool_always_allowed(
        &mut self,
        id: &str,
        tool_name: &str,
        always_allowed: bool,
    ) -> Result<(), McpConfigError> {
        validate_tool_name(id, tool_name)?;
        self.edit_server(id, |server| {
            if always_allowed {
                server.disabled_tools.remove(tool_name);
                server.always_allow_tools.insert(tool_name.to_owned());
            } else {
                server.always_allow_tools.remove(tool_name);
            }
            Ok(())
        })
    }

    pub fn set_tool_disabled(
        &mut self,
        id: &str,
        tool_name: &str,
        disabled: bool,
    ) -> Result<(), McpConfigError> {
        validate_tool_name(id, tool_name)?;
        self.edit_server(id, |server| {
            if disabled {
                server.always_allow_tools.remove(tool_name);
                server.disabled_tools.insert(tool_name.to_owned());
            } else {
                server.disabled_tools.remove(tool_name);
            }
            Ok(())
        })
    }

    fn edit_server(
        &mut self,
        id: &str,
        edit: impl FnOnce(&mut McpServerConfig) -> Result<(), McpConfigError>,
    ) -> Result<(), McpConfigError> {
        let Some(index) = self.servers.iter().position(|server| server.id == id) else {
            return Err(McpConfigError::ServerNotFound(id.to_owned()));
        };
        let original = self.clone();
        edit(&mut self.servers[index])?;
        self.validate().inspect_err(|_| {
            *self = original;
        })
    }

    fn sort_servers(&mut self) {
        self.servers.sort_by(|left, right| left.id.cmp(&right.id));
    }
}

impl McpServerConfig {
    pub fn validate(&self) -> Result<(), McpConfigError> {
        if self.name.trim().is_empty() {
            return Err(McpConfigError::MissingName(self.id.clone()));
        }
        if let McpConfigScope::Project { project_folder_id } = &self.scope
            && project_folder_id.trim().is_empty()
        {
            return Err(McpConfigError::MissingProjectScope(self.id.clone()));
        }
        if self.timeout_ms == 0 {
            return Err(McpConfigError::InvalidTimeout(self.id.clone()));
        }
        if let Some(tool_name) = empty_tool_name(
            self.always_allow_tools
                .iter()
                .chain(self.disabled_tools.iter()),
        ) {
            return Err(McpConfigError::InvalidToolName {
                id: self.id.clone(),
                tool_name,
            });
        }
        if let Some(tool_name) = self
            .always_allow_tools
            .intersection(&self.disabled_tools)
            .next()
            .cloned()
        {
            return Err(McpConfigError::ConflictingToolPolicy {
                id: self.id.clone(),
                tool_name,
            });
        }
        match &self.transport {
            McpTransportConfig::Stdio { command, .. } if command.trim().is_empty() => {
                Err(McpConfigError::MissingStdioCommand(self.id.clone()))
            }
            McpTransportConfig::StreamableHttp { url, .. } => {
                validate_http_url(url).map_err(|_| McpConfigError::InvalidHttpUrl {
                    id: self.id.clone(),
                    url: url.clone(),
                })?;
                self.validate_oauth()
            }
            McpTransportConfig::Stdio { .. } => Ok(()),
        }
    }

    fn validate_oauth(&self) -> Result<(), McpConfigError> {
        let Some(oauth) = &self.oauth else {
            return Ok(());
        };
        if oauth.client_id.trim().is_empty() {
            return Err(McpConfigError::InvalidOAuth {
                id: self.id.clone(),
                reason: "client id is required",
            });
        }
        if oauth.client_secret_ref.trim().is_empty() {
            return Err(McpConfigError::InvalidOAuth {
                id: self.id.clone(),
                reason: "client secret reference is required",
            });
        }
        validate_http_url(&oauth.authorization_url).map_err(|_| McpConfigError::InvalidOAuth {
            id: self.id.clone(),
            reason: "authorization URL must be http or https",
        })?;
        validate_http_url(&oauth.token_url).map_err(|_| McpConfigError::InvalidOAuth {
            id: self.id.clone(),
            reason: "token URL must be http or https",
        })?;
        Ok(())
    }
}

fn default_mcp_timeout_ms() -> u64 {
    30_000
}

fn empty_tool_name<'a>(mut tool_names: impl Iterator<Item = &'a String>) -> Option<String> {
    tool_names
        .find(|tool_name| tool_name.trim().is_empty())
        .cloned()
}

fn validate_tool_name(id: &str, tool_name: &str) -> Result<(), McpConfigError> {
    if tool_name.trim().is_empty() {
        return Err(McpConfigError::InvalidToolName {
            id: id.to_owned(),
            tool_name: tool_name.to_owned(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpConfigChange {
    Added(String),
    Removed(String),
    Updated(String),
    Enabled(String),
    Disabled(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", content = "message", rename_all = "snake_case")]
pub enum McpServerRuntimeStatus {
    Stopped,
    Starting,
    Running,
    Failed(String),
}

impl McpServerRuntimeStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Failed(_) => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum McpOAuthState {
    NotConfigured,
    AuthorizationRequired,
    Authorized { token_ref: String },
    Expired { token_ref: String },
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpRuntimeSnapshot {
    pub server_id: String,
    pub status: McpServerRuntimeStatus,
    pub discovered_tools: Vec<String>,
    pub discovered_resources: Vec<String>,
    pub discovered_resource_templates: Vec<String>,
    pub oauth_state: McpOAuthState,
    pub restart_count: u32,
    pub last_error: Option<String>,
    pub log_excerpt: Option<String>,
    pub updated_at_ms: i64,
}

impl McpRuntimeSnapshot {
    pub fn stopped(server: &McpServerConfig, now_ms: i64) -> Self {
        Self {
            server_id: server.id.clone(),
            status: McpServerRuntimeStatus::Stopped,
            discovered_tools: Vec::new(),
            discovered_resources: Vec::new(),
            discovered_resource_templates: Vec::new(),
            oauth_state: if server.oauth.is_some() {
                McpOAuthState::AuthorizationRequired
            } else {
                McpOAuthState::NotConfigured
            },
            restart_count: 0,
            last_error: None,
            log_excerpt: None,
            updated_at_ms: now_ms,
        }
    }

    pub fn with_discovery(
        mut self,
        tools: impl IntoIterator<Item = impl Into<String>>,
        resources: impl IntoIterator<Item = impl Into<String>>,
        resource_templates: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.discovered_tools = sorted_strings(tools);
        self.discovered_resources = sorted_strings(resources);
        self.discovered_resource_templates = sorted_strings(resource_templates);
        self
    }

    pub fn mark_running(mut self, now_ms: i64) -> Self {
        self.status = McpServerRuntimeStatus::Running;
        self.last_error = None;
        self.updated_at_ms = now_ms;
        self
    }

    pub fn mark_failed(
        mut self,
        error: impl Into<String>,
        log_excerpt: Option<String>,
        now_ms: i64,
    ) -> Self {
        let error = error.into();
        self.status = McpServerRuntimeStatus::Failed(error.clone());
        self.last_error = Some(error);
        self.log_excerpt = log_excerpt.map(trim_log_excerpt);
        self.updated_at_ms = now_ms;
        self
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct McpToolDescriptor {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct McpResourceDescriptor {
    pub uri: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, alias = "mimeType")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct McpResourceTemplateDescriptor {
    #[serde(alias = "uriTemplate")]
    pub uri_template: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, alias = "mimeType")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpDiscoverySnapshot {
    pub tools: Vec<McpToolDescriptor>,
    pub resources: Vec<McpResourceDescriptor>,
    pub resource_templates: Vec<McpResourceTemplateDescriptor>,
}

impl McpDiscoverySnapshot {
    pub fn runtime_snapshot(&self, server: &McpServerConfig, now_ms: i64) -> McpRuntimeSnapshot {
        McpRuntimeSnapshot::stopped(server, now_ms)
            .with_discovery(
                self.tools.iter().map(|tool| tool.name.as_str()),
                self.resources.iter().map(|resource| resource.uri.as_str()),
                self.resource_templates
                    .iter()
                    .map(|template| template.uri_template.as_str()),
            )
            .mark_running(now_ms)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpRuntimeSnapshotSet {
    snapshots: BTreeMap<String, McpRuntimeSnapshot>,
}

impl McpRuntimeSnapshotSet {
    pub fn upsert(&mut self, snapshot: McpRuntimeSnapshot) {
        self.snapshots.insert(snapshot.server_id.clone(), snapshot);
    }

    pub fn remove(&mut self, server_id: &str) -> bool {
        self.snapshots.remove(server_id).is_some()
    }

    pub fn get(&self, server_id: &str) -> Option<&McpRuntimeSnapshot> {
        self.snapshots.get(server_id)
    }

    pub fn list(&self) -> Vec<McpRuntimeSnapshot> {
        self.snapshots.values().cloned().collect()
    }
}

fn sorted_strings(values: impl IntoIterator<Item = impl Into<String>>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(Into::into)
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn trim_log_excerpt(value: String) -> String {
    const MAX_LOG_EXCERPT_BYTES: usize = 4096;
    if value.len() <= MAX_LOG_EXCERPT_BYTES {
        return value;
    }
    let mut end = MAX_LOG_EXCERPT_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...[truncated]", &value[..end])
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct McpJsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl McpJsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct McpJsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<McpJsonRpcErrorObject>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct McpJsonRpcErrorObject {
    pub code: i64,
    pub message: String,
}

pub fn encode_mcp_json_rpc_frame(value: &Value) -> Result<Vec<u8>, McpRuntimeError> {
    let body = serde_json::to_vec(value)?;
    let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub fn decode_mcp_json_rpc_frame(frame: &[u8]) -> Result<Value, McpRuntimeError> {
    let headers_end = frame
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            McpRuntimeError::Protocol("MCP frame missing header terminator".to_owned())
        })?;
    let headers = std::str::from_utf8(&frame[..headers_end])
        .map_err(|error| McpRuntimeError::Protocol(error.to_string()))?;
    let content_length = parse_mcp_content_length(headers)?;
    let body_start = headers_end + 4;
    let body_end = body_start + content_length;
    if frame.len() < body_end {
        return Err(McpRuntimeError::Protocol(
            "MCP frame body is shorter than content-length".to_owned(),
        ));
    }
    Ok(serde_json::from_slice(&frame[body_start..body_end])?)
}

pub struct McpStdioClient {
    server_id: String,
    timeout_ms: u64,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpStdioClient {
    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub async fn start(server: &McpServerConfig) -> Result<Self, McpRuntimeError> {
        server.validate()?;
        let McpTransportConfig::Stdio {
            command,
            args,
            cwd,
            env,
        } = &server.transport
        else {
            return Err(McpRuntimeError::UnsupportedTransport {
                id: server.id.clone(),
                expected: "stdio",
            });
        };
        let mut command_builder = Command::new(command);
        command_builder
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = cwd {
            command_builder.current_dir(cwd);
        }
        command_builder.envs(env);
        let mut child = command_builder
            .spawn()
            .map_err(|error| McpRuntimeError::ProcessStart {
                id: server.id.clone(),
                message: error.to_string(),
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpRuntimeError::MissingPipe {
                id: server.id.clone(),
                pipe: "stdin",
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpRuntimeError::MissingPipe {
                id: server.id.clone(),
                pipe: "stdout",
            })?;
        Ok(Self {
            server_id: server.id.clone(),
            timeout_ms: server.timeout_ms,
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    pub async fn initialize(
        &mut self,
        protocol_version: &str,
        client_name: &str,
    ) -> Result<Value, McpRuntimeError> {
        self.request(
            "initialize",
            Some(serde_json::json!({
                "protocolVersion": protocol_version,
                "clientInfo": {
                    "name": client_name,
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {}
            })),
        )
        .await
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpToolDescriptor>, McpRuntimeError> {
        parse_mcp_tools(self.request("tools/list", None).await?)
    }

    pub async fn list_resources(&mut self) -> Result<Vec<McpResourceDescriptor>, McpRuntimeError> {
        parse_mcp_resources(self.request("resources/list", None).await?)
    }

    pub async fn list_resource_templates(
        &mut self,
    ) -> Result<Vec<McpResourceTemplateDescriptor>, McpRuntimeError> {
        parse_mcp_resource_templates(self.request("resources/templates/list", None).await?)
    }

    pub async fn discover(&mut self) -> Result<McpDiscoverySnapshot, McpRuntimeError> {
        Ok(McpDiscoverySnapshot {
            tools: self.list_tools().await?,
            resources: self.list_resources().await?,
            resource_templates: self.list_resource_templates().await?,
        })
    }

    pub async fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, McpRuntimeError> {
        let id = self.next_id;
        self.next_id += 1;
        let server_id = self.server_id.clone();
        let timeout_ms = self.timeout_ms;
        tokio::time::timeout(Duration::from_millis(timeout_ms), async {
            let request = serde_json::to_value(McpJsonRpcRequest::new(id, method, params))?;
            let frame = encode_mcp_json_rpc_frame(&request)?;
            self.stdin.write_all(&frame).await?;
            self.stdin.flush().await?;
            let response = read_mcp_json_rpc_frame(&mut self.stdout).await?;
            decode_mcp_json_rpc_response(id, response)
        })
        .await
        .map_err(|_| McpRuntimeError::Timeout {
            id: server_id,
            timeout_ms,
        })?
    }

    pub async fn shutdown(mut self) -> Result<(), McpRuntimeError> {
        self.child.kill().await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct McpStreamableHttpClient {
    server_id: String,
    url: String,
    headers: BTreeMap<String, String>,
    timeout_ms: u64,
    client: reqwest::Client,
    next_id: u64,
}

impl McpStreamableHttpClient {
    pub fn new(server: &McpServerConfig) -> Result<Self, McpRuntimeError> {
        server.validate()?;
        let McpTransportConfig::StreamableHttp { url, headers } = &server.transport else {
            return Err(McpRuntimeError::UnsupportedTransport {
                id: server.id.clone(),
                expected: "streamable_http",
            });
        };
        Ok(Self {
            server_id: server.id.clone(),
            url: url.clone(),
            headers: headers.clone(),
            timeout_ms: server.timeout_ms,
            client: reqwest::Client::new(),
            next_id: 1,
        })
    }

    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    pub async fn initialize(
        &mut self,
        protocol_version: &str,
        client_name: &str,
    ) -> Result<Value, McpRuntimeError> {
        self.request(
            "initialize",
            Some(serde_json::json!({
                "protocolVersion": protocol_version,
                "clientInfo": {
                    "name": client_name,
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {}
            })),
        )
        .await
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpToolDescriptor>, McpRuntimeError> {
        parse_mcp_tools(self.request("tools/list", None).await?)
    }

    pub async fn list_resources(&mut self) -> Result<Vec<McpResourceDescriptor>, McpRuntimeError> {
        parse_mcp_resources(self.request("resources/list", None).await?)
    }

    pub async fn list_resource_templates(
        &mut self,
    ) -> Result<Vec<McpResourceTemplateDescriptor>, McpRuntimeError> {
        parse_mcp_resource_templates(self.request("resources/templates/list", None).await?)
    }

    pub async fn discover(&mut self) -> Result<McpDiscoverySnapshot, McpRuntimeError> {
        Ok(McpDiscoverySnapshot {
            tools: self.list_tools().await?,
            resources: self.list_resources().await?,
            resource_templates: self.list_resource_templates().await?,
        })
    }

    pub async fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, McpRuntimeError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = serde_json::to_value(McpJsonRpcRequest::new(id, method, params))?;
        let mut builder = self
            .client
            .post(&self.url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18");
        for (name, value) in &self.headers {
            builder = builder.header(name, value);
        }
        tokio::time::timeout(Duration::from_millis(self.timeout_ms), async {
            let response = builder
                .body(serde_json::to_vec(&request)?)
                .send()
                .await
                .map_err(|error| McpRuntimeError::Http(error.to_string()))?;
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|error| McpRuntimeError::Http(error.to_string()))?;
            if !status.is_success() {
                return Err(McpRuntimeError::HttpStatus {
                    status: status.as_u16(),
                    body,
                });
            }
            let response = parse_streamable_http_response(&body)?;
            decode_mcp_json_rpc_response(id, response)
        })
        .await
        .map_err(|_| McpRuntimeError::Timeout {
            id: self.server_id.clone(),
            timeout_ms: self.timeout_ms,
        })?
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPkceChallenge {
    pub verifier: String,
    pub challenge: String,
    pub method: &'static str,
}

impl McpPkceChallenge {
    pub fn from_verifier(verifier: impl Into<String>) -> Self {
        let verifier = verifier.into();
        let digest = Sha256::digest(verifier.as_bytes());
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
        Self {
            verifier,
            challenge,
            method: "S256",
        }
    }

    pub fn authorization_url(
        &self,
        oauth: &McpOAuthConfig,
        redirect_uri: &str,
        state: &str,
    ) -> Result<String, McpRuntimeError> {
        let mut url =
            Url::parse(&oauth.authorization_url).map_err(|error| McpRuntimeError::OAuth {
                id: oauth.client_id.clone(),
                message: error.to_string(),
            })?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("response_type", "code");
            query.append_pair("client_id", &oauth.client_id);
            query.append_pair("redirect_uri", redirect_uri);
            query.append_pair("state", state);
            query.append_pair("code_challenge", &self.challenge);
            query.append_pair("code_challenge_method", self.method);
            if !oauth.scopes.is_empty() {
                query.append_pair("scope", &oauth.scopes.join(" "));
            }
        }
        Ok(url.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpOAuthCallback {
    pub code: String,
    pub state: String,
}

impl McpOAuthCallback {
    pub fn from_redirect_url(
        server_id: &str,
        redirect_url: &str,
        expected_state: &str,
    ) -> Result<Self, McpRuntimeError> {
        let url = Url::parse(redirect_url).map_err(|error| McpRuntimeError::OAuth {
            id: server_id.to_owned(),
            message: error.to_string(),
        })?;
        let mut code = None;
        let mut state = None;
        let mut error = None;
        let mut error_description = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "code" => code = Some(value.into_owned()),
                "state" => state = Some(value.into_owned()),
                "error" => error = Some(value.into_owned()),
                "error_description" => error_description = Some(value.into_owned()),
                _ => {}
            }
        }
        if let Some(error) = error {
            let message = error_description.unwrap_or(error);
            return Err(McpRuntimeError::OAuth {
                id: server_id.to_owned(),
                message,
            });
        }
        let state = state.ok_or_else(|| McpRuntimeError::OAuth {
            id: server_id.to_owned(),
            message: "callback is missing state".to_owned(),
        })?;
        if state != expected_state {
            return Err(McpRuntimeError::OAuth {
                id: server_id.to_owned(),
                message: "callback state mismatch".to_owned(),
            });
        }
        let code = code.ok_or_else(|| McpRuntimeError::OAuth {
            id: server_id.to_owned(),
            message: "callback is missing authorization code".to_owned(),
        })?;
        Ok(Self { code, state })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct McpOAuthTokenExchangeRequest {
    pub token_url: String,
    pub form: BTreeMap<String, String>,
}

impl fmt::Debug for McpOAuthTokenExchangeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut form = self.form.clone();
        for key in ["client_secret", "code", "code_verifier"] {
            if form.contains_key(key) {
                form.insert(key.to_owned(), "<redacted>".to_owned());
            }
        }
        formatter
            .debug_struct("McpOAuthTokenExchangeRequest")
            .field("token_url", &self.token_url)
            .field("form", &form)
            .finish()
    }
}

impl McpOAuthTokenExchangeRequest {
    pub fn authorization_code(
        oauth: &McpOAuthConfig,
        callback: &McpOAuthCallback,
        redirect_uri: &str,
        pkce: &McpPkceChallenge,
        client_secret: &str,
    ) -> Result<Self, McpRuntimeError> {
        Url::parse(&oauth.token_url).map_err(|error| McpRuntimeError::OAuth {
            id: oauth.client_id.clone(),
            message: error.to_string(),
        })?;
        let mut form = BTreeMap::new();
        form.insert("grant_type".to_owned(), "authorization_code".to_owned());
        form.insert("client_id".to_owned(), oauth.client_id.clone());
        form.insert("client_secret".to_owned(), client_secret.to_owned());
        form.insert("code".to_owned(), callback.code.clone());
        form.insert("redirect_uri".to_owned(), redirect_uri.to_owned());
        form.insert("code_verifier".to_owned(), pkce.verifier.clone());
        Ok(Self {
            token_url: oauth.token_url.clone(),
            form,
        })
    }
}

pub async fn exchange_mcp_oauth_code(
    client: &reqwest::Client,
    server_id: &str,
    request: &McpOAuthTokenExchangeRequest,
    issued_at_ms: i64,
) -> Result<McpOAuthTokenSet, McpRuntimeError> {
    let response = client
        .post(&request.token_url)
        .header("accept", "application/json")
        .form(&request.form)
        .send()
        .await
        .map_err(|error| McpRuntimeError::Http(error.to_string()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| McpRuntimeError::Http(error.to_string()))?;
    if !status.is_success() {
        return Err(McpRuntimeError::HttpStatus {
            status: status.as_u16(),
            body,
        });
    }
    parse_mcp_oauth_token_response(server_id, &body, issued_at_ms)
}

pub fn parse_mcp_oauth_token_response(
    server_id: &str,
    body: &str,
    issued_at_ms: i64,
) -> Result<McpOAuthTokenSet, McpRuntimeError> {
    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: Option<String>,
        refresh_token: Option<String>,
        token_type: Option<String>,
        expires_in: Option<i64>,
        scope: Option<String>,
    }

    let response: TokenResponse = serde_json::from_str(body)?;
    let access_token = response
        .access_token
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| McpRuntimeError::OAuth {
            id: server_id.to_owned(),
            message: "token response is missing access_token".to_owned(),
        })?;
    let expires_at_ms = response
        .expires_in
        .map(|seconds| {
            if seconds < 0 {
                return Err(McpRuntimeError::OAuth {
                    id: server_id.to_owned(),
                    message: "token response expires_in must be non-negative".to_owned(),
                });
            }
            Ok(issued_at_ms.saturating_add(seconds.saturating_mul(1000)))
        })
        .transpose()?;
    Ok(McpOAuthTokenSet {
        access_token,
        refresh_token: response.refresh_token,
        token_type: response.token_type.unwrap_or_else(|| "Bearer".to_owned()),
        expires_at_ms,
        scopes: response
            .scope
            .map(|scope| {
                scope
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    })
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum McpConfigError {
    #[error("MCP server id is required")]
    MissingId,
    #[error("MCP server id is duplicated: {0}")]
    DuplicateId(String),
    #[error("MCP server not found: {0}")]
    ServerNotFound(String),
    #[error("MCP server name is required: {0}")]
    MissingName(String),
    #[error("MCP project scope is missing a project folder id: {0}")]
    MissingProjectScope(String),
    #[error("MCP server timeout must be greater than zero: {0}")]
    InvalidTimeout(String),
    #[error("MCP tool name is invalid for {id}: {tool_name}")]
    InvalidToolName { id: String, tool_name: String },
    #[error("MCP tool cannot be both always allowed and disabled for {id}: {tool_name}")]
    ConflictingToolPolicy { id: String, tool_name: String },
    #[error("MCP stdio command is required: {0}")]
    MissingStdioCommand(String),
    #[error("MCP streamable HTTP URL is invalid for {id}: {url}")]
    InvalidHttpUrl { id: String, url: String },
    #[error("MCP OAuth config is invalid for {id}: {reason}")]
    InvalidOAuth { id: String, reason: &'static str },
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum McpRuntimeError {
    #[error(transparent)]
    Config(#[from] McpConfigError),
    #[error("MCP server {id} uses unsupported transport, expected {expected}")]
    UnsupportedTransport { id: String, expected: &'static str },
    #[error("failed to start MCP server {id}: {message}")]
    ProcessStart { id: String, message: String },
    #[error("MCP server {id} missing {pipe} pipe")]
    MissingPipe { id: String, pipe: &'static str },
    #[error("MCP I/O error: {0}")]
    Io(String),
    #[error("MCP JSON error: {0}")]
    Json(String),
    #[error("MCP protocol error: {0}")]
    Protocol(String),
    #[error("MCP JSON-RPC error {code}: {message}")]
    JsonRpc { code: i64, message: String },
    #[error("MCP HTTP error: {0}")]
    Http(String),
    #[error("MCP HTTP status {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("MCP request timed out for {id} after {timeout_ms}ms")]
    Timeout { id: String, timeout_ms: u64 },
    #[error("MCP OAuth error for {id}: {message}")]
    OAuth { id: String, message: String },
}

impl From<std::io::Error> for McpRuntimeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<serde_json::Error> for McpRuntimeError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error.to_string())
    }
}

fn validate_http_url(value: &str) -> Result<(), url::ParseError> {
    let url = Url::parse(value)?;
    match url.scheme() {
        "http" | "https" => Ok(()),
        _ => Err(url::ParseError::RelativeUrlWithoutBase),
    }
}

async fn read_mcp_json_rpc_frame<R>(reader: &mut R) -> Result<Value, McpRuntimeError>
where
    R: AsyncBufRead + Unpin,
{
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).await?;
        if bytes == 0 {
            return Err(McpRuntimeError::Protocol(
                "MCP stream closed before frame headers completed".to_owned(),
            ));
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        headers.push_str(&line);
    }
    let content_length = parse_mcp_content_length(&headers)?;
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

fn parse_mcp_content_length(headers: &str) -> Result<usize, McpRuntimeError> {
    headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or_else(|| McpRuntimeError::Protocol("MCP frame missing Content-Length".to_owned()))
}

fn decode_mcp_json_rpc_response(expected_id: u64, value: Value) -> Result<Value, McpRuntimeError> {
    let response: McpJsonRpcResponse = serde_json::from_value(value)?;
    if response.jsonrpc != "2.0" {
        return Err(McpRuntimeError::Protocol(
            "MCP JSON-RPC response version is not 2.0".to_owned(),
        ));
    }
    if response.id != Some(expected_id) {
        return Err(McpRuntimeError::Protocol(format!(
            "MCP JSON-RPC response id mismatch: expected {expected_id}, got {:?}",
            response.id
        )));
    }
    if let Some(error) = response.error {
        return Err(McpRuntimeError::JsonRpc {
            code: error.code,
            message: error.message,
        });
    }
    Ok(response.result.unwrap_or(Value::Null))
}

fn parse_streamable_http_response(body: &str) -> Result<Value, McpRuntimeError> {
    let trimmed = body.trim_start();
    if trimmed.starts_with('{') {
        return Ok(serde_json::from_str(trimmed)?);
    }

    let mut event_name = String::new();
    let mut data_lines = Vec::new();
    for line in body.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            if let Some(value) = flush_mcp_sse_event(&event_name, &data_lines)? {
                return Ok(value);
            }
            event_name.clear();
            data_lines.clear();
            continue;
        }
        if let Some(name) = line.strip_prefix("event:") {
            event_name = name.trim().to_owned();
        } else if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.trim().to_owned());
        }
    }
    flush_mcp_sse_event(&event_name, &data_lines)?.ok_or_else(|| {
        McpRuntimeError::Protocol("MCP streamable HTTP response had no JSON-RPC data".to_owned())
    })
}

fn parse_mcp_tools(value: Value) -> Result<Vec<McpToolDescriptor>, McpRuntimeError> {
    #[derive(Deserialize)]
    struct ToolsListResult {
        #[serde(default)]
        tools: Vec<McpToolDescriptor>,
    }

    let mut tools: Vec<McpToolDescriptor> = serde_json::from_value::<ToolsListResult>(value)?
        .tools
        .into_iter()
        .filter(|tool| !tool.name.trim().is_empty())
        .collect();
    tools.sort_by(|left, right| left.name.cmp(&right.name));
    tools.dedup_by(|left, right| left.name == right.name);
    Ok(tools)
}

fn parse_mcp_resources(value: Value) -> Result<Vec<McpResourceDescriptor>, McpRuntimeError> {
    #[derive(Deserialize)]
    struct ResourcesListResult {
        #[serde(default)]
        resources: Vec<McpResourceDescriptor>,
    }

    let mut resources: Vec<McpResourceDescriptor> =
        serde_json::from_value::<ResourcesListResult>(value)?
            .resources
            .into_iter()
            .filter(|resource| !resource.uri.trim().is_empty())
            .collect();
    resources.sort_by(|left, right| left.uri.cmp(&right.uri));
    resources.dedup_by(|left, right| left.uri == right.uri);
    Ok(resources)
}

fn parse_mcp_resource_templates(
    value: Value,
) -> Result<Vec<McpResourceTemplateDescriptor>, McpRuntimeError> {
    #[derive(Deserialize)]
    struct ResourceTemplatesListResult {
        #[serde(default, alias = "resourceTemplates")]
        resource_templates: Vec<McpResourceTemplateDescriptor>,
    }

    let mut resource_templates: Vec<McpResourceTemplateDescriptor> =
        serde_json::from_value::<ResourceTemplatesListResult>(value)?
            .resource_templates
            .into_iter()
            .filter(|template| !template.uri_template.trim().is_empty())
            .collect();
    resource_templates.sort_by(|left, right| left.uri_template.cmp(&right.uri_template));
    resource_templates.dedup_by(|left, right| left.uri_template == right.uri_template);
    Ok(resource_templates)
}

fn flush_mcp_sse_event(
    event_name: &str,
    data_lines: &[String],
) -> Result<Option<Value>, McpRuntimeError> {
    if data_lines.is_empty() {
        return Ok(None);
    }
    if !event_name.is_empty() && event_name != "message" {
        return Ok(None);
    }
    let data = data_lines.join("\n");
    if data == "[DONE]" {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&data)?))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;

    use super::*;

    #[test]
    fn t18_validates_stdio_and_streamable_http_configs() {
        let set = McpConfigSet {
            servers: vec![
                McpServerConfig {
                    id: "stdio".to_owned(),
                    name: "Filesystem".to_owned(),
                    scope: McpConfigScope::Global,
                    enabled: true,
                    transport: McpTransportConfig::Stdio {
                        command: "mcp-fs".to_owned(),
                        args: vec!["--root".to_owned(), ".".to_owned()],
                        cwd: Some(".".to_owned()),
                        env: BTreeMap::new(),
                    },
                    timeout_ms: 30_000,
                    always_allow_tools: BTreeSet::from(["read_file".to_owned()]),
                    disabled_tools: BTreeSet::from(["delete_file".to_owned()]),
                    oauth: None,
                },
                McpServerConfig {
                    id: "http".to_owned(),
                    name: "Remote MCP".to_owned(),
                    scope: McpConfigScope::Project {
                        project_folder_id: "project".to_owned(),
                    },
                    enabled: true,
                    transport: McpTransportConfig::StreamableHttp {
                        url: "https://mcp.example.test/mcp".to_owned(),
                        headers: BTreeMap::new(),
                    },
                    timeout_ms: 30_000,
                    always_allow_tools: BTreeSet::new(),
                    disabled_tools: BTreeSet::new(),
                    oauth: Some(McpOAuthConfig {
                        client_id: "fastrock".to_owned(),
                        client_secret_ref: "secret:mcp:remote".to_owned(),
                        authorization_url: "https://mcp.example.test/oauth/authorize".to_owned(),
                        token_url: "https://mcp.example.test/oauth/token".to_owned(),
                        scopes: vec!["tools".to_owned()],
                    }),
                },
            ],
        };

        assert_eq!(set.validate(), Ok(()));
    }

    #[test]
    fn t18_rejects_invalid_configs() {
        assert_eq!(
            McpConfigSet {
                servers: vec![McpServerConfig {
                    id: "bad".to_owned(),
                    name: "Bad".to_owned(),
                    scope: McpConfigScope::Global,
                    enabled: true,
                    transport: McpTransportConfig::Stdio {
                        command: String::new(),
                        args: Vec::new(),
                        cwd: None,
                        env: BTreeMap::new(),
                    },
                    timeout_ms: 30_000,
                    always_allow_tools: BTreeSet::new(),
                    disabled_tools: BTreeSet::new(),
                    oauth: None,
                }]
            }
            .validate(),
            Err(McpConfigError::MissingStdioCommand("bad".to_owned()))
        );

        let mut conflicting = server("policy", true, "mcp-policy");
        conflicting
            .always_allow_tools
            .insert("dangerous".to_owned());
        conflicting.disabled_tools.insert("dangerous".to_owned());
        assert_eq!(
            conflicting.validate(),
            Err(McpConfigError::ConflictingToolPolicy {
                id: "policy".to_owned(),
                tool_name: "dangerous".to_owned()
            })
        );

        let mut invalid_timeout = server("timeout", true, "mcp-timeout");
        invalid_timeout.timeout_ms = 0;
        assert_eq!(
            invalid_timeout.validate(),
            Err(McpConfigError::InvalidTimeout("timeout".to_owned()))
        );
    }

    #[test]
    fn v14_config_diff_identifies_hot_reload_changes() {
        let old = McpConfigSet {
            servers: vec![server("a", true, "cmd-a"), server("b", true, "cmd-b")],
        };
        let next = McpConfigSet {
            servers: vec![server("a", false, "cmd-a"), server("c", true, "cmd-c")],
        };

        assert_eq!(
            old.diff_for_hot_reload(&next).unwrap(),
            vec![
                McpConfigChange::Disabled("a".to_owned()),
                McpConfigChange::Removed("b".to_owned()),
                McpConfigChange::Added("c".to_owned()),
            ]
        );
    }

    #[test]
    fn t18_gui_config_edits_add_update_remove_and_toggle_servers() {
        let mut set = McpConfigSet::default();

        set.add_server(server("b", true, "cmd-b")).unwrap();
        set.add_server(server("a", false, "cmd-a")).unwrap();
        assert_eq!(
            set.servers
                .iter()
                .map(|server| server.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(
            set.add_server(server("a", true, "cmd-a2")),
            Err(McpConfigError::DuplicateId("a".to_owned()))
        );

        let mut updated = server("a", true, "cmd-a2");
        updated.name = "A updated".to_owned();
        set.update_server(updated).unwrap();
        set.set_server_enabled("b", false).unwrap();

        assert_eq!(set.server("a").unwrap().name, "A updated");
        assert!(!set.server("b").unwrap().enabled);
        assert_eq!(set.remove_server("b").unwrap().id, "b");
        assert_eq!(
            set.remove_server("missing"),
            Err(McpConfigError::ServerNotFound("missing".to_owned()))
        );
    }

    #[test]
    fn t18_gui_tool_policy_toggles_keep_allow_and_disable_sets_exclusive() {
        let mut set = McpConfigSet {
            servers: vec![server("tools", true, "mcp-tools")],
        };

        set.set_tool_always_allowed("tools", "read_file", true)
            .unwrap();
        set.set_tool_disabled("tools", "read_file", true).unwrap();

        let server = set.server("tools").unwrap();
        assert!(!server.always_allow_tools.contains("read_file"));
        assert!(server.disabled_tools.contains("read_file"));

        set.set_tool_disabled("tools", "read_file", false).unwrap();
        set.set_tool_always_allowed("tools", "read_file", true)
            .unwrap();
        let server = set.server("tools").unwrap();
        assert!(server.always_allow_tools.contains("read_file"));
        assert!(!server.disabled_tools.contains("read_file"));
        assert_eq!(
            set.set_tool_disabled("tools", "", true),
            Err(McpConfigError::InvalidToolName {
                id: "tools".to_owned(),
                tool_name: String::new(),
            })
        );
    }

    #[test]
    fn t18_failed_gui_config_edit_leaves_prior_valid_config_intact() {
        let mut set = McpConfigSet {
            servers: vec![server("a", true, "cmd-a")],
        };
        let mut invalid = server("a", true, "");
        invalid.name = "Broken".to_owned();

        assert_eq!(
            set.update_server(invalid),
            Err(McpConfigError::MissingStdioCommand("a".to_owned()))
        );
        assert_eq!(set.server("a").unwrap().name, "a");
        assert_eq!(
            set.server("a").unwrap().transport,
            McpTransportConfig::Stdio {
                command: "cmd-a".to_owned(),
                args: Vec::new(),
                cwd: None,
                env: BTreeMap::new(),
            }
        );
    }

    #[test]
    fn t18_config_json_round_trips_with_stable_transport_type() {
        let set = McpConfigSet {
            servers: vec![server("stdio", true, "mcp-fs")],
        };

        let json = serde_json::to_string(&set).unwrap();
        let decoded: McpConfigSet = serde_json::from_str(&json).unwrap();

        assert!(json.contains(r#""type":"stdio""#));
        assert!(json.contains(r#""scope":{"type":"global"}"#));
        assert!(json.contains(r#""timeout_ms":30000"#));
        assert_eq!(decoded, set);
    }

    #[test]
    fn t18_legacy_config_json_deserializes_with_safe_defaults() {
        let json = r#"{
            "servers": [{
                "id": "legacy",
                "name": "Legacy MCP",
                "enabled": true,
                "transport": {
                    "type": "stdio",
                    "command": "mcp-legacy",
                    "args": [],
                    "cwd": null,
                    "env": {}
                },
                "oauth": null
            }]
        }"#;

        let decoded: McpConfigSet = serde_json::from_str(json).unwrap();
        let server = &decoded.servers[0];

        assert_eq!(server.scope, McpConfigScope::Global);
        assert_eq!(server.timeout_ms, default_mcp_timeout_ms());
        assert!(server.always_allow_tools.is_empty());
        assert!(server.disabled_tools.is_empty());
        assert_eq!(decoded.validate(), Ok(()));
    }

    #[test]
    fn t18_json_rpc_frames_round_trip_with_content_length() {
        let value = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "capabilities": {}
            }
        });

        let frame = encode_mcp_json_rpc_frame(&value).unwrap();
        let decoded = decode_mcp_json_rpc_frame(&frame).unwrap();

        assert!(String::from_utf8_lossy(&frame).starts_with("Content-Length: "));
        assert_eq!(decoded, value);
    }

    #[test]
    fn t18_discovery_parsers_return_sorted_deduped_descriptors() {
        let tools = parse_mcp_tools(serde_json::json!({
            "tools": [
                {"name": "write_file", "description": "Write"},
                {"name": ""},
                {"name": "read_file", "description": "Read"},
                {"name": "read_file", "description": "Duplicate"}
            ]
        }))
        .unwrap();
        let resources = parse_mcp_resources(serde_json::json!({
            "resources": [
                {"uri": "file://b", "name": "B", "mimeType": "text/plain"},
                {"uri": "file://a", "description": "A"},
                {"uri": ""}
            ]
        }))
        .unwrap();
        let templates = parse_mcp_resource_templates(serde_json::json!({
            "resourceTemplates": [
                {"uriTemplate": "file://{path}", "name": "File"},
                {"uri_template": "repo://{owner}/{repo}", "description": "Repo"},
                {"uriTemplate": ""}
            ]
        }))
        .unwrap();

        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["read_file", "write_file"]
        );
        assert_eq!(
            resources
                .iter()
                .map(|resource| resource.uri.as_str())
                .collect::<Vec<_>>(),
            vec!["file://a", "file://b"]
        );
        assert_eq!(resources[1].mime_type.as_deref(), Some("text/plain"));
        assert_eq!(
            templates
                .iter()
                .map(|template| template.uri_template.as_str())
                .collect::<Vec<_>>(),
            vec!["file://{path}", "repo://{owner}/{repo}"]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn t18_stdio_client_starts_server_and_reads_json_rpc_frame() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempdir().unwrap();
        let script_path = temp_dir.path().join("fake-mcp-server.sh");
        let response = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18"}}"#;
        fs::write(
            &script_path,
            format!(
                "#!/bin/sh\ndd bs=1 count=1 >/dev/null 2>/dev/null\nprintf 'Content-Length: {}\\r\\n\\r\\n{}'\nsleep 1\n",
                response.len(),
                response
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).unwrap();
        let mut client = McpStdioClient::start(&McpServerConfig {
            id: "stdio".to_owned(),
            name: "stdio".to_owned(),
            scope: McpConfigScope::Global,
            enabled: true,
            transport: McpTransportConfig::Stdio {
                command: script_path.to_string_lossy().into_owned(),
                args: Vec::new(),
                cwd: Some(temp_dir.path().to_string_lossy().into_owned()),
                env: BTreeMap::new(),
            },
            timeout_ms: 30_000,
            always_allow_tools: BTreeSet::new(),
            disabled_tools: BTreeSet::new(),
            oauth: None,
        })
        .await
        .unwrap();

        let result = client
            .initialize("2025-06-18", "fastrock-test")
            .await
            .unwrap();

        assert_eq!(result["protocolVersion"], "2025-06-18");
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn t18_streamable_http_client_posts_json_rpc() {
        let server = OneShotHttpServer::start(
            200,
            "application/json",
            r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#,
        )
        .await;
        let mut headers = BTreeMap::new();
        headers.insert("authorization".to_owned(), "Bearer token".to_owned());
        let mut client = McpStreamableHttpClient::new(&McpServerConfig {
            id: "http".to_owned(),
            name: "http".to_owned(),
            scope: McpConfigScope::Global,
            enabled: true,
            transport: McpTransportConfig::StreamableHttp {
                url: format!("{}/mcp", server.base_url),
                headers,
            },
            timeout_ms: 30_000,
            always_allow_tools: BTreeSet::new(),
            disabled_tools: BTreeSet::new(),
            oauth: None,
        })
        .unwrap();

        let result = client.request("tools/list", None).await.unwrap();
        let request = server.request().await;
        let request_lower = request.to_ascii_lowercase();

        assert_eq!(result["tools"], serde_json::json!([]));
        assert!(request.starts_with("POST /mcp HTTP/1.1"));
        assert!(request_lower.contains("authorization: bearer token"));
        assert!(request_lower.contains("mcp-protocol-version: 2025-06-18"));
        assert!(request.contains(r#""method":"tools/list""#));
    }

    #[tokio::test]
    async fn t18_streamable_http_client_lists_tools_with_typed_discovery() {
        let server = OneShotHttpServer::start(
            200,
            "application/json",
            r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"read_file","description":"Read files"},{"name":"write_file"}]}}"#,
        )
        .await;
        let mut client = McpStreamableHttpClient::new(&McpServerConfig {
            id: "http-tools".to_owned(),
            name: "http-tools".to_owned(),
            scope: McpConfigScope::Global,
            enabled: true,
            transport: McpTransportConfig::StreamableHttp {
                url: format!("{}/mcp", server.base_url),
                headers: BTreeMap::new(),
            },
            timeout_ms: 30_000,
            always_allow_tools: BTreeSet::new(),
            disabled_tools: BTreeSet::new(),
            oauth: None,
        })
        .unwrap();

        let tools = client.list_tools().await.unwrap();
        let request = server.request().await;

        assert_eq!(
            tools,
            vec![
                McpToolDescriptor {
                    name: "read_file".to_owned(),
                    description: Some("Read files".to_owned()),
                },
                McpToolDescriptor {
                    name: "write_file".to_owned(),
                    description: None,
                },
            ]
        );
        assert!(request.contains(r#""method":"tools/list""#));
    }

    #[tokio::test]
    async fn t18_streamable_http_client_uses_configured_timeout() {
        let server = OneShotHttpServer::start_with_delay(
            200,
            "application/json",
            r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#,
            100,
        )
        .await;
        let mut client = McpStreamableHttpClient::new(&McpServerConfig {
            id: "slow".to_owned(),
            name: "slow".to_owned(),
            scope: McpConfigScope::Global,
            enabled: true,
            transport: McpTransportConfig::StreamableHttp {
                url: format!("{}/mcp", server.base_url),
                headers: BTreeMap::new(),
            },
            timeout_ms: 1,
            always_allow_tools: BTreeSet::new(),
            disabled_tools: BTreeSet::new(),
            oauth: None,
        })
        .unwrap();

        assert_eq!(
            client.request("tools/list", None).await,
            Err(McpRuntimeError::Timeout {
                id: "slow".to_owned(),
                timeout_ms: 1
            })
        );
    }

    #[test]
    fn t18_oauth_pkce_builds_authorization_url_without_plain_verifier() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = McpPkceChallenge::from_verifier(verifier);
        let oauth = McpOAuthConfig {
            client_id: "fastrock".to_owned(),
            client_secret_ref: "secret:mcp:http".to_owned(),
            authorization_url: "https://mcp.example.test/oauth/authorize".to_owned(),
            token_url: "https://mcp.example.test/oauth/token".to_owned(),
            scopes: vec!["tools".to_owned(), "resources".to_owned()],
        };

        let url = challenge
            .authorization_url(&oauth, "http://127.0.0.1/callback", "state-1")
            .unwrap();

        assert_eq!(
            challenge.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert!(url.contains("client_id=fastrock"));
        assert!(url.contains("code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(!url.contains(verifier));
    }

    #[test]
    fn t18_oauth_token_set_debug_redacts_token_material() {
        let tokens = McpOAuthTokenSet {
            access_token: "not-a-real-access-token".to_owned(),
            refresh_token: Some("not-a-real-refresh-token".to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at_ms: Some(100),
            scopes: vec!["tools".to_owned()],
        };
        let debug = format!("{tokens:?}");

        assert!(debug.contains("Bearer"));
        assert!(!debug.contains("not-a-real-access-token"));
        assert!(!debug.contains("not-a-real-refresh-token"));
    }

    #[test]
    fn t18_oauth_callback_rejects_state_mismatch() {
        let error = McpOAuthCallback::from_redirect_url(
            "oauth-server",
            "http://127.0.0.1/callback?code=auth-code&state=wrong",
            "state-1",
        )
        .unwrap_err();

        assert_eq!(
            error,
            McpRuntimeError::OAuth {
                id: "oauth-server".to_owned(),
                message: "callback state mismatch".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn t18_oauth_fake_callback_flow_exchanges_code_for_tokens() {
        let server = OneShotHttpServer::start(
            200,
            "application/json",
            r#"{"access_token":"not-a-real-access-token","refresh_token":"not-a-real-refresh-token","token_type":"Bearer","expires_in":60,"scope":"tools resources"}"#,
        )
        .await;
        let oauth = McpOAuthConfig {
            client_id: "fastrock".to_owned(),
            client_secret_ref: "secret:mcp:http".to_owned(),
            authorization_url: "https://mcp.example.test/oauth/authorize".to_owned(),
            token_url: format!("{}/oauth/token", server.base_url),
            scopes: vec!["tools".to_owned(), "resources".to_owned()],
        };
        let pkce = McpPkceChallenge::from_verifier("not-a-real-verifier");
        let callback = McpOAuthCallback::from_redirect_url(
            "oauth-server",
            "http://127.0.0.1/callback?code=auth-code-1&state=state-1",
            "state-1",
        )
        .unwrap();
        let request = McpOAuthTokenExchangeRequest::authorization_code(
            &oauth,
            &callback,
            "http://127.0.0.1/callback",
            &pkce,
            "not-a-real-client-secret",
        )
        .unwrap();
        let request_debug = format!("{request:?}");

        assert!(!request_debug.contains("auth-code-1"));
        assert!(!request_debug.contains("not-a-real-verifier"));
        assert!(!request_debug.contains("not-a-real-client-secret"));

        let tokens =
            exchange_mcp_oauth_code(&reqwest::Client::new(), "oauth-server", &request, 1_000)
                .await
                .unwrap();
        let raw_request = server.request().await;

        assert!(raw_request.contains("POST /oauth/token HTTP/1.1"));
        assert!(raw_request.contains("grant_type=authorization_code"));
        assert!(raw_request.contains("code=auth-code-1"));
        assert!(raw_request.contains("code_verifier=not-a-real-verifier"));
        assert!(raw_request.contains("client_secret=not-a-real-client-secret"));
        assert_eq!(tokens.access_token, "not-a-real-access-token");
        assert_eq!(
            tokens.refresh_token.as_deref(),
            Some("not-a-real-refresh-token")
        );
        assert_eq!(tokens.token_type, "Bearer");
        assert_eq!(tokens.expires_at_ms, Some(61_000));
        assert_eq!(tokens.scopes, vec!["tools", "resources"]);
    }

    struct OneShotHttpServer {
        base_url: String,
        request_rx: oneshot::Receiver<String>,
    }

    impl OneShotHttpServer {
        async fn start(status: u16, content_type: &'static str, body: &'static str) -> Self {
            Self::start_with_delay(status, content_type, body, 0).await
        }

        async fn start_with_delay(
            status: u16,
            content_type: &'static str,
            body: &'static str,
            response_delay_ms: u64,
        ) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (request_tx, request_rx) = oneshot::channel();
            tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_http_request(&mut stream).await;
                let _ = request_tx.send(request);
                if response_delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(response_delay_ms)).await;
                }
                let reason = match status {
                    200 => "OK",
                    400 => "Bad Request",
                    404 => "Not Found",
                    _ => "Status",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            });
            Self {
                base_url: format!("http://{address}"),
                request_rx,
            }
        }

        async fn request(self) -> String {
            self.request_rx.await.unwrap()
        }
    }

    async fn read_http_request(stream: &mut TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0; 1024];
        loop {
            let bytes_read = stream.read(&mut chunk).await.unwrap();
            if bytes_read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..bytes_read]);
            let Some(headers_end) = http_headers_end(&buffer) else {
                continue;
            };
            let headers = String::from_utf8_lossy(&buffer[..headers_end]);
            let content_length = http_content_length(&headers);
            if buffer.len() >= headers_end + 4 + content_length {
                break;
            }
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }

    fn http_headers_end(buffer: &[u8]) -> Option<usize> {
        buffer.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn http_content_length(headers: &str) -> usize {
        headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())
                    .flatten()
            })
            .unwrap_or(0)
    }

    #[test]
    fn t18_runtime_snapshot_tracks_discovery_oauth_and_last_error() {
        let mut server = server("http", true, "mcp-http");
        server.oauth = Some(McpOAuthConfig {
            client_id: "fastrock".to_owned(),
            client_secret_ref: "secret:mcp:http".to_owned(),
            authorization_url: "https://mcp.example.test/oauth/authorize".to_owned(),
            token_url: "https://mcp.example.test/oauth/token".to_owned(),
            scopes: vec!["tools".to_owned()],
        });

        let snapshot = McpRuntimeSnapshot::stopped(&server, 10)
            .with_discovery(
                ["write_file", "read_file", "read_file"],
                ["file://README.md"],
                ["file://{path}"],
            )
            .mark_running(11)
            .mark_failed("connection reset", Some("stderr line".to_owned()), 12);

        assert_eq!(snapshot.server_id, "http");
        assert_eq!(snapshot.status.label(), "failed");
        assert_eq!(
            snapshot.status,
            McpServerRuntimeStatus::Failed("connection reset".to_owned())
        );
        assert_eq!(snapshot.last_error.as_deref(), Some("connection reset"));
        assert_eq!(snapshot.log_excerpt.as_deref(), Some("stderr line"));
        assert_eq!(snapshot.discovered_tools, vec!["read_file", "write_file"]);
        assert_eq!(snapshot.oauth_state, McpOAuthState::AuthorizationRequired);
        assert_eq!(snapshot.updated_at_ms, 12);
    }

    #[test]
    fn t18_runtime_snapshot_set_upserts_and_removes_statuses() {
        let server = server("filesystem", true, "mcp-fs");
        let snapshot = McpRuntimeSnapshot::stopped(&server, 10).mark_running(11);
        let mut set = McpRuntimeSnapshotSet::default();

        set.upsert(snapshot.clone());
        set.upsert(snapshot.mark_failed("boom", None, 12));

        assert_eq!(
            set.get("filesystem").unwrap().status,
            McpServerRuntimeStatus::Failed("boom".to_owned())
        );
        assert_eq!(set.list().len(), 1);
        assert!(set.remove("filesystem"));
        assert!(set.get("filesystem").is_none());
        assert!(!set.remove("filesystem"));
    }

    fn server(id: &str, enabled: bool, command: &str) -> McpServerConfig {
        McpServerConfig {
            id: id.to_owned(),
            name: id.to_owned(),
            scope: McpConfigScope::Global,
            enabled,
            transport: McpTransportConfig::Stdio {
                command: command.to_owned(),
                args: Vec::new(),
                cwd: None,
                env: BTreeMap::new(),
            },
            timeout_ms: 30_000,
            always_allow_tools: BTreeSet::new(),
            disabled_tools: BTreeSet::new(),
            oauth: None,
        }
    }
}
