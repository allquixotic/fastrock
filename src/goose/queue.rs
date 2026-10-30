//! One atomic queue/dispatch record. Recovered work always waits for Resume.
use super::*;
use serde::{Deserialize, Serialize};

const MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct QueueState {
    version: u32,
    pending: Vec<Value>,
    dispatch: Option<Value>,
    uncertain: Vec<Value>,
    paused: bool,
}
impl Default for QueueState {
    fn default() -> Self {
        Self {
            version: 1,
            pending: vec![],
            dispatch: None,
            uncertain: vec![],
            paused: false,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Recovery {
    pub paused: bool,
    pub pending: usize,
    pub uncertain: Vec<Value>,
}
impl QueueState {
    fn recovery(&self) -> Recovery {
        Recovery {
            paused: self.paused,
            pending: self.pending.len(),
            uncertain: self.uncertain.clone(),
        }
    }
    fn quarantine(&mut self, reason: &str) {
        if let Some(mut item) = self.dispatch.take() {
            self.pending.retain(|queued| queued["id"] != item["id"]);
            if !self.uncertain.iter().any(|old| old["id"] == item["id"]) {
                item["recoveryReason"] = json!(reason);
                self.uncertain.push(item);
            }
        }
        self.paused = true;
    }
    fn recover(mut self) -> Self {
        if self.dispatch.is_some() {
            self.quarantine("Fastrock stopped before this send's completion was saved. Check conversation history and tool results before sending it again.");
        }
        self.paused |= !self.pending.is_empty() || !self.uncertain.is_empty();
        self
    }
}

impl Adapter {
    pub(super) async fn pause_agent_queue(&self, id: &str) -> Result<(), JSONRPCErrorError> {
        self.load_queue(id).await?;
        let mut queues = self.queue.lock().await;
        let state = queues
            .get_mut(id)
            .ok_or_else(|| error("Queue unavailable"))?;
        state.paused = true;
        let result = self.write_queue(id, state).await;
        self.queue_event(id, state.recovery());
        result
    }
    // Callers hold the session gate. The queue mutex serializes disk snapshots
    // with completion callbacks, which must not reacquire that gate.
    pub(super) async fn load_queue(&self, id: &str) -> Result<Recovery, JSONRPCErrorError> {
        let mut queues = self.queue.lock().await;
        if !queues.contains_key(id) {
            let path = self.queue_path(id);
            let state = tokio::task::spawn_blocking(move || read_queue(&path))
                .await
                .map_err(|e| error(e.to_string()))??;
            queues.insert(id.to_owned(), state);
        }
        Ok(queues[id].recovery())
    }
    pub(super) fn queue_event(&self, id: &str, recovery: Recovery) {
        let _ = self.events.try_send(AppServerEvent::GooseRecovery {
            thread_id: ui_id(id),
            recovery,
        });
        let _ = self.emit(json!({"method":"thread/queue/changed","params":{"threadId":ui_id(id)}}));
    }
    async fn write_queue(&self, id: &str, state: &QueueState) -> Result<(), JSONRPCErrorError> {
        let path = self.queue_path(id);
        let state = state.clone();
        tokio::task::spawn_blocking(move || write_queue(&path, &state))
            .await
            .map_err(|e| error(e.to_string()))?
    }
    pub(super) async fn queue_request(self: &Arc<Self>, params: &Value, method: &str) -> Reply {
        let id = session_id(text(params, "threadId"))?.to_string();
        let gate = self.gate(&id).await;
        let lock = gate.lock().await;
        self.load_queue(&id).await?;
        let mut queues = self.queue.lock().await;
        let state = queues
            .get_mut(&id)
            .ok_or_else(|| error("Queue unavailable"))?;
        let before = state.clone();
        let result = match method {
            "thread/queue/list" => return Ok(json!({"data":state.pending,"nextCursor":null})),
            "fastrock/goose/recovery" => return Ok(json!(state.recovery())),
            "fastrock/goose/resumeQueue" => {
                state.paused = false;
                json!(state.recovery())
            }
            "fastrock/goose/dismissRecovery" => {
                let ids = params["ids"]
                    .as_array()
                    .ok_or_else(|| error("Missing reviewed record IDs"))?;
                state.uncertain.retain(|item| !ids.contains(&item["id"]));
                json!(state.recovery())
            }
            "thread/queue/add" => {
                let item = json!({"id":uuid::Uuid::new_v4().to_string(),"input":params["input"],"clientUserMessageId":params["clientUserMessageId"]});
                state.pending.push(item.clone());
                json!({"queuedSubmission":item})
            }
            "thread/queue/update" => {
                let item = state
                    .pending
                    .iter_mut()
                    .find(|q| q["id"] == params["queuedSubmissionId"])
                    .ok_or_else(|| {
                        error("Queued message was already consumed; the edit was not applied")
                    })?;
                item["input"] = params["input"].clone();
                json!({"queuedSubmission":item})
            }
            "thread/queue/delete" => {
                let old = state.pending.len();
                state
                    .pending
                    .retain(|q| q["id"] != params["queuedSubmissionId"]);
                json!({"deleted":state.pending.len()!=old})
            }
            _ => return Err(unsupported(method)),
        };
        if let Err(e) = self.write_queue(&id, state).await {
            *state = before;
            return Err(e);
        }
        let recovery = state.recovery();
        drop(queues);
        drop(lock);
        self.queue_event(&id, recovery);
        if matches!(method, "thread/queue/add" | "fastrock/goose/resumeQueue") {
            self.schedule_queue(id).await;
        }
        self.changed.notify_waiters();
        Ok(result)
    }

    /// Durable handoff before calling Goose. Queue removal and dispatch evidence
    /// commit together; no crash boundary can make the same item pending again.
    pub(super) async fn claim_dispatch(
        &self,
        id: &str,
        tid: &str,
        params: &Value,
    ) -> Result<(), JSONRPCErrorError> {
        self.load_queue(id).await?;
        let mut queues = self.queue.lock().await;
        let state = queues
            .get_mut(id)
            .ok_or_else(|| error("Queue unavailable"))?;
        if state.paused {
            return Err(error(
                "Recovered work is paused. Review it and choose Resume first.",
            ));
        }
        if state.dispatch.is_some() {
            return Err(error("A previous send has not settled yet."));
        }
        let before = state.clone();
        let mut item = if let Some(queued_id) = params["_queuedSubmissionId"].as_str() {
            if state.pending.first().is_none_or(|q| q["id"] != queued_id) {
                return Err(error("Queued message changed before dispatch"));
            }
            state.pending.remove(0)
        } else {
            json!({"id":uuid::Uuid::new_v4().to_string(),"input":params["input"],"clientUserMessageId":params["clientUserMessageId"]})
        };
        item["turnId"] = json!(tid);
        state.dispatch = Some(item);
        if let Err(e) = self.write_queue(id, state).await {
            *state = before;
            return Err(e);
        }
        self.queue_event(id, state.recovery());
        Ok(())
    }

    pub(super) async fn settle_dispatch(&self, id: &str, tid: &str, completed: bool) {
        let mut queues = self.queue.lock().await;
        let Some(state) = queues.get_mut(id) else {
            return;
        };
        if state
            .dispatch
            .as_ref()
            .is_none_or(|item| item["turnId"] != tid)
        {
            return;
        }
        let before = state.clone();
        if completed {
            state.dispatch = None;
        } else {
            state.quarantine("This turn was interrupted or failed. Its tools may have run. Check the history before sending it again.");
        }
        if let Err(e) = self.write_queue(id, state).await {
            *state = before.recover();
            self.queue_error(id, e);
        }
        self.queue_event(id, state.recovery());
    }

    pub(super) async fn pause_closed_queue(&self, id: &str) -> Result<(), JSONRPCErrorError> {
        let mut queues = self.queue.lock().await;
        let Some(state) = queues.get_mut(id) else {
            return Ok(());
        };
        // Closing and reopening a session in this process has the same replay
        // boundary as restarting. Keep the in-memory state paused even if the
        // persistence attempt fails; the old disk dispatch also recovers paused.
        *state = state.clone().recover();
        let result = self.write_queue(id, state).await;
        self.queue_event(id, state.recovery());
        if result.is_ok() {
            queues.remove(id);
        }
        result
    }

    pub(super) fn schedule_queue(
        self: &Arc<Self>,
        id: String,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let adapter = self.clone();
            let _ = self.jobs.send(Box::pin(async move {
                let gate = adapter.gate(&id).await;
                let _lock = gate.lock().await;
                let idle = adapter.sessions.lock().await.get(&id).is_some_and(|s| s.loaded && s.active.is_none());
                if !idle { return }
                let item = {
                    let queues = adapter.queue.lock().await;
                    let Some(state) = queues.get(&id).filter(|s| !s.paused && s.dispatch.is_none()) else { return };
                    let Some(item) = state.pending.first() else { return };
                    item.clone()
                };
                let params = json!({"threadId":ui_id(&id),"input":item["input"],"clientUserMessageId":item["clientUserMessageId"],"_queuedSubmissionId":item["id"]});
                if let Err(e) = adapter.start_turn_locked(&params).await { adapter.queue_error(&id, e); }
            })).await;
        })
    }
    pub(super) fn queue_path(&self, id: &str) -> std::path::PathBuf {
        use base64::Engine;
        self.queue_root.join(format!(
            "{}.json",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id)
        ))
    }
    fn queue_error(&self, id: &str, e: JSONRPCErrorError) {
        let _ = self.emit(json!({"method":"error","params":{"threadId":ui_id(id),"turnId":null,"willRetry":false,"error":{"message":format!("Queued message retained: {}",e.message)}}}));
    }
}

