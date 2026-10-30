//! App-owned scheduling over the same embedded Goose/vwork dispatcher.
mod controls;
pub(crate) mod model;
pub(super) mod webhook;
mod webhook_model;
use super::*;
use model::{Data, Pattern, Run, State, Task};
use std::time::Duration;

pub(super) async fn open(path: std::path::PathBuf) -> anyhow::Result<State> {
    Ok(tokio::task::spawn_blocking(move || State::open(path)).await?)
}
pub(super) async fn drive(adapter: Arc<Adapter>) {
    loop {
        let changed = adapter.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let delay = adapter.schedule_cycle().await;
        match delay {
            Some(delay) => {
                tokio::select! { _ = changed => {}, _ = tokio::time::sleep(delay) => {} }
            }
            None => changed.await,
        }
    }
}
impl Adapter {
    fn schedules_changed(&self) {
        self.changed.notify_waiters();
        let _ = self.events.try_send(AppServerEvent::GooseSchedulesChanged);
    }
    async fn schedule_cycle(self: &Arc<Self>) -> Option<Duration> {
        let _gate = self.schedule_gate.lock().await;
        if self.schedules.lock().await.available().is_err() {
            return None;
        }
        let active: Vec<_> = self
            .schedules
            .lock()
            .await
            .data
            .tasks
            .values()
            .filter_map(|t| {
                t.active
                    .as_ref()
                    .map(|r| (t.id.clone(), t.source.clone(), r.clone()))
            })
            .collect();
        for (id, source, run) in active {
            let result = match &run.child {
                Some(child) => self.delegate_status(&source, child).await,
                None => Err(error(
                    "Occurrence setup did not reach dispatch; inspect the retained workspace",
                )),
            };
            let (status, detail) = match result {
                Ok(result)
                    if matches!(
                        result["status"].as_str(),
                        Some("running" | "queued" | "settling")
                    ) =>
                {
                    continue;
                }
                Ok(result) => (
                    text(&result, "status").to_owned(),
                    if result["requires_human_resume"] == true {
                        "Run paused; open its conversation to review interrupted work".into()
                    } else {
                        String::new()
                    },
                ),
                Err(e) => ("failed".into(), e.message),
            };
            let mut state = self.schedules.lock().await;
            let mut data = state.data.clone();
            if let Some(task) = data.tasks.get_mut(&id) {
                if task.active.as_ref().is_some_and(|r| r.id == run.id) {
                    task.finish(&status, detail, chrono::Utc::now().timestamp());
                    let _ = state.commit(data).await;
                    self.schedules_changed();
                }
            }
        }
        let now = chrono::Utc::now().timestamp();
        self.dispatch_webhook_deliveries(now).await;
        let due: Vec<_> = {
            let state = self.schedules.lock().await;
            if state.available().is_err() {
                return None;
            }
            state
                .data
                .tasks
                .values()
                .filter(|t| {
                    !t.paused && t.active.is_none() && t.next_at.is_some_and(|at| at <= now)
                })
                .map(|t| (t.id.clone(), t.source.clone()))
                .collect()
        };
        for (id, source) in due {
            if self
                .sessions
                .lock()
                .await
                .get(&source)
                .is_some_and(|s| s.active.is_some())
            {
                continue;
            }
            let _ = self.start_schedule(&id, false, None).await;
        }
        let state = self.schedules.lock().await;
        if state.available().is_err() {
            return None;
        }
        let now = chrono::Utc::now().timestamp();
        let next_timer = state
            .data
            .tasks
            .values()
            .filter(|t| !t.paused && t.active.is_none())
            .filter_map(|t| t.next_at)
            .filter(|at| *at > now)
            .min();
        let next_hook = state
            .data
            .tasks
            .values()
            .filter(|t| !t.paused && t.active.is_none())
            .flat_map(|t| {
                t.deliveries
                    .iter()
                    .filter(|d| d.status == "pending" && d.ready > now)
                    .map(|d| d.ready)
            })
            .min();
        let next = next_timer.into_iter().chain(next_hook).min();
        next.map(|at| Duration::from_secs((at - now).clamp(1, 60) as u64))
    }

