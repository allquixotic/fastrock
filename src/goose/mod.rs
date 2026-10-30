//! Embedded Goose adapter. Goose owns agents/config/storage; these are UI projections.
mod capabilities;
mod capability_config;
pub(crate) mod capability_controls;
mod cheap_profile;
mod checkpoint_storage;
mod checkpoints;
mod config_guard;
mod controls;
mod delegation;
mod delegation_tools;
mod deletion;
mod embedded;
mod history;
mod hooks;
mod id_reservation;
pub(crate) mod latency;
mod mcp;
mod model;
mod profile_controls;
pub(crate) mod profiles;
mod queue;
pub(crate) mod recipe_controls;
mod recipe_storage;
mod recipes;
mod recovery_history;
mod restore_extensions;
mod review;
mod schedule;
pub(crate) use queue::Recovery;
mod effort_controls;
mod effort_defaults;
mod rpc;
mod sessions;
mod shell;
mod side;
mod summary_service;

pub(crate) fn summary_diagnostic() -> String {
    summary_service::diagnostic()
}

pub(crate) fn summaries_idle() -> bool {
    summary_service::idle()
}

mod fork;
mod fork_storage;
mod projects;
mod provider_policy;
pub(crate) mod setup;
mod setup_controls;
mod summary_worker;
#[cfg(test)]
mod tests;
mod titles;
mod vwork;
mod workspace;

use crate::transport::{AppServerClient, AppServerEvent, Pending, Reply};
use codex_app_server_protocol::JSONRPCErrorError;
use model::{Session, now, session_id, text, turn, ui_id};
use rpc::{Rpc, TaskGuard};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
#[cfg(test)]
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

type Job = Pin<Box<dyn Future<Output = ()> + Send>>;
pub(crate) fn summary_store() -> Option<std::path::PathBuf> {
    summary_service::home()
}
pub(super) fn error(message: impl Into<String>) -> JSONRPCErrorError {
    JSONRPCErrorError {
        code: -32000,
        message: message.into(),
        data: None,
    }
}
fn unsupported(method: &str) -> JSONRPCErrorError {
    JSONRPCErrorError {
        code: -32601,
        message: format!("{method} is unavailable with this Goose ACP connection."),
        data: None,
    }
}

enum Answer {
    Permission {
        id: Value,
        session: String,
        options: Vec<(String, Value)>,
    },
    Tool {
        session: String,
        result: oneshot::Sender<Value>,
    },
}

struct Adapter {
    rpc: Rpc,
    events: mpsc::Sender<AppServerEvent>,
    sessions: Mutex<HashMap<String, Session>>,
    gates: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    answers: Mutex<HashMap<String, Answer>>,
    jobs: mpsc::Sender<Job>,
    capabilities: Mutex<Value>,
    defaults: Mutex<Value>,
    cwd: String,
    mcp: mcp::Bridge,
    queue: Mutex<HashMap<String, queue::QueueState>>,
    queue_root: std::path::PathBuf,
    vwork_paths: fastrock_workspaces::vwork::DiscoveryPaths,
    embedded_runtime: bool,
    summaries: Option<Arc<summary_service::Service>>,
    deletions: Mutex<deletion::State>,
    changed: tokio::sync::Notify,
    schedules: Mutex<schedule::model::State>,
    schedule_gate: Mutex<()>,
    webhook_listener: Mutex<schedule::webhook::Listener>,
}

pub(crate) async fn connect(
    options: &crate::startup::StartupOptions,
) -> anyhow::Result<AppServerClient> {
    anyhow::ensure!(
        options.cli_overrides.is_empty(),
        "Codex -c overrides do not apply to embedded Goose. Use Goose configuration or provider profiles."
    );
    let cwd = options
        .cwd
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow::anyhow!("Working directory unavailable"))?;
    tokio::task::spawn_blocking(|| config_guard::validate(goose_runtime::config::Config::global()))
        .await??;
    profiles::prepare().await?;
    hooks::prepare(cwd.clone()).await?;
    let connection = embedded::connect().await?;
    connect_streams(
        connection.reader,
        connection.writer,
        vec![connection.task],
        None,
        cwd,
        crate::rally::store::Store::new()
            .directory
            .join("goose-queues"),
        fastrock_workspaces::vwork::DiscoveryPaths::current(),
    )
    .await
}

