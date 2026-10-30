//! User-requested shell commands: durable intent, owned process, no model call.
use super::*;
use goose_provider_types::conversation::message::Message;
use goose_runtime::{config::paths::Paths, session::SessionManager};
use tokio::sync::watch;

#[derive(Clone)]
pub(super) struct Running {
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}
impl Running {
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
    }
    pub async fn stop(mut self) -> Result<(), JSONRPCErrorError> {
        self.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            while !*self.done.borrow_and_update() {
                self.done
                    .changed()
                    .await
                    .map_err(|_| error("Command cleanup stopped unexpectedly"))?;
            }
            Ok(())
        })
        .await
        .map_err(|_| error("Command is still stopping; retain this conversation for recovery"))?
    }
}

impl Adapter {
    pub(super) async fn shell_command(self: &Arc<Self>, params: &Value) -> Reply {
        if !self.embedded_runtime {
            return Err(unsupported("User shell commands require embedded Goose"));
        }
        let id = session_id(text(params, "threadId"))?.to_owned();
        let command = text(params, "command").to_owned();
        if command.trim().is_empty() || command.len() > 64 * 1024 || command.contains('\0') {
            return Err(error(
                "Command must be nonempty, contain no NUL, and be at most 64 KiB",
            ));
        }
        let timeout = match &params["timeoutMs"] {
            Value::Null => 3_600_000,
            value => value
                .as_u64()
                .filter(|n| *n <= 86_400_000)
                .ok_or_else(|| error("Command timeout must be 0–86400000 milliseconds"))?,
        };
        let gate = self.gate(&id).await;
        let _guard = gate.lock().await;
        let context = {
            let sessions = self.sessions.lock().await;
            let session = sessions
                .get(&id)
                .filter(|s| s.loaded)
                .ok_or_else(|| error("Open the conversation before running a command"))?;
            if session.active.is_some() || session.recovery_only || session.shell.is_some() {
                return Err(error(
                    "Wait for the current turn and resolve interrupted history before running a command",
                ));
            }
            crate::terminal::CommandContext {
                cwd: std::path::PathBuf::from(text(&session.info, "cwd")),
                workspace: session.binding.workspace.clone(),
            }
        };
        if self.load_queue(&id).await?.paused {
            return Err(error(
                "Recovered work is paused. Review it and choose Resume first.",
            ));
        }
        let tid = uuid::Uuid::new_v4().to_string();
        let message_id = format!("user-command-{tid}");
        let input = json!([{"type":"text","text":format!("!{command}"),"textElements":[]}]);
        self.checkpoint_capture(&id, &tid, "before", "Before user command")
            .await
            .map_err(|e| error(format!("Save command checkpoint: {e:#}")))?;
        self.claim_dispatch(&id, &tid, &json!({"input":input,"userCommand":true}))
            .await?;
        let manager = SessionManager::new(Paths::data_dir());
        let intent = Message::user().with_id(&message_id).with_text(format!("User-requested shell command (not a request for the assistant to execute it):\n{command}\n\nExecution was requested. If no result follows, its outcome is uncertain; never rerun it automatically."));
        if manager.add_message(&id, &intent).await.is_err() {
            self.settle_dispatch(&id, &tid, false).await;
            return Err(error(
                "Could not save command intent; the command was not launched",
            ));
        }
        let (cancel, receiver) = watch::channel(false);
        let (done, finished) = watch::channel(false);
        let mut started = turn(&tid, "inProgress");
        let user = json!({"type":"userMessage","id":message_id,"content":input});
        let item = json!({"type":"commandExecution","id":format!("command-{tid}"),"command":command,"cwd":context.cwd,
            "source":"userShell","status":"inProgress","commandActions":[],"aggregatedOutput":"","exitCode":null,"durationMs":null,"processId":null});
        started["gooseMessageId"] = json!(message_id);
        started["items"] = json!([user, item]);
        {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(&id)
                .ok_or_else(|| error("Conversation closed"))?;
            session.active = Some(tid.clone());
            session.run_id = None;
            session.cancel_requested = false;
            session.shell = Some(Running {
                cancel,
                done: finished,
            });
            session.turns.push(started.clone());
        }
        self.emit(
            json!({"method":"turn/started","params":{"threadId":ui_id(&id),"turn":started}}),
        )?;
        self.emit(json!({"method":"thread/status/changed","params":{"threadId":ui_id(&id),"status":{"type":"active","activeFlags":[]}}}))?;
        self.emit(json!({"method":"item/completed","params":{"threadId":ui_id(&id),"turnId":tid,"item":user}}))?;
        self.emit(json!({"method":"item/started","params":{"threadId":ui_id(&id),"turnId":tid,"item":item}}))?;
        let adapter = self.clone();
        self.jobs.send(Box::pin(async move {
            let (progress, mut updates) = watch::channel(None::<String>);
            let execution = crate::terminal::command::run(context, command.clone(), std::time::Duration::from_millis(timeout), receiver,
                move |text| { progress.send_replace(Some(text)); });
            tokio::pin!(execution);
            let began = std::time::Instant::now();
            let mut delivered = String::new();
            let outcome = loop {
                tokio::select! {
                    result = &mut execution => break result,
                    changed = updates.changed() => {
                        if changed.is_ok() {
                            let next = updates.borrow_and_update().clone();
                            if let Some(next) = next {
                                // Partial UTF-8 sequences may change the final replacement
                                // character; the authoritative final item corrects that tail.
                                if let Some(delta) = next.strip_prefix(&delivered) {
                                    let _ = adapter.emit(json!({"method":"item/commandExecution/outputDelta","params":{"threadId":ui_id(&id),"turnId":tid,"itemId":format!("command-{tid}"),"delta":delta}}));
                                    delivered = next;
                                }
                            }
                        }
                    }
                }
            };
            let (output, status, code, interrupted, cwd) = match outcome {
                Ok(result) => (result.output, result.status, result.exit_code, result.interrupted, result.cwd),
                Err(error) => (String::new(), format!("Command failed: {error:#}"), None, true, String::new()),
            };
            let clean = crate::transcript::strip_ansi(&output);
            let report = format!("User-requested shell command result:\nCommand: {command}\nStatus: {status}\nExit code: {}\n\n{}", code.map_or_else(|| "unavailable".into(), |c| c.to_string()), if clean.is_empty() { "(no output)" } else { &clean });
            let saved = manager.add_message(&id, &Message::assistant().with_id(format!("command-result-{tid}")).with_text(&report)).await.is_ok();
            {
                let mut sessions = adapter.sessions.lock().await;
                if let Some(session) = sessions.get_mut(&id).filter(|s| s.active.as_deref() == Some(&tid)) {
                    if let Some(item) = session.turn_mut()["items"].as_array_mut().and_then(|items| items.iter_mut().find(|i| i["type"] == "commandExecution")) {
                        item["aggregatedOutput"] = json!(format!("{output}\n{status}"));
                        item["status"] = json!(if code == Some(0) && !interrupted { "completed" } else { "failed" });
                        item["exitCode"] = json!(code);
                        if !cwd.is_empty() { item["cwd"] = json!(cwd); }
                        item["durationMs"] = json!(began.elapsed().as_millis().min(i64::MAX as u128) as i64);
                    }
                }
            }
            let reply = if saved { Ok(json!({"stopReason":if interrupted { "cancelled" } else { "end_turn" },"userCommand":true})) }
                else { Err(error("Command finished but its result could not be saved. Preserve the visible output and review recovery before resuming.")) };
            adapter.finish_turn(&id, &tid, reply).await;
            if let Some(session) = adapter.sessions.lock().await.get_mut(&id) { session.shell = None; }
            done.send_replace(true);
        })).await.map_err(|_| error("Backend stopped before command launch; review the retained intent"))?;
        Ok(json!({}))
    }
}
