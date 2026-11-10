//! In-memory ACP routing across public Goose connections. Services snapshot
//! hooks, so a new/explicitly resumed chat gets its own project-scoped services.
//! Storage stays shared; active chats keep their hooks until they are closed.
use anyhow::{Result, anyhow, ensure};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{DuplexStream, ReadHalf, WriteHalf},
    sync::Mutex as AsyncMutex,
    task::JoinHandle,
};
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

type Reader = FramedRead<ReadHalf<DuplexStream>, LinesCodec>;
type Writer = Arc<AsyncMutex<FramedWrite<WriteHalf<DuplexStream>, LinesCodec>>>;
type Shared = Arc<Mutex<State>>;
const MAX_FRAME: usize = 32 * 1024 * 1024;
const INIT_ID: &str = "fastrock-embedded-initialize";

struct Endpoint {
    writer: Writer,
    tasks: Vec<JoinHandle<()>>,
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
struct Pending {
    connection: u64,
    method: String,
    session: String,
}
#[derive(Default)]
struct State {
    connections: HashMap<u64, Endpoint>,
    sessions: HashMap<String, u64>,
    pending: HashMap<String, Pending>,
    answers: HashMap<String, (u64, Value)>,
    next_answer: u64,
}
struct Cleanup(Shared);
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lock() {
            state.connections.clear();
        }
    }
}

fn streams(reader: ReadHalf<DuplexStream>, writer: WriteHalf<DuplexStream>) -> (Reader, Writer) {
    (
        FramedRead::new(reader, LinesCodec::new_with_max_length(MAX_FRAME)),
        Arc::new(AsyncMutex::new(FramedWrite::new(
            writer,
            LinesCodec::new_with_max_length(MAX_FRAME),
        ))),
    )
}
async fn send(writer: &Writer, value: &Value) -> Result<()> {
    writer
        .lock()
        .await
        .send(serde_json::to_string(value)?)
        .await?;
    Ok(())
}
async fn endpoint(cwd: Option<PathBuf>, initialize: Option<&Value>) -> Result<(Endpoint, Reader)> {
    let connection = super::embedded::connect_session(cwd).await?;
    let (mut reader, writer) = streams(connection.reader, connection.writer);
    let endpoint = Endpoint {
        writer,
        tasks: vec![connection.task],
    };
    if let Some(params) = initialize {
        send(
            &endpoint.writer,
            &json!({"jsonrpc":"2.0","id":INIT_ID,"method":"initialize","params":params}),
        )
        .await?;
        let line = tokio::time::timeout(std::time::Duration::from_secs(60), reader.next())
            .await?
            .ok_or_else(|| anyhow!("Runtime closed during initialization"))??;
        let reply: Value = serde_json::from_str(&line)?;
        ensure!(
            reply["id"] == INIT_ID && reply.get("error").is_none(),
            "Runtime initialization failed"
        );
    }
    Ok((endpoint, reader))
}

/// Called after translating one response, before forwarding it. Lifecycle
/// replies establish bindings before the client can dispatch its next request.
fn output(state: &mut State, connection: u64, value: &mut Value) -> Option<u64> {
    let id = value.get("id")?.clone();
    if value.get("method").is_some() {
        state.next_answer += 1;
        let routed = json!(format!(
            "fastrock-embedded-server-{connection}-{}",
            state.next_answer
        ));
        state.answers.insert(routed.to_string(), (connection, id));
        value["id"] = routed;
        return None;
    }
    let key = id.to_string();
    if state.pending.get(&key)?.connection != connection {
        return None;
    }
    let pending = state.pending.remove(&key)?;
    let success = value.get("error").is_none();
    match pending.method.as_str() {
        "session/new" | "session/load" | "session/fork" if success => {
            let session = value["result"]["sessionId"]
                .as_str()
                .unwrap_or(&pending.session);
            if !session.is_empty() {
                state.sessions.insert(session.into(), connection);
            }
        }
        "session/close" | "session/delete" if success => {
            state.sessions.remove(&pending.session);
        }
        _ => {}
    }
    // Close may reply before the cancelled prompt. Drain every outstanding
    // reply so the adapter never keeps an unresolved, deadline-free prompt.
    (connection != 0
        && !state.sessions.values().any(|c| *c == connection)
        && !state.pending.values().any(|p| p.connection == connection))
    .then_some(connection)
}

fn register(state: &Shared, id: u64, endpoint: Endpoint, mut reader: Reader, client: Writer) {
    state.lock().unwrap().connections.insert(id, endpoint);
    let shared = state.clone();
    let pump = tokio::spawn(async move {
        let result: Result<()> = async {
            while let Some(line) = reader.next().await {
                let mut value: Value = serde_json::from_str(&line?)?;
                let release = output(&mut shared.lock().unwrap(), id, &mut value);
                send(&client, &value).await?;
                if let Some(id) = release {
                    // The close reply is already delivered. Abort the runtime
                    // only after Goose has completed its supported close call.
                    let mut state = shared.lock().unwrap();
                    state.answers.retain(|_, (owner, _)| *owner != id);
                    state.connections.remove(&id);
                    return Ok(());
                }
            }
            Err(anyhow!("Runtime closed"))
        }
        .await;
        if result.is_err() {
            let _ = send(&client, &json!({"method":"fastrock/disconnected","params":{"message":"Embedded conversation connection closed."}})).await;
        }
    });
    if let Some(endpoint) = state.lock().unwrap().connections.get_mut(&id) {
        endpoint.tasks.push(pump);
    } else {
        pump.abort();
    }
}