#[cfg(test)]
async fn connect_command(
    mut command: tokio::process::Command,
    cwd: std::path::PathBuf,
    queue_root: std::path::PathBuf,
) -> anyhow::Result<AppServerClient> {
    command
        .current_dir(&cwd)
        .env_remove("FASTROCK_RALLY_TOKEN")
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(|e| anyhow::anyhow!("Unable to start Goose ACP: {e}. Install Goose CLI (v1.56.0 or newer recommended), run `goose configure`, and put goose on PATH, or set FASTROCK_GOOSE to its executable."))?;
    let input = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Goose stdin unavailable"))?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Goose stdout unavailable"))?;
    let stderr = child.stderr.take();
    let mut tasks = Vec::new();
    if let Some(stderr) = stderr {
        tasks.push(tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            // Provider errors can contain credentials. Drain without persisting stderr.
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
    let paths = fastrock_workspaces::vwork::DiscoveryPaths {
        user_home: cwd.clone(),
        configs: vec![cwd.join("config.yaml")],
        search: vec![],
        default_vwork_home: cwd.join("vwork"),
    };
    connect_streams(output, input, tasks, Some(child), cwd, queue_root, paths).await
}

async fn connect_streams<R, W>(
    output: R,
    input: W,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    child: Option<tokio::process::Child>,
    cwd: std::path::PathBuf,
    queue_root: std::path::PathBuf,
    vwork_paths: fastrock_workspaces::vwork::DiscoveryPaths,
) -> anyhow::Result<AppServerClient>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut guard = TaskGuard(tasks);
    let (rpc, mut incoming, io_tasks) = Rpc::stream(output, input);
    guard.0.extend(io_tasks);
    let (events_tx, events) = mpsc::channel(512);
    let (jobs_tx, mut jobs_rx) = mpsc::channel::<Job>(256);
    guard.0.push(tokio::spawn(async move {
        let mut running = tokio::task::JoinSet::new();
        loop { tokio::select! {
            job = jobs_rx.recv() => match job { Some(job) => { running.spawn(job); }, None => break },
            _ = running.join_next(), if !running.is_empty() => {},
        }}
    }));
    let bridge = mcp::Bridge::new().await?;
    let summaries = if child.is_none() {
        let bindings = vwork::configured_embedded(vwork_paths.clone(), cwd.clone())
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        let home = bindings
            .first()
            .map(|b| b.home.clone())
            .unwrap_or_else(|| vwork_paths.default_vwork_home.clone());
        Some(summary_service::Service::start(home))
    } else {
        None
    };
    let adapter = Arc::new(Adapter {
        rpc: rpc.clone(),
        events: events_tx.clone(),
        sessions: Mutex::new(HashMap::new()),
        gates: Mutex::new(HashMap::new()),
        answers: Mutex::new(HashMap::new()),
        jobs: jobs_tx,
        capabilities: Mutex::new(Value::Null),
        defaults: Mutex::new(Value::Null),
        cwd: cwd.to_string_lossy().into_owned(),
        mcp: bridge,
        queue: Mutex::new(HashMap::new()),
        schedules: Mutex::new(schedule::open(queue_root.join("schedules.json")).await?),
        schedule_gate: Mutex::new(()),
        webhook_listener: Mutex::new(Default::default()),
        deletions: Mutex::new(deletion::open(queue_root.join("deletions")).await?),
        queue_root,
        summaries,
        vwork_paths,
        embedded_runtime: child.is_none(),
        changed: tokio::sync::Notify::new(),
    });
    let read_adapter = adapter.clone();
    let pending: Pending = Arc::new(StdMutex::new(HashMap::new()));
    let read_pending = pending.clone();
    guard.0.push(tokio::spawn(async move {
        while let Some(value) = incoming.recv().await {
            if value.get("method").is_none() {
                if read_adapter.rpc.complete(&value).is_err() {
                    break;
                }
            } else if text(&value, "method") == "fastrock/disconnected" {
                if let Ok(mut p) = read_pending.lock() {
                    p.clear();
                }
                read_adapter.rpc.disconnected();
                read_adapter.answers.lock().await.clear();
                let _ = events_tx
                    .send(AppServerEvent::Disconnected {
                        message: text(&value["params"], "message").into(),
                    })
                    .await;
                return;
            } else if let Err(e) = read_adapter.incoming(value).await {
                read_adapter.rpc.disconnected();
                read_adapter.answers.lock().await.clear();
                if let Ok(mut p) = read_pending.lock() {
                    p.clear();
                }
                let _ = events_tx
                    .send(AppServerEvent::Disconnected {
                        message: format!("Goose ACP event error: {}", e.message),
                    })
                    .await;
                return;
            }
        }
        read_adapter.rpc.disconnected();
        read_adapter.answers.lock().await.clear();
        let _ = events_tx
            .send(AppServerEvent::Disconnected {
                message: "Goose ACP event stream ended.".into(),
            })
            .await;
    }));
    let hello = rpc
        .call(
            "initialize",
            json!({"protocolVersion":1,
        "clientInfo":{"name":"fastrock","version":env!("CARGO_PKG_VERSION")},
        // Goose executes file/terminal tools itself. Never advertise unimplemented client tools.
        "clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false,
            "_meta":{"goose":{"customNotifications":true,"toolCallLabelEnrichment":false}}}}),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Goose ACP initialization: {}", e.message))?;
    anyhow::ensure!(
        hello["protocolVersion"] == 1,
        "Unsupported Goose ACP protocol version: {}",
        hello["protocolVersion"]
    );
    *adapter.capabilities.lock().await = hello["agentCapabilities"].clone();
    // Optional, read-only Goose extension; base ACP does not define global defaults.
    if let Ok(defaults) = rpc.call("_goose/unstable/defaults/read", json!({})).await {
        *adapter.defaults.lock().await = defaults;
    }
    let (writer, mut requests) = mpsc::channel::<Value>(256);
    if adapter.embedded_runtime {
        adapter
            .recover_deletions()
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        if let Some(task) = schedule::webhook::listen(adapter.clone()).await {
            guard.0.push(task);
        }
        guard.0.push(tokio::spawn(schedule::drive(adapter.clone())));
    }
    let request_pending = pending.clone();
    guard.0.push(tokio::spawn(async move {
        let mut running = tokio::task::JoinSet::new();
        loop { tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else { break };
                let adapter = adapter.clone();
                let pending = request_pending.clone();
                running.spawn(async move {
                    if request.get("method").is_none() { adapter.answer(request).await; return }
                    let id = request["id"].clone();
                    let result = adapter.request(&request).await;
                    let reply = match result { Ok(result) => json!({"id":id,"result":result}), Err(error) => json!({"id":id,"error":error}) };
                    let _ = crate::transport::dispatch(reply,&pending,&adapter.events).await;
                });
            },
            _ = running.join_next(), if !running.is_empty() => {},
        }}
    }));
    let metadata = json!({"gooseAdapter":true,"userAgent":format!("Goose {} (ACP)",text(&hello["agentInfo"],"version")),
        "platformOs":std::env::consts::OS,"platformFamily":std::env::consts::FAMILY});
    Ok(AppServerClient::adapter(
        writer,
        pending,
        events,
        std::mem::take(&mut guard.0),
        child,
        metadata,
    ))
}

