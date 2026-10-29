//! Map legacy and new vwork sessions to the embedded owner before activation.
use super::*;
use fastrock_workspaces::vwork::{McpBinding, discover_mcp};

impl Adapter {
    pub(super) async fn new_vwork_servers(
        &self,
        cwd: &str,
    ) -> Result<(Vec<Value>, Vec<McpBinding>), JSONRPCErrorError> {
        if self.capabilities.lock().await["mcpCapabilities"]["http"] != true {
            return Ok((vec![], vec![]));
        }
        let bindings = self.configured_vwork(cwd).await?;
        self.vwork_servers(bindings).await
    }

    async fn configured_vwork(&self, cwd: &str) -> Result<Vec<McpBinding>, JSONRPCErrorError> {
        if !self.embedded_runtime {
            return discover_mcp(&self.vwork_paths)
                .await
                .map_err(|e| error(e.to_string()));
        }
        super::hooks::prepare(std::path::PathBuf::from(cwd))
            .await
            .map_err(|_| error("Unable to prepare embedded vwork hooks"))?;
        configured_embedded(self.vwork_paths.clone(), std::path::PathBuf::from(cwd)).await
    }

    async fn vwork_servers(
        &self,
        bindings: Vec<McpBinding>,
    ) -> Result<(Vec<Value>, Vec<McpBinding>), JSONRPCErrorError> {
        let mut servers = Vec::new();
        let mut shared = Vec::new();
        for binding in bindings {
            match self.mcp.vwork_server(&binding).await {
                Ok(Some(server)) => {
                    servers.push(server);
                    shared.push(binding);
                }
                Ok(None) => {}
                Err(error) => {
                    return Err(super::error(format!("Embedded vwork unavailable: {error}")));
                }
            }
        }
        Ok((servers, shared))
    }