pub(super) async fn serve(stream: DuplexStream) {
    let state = Shared::default();
    let _cleanup = Cleanup(state.clone());
    // Errors are intentionally not logged: upstream diagnostics may include
    // provider credentials. Closing the pipe reaches the adapter's recovery UI.
    let _ = run(stream, state).await;
}
async fn run(stream: DuplexStream, state: Shared) -> Result<()> {
    let (reader, writer) = tokio::io::split(stream);
    let (mut input, client) = streams(reader, writer);
    let (control, reader) = endpoint(None, None).await?;
    register(&state, 0, control, reader, client.clone());
    let mut initialize = None;
    let mut next = 0;
    while let Some(line) = input.next().await {
        let mut value: Value = serde_json::from_str(&line?)?;
        let method = value["method"].as_str().unwrap_or_default().to_owned();
        let session = value["params"]["sessionId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let connection = if method.is_empty() {
            let answer = state
                .lock()
                .unwrap()
                .answers
                .remove(&value["id"].to_string());
            let Some((connection, original)) = answer else {
                continue;
            };
            value["id"] = original;
            connection
        } else {
            if method == "initialize" {
                initialize = Some(value["params"].clone());
            }
            let existing = state.lock().unwrap().sessions.get(&session).copied();
            let connection = if matches!(method.as_str(), "session/new" | "session/load")
                && existing.is_none()
            {
                let params = initialize
                    .as_ref()
                    .ok_or_else(|| anyhow!("Initialize first"))?;
                let cwd = value["params"]["cwd"].as_str().map(PathBuf::from);
                match endpoint(cwd, Some(params)).await {
                    Ok((endpoint, reader)) => {
                        next += 1;
                        register(&state, next, endpoint, reader, client.clone());
                        next
                    }
                    Err(_) => {
                        send(&client, &json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32603,"message":"Could not initialize conversation runtime."}})).await?;
                        continue;
                    }
                }
            } else {
                existing.unwrap_or(0)
            };
            if let Some(id) = value.get("id") {
                state.lock().unwrap().pending.insert(
                    id.to_string(),
                    Pending {
                        connection,
                        method,
                        session,
                    },
                );
            }
            connection
        };
        let writer = state
            .lock()
            .unwrap()
            .connections
            .get(&connection)
            .map(|c| c.writer.clone())
            .ok_or_else(|| anyhow!("Conversation connection closed"))?;
        send(&writer, &value).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v106_connections_namespace_answers_and_bind_before_reply() {
        let mut state = State::default();
        let mut first = json!({"id":0,"method":"session/request_permission"});
        let mut second = first.clone();
        output(&mut state, 1, &mut first);
        output(&mut state, 2, &mut second);
        assert_ne!(first["id"], second["id"]);
        assert_eq!(state.answers[&first["id"].to_string()], (1, json!(0)));
        assert_eq!(state.answers[&second["id"].to_string()], (2, json!(0)));
        state.pending.insert(
            json!("load").to_string(),
            Pending {
                connection: 2,
                method: "session/load".into(),
                session: "saved".into(),
            },
        );
        assert_eq!(
            output(&mut state, 2, &mut json!({"id":"load","result":{}})),
            None
        );
        assert_eq!(state.sessions["saved"], 2);
        state.pending.insert(
            json!("close").to_string(),
            Pending {
                connection: 2,
                method: "session/close".into(),
                session: "saved".into(),
            },
        );
        assert_eq!(
            output(&mut state, 2, &mut json!({"id":"close","result":{}})),
            Some(2)
        );
        assert!(!state.sessions.contains_key("saved"));
    }

    #[test]
    fn v106_close_drains_the_cancelled_prompt_reply() {
        let mut state = State::default();
        state.sessions.insert("s".into(), 1);
        for method in ["session/prompt", "session/close"] {
            state.pending.insert(
                json!(method).to_string(),
                Pending {
                    connection: 1,
                    method: method.into(),
                    session: "s".into(),
                },
            );
        }
        assert_eq!(
            output(
                &mut state,
                1,
                &mut json!({"id":"session/close","result":{}})
            ),
            None
        );
        assert_eq!(
            output(
                &mut state,
                1,
                &mut json!({"id":"session/prompt","error":{"code":-32603,"message":"cancelled"}})
            ),
            Some(1)
        );
        assert!(state.pending.is_empty());
    }
}