impl Adapter {
    fn config_event(&self, id: &str, options: Vec<Value>) -> Result<(), JSONRPCErrorError> {
        let options = if self.embedded_runtime {
            provider_policy::visible_options(options)
        } else {
            options
        };
        self.events
            .try_send(AppServerEvent::GooseConfig {
                thread_id: ui_id(id),
                options,
            })
            .map_err(|_| error("Goose UI event queue overflow or closed"))
    }
    fn emit(&self, mut value: Value) -> Result<(), JSONRPCErrorError> {
        stamp_item_event(&mut value);
        let event = if value.get("id").is_some() {
            serde_json::from_value(value).map(|v| AppServerEvent::ServerRequest(Box::new(v)))
        } else {
            serde_json::from_value(value).map(|v| AppServerEvent::ServerNotification(Box::new(v)))
        }
        .map_err(|e| error(format!("ACP presentation decode: {e}")))?;
        self.events
            .try_send(event)
            .map_err(|_| error("Goose UI event queue overflow or closed"))
    }
    async fn gate(&self, id: &str) -> Arc<Mutex<()>> {
        self.gates
            .lock()
            .await
            .entry(id.to_owned())
            .or_default()
            .clone()
    }
    async fn incoming(&self, value: Value) -> Result<(), JSONRPCErrorError> {
        let params = &value["params"];
        match text(&value, "method") {
            "session/update" => {
                let id = text(params, "sessionId");
                let mut sessions = self.sessions.lock().await;
                let session = sessions
                    .entry(id.into())
                    .or_insert_with(|| Session::new(id, &self.cwd));
                if matches!(
                    params["update"]["sessionUpdate"].as_str(),
                    Some("agent_message_chunk" | "agent_thought_chunk" | "tool_call")
                ) {
                    if let Some(start) = session.first_response_since.take() {
                        latency::record(latency::Phase::FirstResponse, start);
                    }
                }
                for event in session.update(&params["update"]) {
                    self.emit(event)?;
                }
                if params["update"]["sessionUpdate"] == "config_option_update" {
                    self.config_event(id, session.config.clone())?;
                }
                if let Some(run) = params["update"]["_meta"]["goose"].get("activeRunId") {
                    session.run_id = run.as_str().map(str::to_owned);
                    if session.run_id.is_some() && session.cancel_requested {
                        // Goose may register its run after an immediately following Cancel.
                        self.rpc.send(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":id}})).await?;
                    }
                }
                if !params["update"]["title"].is_null()
                    && params["update"]["sessionUpdate"] == "session_info_update"
                    && !session.replaying
                {
                    self.emit(json!({"method":"thread/name/updated","params":{"threadId":ui_id(id),"threadName":session.info["title"]}}))?;
                }
            }
            "_goose/unstable/session/update" => {
                let mut sessions = self.sessions.lock().await;
                if let Some(session) = sessions.get_mut(text(params, "sessionId")) {
                    if params["update"]["sessionUpdate"] == "active_run_update" {
                        session.run_id = params["update"]["runId"].as_str().map(str::to_owned);
                    }
                }
            }
            "session/request_permission" => {
                let id = text(params, "sessionId");
                let (turn_id, title) = {
                    let sessions = self.sessions.lock().await;
                    let session = sessions
                        .get(id)
                        .ok_or_else(|| error("Permission for unknown Goose session"))?;
                    (
                        session
                            .active
                            .clone()
                            .ok_or_else(|| error("Permission outside active Goose prompt"))?,
                        text(&params["toolCall"], "title").to_string(),
                    )
                };
                let request_id = format!("goose-permission-{}", uuid::Uuid::new_v4());
                let options: Vec<(String, Value)> = params["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(|(i, o)| {
                        (
                            format!("{}. {}", i + 1, text(o, "name")),
                            o["optionId"].clone(),
                        )
                    })
                    .collect();
                let choices: Vec<Value> = options
                    .iter()
                    .zip(params["options"].as_array().into_iter().flatten())
                    .map(|((label, _), o)| json!({"label":label,"description":text(o,"kind")}))
                    .collect();
                self.answers.lock().await.insert(
                    request_id.clone(),
                    Answer::Permission {
                        id: value["id"].clone(),
                        session: id.into(),
                        options,
                    },
                );
                let details = params["toolCall"]["rawInput"]
                    .as_object()
                    .map(|_| format!("\n{}", params["toolCall"]["rawInput"]))
                    .unwrap_or_default();
                self.emit(json!({"id":request_id,"method":"item/tool/requestUserInput","params":{"threadId":ui_id(id),"turnId":turn_id,
                    "itemId":params["toolCall"]["toolCallId"],"isBlocking":true,
                    "questions":[{"id":"permission","header":"Goose permission","question":format!("{title}{details}"),"isOther":false,"isSecret":false,"options":choices}]}}))?;
            }
            _ if value.get("id").is_some() => {
                self.rpc.send(json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32601,"message":"Client method not supported by Fastrock"}})).await?;
            }
            _ => {} // ACP permits future notification types.
        }
        Ok(())
    }
    async fn answer(&self, value: Value) {
        let answer = self.answers.lock().await.remove(text(&value, "id"));
        match answer {
            Some(Answer::Permission { id, options, .. }) => {
                let selected = value["result"]["answers"]["permission"]["answers"]
                    .as_array()
                    .filter(|a| a.len() == 1)
                    .and_then(|a| a[0].as_str());
                let option = selected.and_then(|s| options.iter().find(|(label, _)| label == s));
                let outcome = match option {
                    Some((_, option_id)) => json!({"outcome":"selected","optionId":option_id}),
                    None => json!({"outcome":"cancelled"}),
                };
                let _ = self
                    .rpc
                    .send(json!({"jsonrpc":"2.0","id":id,"result":{"outcome":outcome}}))
                    .await;
            }
            Some(Answer::Tool { result, .. }) => {
                let _ = result.send(value);
            }
            None => {}
        }
    }

    async fn request(self: &Arc<Self>, request: &Value) -> Reply {
        let method = text(request, "method");
        let params = &request["params"];
        if method != "thread/delete"
            && let Some(id) = params["threadId"]
                .as_str()
                .and_then(|id| id.strip_prefix("goose:"))
        {
            if let Err(e) = self.deletion_ready(id).await {
                if method == "thread/unsubscribe" {
                    return Ok(json!({"status":"unsubscribed"}));
                }
                return Err(e);
            }
        }
        match method {
            "thread/delete" => self.delete_session(params).await,
            "fastrock/thread/deletions" => self.deletion_page(params).await,
            "fastrock/shutdown" => {
                // Stop scheduling even if its storage cannot be saved; existing
                // sessions must still be cancelled and closed on explicit Quit.
                let schedule_error = self.pause_schedules().await.err();
                let ids: Vec<_> = self.sessions.lock().await.keys().cloned().collect();
                for id in ids {
                    self.close_session(
                        &json!({"threadId":format!("goose:{id}"),"preserveTemporary":true}),
                    )
                    .await?;
                }
                if let Some(e) = schedule_error {
                    return Err(e);
                }
                Ok(json!({}))
            }
            "config/read" => {
                let defaults = self.defaults.lock().await;
                Ok(
                    json!({"config":{"model":defaults["modelId"].as_str().unwrap_or("__goose_default__"),"model_provider":"goose",
                    "model_providers":{"goose":{"name":"Goose ACP","wire_api":"responses"}}},"origins":{},"layers":[]}),
                )
            }
            "configRequirements/read" => Ok(json!({"requirements":null})),
            "account/read" => Ok(json!({"account":null,"requiresOpenaiAuth":false})),
            "model/list" => {
                let defaults = self.defaults.lock().await;
                let id = defaults["modelId"].as_str().unwrap_or("__goose_default__");
                Ok(
                    json!({"data":[{"id":id,"model":id,"displayName":if id=="__goose_default__"{"Goose configured default"}else{id},
                    "description":"Configured by Goose. Open a conversation to select its ACP model and provider.","hidden":false,
                    "supportedReasoningEfforts":[],"defaultReasoningEffort":"medium","isDefault":true}],"nextCursor":null}),
                )
            }
            "thread/start" => self.new_session(params).await,
            method if method.starts_with("fastrock/recipe/") => {
                self.recipe_request(params, method.trim_start_matches("fastrock/recipe/"))
                    .await
            }
            method if method.starts_with("fastrock/capability/") => {
                self.capability_request(params, method.rsplit('/').next().unwrap())
                    .await
            }
            method if method.starts_with("fastrock/schedule/") => {
                self.schedule_request(params, method).await
            }
            "fastrock/project/list" => self.project_catalog().await,
            "fastrock/project/open" => self.open_project(params).await,
            "fastrock/project/details" => self.project_settings(params, false).await,
            "fastrock/project/settings" => self.project_settings(params, true).await,
            "fastrock/thread/location" => self.thread_location(params).await,
            "thread/fork" => self.fork_session(params).await,
            "thread/list" => self.list_sessions(params).await,
            "fastrock/thread/metadata" => self.sidebar_metadata(params).await,
            "thread/loaded/list" => Ok(
                json!({"data":self.sessions.lock().await.iter().filter(|(_,s)|s.loaded).map(|(id,_)|ui_id(id)).collect::<Vec<_>>(),"nextCursor":null}),
            ),
            "thread/resume" | "thread/read" => {
                self.load_session(params, method == "thread/resume").await
            }
            "thread/turns/list" | "thread/items/list" => self.history(params, method).await,
            "fastrock/history/window" => self.history_window(params).await,
            "turn/start" => self.start_turn(params).await,
            "thread/shellCommand" => self.shell_command(params).await,
            "review/start" => self.review_request(params, "start").await,
            "fastrock/review/catalog" => self.review_request(params, "catalog").await,
            "fastrock/review/preview" => self.review_request(params, "preview").await,
            "turn/interrupt" => {
                let id = session_id(text(params, "threadId"))?;
                self.interrupt_tree(id).await?;
                Ok(json!({}))
            }
            "turn/steer" => self.steer(params).await,
            "thread/queue/add"
            | "fastrock/goose/recovery"
            | "fastrock/goose/resumeQueue"
            | "fastrock/goose/dismissRecovery"
            | "thread/queue/list"
            | "thread/queue/update"
            | "thread/queue/delete"
            | "thread/queue/reorder" => self.queue_request(params, method).await,
            "thread/queue/start" => self.start_queued(params).await,
            "thread/name/set"
            | "thread/setName"
            | "thread/archive"
            | "thread/unarchive"
            | "thread/compact/start" => self.manage(params, method).await,
            "thread/inject_items" => self.inject_side_boundary(params).await,
            "thread/unsubscribe" => self.close_session(params).await,
            "fastrock/thread/regenerateTitle" => self.regenerate_title(params).await,
            "fastrock/checkpoint/list"
            | "fastrock/checkpoint/capture"
            | "fastrock/checkpoint/review"
            | "fastrock/checkpoint/restore"
            | "fastrock/checkpoint/rollback"
            | "fastrock/checkpoint/keep" => {
                self.checkpoint_request(params, method.rsplit('/').next().unwrap())
                    .await
            }
            "fastrock/goose/terminalContext" => {
                let id = session_id(text(params, "threadId"))?;
                let sessions = self.sessions.lock().await;
                let s = sessions
                    .get(id)
                    .ok_or_else(|| error("Open a Goose conversation first"))?;
                Ok(json!({"cwd": s.info["cwd"], "workspace": s.binding.workspace}))
            }
            "fastrock/goose/config" => {
                let id = session_id(text(params, "threadId"))?;
                let sessions = self.sessions.lock().await;
                let s = sessions
                    .get(id)
                    .ok_or_else(|| error("Open a Goose conversation first"))?;
                let options = if self.embedded_runtime {
                    provider_policy::visible_options(s.config.clone())
                } else {
                    s.config.clone()
                };
                Ok(json!({"configOptions":options,"busy":s.active.is_some()}))
            }
            "fastrock/goose/saveEffortDefault" => self.save_effort_default(params).await,
            "fastrock/goose/setConfig" => {
                let id = session_id(text(params, "threadId"))?;
                let gate = self.gate(id).await;
                let _lock = gate.lock().await;
                if self
                    .sessions
                    .lock()
                    .await
                    .get(id)
                    .is_some_and(|s| s.active.is_some())
                {
                    return Err(error(
                        "Wait for the current Goose turn before changing its configuration",
                    ));
                }
                self.set_config(id, text(params, "configId"), &params["value"])
                    .await
            }
            "collaborationMode/list"
            | "skills/list"
            | "project/list"
            | "thread/section/list"
            | "experimentalFeature/list" => Ok(json!({"data":[],"nextCursor":null})),
            _ => Err(unsupported(method)),
        }
    }
}

// UI lifecycle timestamps are required even when ACP omits timing information.
fn stamp_item_event(value: &mut Value) {
    let key = match text(value, "method") {
        "item/started" => "startedAtMs",
        "item/completed" => "completedAtMs",
        _ => return,
    };
    value["params"][key] = json!(chrono::Utc::now().timestamp_millis());
}