fn read_json(path: &std::path::Path) -> Result<Option<Value>, JSONRPCErrorError> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(format!("Read queued drafts: {e}"))),
        Ok(m) if m.len() > MAX_BYTES => {
            return Err(error("Queued drafts exceed 16 MiB; original file retained"));
        }
        Ok(_) => {}
    }
    let bytes = std::fs::read(path).map_err(|e| error(format!("Read queued drafts: {e}")))?;
    serde_json::from_slice(&bytes).map(Some).map_err(|e| {
        error(format!(
            "Invalid queued drafts; original file retained: {e}"
        ))
    })
}
fn read_queue(path: &std::path::Path) -> Result<QueueState, JSONRPCErrorError> {
    let value = read_json(path)?;
    let mut state = match value {
        Some(Value::Array(pending)) => {
            // Legacy two-file journal: keep evidence even if the old queue
            // removal had not committed. Never reinterpret it as safe to retry.
            let dispatch = read_json(&path.with_extension("last-dispatch.json"))?;
            QueueState {
                pending,
                dispatch,
                ..QueueState::default()
            }
        }
        Some(value) => serde_json::from_value::<QueueState>(value)
            .map_err(|e| error(format!("Invalid queue state; original file retained: {e}")))?,
        None => QueueState {
            dispatch: read_json(&path.with_extension("last-dispatch.json"))?,
            ..QueueState::default()
        },
    };
    if state.version != 1 {
        return Err(error("Unknown queue version; original file retained"));
    }
    validate_items(&state)?;
    state = state.recover();
    Ok(state)
}
fn write_queue(path: &std::path::Path, state: &QueueState) -> Result<(), JSONRPCErrorError> {
    validate_items(state)?;
    let bytes = serde_json::to_vec(state).map_err(|e| error(e.to_string()))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("Queued drafts exceed 16 MiB"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| error(format!("Save queued drafts: {e}")))?;
    }
    crate::rally::store::atomic_write(path, &bytes)
        .map_err(|e| error(format!("Save queued drafts: {e}")))
}

