//! ACP JSON-RPC transport. Requests are multiplexed; a prompt has no RPC deadline.
use crate::transport::{Pending, Reply};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub(super) struct Rpc {
    pub writer: mpsc::Sender<Value>,
    pending: Pending,
    next: Arc<AtomicU64>,
}

impl Rpc {
    pub fn stream<R, W>(
        reader: R,
        mut writer: W,
    ) -> (
        Self,
        mpsc::Receiver<Value>,
        Vec<tokio::task::JoinHandle<()>>,
    )
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel::<Value>(256);
        let (incoming, events) = mpsc::channel(512);
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let write_pending = pending.clone();
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
            if let Ok(mut p) = write_pending.lock() {
                p.clear();
            }
        });
        let read_task = tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let result: anyhow::Result<()> = async {
                loop {
                    let bytes = crate::transport::read_frame(&mut reader).await?;
                    if bytes.is_empty() {
                        break;
                    }
                    let value: Value = serde_json::from_slice(&bytes)?;
                    // Dispatch in wire order: load's history updates precede its reply.
                    // Bounded backpressure lets large history replays complete without loss.
                    incoming.send(value).await?;
                }
                Ok(())
            }
            .await;
            let message = result
                .err()
                .map(|e| format!("Goose ACP disconnected: {e}"))
                .unwrap_or_else(|| "Goose ACP process exited.".into());
            let _ = incoming
                .send(json!({"method":"fastrock/disconnected","params":{"message":message}}))
                .await;
        });
        (
            Self {
                writer: tx,
                pending,
                next: Arc::new(AtomicU64::new(1)),
            },
            events,
            vec![write_task, read_task],
        )
    }

    pub fn disconnected(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
    }

    pub fn complete(&self, value: &Value) -> anyhow::Result<()> {
        if let Some(id) = value.get("id") {
            let sender = self
                .pending
                .lock()
                .map_err(|_| anyhow::anyhow!("ACP registry unavailable"))?
                .remove(&id.to_string());
            if let Some(sender) = sender {
                let reply = match value.get("error") {
                    Some(error) => Err(serde_json::from_value(error.clone())?),
                    None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = sender.send(reply);
            }
        }
        Ok(())
    }

    pub async fn begin(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Call, codex_app_server_protocol::JSONRPCErrorError> {
        let id = format!("fastrock-acp-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let key = json!(id).to_string();
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| super::error("ACP registry unavailable"))?
            .insert(key.clone(), tx);
        let call = Call {
            pending: self.pending.clone(),
            key,
            rx,
            method: method.to_string(),
        };
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        Ok(call)
    }

    pub async fn call(&self, method: &str, params: Value) -> Reply {
        let start = std::time::Instant::now();
        let reply = self.begin(method, params).await?.wait().await;
        // Only fixed operation names and timings, never request bodies, IDs,
        // endpoint URLs or provider diagnostics that could contain credentials.
        use super::latency::{self, Phase};
        if let Some(phase) = match method {
            "session/new" => Some(Phase::NewSession),
            "session/load" => Some(Phase::LoadSession),
            "session/set_config_option" => Some(Phase::Configure),
            _ => None,
        } {
            latency::record(phase, start);
        }
        reply
    }

    pub async fn send(
        &self,
        value: Value,
    ) -> Result<(), codex_app_server_protocol::JSONRPCErrorError> {
        self.writer
            .send(value)
            .await
            .map_err(|_| super::error("Goose connection closed"))
    }
}

pub(super) struct TaskGuard(pub Vec<tokio::task::JoinHandle<()>>);
impl Drop for TaskGuard {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

pub(super) struct Call {
    pending: Pending,
    key: String,
    rx: oneshot::Receiver<Reply>,
    method: String,
}
impl Call {
    pub async fn wait(mut self) -> Reply {
        let reply = if self.method == "session/prompt" {
            (&mut self.rx).await
        } else {
            tokio::time::timeout(Duration::from_secs(60), &mut self.rx)
                .await
                .map_err(|_| super::error(format!("Goose {} timed out", self.method)))?
        };
        reply.map_err(|_| super::error("Goose disconnected before replying"))?
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.key);
        }
    }
}
