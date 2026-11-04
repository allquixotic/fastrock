//! Saved messages are readable without credentials, extensions or a workspace.
//! Agent activation is a separate, explicit Resume operation.
use super::*;
use goose_provider_types::conversation::{Conversation, message::MessageContentBlock as Block};

pub(super) fn project_saved(session: &mut Session, conversation: &Conversation) {
    session.replaying = true;
    for (index, original) in conversation.messages().iter().enumerate() {
        if !original.is_user_visible() {
            continue;
        }
        let message = original.user_visible_content();
        let id = message
            .id
            .clone()
            .unwrap_or_else(|| format!("stored-{index}"));
        let user = serde_json::to_value(&message.role).ok() == Some(json!("user"));
        for block in &message.content {
            let mut update = match block {
                Block::Text(t) => chunk(user, false, json!({"type":"text","text":t.text})),
                Block::Image(i) => chunk(
                    user,
                    false,
                    json!({"type":"image","data":i.data,"mimeType":i.mime_type}),
                ),
                Block::Thinking(t) => chunk(false, true, json!({"type":"text","text":t.thinking})),
                Block::Error(e) => chunk(user, false, json!({"type":"text","text":e.message})),
                Block::ToolRequest(t) => match &t.tool_call {
                    Ok(call) => {
                        json!({"sessionUpdate":"tool_call","toolCallId":t.id,"title":call.name,"rawInput":call.arguments,"status":"pending"})
                    }
                    Err(e) => {
                        json!({"sessionUpdate":"tool_call","toolCallId":t.id,"title":"Invalid tool request","status":"failed","rawOutput":e.to_string()})
                    }
                },
                Block::ToolResponse(t) => match &t.tool_result {
                    Ok(result) => {
                        json!({"sessionUpdate":"tool_call_update","toolCallId":t.id,"status":if result.is_error==Some(true){"failed"}else{"completed"},"content":result.content.iter().map(|c|json!({"type":"content","content":c})).collect::<Vec<_>>()})
                    }
                    Err(e) => {
                        json!({"sessionUpdate":"tool_call_update","toolCallId":t.id,"status":"failed","rawOutput":e.to_string()})
                    }
                },
                Block::SystemNotification(_) => continue,
                // Keep documents and recovery confirmations inspectable; do
                // not turn them into executable actions or approval requests.
                _ => chunk(
                    false,
                    false,
                    json!({"type":"text","text":block.to_string()}),
                ),
            };
            update["messageId"] = json!(id);
            update["_meta"] = json!({"goose":{"messageId":id,"created":message.created,"steer":message.metadata.steer}});
            session.update(&update);
        }
        if message.metadata.output_token_limit_reached {
            let mut update = chunk(
                false,
                false,
                json!({"type":"text","text":"Response stopped because the model reached its output-token limit."}),
            );
            update["messageId"] = json!(id);
            session.update(&update);
        }
    }
    session.recovery_only = super::recovery_history::uncertain(conversation);
    if session.recovery_only {
        session.update(&json!({"sessionUpdate":"agent_message_chunk","messageId":"fastrock-recovery-notice",
            "content":{"type":"text","text":"**Recovery review required**\n\nThis conversation contains tool calls whose outcomes are uncertain. Open Checkpoints to restore a completed boundary, or fork before the interrupted turn. No saved tools have been resumed."}}));
    }
    session.replaying = false;
    session.loaded = true;
    session.activation_pending = true;
}

fn chunk(user: bool, thought: bool, content: Value) -> Value {
    json!({"sessionUpdate":if user{"user_message_chunk"}else if thought{"agent_thought_chunk"}else{"agent_message_chunk"},"content":content})
}

impl Adapter {
    pub(super) async fn open_saved_history(
        self: &Arc<Self>,
        params: &Value,
        resume: bool,
    ) -> Reply {
        let id = session_id(text(params, "threadId"))?;
        // Browsing already-published history must not wait behind a slow Resume.
        if let Some(reply) = self.saved_history_response(id, params, resume).await? {
            return Ok(reply);
        }
        let _timing = super::latency::Timer::start(super::latency::Phase::RestoreHistory);
        let _foreground = self.summaries.as_ref().map(|s| s.foreground());
        let gate = self.gate(id).await;
        let _lock = gate.lock().await;
        if let Some(reply) = self.saved_history_response(id, params, resume).await? {
            return Ok(reply);
        }
        self.deletion_ready(id).await?;
        let stored = goose_runtime::session::SessionManager::instance()
            .get_session(id, true)
            .await
            .map_err(|e| error(format!("Read saved conversation: {e}")))?;
        let mut session = Session::new(id, &stored.working_dir.to_string_lossy());
        session.info = json!({"sessionId":id,"cwd":stored.working_dir,"title":stored.name,"updatedAt":stored.updated_at,
            "_meta":{"createdAt":stored.created_at,"lastMessageAt":stored.last_message_at,"lastMessageSnippet":stored.last_message_snippet,
            "providerId":stored.provider_name,"modelId":stored.model_config.as_ref().map(|m|m.model_name.as_str())}});
        session.binding = self.binding_record(id).await?;
        session.ephemeral = session.binding.side;
        let conversation = stored.conversation.unwrap_or_default();
        // Projection/cold-cache preparation stays off Tokio's IO workers and
        // never holds the cross-conversation session registry while doing IO.
        let session = tokio::task::spawn_blocking(move || {
            project_saved(&mut session, &conversation);
            session
        })
        .await
        .map_err(|_| error("Saved history projection failed"))?;
        self.sessions.lock().await.insert(id.to_owned(), session);
        // Queue recovery is independent of browsing. Its notification asks the
        // UI to refresh drafts separately; slow sidecar IO cannot hide messages.
        // Pausing must preserve any already-published interrupted sends. The
        // separate queue refresh publishes its full state, including cache hits.
        let _ = self.events.try_send(AppServerEvent::GooseHistoryPaused {
            thread_id: ui_id(id),
        });
        let _ = self.emit(json!({"method":"thread/queue/changed","params":{"threadId":ui_id(id)}}));
        self.saved_history_response(id, params, resume)
            .await?
            .ok_or_else(|| error("Saved history unavailable"))
    }