    // Caller holds schedule_gate through claim, binding and dispatch. Shutdown
    // takes the same gate before closing sessions, so no child can escape Quit.
    async fn start_schedule(
        self: &Arc<Self>,
        id: &str,
        manual: bool,
        delivery: Option<&str>,
    ) -> Reply {
        let (task, run) = {
            let mut state = self.schedules.lock().await;
            state.available()?;
            let mut data = state.data.clone();
            let task = data
                .tasks
                .get_mut(id)
                .ok_or_else(|| error("Schedule no longer exists"))?;
            let run = task.claim(chrono::Utc::now().timestamp(), manual)?;
            let prompt = if let Some(delivery) = delivery {
                let item = task
                    .deliveries
                    .iter_mut()
                    .find(|d| d.id == delivery && d.status == "pending")
                    .ok_or_else(|| error("Webhook delivery changed before dispatch"))?;
                item.status = "starting".into();
                item.run = Some(run.id.clone());
                Some(item.prompt.clone())
            } else {
                None
            };
            let mut task = task.clone();
            if let Some(prompt) = prompt {
                task.prompt = prompt;
            }
            state.commit(data).await?;
            (task, run)
        };
        self.schedules_changed();
        let result = self.dispatch_scheduled(&task, &run).await;
        if let Err(e) = &result {
            let mut state = self.schedules.lock().await;
            let mut data = state.data.clone();
            if let Some(task) = data.tasks.get_mut(id) {
                task.finish("failed", e.message.clone(), chrono::Utc::now().timestamp());
            }
            // A failed write disables dispatch and preserves its prior claim.
            let _ = state.commit(data).await;
        }
        self.schedules_changed();
        result
    }
    async fn dispatch_scheduled(self: &Arc<Self>, task: &Task, run: &Run) -> Reply {
        let delegation = super::delegation::Delegation {
            parent: task.source.clone(),
            source_turn: String::new(),
            task_id: format!("schedule-{}", run.id),
            instructions: task.prompt.clone(),
            model: None,
            effort: None,
            scheduled: true,
            label: Some(task.title.clone()),
        };
        let result = self
            .fork_bound(&json!({"threadId":ui_id(&task.source)}), Some(delegation))
            .await?;
        if result["reused"] == true {
            return Err(error(
                "Occurrence already has a child; inspect it before starting a new run",
            ));
        }
        let child = session_id(text(&result["thread"], "id"))?.to_owned();
        {
            let mut state = self.schedules.lock().await;
            state.available()?;
            let mut data = state.data.clone();
            let record = data
                .tasks
                .get_mut(&task.id)
                .ok_or_else(|| error("Schedule disappeared"))?;
            let active = record
                .active
                .as_mut()
                .filter(|r| r.id == run.id)
                .ok_or_else(|| error("Occurrence changed before dispatch"))?;
            active.child = Some(child.clone());
            active.status = "running".into();
            for receipt in record
                .deliveries
                .iter_mut()
                .filter(|d| d.run.as_deref() == Some(&run.id))
            {
                receipt.child = Some(child.clone());
            }
            record.revision += 1;
            state.commit(data).await?;
        }
        // Claim and child identity are durable before adding executable work.
        self.queue_request(
            &json!({"threadId":ui_id(&child),"input":[{"type":"text","text":task.prompt}],
            "clientUserMessageId":format!("schedule:{}:{}",task.id,run.id)}),
            "thread/queue/add",
        )
        .await?;
        Ok(json!({"threadId":ui_id(&child)}))
    }
    pub(super) async fn pause_schedules(&self) -> Result<(), JSONRPCErrorError> {
        let _gate = self.schedule_gate.lock().await;
        let mut state = self.schedules.lock().await;
        state.stopping = true;
        // Do not replace a corrupt/unreadable original with an empty snapshot.
        if state.error.is_none() {
            let mut data = state.data.clone();
            for task in data.tasks.values_mut() {
                task.paused = true;
                task.revision += 1;
            }
            state.commit(data).await?;
        }
        self.schedules_changed();
        Ok(())
    }
    pub(super) async fn schedule_request(self: &Arc<Self>, params: &Value, method: &str) -> Reply {
        if method == "fastrock/schedule/list" {
            let state = self.schedules.lock().await;
            let tasks = json!(state.data.tasks.values().collect::<Vec<_>>());
            let problem = state.error.clone();
            drop(state);
            let sessions = self.sessions.lock().await;
            let mut sources: Vec<_> = sessions.iter().filter(|(_,s)| !s.ephemeral && s.binding.workspace.is_some())
                .map(|(id,s)| json!({"id":ui_id(id),"name":s.info["title"].as_str().filter(|s| !s.is_empty()).unwrap_or(id)})).collect();
            sources.sort_by(|a, b| text(a, "id").cmp(text(b, "id")));
            let listener = self.webhook_listener.lock().await;
            return Ok(json!({"tasks":tasks,"sources":sources,"error":problem,
                "webhookAddress":listener.address,"webhookError":listener.error}));
        }
        let _gate = self.schedule_gate.lock().await;
        if method == "fastrock/schedule/save" {
            return self.save_schedule(params).await;
        }
        let id = text(params, "id");
        let mut state = self.schedules.lock().await;
        if method == "fastrock/schedule/stop" {
            // Stopping the same occurrence remains valid if its publication
            // changed the metadata revision or storage has since failed.
            let child = state
                .data
                .tasks
                .get(id)
                .ok_or_else(|| error("Schedule no longer exists"))?
                .stop_target(text(params, "runId"))?
                .to_owned();
            drop(state);
            self.interrupt_tree(&child).await?;
            return Ok(json!({}));
        }
        state.available()?;
        let mut data = state.data.clone();
        let task = data
            .tasks
            .get_mut(id)
            .ok_or_else(|| error("Schedule no longer exists"))?;
        if task.revision != params["revision"].as_u64().unwrap_or(0) {
            return Err(error(
                "Schedule changed; select it again to reload before applying this action",
            ));
        }
        if matches!(
            method,
            "fastrock/schedule/copy-key" | "fastrock/schedule/rotate-key"
        ) {
            if !matches!(task.pattern, Pattern::Webhook { .. }) {
                return Err(error("Select a webhook schedule"));
            }
            if method.ends_with("rotate-key") {
                webhook::new_key(id).await?;
                task.revision += 1;
                state.commit(data).await?;
                self.schedules_changed();
                return Ok(json!({}));
            }
            let id = id.to_owned();
            let key = tokio::task::spawn_blocking(move || crate::vault::get("webhook", &id))
                .await
                .map_err(|_| error("Credential worker failed"))?
                .map_err(|e| error(e.to_string()))?;
            return Ok(json!({"key":key}));
        }
        match method {
            "fastrock/schedule/run" => {
                drop(state);
                if matches!(task.pattern, Pattern::Webhook { .. }) {
                    return Err(error("Send a signed webhook to run this schedule"));
                }
                return self.start_schedule(id, true, None).await;
            }
            "fastrock/schedule/pause" => {
                task.paused = true;
                task.revision += 1;
            }
            "fastrock/schedule/resume" => {
                task.next_at = task.pattern.next_after(chrono::Utc::now().timestamp());
                if task.next_at.is_none() && !matches!(task.pattern, Pattern::Webhook { .. }) {
                    return Err(error(
                        "This time has passed. Edit the schedule or choose Run now for a new occurrence",
                    ));
                }
                task.paused = false;
                task.revision += 1;
            }
            "fastrock/schedule/delete" => {
                if task.active.is_some() || task.deliveries.iter().any(|d| d.status == "pending") {
                    return Err(error(
                        "Stop the current run and discard pending deliveries before deleting its schedule",
                    ));
                }
                data.tasks.remove(id);
            }
            "fastrock/schedule/discard-pending" => {
                for d in task.deliveries.iter_mut().filter(|d| d.status == "pending") {
                    d.status = "discarded".into();
                    d.prompt.clear();
                }
                task.revision += 1;
            }
            _ => return Err(unsupported(method)),
        }
        state.commit(data).await?;
        if method == "fastrock/schedule/delete" {
            let id = id.to_owned();
            let _ = tokio::task::spawn_blocking(move || crate::vault::remove("webhook", &id)).await;
        }
        self.schedules_changed();
        Ok(json!({}))
    }
    async fn save_schedule(&self, params: &Value) -> Reply {
        let source = session_id(text(params, "source"))?.to_owned();
        if self.binding_record(&source).await?.workspace.is_none() {
            return Err(error("Choose a conversation bound to a virtual workspace"));
        }
        let pattern: Pattern = serde_json::from_value(params["pattern"].clone())
            .map_err(|_| error("Invalid schedule timing"))?;
        pattern.validate()?;
        if matches!(pattern, Pattern::Webhook { .. })
            && self.webhook_listener.lock().await.address.is_none()
        {
            return Err(error(
                "Webhook listener is unavailable; correct its address and restart Fastrock",
            ));
        }
        let next = pattern.next_after(chrono::Utc::now().timestamp());
        let mut state = self.schedules.lock().await;
        state.available()?;
        let mut data: Data = state.data.clone();
        let id = text(params, "id");
        let mut task = if id.is_empty() {
            let create_id = uuid::Uuid::parse_str(text(params, "createId"))
                .map_err(|_| error("Missing schedule creation identity"))?
                .to_string();
            if let Some(existing) = data.tasks.get(&create_id) {
                if existing.source != source
                    || existing.title != text(params, "title").trim()
                    || existing.prompt != text(params, "prompt")
                    || existing.pattern != pattern
                {
                    return Err(error(
                        "This form already created a schedule. Select it to edit or choose New schedule",
                    ));
                }
                return Ok(json!({"task":existing}));
            }
            Task {
                id: create_id,
                revision: 0,
                title: String::new(),
                prompt: String::new(),
                source: source.clone(),
                pattern: pattern.clone(),
                paused: false,
                next_at: next,
                active: None,
                last: None,
                deliveries: Vec::new(),
            }
        } else {
            let old = data
                .tasks
                .get(id)
                .ok_or_else(|| error("Schedule no longer exists"))?;
            if old.revision != params["revision"].as_u64().unwrap_or(0)
                || old.active.is_some()
                || old.deliveries.iter().any(|d| d.status == "pending")
            {
                return Err(error(
                    "Schedule changed, is running, or has pending deliveries. Refresh after work finishes or discard pending deliveries before editing",
                ));
            }
            old.clone()
        };
        task.title = text(params, "title").trim().into();
        task.prompt = text(params, "prompt").into();
        task.source = source;
        task.pattern = pattern;
        task.next_at = if matches!(task.pattern, Pattern::Webhook { .. }) {
            None
        } else {
            Some(next.ok_or_else(|| error("Choose a future time"))?)
        };
        task.revision += 1;
        task.validate()?;
        let create_key = matches!(task.pattern, Pattern::Webhook { .. })
            && data
                .tasks
                .get(&task.id)
                .is_none_or(|t| !matches!(t.pattern, Pattern::Webhook { .. }));
        let remove_key = !matches!(task.pattern, Pattern::Webhook { .. })
            && data
                .tasks
                .get(&task.id)
                .is_some_and(|t| matches!(t.pattern, Pattern::Webhook { .. }));
        if create_key {
            webhook::new_key(&task.id).await?;
        }
        data.tasks.insert(task.id.clone(), task.clone());
        if let Err(e) = state.commit(data).await {
            if create_key {
                let id = task.id.clone();
                let _ =
                    tokio::task::spawn_blocking(move || crate::vault::remove("webhook", &id)).await;
            }
            return Err(e);
        }
        if remove_key {
            let id = task.id.clone();
            let _ = tokio::task::spawn_blocking(move || crate::vault::remove("webhook", &id)).await;
        }
        self.schedules_changed();
        Ok(json!({"task":task}))
    }
}
