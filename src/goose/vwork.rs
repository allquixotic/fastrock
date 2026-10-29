//! Reuse the vwork owner through its authenticated MCP API, without per-chat bridges.
use super::*;
use fastrock_workspaces::vwork::{DiscoveryPaths, McpBinding, discover_mcp};

impl Adapter {
    pub(super) async fn new_vwork_servers(&self) -> (Vec<Value>, Vec<McpBinding>) {
        if self.capabilities.lock().await["mcpCapabilities"]["http"] != true {
            return (vec![], vec![]);
        }
        let bindings = discover_mcp(&DiscoveryPaths::current())
            .await
            .unwrap_or_default();
        self.vwork_servers(bindings).await
    }

    async fn vwork_servers(&self, bindings: Vec<McpBinding>) -> (Vec<Value>, Vec<McpBinding>) {
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
                    tracing::warn!(%error, "Shared vwork unavailable; retain Goose's configured transport")
                }
            }
        }
        (servers, shared)
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
    ) -> Result<Vec<Value>, JSONRPCErrorError> {
        if self.capabilities.lock().await["mcpCapabilities"]["http"] != true {
            return Ok(vec![]);
        }
        let configured = discover_mcp(&DiscoveryPaths::current())
            .await
            .unwrap_or_default();
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
            if extension["type"] != "mcp"
                || extension["envKeys"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty())
                || extension["availableTools"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty())
            {
                continue;
            }
            let server = &extension["server"];
            if let Some(binding) = configured
                .iter()
                .find(|binding| binding.matches_stdio(server))
            {
                bindings.push(binding.clone());
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
                _ if saved.contains(&binding) => {
                    // A downgraded/unavailable daemon can still use the original stdio
                    // definition. Never leave a stale HTTP route in a saved conversation.
                    servers.push(json!({"name":binding.name,"command":binding.executable,"args":binding.args,"env":[],"_meta":{"timeout":binding.timeout_seconds}}));
                }
                _ => {}
            }
        }
        self.save_vwork_bindings(id, &persisted)?;
        Ok(servers)
    }
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