    async fn saved_history_response(
        &self,
        id: &str,
        params: &Value,
        resume: bool,
    ) -> Result<Option<Value>, JSONRPCErrorError> {
        let mut sessions = self.sessions.lock().await;
        let Some(session) = sessions.get_mut(id).filter(|s| s.loaded) else {
            return Ok(None);
        };
        let mut response = if resume {
            session.response()
        } else {
            json!({"thread":session.thread()})
        };
        if params["includeTurns"] == true {
            response["thread"]["turns"] = json!(
                session
                    .turns
                    .all_turns()
                    .map_err(|_| error("Read saved history failed"))?
            );
        }
        Ok(Some(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use goose_provider_types::conversation::message::Message;
    #[test]
    fn v100_storage_projection_preserves_messages_without_activation() {
        let mut hidden = Message::assistant().with_text("hidden context");
        hidden.metadata.user_visible = false;
        let conversation = Conversation::new_unvalidated([
            Message::user().with_text("Original question").with_id("u1"),
            Message::assistant()
                .with_text("Original answer")
                .with_id("a1"),
            hidden,
        ]);
        let mut session = Session::new("saved", "C:\\unavailable");
        project_saved(&mut session, &conversation);
        assert!(session.loaded && session.activation_pending && !session.recovery_only);
        let history = json!(session.turns.all_turns().unwrap());
        let text = history.to_string();
        assert!(text.contains("Original question") && text.contains("Original answer"));
        assert!(!text.contains("hidden context"));
        assert_eq!(history[0]["gooseMessageId"], "u1");
        session.replaying = true;
        session.discard_replay = true;
        session.update(&json!({"sessionUpdate":"agent_message_chunk","messageId":"a1","content":{"type":"text","text":"duplicate"}}));
        assert_eq!(json!(session.turns.all_turns().unwrap()), history);
    }
    #[test]
    fn v100_storage_projection_preserves_tools_thinking_images_and_steering() {
        let conversation = Conversation::new_unvalidated([
            Message::user()
                .with_id("question")
                .with_text("Inspect")
                .with_image("aW1hZ2U=", "image/png"),
            Message::assistant()
                .with_id("request")
                .with_thinking("visible thought", "signature")
                .with_tool_request(
                    "call",
                    Ok(serde_json::from_value(
                        json!({"name":"read_file","arguments":{"path":"file.rs"}}),
                    )
                    .unwrap()),
                ),
            Message::user().with_id("result").with_tool_response(
                "call",
                Ok(serde_json::from_value(
                    json!({"content":[{"type":"text","text":"saved tool output"}],"isError":false}),
                )
                .unwrap()),
            ),
            Message::user()
                .with_id("steer")
                .with_text("Keep original")
                .with_steer(),
            Message::assistant().with_id("answer").with_text("Finished"),
        ]);
        let mut session = Session::new("stored", "C:\\offline");
        project_saved(&mut session, &conversation);
        let history = session.turns.all_turns().unwrap();
        assert_eq!(
            history.len(),
            1,
            "Tool results and steering must not split a turn"
        );
        assert_eq!(history[0]["gooseMessageId"], "question");
        let items = history[0]["items"].as_array().unwrap();
        let tool = items.iter().find(|i| i["id"] == "call").unwrap();
        assert_eq!(tool["tool"], "read_file");
        assert_eq!(tool["status"], "completed");
        assert_eq!(tool["arguments"]["path"], "file.rs");
        let serialized = json!(items).to_string();
        for expected in [
            "visible thought",
            "saved tool output",
            "data:image/png;base64,aW1hZ2U=",
            "Keep original",
            "Finished",
        ] {
            assert!(serialized.contains(expected), "Missing {expected}");
        }
        assert!(!session.recovery_only);
    }
}