    pub(super) fn save_vwork_bindings(
        &self,
        id: &str,
        bindings: &[McpBinding],
    ) -> Result<(), JSONRPCErrorError> {
        if bindings.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.queue_root).map_err(|e| error(e.to_string()))?;
        let bytes = serde_json::to_vec(bindings).map_err(|e| error(e.to_string()))?;
        crate::rally::store::atomic_write(&self.queue_path(id).with_extension("vwork.json"), &bytes)
            .map_err(|e| error(format!("Save vwork session bindings: {e}")))
    }

    pub(super) async fn load_vwork_servers(
        &self,
        id: &str,
        cwd: &str,
    ) -> Result<Vec<Value>, JSONRPCErrorError> {
        if self.capabilities.lock().await["mcpCapabilities"]["http"] != true {
            return Ok(vec![]);
        }
        let configured = self.configured_vwork(cwd).await?;
        let path = self.queue_path(id).with_extension("vwork.json");
        let saved: Vec<McpBinding> = match tokio::fs::metadata(&path).await {
            Ok(meta) => {
                if meta.len() > 1024 * 1024 {
                    return Err(error("Stored vwork bindings exceed 1 MiB"));
                }
                serde_json::from_slice(
                    &tokio::fs::read(&path)
                        .await
                        .map_err(|e| error(e.to_string()))?,
                )
                .map_err(|e| error(format!("Invalid vwork session bindings: {e}")))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(error(e.to_string())),
        };
        if configured.is_empty() && saved.is_empty() {
            return Ok(vec![]);
        }
        // Query persisted session configuration before activation. Never add an extension
        // merely because it is globally enabled: restricted/older sessions retain their set.
        let selection = match self
            .rpc
            .call(
                "_goose/unstable/session/extensions/list",
                json!({"sessionId":id}),
            )
            .await
        {
            Ok(selection) => selection,
            Err(_) if saved.is_empty() => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        let mut bindings = Vec::new();
        for entry in selection["extensions"].as_array().into_iter().flatten() {
            let extension = &entry["extension"];
            let server = &extension["server"];
            if extension["type"] != "mcp"
                || extension["envKeys"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty())
                || extension["availableTools"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty())
            {
                if is_vwork_stdio(server) {
                    return Err(error(
                        "This session has unsupported vwork environment or tool restrictions; update its extension before resuming",
                    ));
                }
                continue;
            }
            if let Some(binding) = configured
                .iter()
                .find(|binding| binding.matches_stdio(server))
            {
                bindings.push(binding.clone());
            } else if is_vwork_stdio(server) {
                return Err(error(
                    "This session's vwork configuration cannot be mapped to the embedded store; update its vwork extension before resuming",
                ));
            } else if local_shared_server(server)
                && let Some(binding) = saved.iter().find(|binding| server["name"] == binding.name)
            {
                // These definitions were saved when Fastrock replaced this session's
                // stdio entry. Keep its original workspace even if new-chat defaults change.
                bindings.push(binding.clone());
            }
        }
        let mut servers = Vec::new();
        let mut persisted = Vec::new();
        for binding in bindings {
            match self.mcp.vwork_server(&binding).await {
                Ok(Some(server)) => {
                    servers.push(server);
                    persisted.push(binding);
                }
                Err(e) => return Err(error(format!("Embedded vwork unavailable: {e}"))),
                Ok(None) => return Err(error("Embedded vwork MCP unavailable")),
            }
        }
        self.save_vwork_bindings(id, &persisted)?;
        Ok(servers)
    }
}

fn is_vwork_stdio(server: &Value) -> bool {
    server["command"].as_str().is_some_and(|command| {
        command.rsplit(['/', '\\']).next().is_some_and(|binary| {
            binary.eq_ignore_ascii_case("vwork") || binary.eq_ignore_ascii_case("vwork.exe")
        })
    })
}

fn local_shared_server(server: &Value) -> bool {
    if server["type"] != "http" || !server["headers"].as_array().is_some_and(|h| h.is_empty()) {
        return false;
    }
    let Some(url) = server["url"]
        .as_str()
        .and_then(|url| reqwest::Url::parse(url).ok())
    else {
        return false;
    };
    url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port().is_some_and(|port| port != 0)
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && uuid::Uuid::parse_str(url.path().trim_start_matches('/')).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v35_restore_only_local_shared_routes() {
        let path = uuid::Uuid::new_v4();
        let server = |url: String| json!({"type":"http","name":"vwork","url":url,"headers":[]});
        assert!(local_shared_server(&server(format!(
            "http://127.0.0.1:12345/{path}"
        ))));
        for url in [
            format!("http://example.com:12345/{path}"),
            format!("http://127.0.0.1:12345@evil.test/{path}"),
            format!("http://localhost:12345/{path}"),
            format!("https://127.0.0.1:12345/{path}"),
            format!("http://127.0.0.1:12345/{path}?x=1"),
            "http://127.0.0.1:12345/other-mcp".into(),
        ] {
            assert!(!local_shared_server(&server(url.clone())), "{url}");
        }
    }
}

/// One public Goose configuration snapshot for both tools and search storage.
pub(super) async fn configured_embedded(
    paths: fastrock_workspaces::vwork::DiscoveryPaths,
    cwd: std::path::PathBuf,
) -> Result<Vec<McpBinding>, JSONRPCErrorError> {
    tokio::task::spawn_blocking(move || {
        let mut entries = serde_json::Map::new();
        for entry in goose_runtime::config::extensions::get_all_extensions() {
            let mut value = serde_json::to_value(&entry.config)
                .map_err(|_| error("Invalid Goose extension"))?;
            value["enabled"] = json!(entry.enabled);
            entries.insert(entry.config.name(), value);
        }
        for extension in goose_runtime::plugins::mcp_servers::enabled_plugin_mcp_servers(Some(&cwd))
        {
            let mut value = serde_json::to_value(&extension)
                .map_err(|_| error("Invalid Goose plugin extension"))?;
            value["enabled"] = json!(true);
            entries.insert(extension.name(), value);
        }
        fastrock_workspaces::vwork::bindings_from_extensions(&paths, Value::Object(entries))
            .map_err(|e| error(e.to_string()))
    })
    .await
    .map_err(|_| error("Goose extension discovery failed"))?
}
