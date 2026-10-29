use super::*;
use base64::Engine;

impl Adapter {
    pub(super) async fn new_session(self: &Arc<Self>, params: &Value) -> Reply {
        let cwd = params["cwd"].as_str().unwrap_or(&self.cwd);
        let tools = params["dynamicTools"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !tools.is_empty() && self.capabilities.lock().await["mcpCapabilities"]["http"] != true {
            return Err(error(
                "This Goose does not advertise HTTP MCP support required for Fastrock tools. Update Goose or disable these tools.",
            ));
        }
        let registration = if tools.is_empty() {
            None
        } else {
            Some(self.mcp.register(Arc::downgrade(self), tools.clone()).await)
        };
        let mut servers = registration
            .as_ref()
            .map(|r| vec![r.server()])
            .unwrap_or_default();
        let restricted =
            params["config"]["features.shell_tool"] == false || params["sandbox"] == "read-only";
        let bindings = if restricted {
            vec![]
        } else {
            let (vwork, bindings) = self.new_vwork_servers().await;
            servers.extend(vwork);
            bindings
        };
        let mut args = json!({"cwd":cwd,"mcpServers":servers,"_meta":{}});
        if params["ephemeral"] == true {
            args["_meta"]["hidden"] = json!(true);
        }
        if restricted {
            args["_meta"]["enabledExtensions"] = json!(
                servers
                    .iter()
                    .map(|server| json!({"type":"mcp","server":server}))
                    .collect::<Vec<_>>()
            );
        }
        if let Some(provider) = params["modelProvider"]
            .as_str()
            .filter(|p| !matches!(*p, "goose" | "openai"))
        {
            args["_meta"]["provider"] = json!(provider);
        }
        let reply = self.rpc.call("session/new", args).await;
        let response = match reply {
            Ok(r) => r,
            Err(e) => {
                if let Some(r) = registration {
                    self.mcp.remove(&r.key).await;
                }
                return Err(e);
            }
        };
        let id = response["sessionId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| error("Goose returned an empty session ID"))?;
        if let Some(r) = registration {
            self.mcp.bind(&r.key, id).await;
        }
        let setup = async {
            if params["ephemeral"] != true { self.save_vwork_bindings(id, &bindings)?; }
            if !tools.is_empty() && params["ephemeral"] != true {
                std::fs::create_dir_all(&self.queue_root).map_err(|e|error(format!("Save session tool definitions: {e}")))?;
                let bytes=serde_json::to_vec(&tools).map_err(|e|error(e.to_string()))?;
                crate::rally::store::atomic_write(&self.queue_path(id).with_extension("tools.json"),&bytes).map_err(|e|error(format!("Save session tool definitions: {e}")))?;
            }
            if restricted {
                let extensions = self.rpc.call("_goose/unstable/session/extensions/list",json!({"sessionId":id})).await?;
                let actual = extensions["extensions"].as_array().ok_or_else(||error("Cannot verify Goose's restricted tool configuration"))?;
                if actual.len()!=servers.len() || actual.iter().any(|e| e["extension"]["type"]!="mcp" || !servers.iter().any(|s|s["url"]==e["extension"]["server"]["url"])) {
                    return Err(error("Goose enabled tools outside this restricted Fastrock session"));
                }
            }
            let instructions=[params["baseInstructions"].as_str(),params["developerInstructions"].as_str()].into_iter().flatten().collect::<Vec<_>>().join("\n\n");
            if !instructions.is_empty() {
                self.rpc.call("_goose/unstable/session/system-prompt/set",json!({"sessionId":id,"mode":"append","key":"fastrock","text":instructions})).await?;
            }
            Ok::<_,JSONRPCErrorError>(())
        }.await;
        if let Err(e) = setup {
            let _ = self
                .rpc
                .call("session/delete", json!({"sessionId":id}))
                .await;
            self.mcp.remove_session(id).await;
            return Err(e);
        }
        {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .entry(id.into())
                .or_insert_with(|| Session::new(id, cwd));
            session.info["sessionId"] = json!(id);
            session.info["cwd"] = json!(response["_meta"]["workingDir"].as_str().unwrap_or(cwd));
            session.config = config_options(&response);
            session.loaded = true;
            session.ephemeral = params["ephemeral"] == true;
            self.config_event(id, session.config.clone())?;
        }
        if let Some(model) = params["model"]
            .as_str()
            .filter(|s| !s.is_empty() && *s != "__goose_default__")
        {
            let current = self
                .sessions
                .lock()
                .await
                .get(id)
                .map(|s| s.config_value("model").to_owned())
                .unwrap_or_default();
            if current != model {
                self.set_config(id, "model", &json!(model)).await?;
            }
        }
        let result = self
            .sessions
            .lock()
            .await
            .get(id)
            .ok_or_else(|| error("Goose session disappeared"))?
            .response();
        self.emit(json!({"method":"thread/started","params":{"thread":result["thread"]}}))?;
        Ok(result)
    }

    pub(super) async fn list_sessions(&self, params: &Value) -> Reply {
        if !params["parentThreadId"].is_null() || !params["ancestorThreadId"].is_null() {
            return Err(unsupported("Goose sub-agent thread enumeration"));
        }
        if self.capabilities.lock().await["sessionCapabilities"]["list"].is_null() {
            return Err(unsupported("session/list"));
        }
        let mut args = json!({"_meta":{"goose":{"includeLastMessageSnippet":true}}});
        for key in ["cursor", "cwd"] {
            if !params[key].is_null() {
                args[key] = params[key].clone();
            }
        }
        if let Some(search) = params["searchTerm"].as_str().filter(|s| !s.is_empty()) {
            args["_meta"]["query"] = json!(search);
        }
        let reply = self.rpc.call("session/list", args).await?;
        let mut sessions = self.sessions.lock().await;
        let mut data = Vec::new();
        for info in reply["sessions"].as_array().into_iter().flatten() {
            let id = text(info, "sessionId");
            if id.is_empty() {
                continue;
            }
            let session = sessions.entry(id.to_string()).or_default();
            session.info = info.clone();
            let archived = !info["_meta"]["archivedAt"].is_null();
            if !session.ephemeral && archived == params["archived"].as_bool().unwrap_or(false) {
                data.push(session.thread());
            }
        }
        Ok(json!({"data":data,"nextCursor":reply["nextCursor"],"backwardsCursor":null}))
    }

    pub(super) async fn load_session(self: &Arc<Self>, params: &Value, resume: bool) -> Reply {
        let id = session_id(text(params, "threadId"))?;
        let gate = self.gate(id).await;
        let _lock = gate.lock().await;
        let known = self.sessions.lock().await.contains_key(id);
        if !known {
            let info = self
                .rpc
                .call("_goose/unstable/session/info", json!({"sessionId":id}))
                .await?;
            let mut s = Session::new(id, &self.cwd);
            s.info = info["session"].clone();
            self.sessions.lock().await.insert(id.to_string(), s);
        }
        let needs_load = self
            .sessions
            .lock()
            .await
            .get(id)
            .is_some_and(|s| !s.loaded);
        if needs_load {
            if self.capabilities.lock().await["loadSession"] != true {
                return Err(unsupported("session/load"));
            }
            let cwd = {
                let mut sessions = self.sessions.lock().await;
                let s = sessions
                    .get_mut(id)
                    .ok_or_else(|| error("Session unavailable"))?;
                text(&s.info, "cwd").to_string()
            };
            let mut servers = self.mcp.servers(id).await;
            if servers.is_empty() {
                let path = self.queue_path(id).with_extension("tools.json");
                match std::fs::metadata(&path) {
                    Ok(meta) => {
                        if meta.len() > 8 * 1024 * 1024 {
                            return Err(error("Stored session tools exceed 8 MiB"));
                        }
                        let bytes = std::fs::read(path)
                            .map_err(|e| error(format!("Read session tools: {e}")))?;
                        let tools = serde_json::from_slice(&bytes)
                            .map_err(|e| error(format!("Invalid saved session tools: {e}")))?;
                        let registration = self.mcp.register(Arc::downgrade(self), tools).await;
                        self.mcp.bind(&registration.key, id).await;
                        servers.push(registration.server());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(error(format!("Read session tools: {e}"))),
                }
            }
            servers.extend(self.load_vwork_servers(id).await?);
            {
                let mut sessions = self.sessions.lock().await;
                let s = sessions
                    .get_mut(id)
                    .ok_or_else(|| error("Session unavailable"))?;
                s.replaying = true;
                s.turns.clear();
                s.last_chunk.clear();
            }
            let loaded = self
                .rpc
                .call(
                    "session/load",
                    json!({"sessionId":id,"cwd":cwd,"mcpServers":servers}),
                )
                .await;
            let mut sessions = self.sessions.lock().await;
            let s = sessions
                .get_mut(id)
                .ok_or_else(|| error("Session unavailable"))?;
            s.replaying = false;
            let response = match loaded {
                Ok(r) => r,
                Err(e) => {
                    s.turns.clear();
                    return Err(e);
                }
            };
            s.config = config_options(&response);
            s.loaded = true;
            self.config_event(id, s.config.clone())?;
        }
        let sessions = self.sessions.lock().await;
        let s = sessions
            .get(id)
            .ok_or_else(|| error("Session unavailable"))?;
        let mut result = if resume {
            s.response()
        } else {
            json!({"thread":s.thread()})
        };
        if params["includeTurns"] == true {
            result["thread"]["turns"] = json!(s.turns);
        }
        Ok(result)
    }

    pub(super) async fn history(&self, params: &Value, method: &str) -> Reply {
        let id = session_id(text(params, "threadId"))?;
        let sessions = self.sessions.lock().await;
        let s = sessions
            .get(id)
            .filter(|s| s.loaded)
            .ok_or_else(|| error("Load the Goose session before reading history"))?;
        let mut data = if method == "thread/turns/list" {
            s.turns
                .iter()
                .map(|t| {
                    let mut t = t.clone();
                    t["items"] = json!([]);
                    t["itemsView"] = json!("notLoaded");
                    t
                })
                .collect::<Vec<_>>()
        } else {
            s.turns
                .iter()
                .flat_map(|t| {
                    t["items"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|i| json!({"turnId":t["id"],"item":i}))
                })
                .collect()
        };
        if params["sortDirection"] != "asc" {
            data.reverse();
        }
        let offset = params["cursor"]
            .as_str()
            .map(|s| {
                s.strip_prefix("goose-page:")
                    .and_then(|n| n.parse::<usize>().ok())
                    .ok_or_else(|| error("Invalid Goose history cursor"))
            })
            .transpose()?
            .unwrap_or(0);
        let limit = params["limit"].as_u64().unwrap_or(100).clamp(1, 100) as usize;
        let next = (offset.saturating_add(limit) < data.len())
            .then(|| format!("goose-page:{}", offset + limit));
        Ok(
            json!({"data":data.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),"nextCursor":next,"backwardsCursor":null}),
        )
    }

    pub(super) async fn set_config(&self, id: &str, config_id: &str, value: &Value) -> Reply {
        {
            let sessions = self.sessions.lock().await;
            let s = sessions
                .get(id)
                .ok_or_else(|| error("Goose session unavailable"))?;
            let option = s
                .config
                .iter()
                .find(|o| o["id"] == config_id)
                .ok_or_else(|| unsupported(config_id))?;
            if !option_values(option).iter().any(|o| o["value"] == *value) {
                return Err(error(format!(
                    "Goose has not offered this {config_id} value for this session"
                )));
            }
        }
        let result = self
            .rpc
            .call(
                "session/set_config_option",
                json!({"sessionId":id,"configId":config_id,"value":value}),
            )
            .await?;
        if let Some(s) = self.sessions.lock().await.get_mut(id) {
            s.config = config_options(&result);
            self.config_event(id, s.config.clone())?;
        }
        Ok(result)
    }

    pub(super) async fn start_turn(self: &Arc<Self>, params: &Value) -> Reply {
        let id = session_id(text(params, "threadId"))?.to_string();
        let gate = self.gate(&id).await;
        let _lock = gate.lock().await;
        self.start_turn_locked(params).await
    }
    pub(super) async fn start_turn_locked(self: &Arc<Self>, params: &Value) -> Reply {
        if !params["outputSchema"].is_null() {
            return Err(unsupported("Structured output constraints"));
        }
        let id = session_id(text(params, "threadId"))?.to_string();
        {
            let sessions = self.sessions.lock().await;
            let s = sessions
                .get(&id)
                .filter(|s| s.loaded)
                .ok_or_else(|| error("Open the Goose session before sending"))?;
            if s.active.is_some() {
                return Err(error("This Goose conversation already has an active turn"));
            }
        }
        // ACP mode/config options are authoritative; Codex-only knobs are not sent.
        for (field, option) in [("model", "model"), ("effort", "thinking_effort")] {
            if let Some(value) = params[field]
                .as_str()
                .filter(|s| !s.is_empty() && *s != "__goose_default__")
            {
                let current = self
                    .sessions
                    .lock()
                    .await
                    .get(&id)
                    .map(|s| s.config_value(option).to_string())
                    .unwrap_or_default();
                if current != value {
                    self.set_config(&id, option, &json!(value)).await?;
                }
            }
        }
        let prompt = prompt(&params["input"])?;
        if prompt.iter().any(|c| c["type"] == "image")
            && self.capabilities.lock().await["promptCapabilities"]["image"] != true
        {
            return Err(unsupported("image prompts"));
        }
        let tid = uuid::Uuid::new_v4().to_string();
        let mut started = turn(&tid, "inProgress");
        let user = json!({"type":"userMessage","id":format!("user-{tid}"),"clientId":params["clientUserMessageId"],"content":params["input"]});
        started["items"] = json!([user]);
        {
            let mut sessions = self.sessions.lock().await;
            let s = sessions
                .get_mut(&id)
                .ok_or_else(|| error("Session unavailable"))?;
            s.active = Some(tid.clone());
            s.run_id = None;
            s.cancel_requested = false;
            s.last_chunk.clear();
            s.echo_user = Some(
                prompt
                    .iter()
                    .filter_map(|c| c["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            s.turns.push(started.clone());
        }
        self.emit(
            json!({"method":"turn/started","params":{"threadId":ui_id(&id),"turn":started}}),
        )?;
        self.emit(json!({"method":"item/completed","params":{"threadId":ui_id(&id),"turnId":tid,"item":user}}))?;
        // Enqueue the prompt before acknowledging it; a subsequent Stop cannot overtake it.
        let call = match self
            .rpc
            .begin("session/prompt", json!({"sessionId":id,"prompt":prompt}))
            .await
        {
            Ok(call) => call,
            Err(e) => {
                self.finish_turn(&id, &tid, Err(e.clone())).await;
                return Err(e);
            }
        };
        let adapter = self.clone();
        let response_turn = started.clone();
        self.jobs
            .send(Box::pin(async move {
                let reply = call.wait().await;
                adapter.finish_turn(&id, &tid, reply).await;
            }))
            .await
            .map_err(|_| error("Goose stopped before prompt dispatch"))?;
        Ok(json!({"turn":response_turn}))
    }

    async fn finish_turn(self: &Arc<Self>, id: &str, tid: &str, reply: Reply) {
        let completed = reply.as_ref().is_ok_and(|r| r["stopReason"] == "end_turn");
        self.cancel_answers(id).await;
        let mut sessions = self.sessions.lock().await;
        let Some(s) = sessions.get_mut(id) else {
            return;
        };
        if s.active.as_deref() != Some(tid) {
            return;
        }
        s.active = None;
        s.run_id = None;
        s.cancel_requested = false;
        s.echo_user = None;
        s.info["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
        s.info["_meta"]["lastMessageAt"] = s.info["updatedAt"].clone();
        let t = s.turn_mut();
        match reply {
            Ok(reply) => {
                t["status"] = json!(if reply["stopReason"] == "cancelled" {
                    "interrupted"
                } else {
                    "completed"
                });
                if let Some(reason) = reply["stopReason"]
                    .as_str()
                    .filter(|r| !matches!(*r, "end_turn" | "cancelled"))
                {
                    if let Some(items) = t["items"].as_array_mut() {
                        items.push(json!({"type":"agentMessage","id":format!("stop-{tid}"),"text":format!("Goose stopped: {reason}.")}));
                    }
                }
            }
            Err(e) => {
                t["status"] = json!("failed");
                t["error"] =
                    json!({"message":e.message,"additionalDetails":e.data.map(|d|d.to_string())});
            }
        }
        t["completedAt"] = json!(now());
        for item in t["items"].as_array().into_iter().flatten() {
            let _=self.emit(json!({"method":"item/completed","params":{"threadId":ui_id(id),"turnId":tid,"item":item}}));
        }
        let _ =
            self.emit(json!({"method":"turn/completed","params":{"threadId":ui_id(id),"turn":t}}));
        drop(sessions);
        if completed {
            self.schedule_queue(id.to_string()).await;
        }
    }

    pub(super) async fn steer(&self, params: &Value) -> Reply {
        let id = session_id(text(params, "threadId"))?;
        let run = {
            let sessions = self.sessions.lock().await;
            let s = sessions
                .get(id)
                .ok_or_else(|| error("Goose session unavailable"))?;
            if s.active.as_deref() != params["expectedTurnId"].as_str() {
                return Err(error(
                    "The Goose turn has changed; send again or queue the message",
                ));
            }
            s.run_id
                .clone()
                .ok_or_else(|| error("Goose has not reported a steerable run yet"))?
        };
        let reply = self
            .rpc
            .call(
                "_goose/unstable/session/steer",
                json!({"sessionId":id,"expectedRunId":run,"prompt":prompt(&params["input"])?}),
            )
            .await?;
        if let Some(message_id) = reply["messageId"].as_str() {
            let mut sessions = self.sessions.lock().await;
            if let Some(s) = sessions.get_mut(id) {
                s.user_clients
                    .insert(message_id.into(), params["clientUserMessageId"].clone());
                let tid = s.active.clone();
                if let Some(item) = s.turn_mut()["items"]
                    .as_array_mut()
                    .and_then(|items| items.iter_mut().find(|i| i["id"] == message_id))
                {
                    item["clientId"] = params["clientUserMessageId"].clone();
                    self.emit(json!({"method":"item/completed","params":{"threadId":ui_id(id),"turnId":tid,"item":item}}))?;
                }
            }
        }
        Ok(json!({"turnId":params["expectedTurnId"]}))
    }

    pub(super) async fn manage(self: &Arc<Self>, params: &Value, method: &str) -> Reply {
        let id = session_id(text(params, "threadId"))?;
        let suffix = match method {
            "thread/setName" => "rename",
            "thread/archive" => "archive",
            "thread/unarchive" => "unarchive",
            _ => "compact",
        };
        let mut args = json!({"sessionId":id});
        if suffix == "rename" {
            args["title"] = params["name"].clone();
        }
        let result = self
            .rpc
            .call(&format!("_goose/unstable/session/{suffix}"), args)
            .await?;
        if suffix == "rename" {
            if let Some(s) = self.sessions.lock().await.get_mut(id) {
                s.info["title"] = params["name"].clone();
            }
            self.emit(json!({"method":"thread/name/updated","params":{"threadId":ui_id(id),"threadName":params["name"]}}))?;
        }
        if suffix == "unarchive" {
            return self.load_session(params, false).await;
        }
        Ok(result)
    }

    pub(super) async fn cancel_answers(&self, id: &str) {
        let mut answers = self.answers.lock().await;
        let keys = answers
            .iter()
            .filter(|(_, a)| match a {
                Answer::Permission { session, .. } | Answer::Tool { session, .. } => session == id,
            })
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(Answer::Permission { id: rpc_id, .. }) = answers.remove(&key) {
                let _=self.rpc.send(json!({"jsonrpc":"2.0","id":rpc_id,"result":{"outcome":{"outcome":"cancelled"}}})).await;
            }
        }
    }
    pub(super) async fn close_session(&self, params: &Value) -> Reply {
        let id = session_id(text(params, "threadId"))?;
        let gate = self.gate(id).await;
        let _lock = gate.lock().await;
        let ephemeral = self
            .sessions
            .lock()
            .await
            .get(id)
            .is_some_and(|s| s.ephemeral);
        let method = if ephemeral {
            "session/delete"
        } else {
            "session/close"
        };
        let capability = if ephemeral { "delete" } else { "close" };
        if !self.capabilities.lock().await["sessionCapabilities"][capability].is_null() {
            self.rpc.call(method, json!({"sessionId":id})).await?;
        }
        self.cancel_answers(id).await;
        self.mcp.remove_session(id).await;
        self.sessions.lock().await.remove(id);
        Ok(json!({"status":"unsubscribed"}))
    }
}

pub(super) fn config_options(response: &Value) -> Vec<Value> {
    if let Some(options) = response["configOptions"].as_array() {
        return options.clone();
    }
    // Older ACP agents expose modes/models separately. Keep their exact IDs.
    let mut options = Vec::new();
    for (key, id, name, current, available) in [
        ("modes", "mode", "Mode", "currentModeId", "availableModes"),
        (
            "models",
            "model",
            "Model",
            "currentModelId",
            "availableModels",
        ),
    ] {
        if let Some(choices) = response[key][available].as_array() {
            options.push(json!({"id":id,"name":name,"type":"select","currentValue":response[key][current],
                "options":choices.iter().map(|o|json!({"value":o["id"].as_str().or_else(||o["modelId"].as_str()),"name":o["name"],"description":o["description"]})).collect::<Vec<_>>()}));
        }
    }
    options
}
pub(crate) fn option_values(option: &Value) -> Vec<Value> {
    option["options"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|v| {
            v["options"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![v.clone()])
        })
        .collect()
}
fn prompt(input: &Value) -> Result<Vec<Value>, JSONRPCErrorError> {
    input.as_array().ok_or_else(||error("Prompt input must be an array"))?.iter().map(|part|match text(part,"type"){
        "text"=>Ok(json!({"type":"text","text":part["text"]})),
        "image"=>{
            let url=text(part,"url");
            let (header,data)=url.split_once(",").ok_or_else(||error("Goose image input must be an embedded data URL"))?;
            let mime=header.strip_prefix("data:").and_then(|s|s.strip_suffix(";base64")).ok_or_else(||error("Unsupported image encoding"))?;
            base64::engine::general_purpose::STANDARD.decode(data).map_err(|_|error("Invalid image data"))?;
            Ok(json!({"type":"image","mimeType":mime,"data":data}))
        }
        "localImage"=>{
            let path=std::path::Path::new(text(part,"path"));
            let size=std::fs::metadata(path).map_err(|e|error(format!("Image metadata: {e}")))?.len();
            if size>16*1024*1024{return Err(error("Image exceeds 16 MiB"))}
            let bytes=std::fs::read(path).map_err(|e|error(format!("Image read: {e}")))?;
            let format=image::guess_format(&bytes).map_err(|_|error("Unsupported image format"))?;
            Ok(json!({"type":"image","mimeType":format.to_mime_type(),"data":base64::engine::general_purpose::STANDARD.encode(bytes)}))
        },
        _=>Err(error(format!("Goose does not support this input type: {}",text(part,"type")))),
    }).collect()
}
