//! Durable app-owned task lineage over unchanged Goose and embedded vwork.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Delegation {
    pub parent: String,
    pub source_turn: String,
    pub task_id: String,
    pub instructions: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}
impl Delegation {
    pub fn same_task(&self, other: &Self) -> bool {
        self.parent == other.parent
            && self.task_id == other.task_id
            && self.instructions == other.instructions
            && self.model == other.model
            && self.effort == other.effort
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct Outcome {
    pub turn_id: String,
    pub status: String,
    pub text: String,
}

impl Adapter {
    pub(super) async fn delegation_index(
        &self,
        parent: &str,
    ) -> Result<BTreeMap<String, String>, JSONRPCErrorError> {
        Ok(super::fork::read_saved(
            &self.queue_path(parent).with_extension("delegates.json"),
            1024 * 1024,
        )
        .await?
        .unwrap_or_default())
    }

    // The parent's gate serializes creation, identity lookup and this commit.
    pub(super) async fn save_delegation_index(
        &self,
        parent: &str,
        task: &str,
        child: &str,
    ) -> Result<(), JSONRPCErrorError> {
        let mut index = self.delegation_index(parent).await?;
        if index.insert(task.into(), child.into()).is_some() {
            return Err(error("Delegate task identity already exists"));
        }
        let bytes =
            serde_json::to_vec(&index).map_err(|_| error("Encode delegate index failed"))?;
        if bytes.len() > 1024 * 1024 {
            return Err(error(
                "Delegate index exceeds supported size; retain the created workspace for recovery",
            ));
        }
        let path = self.queue_path(parent).with_extension("delegates.json");
        tokio::task::spawn_blocking(move || crate::rally::store::atomic_write(&path, &bytes))
            .await
            .map_err(|_| error("Save delegate index worker failed"))?
            .map_err(|_| error("Save delegate index failed"))
    }

    pub(super) async fn save_delegate_outcome(
        &self,
        id: &str,
        outcome: Outcome,
    ) -> Result<(), JSONRPCErrorError> {
        let path = self.queue_path(id).with_extension("delegate-result.json");
        let bytes =
            serde_json::to_vec(&outcome).map_err(|_| error("Encode delegate result failed"))?;
        tokio::task::spawn_blocking(move || crate::rally::store::atomic_write(&path, &bytes))
            .await
            .map_err(|_| error("Save delegate result worker failed"))?
            .map_err(|_| {
                error("Save delegate result failed; inspect its conversation before resuming")
            })
    }

    pub(super) async fn delegation_tool(
        self: &Arc<Self>,
        parent: &str,
        turn: &str,
        name: &str,
        args: &Value,
    ) -> Reply {
        match name {
            "delegate_task" => {
                let args: super::delegation_tools::Create = serde_json::from_value(args.clone())
                    .map_err(|_| error("Invalid delegate arguments"))?;
                args.validate()?;
                let task = Delegation {
                    parent: parent.into(),
                    source_turn: turn.into(),
                    task_id: args.task_id,
                    instructions: args.instructions,
                    model: args.model,
                    effort: args.effort,
                };
                let result = self
                    .fork_bound(&json!({"threadId":ui_id(parent)}), Some(task.clone()))
                    .await?;
                let child = session_id(text(&result["thread"], "id"))?;
                if result["reused"] == true {
                    return self.delegate_status(parent, child).await;
                }
                if let Some(model) = &task.model {
                    self.set_config(child, "model", &json!(model)).await?;
                }
                if let Some(effort) = &task.effort {
                    self.set_config(child, "thinking_effort", &json!(effort))
                        .await?;
                }
                if !self
                    .sessions
                    .lock()
                    .await
                    .get(parent)
                    .is_some_and(|s| s.active.as_deref() == Some(turn) && !s.cancel_requested)
                {
                    return Err(error(format!(
                        "Parent stopped during delegation; child {} was retained without running",
                        ui_id(child)
                    )));
                }
                self.queue_request(&json!({"threadId":ui_id(child),"input":[{"type":"text","text":task.instructions}],"clientUserMessageId":format!("delegate:{}:{}",parent,task.task_id)}),"thread/queue/add").await?;
                Ok(
                    json!({"thread_id":ui_id(child),"task_id":task.task_id,"status":"queued","isolated_workspace":true,"integration":"explicit review required"}),
                )
            }
            "list_delegates" => {
                let index = self.delegation_index(parent).await?;
                let cursor = args["cursor"].as_str().unwrap_or("");
                let entries: Vec<_> = index
                    .iter()
                    .filter(|(key, _)| key.as_str() > cursor)
                    .take(51)
                    .collect();
                let mut children = Vec::new();
                for (task, child) in entries.iter().take(50) {
                    let record = self.binding_record(child).await?;
                    if record
                        .delegation
                        .as_ref()
                        .is_none_or(|d| d.parent != parent || d.task_id != **task)
                    {
                        return Err(error("Invalid delegate lineage record"));
                    }
                    let recovery = self.load_queue(child).await?;
                    let active = self
                        .sessions
                        .lock()
                        .await
                        .get(*child)
                        .is_some_and(|s| s.active.is_some());
                    children.push(json!({"task_id":task,"thread_id":ui_id(child),"status":if active{"running"}else if recovery.paused{"paused"}else if recovery.pending>0{"queued"}else{"idle"}}));
                }
                Ok(
                    json!({"children":children,"next_cursor":if entries.len()>50{Some(entries[49].0)}else{None}}),
                )
            }
            "wait_delegate" | "send_to_delegate" | "interrupt_delegate" => {
                let child = session_id(text(args, "thread_id"))?;
                self.require_child(parent, child).await?;
                match name {
                    "interrupt_delegate" => {
                        self.interrupt_tree(child).await?;
                        Ok(
                            json!({"thread_id":ui_id(child),"status":"stop_requested","requires_human_resume":true}),
                        )
                    }
                    "send_to_delegate" => {
                        let message = text(args, "message");
                        if message.trim().is_empty() || message.len() > 32768 {
                            return Err(error("Follow-up must contain 1–32768 UTF-8 bytes"));
                        }
                        self.load_session(&json!({"threadId":ui_id(child)}), false)
                            .await?;
                        self.queue_request(&json!({"threadId":ui_id(child),"input":[{"type":"text","text":message}],"clientUserMessageId":uuid::Uuid::new_v4().to_string()}),"thread/queue/add").await
                    }
                    _ => {
                        let deadline =
                            tokio::time::Instant::now() + std::time::Duration::from_secs(30);
                        loop {
                            let changed = self.changed.notified();
                            tokio::pin!(changed);
                            changed.as_mut().enable();
                            let result = self.delegate_status(parent, child).await?;
                            if !matches!(
                                result["status"].as_str(),
                                Some("running" | "queued" | "settling")
                            ) {
                                return Ok(result);
                            }
                            if !self.sessions.lock().await.get(parent).is_some_and(|s| {
                                s.active.as_deref() == Some(turn) && !s.cancel_requested
                            }) {
                                return Err(error("Parent stopped waiting"));
                            }
                            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                                return Ok(result);
                            }
                        }
                    }
                }
            }
            _ => Err(error("Unknown Fastrock delegation tool")),
        }
    }

    async fn require_child(&self, parent: &str, child: &str) -> Result<(), JSONRPCErrorError> {
        let record = self.binding_record(child).await?;
        if parent == child
            || record
                .delegation
                .as_ref()
                .is_none_or(|d| d.parent != parent)
        {
            return Err(error("This conversation is not your direct delegate"));
        }
        Ok(())
    }

    async fn delegate_status(&self, parent: &str, child: &str) -> Reply {
        self.require_child(parent, child).await?;
        let outcome: Option<Outcome> = super::fork::read_saved(
            &self
                .queue_path(child)
                .with_extension("delegate-result.json"),
            256 * 1024,
        )
        .await?;
        let recovery = self.load_queue(child).await?;
        let sessions = self.sessions.lock().await;
        let session = sessions.get(child);
        let active = session.is_some_and(|s| s.active.is_some());
        let settling = session
            .and_then(|s| s.turns.last())
            .and_then(|t| t["id"].as_str())
            .is_some_and(|id| {
                !id.starts_with("history-") && outcome.as_ref().is_none_or(|o| o.turn_id != id)
            });
        let status = if active {
            "running"
        } else if recovery.paused {
            "paused"
        } else if recovery.pending > 0 {
            "queued"
        } else if settling {
            "settling"
        } else {
            outcome
                .as_ref()
                .map(|o| o.status.as_str())
                .unwrap_or("idle")
        };
        Ok(
            json!({"thread_id":ui_id(child),"status":status,"pending":recovery.pending,"uncertain":recovery.uncertain.len(),"result":if status=="completed"{outcome.as_ref().map(|o|o.text.as_str())}else{None},"requires_human_resume":recovery.paused,"integration":"explicit review required"}),
        )
    }

    pub(super) async fn child_threads(&self, params: &Value) -> Reply {
        let parent = session_id(text(params, "parentThreadId"))?;
        let index = self.delegation_index(parent).await?;
        let cursor = params["cursor"].as_str().unwrap_or("");
        let limit = params["limit"].as_u64().unwrap_or(50).clamp(1, 100) as usize;
        let entries: Vec<_> = index
            .iter()
            .filter(|(key, _)| key.as_str() > cursor)
            .take(limit + 1)
            .collect();
        let mut data = Vec::new();
        for (_, child) in entries.iter().take(limit) {
            self.require_child(parent, child).await?;
            let info = self
                .rpc
                .call("_goose/unstable/session/info", json!({"sessionId":child}))
                .await?;
            let binding = self.binding_record(child).await?;
            let mut sessions = self.sessions.lock().await;
            let session = sessions.entry((*child).clone()).or_default();
            session.info = info["session"].clone();
            session.binding = binding;
            if (!session.info["_meta"]["archivedAt"].is_null()) == (params["archived"] == true) {
                data.push(session.thread());
            }
        }
        Ok(
            json!({"data":data,"nextCursor":if entries.len()>limit{Some(entries[limit-1].0)}else{None}}),
        )
    }

    pub(super) async fn interrupt_tree(&self, root: &str) -> Result<(), JSONRPCErrorError> {
        let ids = {
            let mut sessions = self.sessions.lock().await;
            let mut ids = vec![root.to_owned()];
            let mut cursor = 0;
            while cursor < ids.len() {
                let parent = ids[cursor].clone();
                cursor += 1;
                for (id, s) in sessions.iter() {
                    if s.binding
                        .delegation
                        .as_ref()
                        .is_some_and(|d| d.parent == parent)
                        && !ids.contains(id)
                    {
                        ids.push(id.clone());
                    }
                }
            }
            for id in &ids {
                if let Some(s) = sessions.get_mut(id) {
                    s.cancel_requested = true;
                }
            }
            ids
        };
        let mut failure = None;
        for id in ids {
            // Serialize cancellation after any in-flight prompt handoff. A
            // queued start must not overtake Stop and clear cancel_requested.
            let gate = self.gate(&id).await;
            let _lock = gate.lock().await;
            if let Some(session) = self.sessions.lock().await.get_mut(&id) {
                session.cancel_requested = true;
            }
            if let Err(e) = self.pause_agent_queue(&id).await {
                failure = Some(e);
            }
            if let Err(e) = self
                .rpc
                .send(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":id}}))
                .await
            {
                failure = Some(e);
            }
            self.cancel_answers(&id).await;
        }
        self.changed.notify_waiters();
        match failure {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

pub(super) fn outcome(turn: &Value) -> Outcome {
    let mut result = String::new();
    let mut remaining = 20000;
    for message in turn["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i["type"] == "agentMessage")
        .filter_map(|i| i["text"].as_str())
    {
        if !result.is_empty() {
            for ch in "\n\n".chars().take(remaining) {
                result.push(ch);
                remaining -= 1;
            }
        }
        for ch in message.chars().take(remaining) {
            result.push(ch);
            remaining -= 1;
        }
        if remaining == 0 {
            break;
        }
    }
    Outcome {
        turn_id: text_field(turn, "id"),
        status: text_field(turn, "status"),
        text: result,
    }
}
fn text_field(v: &Value, key: &str) -> String {
    text(v, key).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v55_task_identity_ignores_retry_turn_but_rejects_changed_instructions() {
        let task = Delegation {
            parent: "parent".into(),
            source_turn: "first".into(),
            task_id: "build".into(),
            instructions: "task".into(),
            model: None,
            effort: None,
        };
        let mut retry = task.clone();
        retry.source_turn = "later".into();
        assert!(task.same_task(&retry));
        retry.instructions = "different".into();
        assert!(!task.same_task(&retry));
        let result = outcome(
            &json!({"id":"turn","status":"completed","items":[{"type":"reasoning","text":"private reasoning"},{"type":"agentMessage","text":"answer"},{"type":"mcpToolCall","text":"tool secret"}]}),
        );
        assert_eq!(result.text, "answer");
        let long = outcome(
            &json!({"id":"turn","items":[{"type":"agentMessage","text":"é".repeat(20001)}]}),
        );
        assert_eq!(long.text.chars().count(), 20000);
    }
}
