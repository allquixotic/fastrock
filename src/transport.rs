//! Bounded JSON-RPC transport to the user's installed Codex. No inference here.
use codex_app_server_protocol::{
    ClientRequest, JSONRPCErrorError, RequestId, ServerNotification, ServerRequest,
};
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
const MAX_FRAME: usize = 32 * 1024 * 1024;
pub(crate) const DEFAULT_IN_PROCESS_CHANNEL_CAPACITY: usize = 512;
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RemoteAppServerEndpoint {
    UnixSocket {
        socket_path: AbsolutePathBuf,
    },
    WebSocket {
        websocket_url: String,
        auth_token: Option<String>,
    },
}
pub(crate) fn app_server_control_socket_path(home: &Path) -> anyhow::Result<AbsolutePathBuf> {
    Ok(AbsolutePathBuf::from_absolute_path(
        home.join("app-server.sock"),
    )?)
}
#[derive(Clone, Debug)]
pub(crate) struct RemoteAppServerConnectArgs {
    pub endpoint: RemoteAppServerEndpoint,
    pub client_name: String,
    pub client_version: String,
    pub experimental_api: bool,
    pub mcp_server_openai_form_elicitation: bool,
    pub opt_out_notification_methods: Vec<String>,
    pub channel_capacity: usize,
}
#[derive(Clone, Debug)]
pub(crate) enum AppServerEvent {
    Lagged { skipped: usize },
    ServerNotification(Box<ServerNotification>),
    ServerRequest(Box<ServerRequest>),
    Disconnected { message: String },
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum TypedRequestError {
    #[error("{method} transport error: {source}")]
    Transport { method: String, source: io::Error },
    #[error("{method} failed: {}", rpc_error.message)]
    Server {
        method: String,
        rpc_error: JSONRPCErrorError,
    },
    #[error("{method} response decode error: {source}")]
    Deserialize {
        method: String,
        source: serde_json::Error,
    },
}
type Reply = Result<Value, JSONRPCErrorError>;
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Reply>>>>;
#[derive(Clone)]
pub(crate) struct AppServerRequestHandle {
    writer: mpsc::Sender<Value>,
    pending: Pending,
}
impl AppServerRequestHandle {
    pub async fn request_typed<T: DeserializeOwned>(
        &self,
        request: ClientRequest,
    ) -> Result<T, TypedRequestError> {
        let method = request.method_name().to_string();
        let value =
            serde_json::to_value(request).map_err(|source| TypedRequestError::Deserialize {
                method: method.clone(),
                source,
            })?;
        let result = self
            .raw(value)
            .await
            .map_err(|source| TypedRequestError::Transport {
                method: method.clone(),
                source,
            })?
            .map_err(|source| TypedRequestError::Server {
                method: method.clone(),
                rpc_error: source,
            })?;
        serde_json::from_value(result)
            .map_err(|source| TypedRequestError::Deserialize { method, source })
    }
    pub async fn raw(&self, value: Value) -> io::Result<Reply> {
        let id = value["id"].to_string();
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| io::Error::other("request registry unavailable"))?;
            if pending.contains_key(&id) {
                return Err(io::Error::other("Duplicate Codex request id"));
            }
            pending.insert(id.clone(), tx);
        }
        struct Cleanup {
            pending: Pending,
            id: String,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Ok(mut p) = self.pending.lock() {
                    p.remove(&self.id);
                }
            }
        }
        let _cleanup = Cleanup {
            pending: self.pending.clone(),
            id,
        };
        self.writer
            .send(value)
            .await
            .map_err(|_| io::Error::other("Codex connection closed"))?;
        tokio::time::timeout(Duration::from_secs(90), rx)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Codex request timed out"))?
            .map_err(|_| io::Error::other("Codex disconnected before replying"))
    }
}
pub(crate) struct AppServerClient {
    handle: AppServerRequestHandle,
    events: mpsc::Receiver<AppServerEvent>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    child: Option<tokio::process::Child>,
    metadata: Value,
}
impl AppServerClient {
    pub async fn external(options: &crate::startup::StartupOptions) -> anyhow::Result<Self> {
        let exe = std::env::var_os("FASTROCK_CODEX").unwrap_or_else(|| "codex".into());
        // Only the explicit fixture executable bypasses the installed CLI version probe.
        if std::env::var_os("FASTROCK_CODEX_FIXTURE").is_none() {
            let mut probe = tokio::process::Command::new(&exe);
            probe.arg("--version");
            #[cfg(windows)]
            probe.creation_flags(0x08000000);
            let output = tokio::time::timeout(Duration::from_secs(15), probe.output()).await??;
            let version = String::from_utf8_lossy(&output.stdout);
            let numeric = version
                .split_whitespace()
                .find(|v| v.starts_with(|c: char| c.is_ascii_digit()))
                .unwrap_or("");
            let parts = numeric
                .split('.')
                .filter_map(|v| v.parse::<u32>().ok())
                .collect::<Vec<_>>();
            anyhow::ensure!(
                output.status.success() && parts.len() >= 2 && (parts[0] > 0 || parts[1] >= 162),
                "Install Codex CLI 0.162.0 or newer and put codex on PATH."
            );
        }
        let mut command = tokio::process::Command::new(&exe);
        if let Some(script) = std::env::var_os("FASTROCK_CODEX_FIXTURE") {
            // Explicit local test injection: Python executable + checked-in fixture script.
            command.arg(script);
        }
        command.arg("app-server");
        for (key, value) in &options.cli_overrides {
            command.arg("-c").arg(format!("{key}={value}"));
        }
        if let Some(cwd) = options.cwd.as_ref().filter(|p| p.is_dir()) {
            command.current_dir(cwd);
        }
        command
            .env_remove("FASTROCK_RALLY_TOKEN")
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command
            .spawn()
            .map_err(|e| anyhow::anyhow!("Unable to launch installed codex app-server: {e}"))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Codex stdin unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Codex stdout unavailable"))?;
        let stderr = child.stderr.take();
        let mut client = Self::stream(output, input, Some(child));
        if let Some(stderr) = stderr {
            client.tasks.push(tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                // Drain without logging server/environment secrets.
                loop {
                    match reader.fill_buf().await {
                        Ok([]) | Err(_) => break,
                        Ok(bytes) => {
                            let n = bytes.len();
                            reader.consume(n);
                        }
                    }
                }
            }));
        }
        client
            .initialize(crate::startup::GUI_CLIENT_NAME, env!("CARGO_PKG_VERSION"))
            .await?;
        Ok(client)
    }
    fn stream<R, W>(reader: R, mut writer: W, child: Option<tokio::process::Child>) -> Self
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel::<Value>(256);
        let (events_tx, events) = mpsc::channel(512);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let read_pending = pending.clone();
        let write_task = tokio::spawn(async move {
            while let Some(value) = rx.recv().await {
                let Ok(mut bytes) = serde_json::to_vec(&value) else {
                    break;
                };
                bytes.push(b'\n');
                if writer.write_all(&bytes).await.is_err() || writer.flush().await.is_err() {
                    break;
                }
            }
        });
        let read_task = tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let result: io::Result<()> = async {
                loop {
                    let bytes = read_frame(&mut reader).await?;
                    if bytes.is_empty() {
                        break;
                    }
                    let value: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                    dispatch(value, &read_pending, &events_tx).await?;
                }
                Ok(())
            }
            .await;
            if let Ok(mut pending) = read_pending.lock() {
                pending.clear();
            }
            let message = result
                .err()
                .map(|e| format!("External Codex disconnected: {e}"))
                .unwrap_or_else(|| "External Codex app-server exited.".into());
            let _ = events_tx
                .send(AppServerEvent::Disconnected { message })
                .await;
        });
        Self {
            handle: AppServerRequestHandle {
                writer: tx,
                pending,
            },
            events,
            tasks: vec![write_task, read_task],
            child,
            metadata: Value::Null,
        }
    }
    pub async fn connect(args: RemoteAppServerConnectArgs) -> anyhow::Result<Self> {
        let mut client = match args.endpoint {
            RemoteAppServerEndpoint::UnixSocket { socket_path } => {
                #[cfg(unix)]
                {
                    let stream = tokio::net::UnixStream::connect(socket_path.as_path()).await?;
                    let (r, w) = stream.into_split();
                    Self::stream(r, w, None)
                }
                #[cfg(not(unix))]
                {
                    let _ = socket_path;
                    anyhow::bail!(
                        "Unix sockets are unavailable on Windows; use installed Codex or WebSocket."
                    )
                }
            }
            RemoteAppServerEndpoint::WebSocket {
                websocket_url,
                auth_token,
            } => {
                use tokio_tungstenite::tungstenite::client::IntoClientRequest;
                let mut request = websocket_url.into_client_request()?;
                if let Some(token) = auth_token {
                    request
                        .headers_mut()
                        .insert("Authorization", format!("Bearer {token}").parse()?);
                }
                let (socket, _) = tokio_tungstenite::connect_async(request).await?;
                let (mut sink, mut stream) = socket.split();
                let (tx, mut rx) = mpsc::channel::<Value>(256);
                let (events_tx, events) = mpsc::channel(512);
                let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
                let read_pending = pending.clone();
                let write_task = tokio::spawn(async move {
                    while let Some(v) = rx.recv().await {
                        if sink
                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                v.to_string().into(),
                            ))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                let read_task = tokio::spawn(async move {
                    while let Some(Ok(m)) = stream.next().await {
                        if m.is_text() {
                            if m.len() > MAX_FRAME {
                                break;
                            }
                            let Ok(v) = serde_json::from_str(m.to_text().unwrap_or("")) else {
                                break;
                            };
                            if dispatch(v, &read_pending, &events_tx).await.is_err() {
                                break;
                            }
                        }
                    }
                    if let Ok(mut p) = read_pending.lock() {
                        p.clear();
                    }
                    let _ = events_tx
                        .send(AppServerEvent::Disconnected {
                            message: "Remote Codex disconnected.".into(),
                        })
                        .await;
                });
                Self {
                    handle: AppServerRequestHandle {
                        writer: tx,
                        pending,
                    },
                    events,
                    tasks: vec![write_task, read_task],
                    child: None,
                    metadata: Value::Null,
                }
            }
        };
        client
            .initialize(&args.client_name, &args.client_version)
            .await?;
        Ok(client)
    }
    async fn initialize(&mut self, name: &str, version: &str) -> anyhow::Result<()> {
        self.metadata = self.handle.raw(json!({"id":"fastrock-initialize","method":"initialize","params":{"clientInfo":{"name":name,"version":version},"capabilities":{"experimentalApi":true}}})).await?
            .map_err(|e|anyhow::anyhow!("Codex initialization failed: {}",e.message))?;
        self.handle
            .writer
            .send(json!({"method":"initialized","params":{}}))
            .await?;
        Ok(())
    }
    pub fn platform_os(&self) -> Option<&str> {
        self.metadata["platformOs"].as_str()
    }
    pub fn platform_family(&self) -> Option<&str> {
        self.metadata["platformFamily"].as_str()
    }
    pub fn server_version(&self) -> Option<&str> {
        self.metadata["userAgent"].as_str()
    }
    pub fn codex_home(&self) -> Option<&str> {
        self.metadata["codexHome"].as_str()
    }
    pub fn request_handle(&self) -> AppServerRequestHandle {
        self.handle.clone()
    }
    pub async fn next_event(&mut self) -> Option<AppServerEvent> {
        self.events.recv().await
    }
    pub async fn resolve_server_request(&self, id: RequestId, result: Value) -> io::Result<()> {
        self.handle
            .writer
            .send(json!({"id":id,"result":result}))
            .await
            .map_err(io::Error::other)
    }
    pub async fn reject_server_request(
        &self,
        id: RequestId,
        error: JSONRPCErrorError,
    ) -> io::Result<()> {
        self.handle
            .writer
            .send(json!({"id":id,"error":error}))
            .await
            .map_err(io::Error::other)
    }
    pub async fn shutdown(mut self) -> io::Result<()> {
        for task in self.tasks.drain(..) {
            task.abort();
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            tokio::time::timeout(Duration::from_secs(3), child.wait())
                .await
                .map_err(io::Error::other)??;
        }
        Ok(())
    }
}
impl Drop for AppServerClient {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
    }
}
async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut frame = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(frame);
        }
        let n = available
            .iter()
            .position(|b| *b == b'\n')
            .map(|n| n + 1)
            .unwrap_or(available.len());
        if frame.len() + n > MAX_FRAME {
            return Err(io::Error::other("Codex JSON frame exceeds 32 MiB"));
        }
        let done = available[n - 1] == b'\n';
        frame.extend_from_slice(&available[..n]);
        reader.consume(n);
        if done {
            return Ok(frame);
        }
    }
}
async fn dispatch(
    value: Value,
    pending: &Pending,
    events: &mpsc::Sender<AppServerEvent>,
) -> io::Result<()> {
    if value.get("method").is_some() {
        let event = if value.get("id").is_some() {
            serde_json::from_value(value).map(|r| AppServerEvent::ServerRequest(Box::new(r)))
        } else {
            serde_json::from_value(value).map(|n| AppServerEvent::ServerNotification(Box::new(n)))
        };
        if let Ok(event) = event {
            events
                .try_send(event)
                .map_err(|_| io::Error::other("Codex event queue overflow or closed"))?;
        }
    } else if let Some(id) = value.get("id") {
        let sender = pending
            .lock()
            .map_err(|_| io::Error::other("request registry unavailable"))?
            .remove(&id.to_string());
        if let Some(sender) = sender {
            let result = if let Some(error) = value.get("error") {
                Err(serde_json::from_value(error.clone()).map_err(io::Error::other)?)
            } else {
                Ok(value.get("result").cloned().unwrap_or(Value::Null))
            };
            let _ = sender.send(result);
        }
    }
    Ok(())
}
pub(crate) type RemoteAppServerClient = AppServerClient;

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn v16_responses_do_not_wait_for_notifications() {
        let (client_io, server_io) = tokio::io::duplex(1024 * 64);
        let (reader, writer) = tokio::io::split(client_io);
        let mut client = AppServerClient::stream(reader, writer, None);
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let server = tokio::spawn(async move {
            let mut reader = BufReader::new(server_reader);
            let request: Value =
                serde_json::from_slice(&read_frame(&mut reader).await.unwrap()).unwrap();
            server_writer
                .write_all(
                    format!("{}\n", json!({"id":request["id"],"result":{"ok":true}})).as_bytes(),
                )
                .await
                .unwrap();
            server_writer
                .write_all(
                    format!(
                        "{}\n",
                        json!({"method":"warning","params":{"message":"fixture","threadId":null}})
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let result = client
            .handle
            .raw(json!({"id":"fixture-1","method":"fixture/read","params":{}}))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result["ok"], true);
        assert!(matches!(
            client.next_event().await,
            Some(AppServerEvent::ServerNotification(_))
        ));
        server.await.unwrap();
        client.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn v16_cancellation_removes_pending_request() {
        let (a, b) = tokio::io::duplex(4096);
        let (r, w) = tokio::io::split(a);
        let client = AppServerClient::stream(r, w, None);
        let handle = client.request_handle();
        let task = tokio::spawn(async move {
            handle
                .raw(json!({"id":"cancelled","method":"fixture"}))
                .await
        });
        let mut reader = BufReader::new(b);
        let _ = read_frame(&mut reader).await.unwrap();
        task.abort();
        let _ = task.await;
        assert!(client.handle.pending.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn v16_oversized_frame_is_bounded_before_allocation() {
        let bytes = vec![b'x'; MAX_FRAME + 1];
        let mut reader = BufReader::new(bytes.as_slice());
        assert!(read_frame(&mut reader).await.is_err());
    }
    #[tokio::test]
    async fn v16_disconnect_releases_pending_calls() {
        let (a, b) = tokio::io::duplex(4096);
        let (r, w) = tokio::io::split(a);
        let client = AppServerClient::stream(r, w, None);
        let handle = client.request_handle();
        let task = tokio::spawn(async move {
            handle
                .raw(json!({"id":"disconnect","method":"fixture"}))
                .await
        });
        let mut reader = BufReader::new(b);
        let _ = read_frame(&mut reader).await.unwrap();
        drop(reader);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
    }
}

#[cfg(test)]
mod overload_tests {
    use super::*;
    #[tokio::test]
    async fn v16_notification_overload_fails_without_blocking_replies() {
        let (sender, _receiver) = mpsc::channel(1);
        let pending = Pending::default();
        let event = serde_json::json!({"method":"thread/started","params":{"thread":{"id":"fixture","sessionId":"fixture","preview":"","ephemeral":true,"modelProvider":"openai","createdAt":0,"updatedAt":0,"status":{"type":"idle"},"cwd":std::env::current_dir().unwrap(),"cliVersion":"0.162.0","source":"appServer","turns":[]}}});
        dispatch(event.clone(), &pending, &sender).await.unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(50),
                dispatch(event, &pending, &sender)
            )
            .await
            .unwrap()
            .is_err()
        );
    }
}