fn validate_items(state: &QueueState) -> Result<(), JSONRPCErrorError> {
    for item in state
        .pending
        .iter()
        .chain(state.dispatch.iter())
        .chain(state.uncertain.iter())
    {
        if item["id"].as_str().is_none_or(str::is_empty) || !item["input"].is_array() {
            return Err(error(
                "Invalid queued message identity or input; original file retained",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: &str) -> Value {
        json!({"id":id,"clientUserMessageId":id,"input":[{"type":"text","text":"Retain UTF-8 é"},{"type":"localImage","path":"fixture.png"}]})
    }
    #[test]
    fn v41_atomic_dispatch_recovery_never_requeues_uncertain_input() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("queue.json");
        let mut state = QueueState {
            pending: vec![item("a"), item("b")],
            ..Default::default()
        };
        write_queue(&path, &state).unwrap();
        let before = read_queue(&path).unwrap();
        assert!(before.paused);
        assert_eq!(before.pending.len(), 2);
        state.dispatch = Some(state.pending.remove(0));
        write_queue(&path, &state).unwrap();
        let recovered = read_queue(&path).unwrap();
        assert!(recovered.paused);
        assert_eq!(recovered.pending, vec![item("b")]);
        assert_eq!(recovered.uncertain[0]["input"], item("a")["input"]);
        assert!(recovered.dispatch.is_none());
        write_queue(&path, &recovered).unwrap();
        assert_eq!(read_queue(&path).unwrap().uncertain.len(), 1);
    }
    #[test]
    fn v41_legacy_dispatch_overlap_is_quarantined_and_files_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("queue.json");
        let legacy = serde_json::to_vec(&vec![item("a"), item("b")]).unwrap();
        let dispatch = path.with_extension("last-dispatch.json");
        std::fs::write(&path, &legacy).unwrap();
        std::fs::write(&dispatch, serde_json::to_vec(&item("a")).unwrap()).unwrap();
        let mut recovered = read_queue(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), legacy);
        assert_eq!(recovered.pending, vec![item("b")]);
        assert_eq!(recovered.uncertain[0]["id"], "a");
        recovered.uncertain.clear();
        write_queue(&path, &recovered).unwrap();
        assert!(
            read_queue(&path).unwrap().uncertain.is_empty(),
            "A migrated legacy file must not resurrect a reviewed record"
        );
        assert!(dispatch.exists());
    }
    #[test]
    fn v41_unknown_queue_format_fails_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("queue.json");
        let mut state = QueueState::default();
        state.version = 999;
        write_queue(&path, &state).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(read_queue(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    #[test]
    fn v41_invalid_legacy_items_cannot_drop_pending_input() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("queue.json");
        let bytes = br#"[{"input":[{"type":"text","text":"Keep me"}]}]"#;
        std::fs::write(&path, bytes).unwrap();
        std::fs::write(
            path.with_extension("last-dispatch.json"),
            br#"{"input":[]}"#,
        )
        .unwrap();
        assert!(read_queue(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
