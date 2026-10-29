//! Session-scoped Streamable HTTP MCP for the existing Fastrock tools.
//! This bridge owns no Rally credentials and executes no writes itself.
use super::*;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use fastrock_workspaces::vwork::{McpBinding, VworkProvider};
use std::sync::Weak;

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;

#[derive(Clone)]
struct Route {
    adapter: Weak<Adapter>,
    tools: Vec<Value>,
    session: Option<String>,
    vwork: Option<Arc<VworkRoute>>,
}
struct VworkRoute {
    binding: McpBinding,
    client: VworkProvider,
}
#[derive(Clone)]
struct Shared {
    routes: Arc<Mutex<HashMap<String, Route>>>,
    authority: String,
}
pub(super) struct Bridge {
    state: Shared,
    task: tokio::task::JoinHandle<()>,
    vwork: Mutex<Vec<(McpBinding, String)>>,
}
pub(super) struct Registration {
    pub key: String,
    url: String,
}
impl Registration {
    pub fn server(&self) -> Value {
        json!({"type":"http","name":"fastrock","url":self.url,"headers":[]})
    }
}
impl Bridge {
    pub async fn new() -> anyhow::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let authority = listener.local_addr()?.to_string();
        let state = Shared {
            routes: Arc::new(Mutex::new(HashMap::new())),
            authority,
        };
        let router = Router::new()
            .route("/{key}", post(handle))
            .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Ok(Self {
            state,
            task,
            vwork: Mutex::new(Vec::new()),
        })
    }
    pub async fn register(&self, adapter: Weak<Adapter>, tools: Vec<Value>) -> Registration {
        let key = uuid::Uuid::new_v4().to_string();
        self.state.routes.lock().await.insert(
            key.clone(),
            Route {
                adapter,
                tools: flatten_tools(tools),
                session: None,
                vwork: None,
            },
        );
        Registration {
            url: format!("http://{}/{key}", self.state.authority),
            key,
        }
    }
    /// One route per binding; all sessions call the same authenticated database owner.
    pub async fn vwork_server(&self, binding: &McpBinding) -> anyhow::Result<Option<Value>> {
        let mut shared = self.vwork.lock().await;
        let client = VworkProvider::new(&binding.installation())?;
        anyhow::ensure!(
            client.shared_mcp_supported().await?,
            "Embedded vwork MCP unavailable"
        );
        let key = if let Some((_, key)) = shared.iter().find(|(b, _)| b == binding) {
            key.clone()
        } else {
            let key = uuid::Uuid::new_v4().to_string();
            self.state.routes.lock().await.insert(
                key.clone(),
                Route {
                    adapter: Weak::new(),
                    tools: vec![],
                    session: None,
                    vwork: Some(Arc::new(VworkRoute {
                        binding: binding.clone(),
                        client,
                    })),
                },
            );
            shared.push((binding.clone(), key.clone()));
            key
        };
        Ok(Some(
            json!({"type":"http","name":binding.name,"url":format!("http://{}/{key}", self.state.authority),"headers":[],"_meta":{"timeout":binding.timeout_seconds}}),
        ))
    }
    pub async fn bind(&self, key: &str, session: &str) {
        if let Some(route) = self.state.routes.lock().await.get_mut(key) {
            route.session = Some(session.into());
        }
    }
    pub async fn remove(&self, key: &str) {
        self.state.routes.lock().await.remove(key);
    }
    pub async fn remove_session(&self, id: &str) {
        self.state
            .routes
            .lock()
            .await
            .retain(|_, r| r.session.as_deref() != Some(id));
    }
    pub async fn servers(&self, id: &str) -> Vec<Value> {
        self.state
            .routes
            .lock()
            .await
            .iter()
            .filter(|(_, r)| r.session.as_deref() == Some(id))
            .map(|(key, _)| {
                Registration {
                    key: key.clone(),
                    url: format!("http://{}/{key}", self.state.authority),
                }
                .server()
            })
            .collect()
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle(
    State(state): State<Shared>,
    Path(key): Path<String>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    // Not a browser API. Bind both Host and a high-entropy per-session route.
    if headers.contains_key("origin")
        || headers.get("host").and_then(|h| h.to_str().ok()) != Some(state.authority.as_str())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let route = state.routes.lock().await.get(&key).cloned();
    let Some(route) = route else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if request.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    if let Some(vwork) = &route.vwork {
        let response = match vwork
            .client
            .mcp_request(
                vwork.binding.workspace.as_deref(),
                request.clone(),
                std::time::Duration::from_secs(vwork.binding.timeout_seconds),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":error.to_string()}})
            }
        };
        return Json(response).into_response();
    }
    let result = dispatch(&route, &request).await;
    let response = match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
        Err(e) => json!({"jsonrpc":"2.0","id":request["id"],"error":e}),
    };
    Json(response).into_response()
}
async fn dispatch(route: &Route, request: &Value) -> Reply {
    match text(request, "method") {
        "initialize" => Ok(
            json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"fastrock","version":env!("CARGO_PKG_VERSION")}}),
        ),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(
            json!({"tools":route.tools.iter().map(|t|json!({"name":t["name"],"description":t["description"],"inputSchema":t["inputSchema"]})).collect::<Vec<_>>()}),
        ),
        "tools/call" => {
            let adapter = route
                .adapter
                .upgrade()
                .ok_or_else(|| error("Fastrock closed"))?;
            let session = route
                .session
                .as_deref()
                .ok_or_else(|| error("Goose session is still starting"))?;
            let params = &request["params"];
            let name = text(params, "name");
            let tool = route
                .tools
                .iter()
                .find(|t| t["name"] == name)
                .ok_or_else(|| error("Tool not registered for this conversation"))?;
            let turn_id = adapter
                .sessions
                .lock()
                .await
                .get(session)
                .and_then(|s| s.active.clone())
                .ok_or_else(|| error("No active prompt for this tool call"))?;
            let call_id = format!("goose-tool-{}", uuid::Uuid::new_v4());
            let (tx, rx) = oneshot::channel();
            adapter.answers.lock().await.insert(
                call_id.clone(),
                Answer::Tool {
                    session: session.into(),
                    result: tx,
                },
            );
            if let Err(e)=adapter.emit(json!({"id":call_id,"method":"item/tool/call","params":{"threadId":ui_id(session),"turnId":turn_id,"callId":call_id,"namespace":tool["namespace"],"tool":tool["originalName"],"arguments":params["arguments"]}})) {
                adapter.answers.lock().await.remove(&call_id);return Err(e)
            }
            // Do not keep the adapter alive while its UI owns this pending reply.
            drop(adapter);
            let reply = rx.await.map_err(|_| error("Tool call cancelled"))?;
            if !reply["error"].is_null() {
                return Err(error(text(&reply["error"], "message")));
            }
            let result = &reply["result"];
            let content: Vec<Value> = result["contentItems"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| {
                    if c["type"] == "inputText" {
                        json!({"type":"text","text":c["text"]})
                    } else {
                        json!({"type":"text","text":"Unsupported tool output media"})
                    }
                })
                .collect();
            Ok(json!({"content":content,"isError":result["success"]!=true}))
        }
        other => Err(unsupported(other)),
    }
}

fn flatten_tools(tools: Vec<Value>) -> Vec<Value> {
    tools
        .into_iter()
        .flat_map(|tool| {
            if let Some(children) = tool["tools"].as_array() {
                children
                    .iter()
                    .map(|child| {
                        let mut child = child.clone();
                        child["namespace"] = tool["name"].clone();
                        child["originalName"] = child["name"].clone();
                        child["name"] =
                            json!(format!("{}__{}", text(&tool, "name"), text(&child, "name")));
                        child
                    })
                    .collect::<Vec<_>>()
            } else {
                let mut tool = tool;
                tool["originalName"] = tool["name"].clone();
                vec![tool]
            }
        })
        .collect()
}
