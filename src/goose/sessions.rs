use super::*;
use base64::Engine;

impl Adapter {
    pub(super) async fn new_session(self: &Arc<Self>, params: &Value) -> Reply {
        let _foreground = self.summaries.as_ref().map(|s| s.foreground());
        let provider = if self.embedded_runtime {
            let requested = params["modelProvider"]
                .as_str()
                .filter(|name| !name.is_empty() && *name != "goose");
            super::provider_policy::selected(requested.map(str::to_owned))
                .await
                .map_err(|e| error(e.to_string()))?;
            // Omit the default alias: an explicit provider makes Goose choose
            // its registry model instead of the user's saved default model.
            requested.map(str::to_owned)
        } else {
            None
        };
        let workspace = self.selected_workspace(params).await?;
        let workspace_cwd = workspace
            .as_ref()
            .map(|w| w.cwd.to_string_lossy().into_owned());
        let cwd = workspace_cwd
            .as_deref()
            .unwrap_or_else(|| params["cwd"].as_str().unwrap_or(&self.cwd));
        let mut tools = params["dynamicTools"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if workspace.is_some() {
            super::delegation_tools::add(&mut tools);
        }
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
            let result = self.new_vwork_servers(cwd, workspace.as_ref()).await;
            let (vwork, bindings) = match result {
                Ok(value) => value,
                Err(error) => {
                    if let Some(registration) = &registration {
                        self.mcp.remove(&registration.key).await;
                    }
                    return Err(error);
                }
            };
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
        if let Some(provider) = provider.as_deref().or_else(|| {
            params["modelProvider"]
                .as_str()
                .filter(|p| !matches!(*p, "goose" | "openai"))
        }) {
            args["_meta"]["provider"] = json!(provider);
        }
        match super::recipes::session_recipe(params, &servers).await {
            Ok(Some(deeplink)) => args["_meta"]["recipeDeeplink"] = json!(deeplink),
            Ok(None) => {}
            Err(error) => {
                if let Some(registration) = &registration {
                    self.mcp.remove(&registration.key).await;
                }
                return Err(error);
            }
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
            if workspace.is_some() {
                self.save_binding_record(id, &super::workspace::BindingRecord { workspace: workspace.clone(), forked_from: None, delegation: None, side: false })?;
            }
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
            if let Some(workspace) = &workspace {
                self.rpc.call("_goose/unstable/session/system-prompt/set", json!({"sessionId":id,"mode":"append","key":"fastrock-workspace","text":super::workspace::instructions(workspace)})).await?;
            }
            self.restrict_native_delegation(id).await?;
            Ok::<_,JSONRPCErrorError>(())
        }.await;
        if let Err(e) = setup {
            // Bindings may already exist. Never make their identity reusable.
            let safe_to_delete = !self.embedded_runtime
                || super::id_reservation::before_delete(
                    &goose_runtime::session::SessionManager::new(
                        goose_runtime::config::paths::Paths::data_dir(),
                    ),
                    id,
                )
                .await
                .is_ok();
            if safe_to_delete {
                let _ = self
                    .rpc
                    .call("session/delete", json!({"sessionId":id}))
                    .await;
            }
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
            if let Some(title) = response["_meta"]["recipe"]["title"].as_str() {
                session.info["title"] = json!(title);
            }
            session.config = config_options(&response);
            session.loaded = true;
            session.ephemeral = params["ephemeral"] == true;
            session.binding.workspace = workspace;
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
        self.apply_effort_default(id).await?;
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

    /// Metadata-only lookup for pinned rows outside the first catalog page.
    /// Does not load an agent, extensions, history, or provider credentials.
    pub(super) async fn sidebar_metadata(&self, params: &Value) -> Reply {
        let ids = params["ids"]
            .as_array()
            .filter(|ids| ids.len() <= 128)
            .ok_or_else(|| error("Expected at most 128 conversation IDs"))?;
        let mut data = Vec::new();
        for value in ids {
            let id = session_id(
                value
                    .as_str()
                    .ok_or_else(|| error("Invalid conversation ID"))?,
            )?;
            if id.len() > 512 {
                return Err(error("Invalid conversation ID"));
            }
            let Ok(info) = self
                .rpc
                .call("_goose/unstable/session/info", json!({"sessionId":id}))
                .await
            else {
                continue;
            };
            let info = &info["session"];
            if !info["_meta"]["archivedAt"].is_null()
                || matches!(
                    info["_meta"]["sessionType"].as_str(),
                    Some("hidden" | "sub_agent" | "terminal")
                )
            {
                continue;
            }
            if self
                .sessions
                .lock()
                .await
                .get(id)
                .is_some_and(|s| s.ephemeral)
            {
                continue;
            }
            let mut session = Session::new(id, &self.cwd);
            session.info = info.clone();
            session.binding = self.binding_record(id).await?;
            if !session.binding.side && session.binding.delegation.is_none() {
                data.push(session.thread());
            }
        }
        Ok(json!({"data": data}))
    }

    pub(super) async fn list_sessions(&self, params: &Value) -> Reply {
        if !params["parentThreadId"].is_null() {
            return self.child_threads(params).await;
        }
        if !params["ancestorThreadId"].is_null() {
            return Err(unsupported(
                "Descendant enumeration; list direct children recursively",
            ));
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
        let mut data = Vec::new();
        for info in reply["sessions"].as_array().into_iter().flatten() {
            let id = text(info, "sessionId");
            if id.is_empty() || self.deleted(id).await {
                continue;
            }
            let binding = self.binding_record(id).await?;
            let mut sessions = self.sessions.lock().await;
            let session = sessions.entry(id.to_string()).or_default();
            session.ephemeral |= binding.side;
            session.binding = binding;
            session.info = info.clone();
            let archived = !info["_meta"]["archivedAt"].is_null();
            if !session.ephemeral && archived == params["archived"].as_bool().unwrap_or(false) {
                data.push(session.thread());
            }
        }
        Ok(json!({"data":data,"nextCursor":reply["nextCursor"],"backwardsCursor":null}))
    }

    pub(super) async fn load_session(self: &Arc<Self>, params: &Value, resume: bool) -> Reply {
        let _foreground = self.summaries.as_ref().map(|s| s.foreground());
        let id = session_id(text(params, "threadId"))?;
        let gate = self.gate(id).await;
        let _lock = gate.lock().await;
        self.deletion_ready(id).await?;
        let known = self.sessions.lock().await.contains_key(id);
        if !known {
            let info = self
                .rpc
                .call("_goose/unstable/session/info", json!({"sessionId":id}))
                .await?;
            let mut s = Session::new(id, &self.cwd);
            s.info = info["session"].clone();
            s.binding = self.binding_record(id).await?;
            s.ephemeral = s.binding.side;
            self.sessions.lock().await.insert(id.to_string(), s);
        }
        let needs_load = self
            .sessions
            .lock()
            .await
            .get(id)
            .is_some_and(|s| !s.loaded);
        if needs_load && !self.recovery_history(id).await? {
            if self.embedded_runtime {
                super::provider_policy::restored(id)
                    .await
                    .map_err(|e| error(e.to_string()))?;
                super::restore_extensions::prepare(id)
                    .await
                    .map_err(|e| error(format!("Restore Goose tools: {e:#}")))?;
            }
            if self.capabilities.lock().await["loadSession"] != true {
                return Err(unsupported("session/load"));
            }
            let (cwd, workspace, side) = {
                let mut sessions = self.sessions.lock().await;
                let s = sessions
                    .get_mut(id)
                    .ok_or_else(|| error("Session unavailable"))?;
                (
                    text(&s.info, "cwd").to_string(),
                    s.binding.workspace.clone(),
                    s.binding.side,
                )
            };
            if let Some(workspace) = &workspace {
                let resolved = workspace
                    .selection
                    .clone()
                    .resolve()
                    .await
                    .map_err(|e| error(format!("Restore bound workspace: {e:#}")))?;
                if tokio::fs::canonicalize(&cwd).await.ok().as_ref() != Some(&resolved.cwd) {
                    return Err(error(
                        "The saved conversation directory differs from its workspace; preserve the binding for recovery",
                    ));
                }
            }
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
                        let mut tools = serde_json::from_slice(&bytes)
                            .map_err(|e| error(format!("Invalid saved session tools: {e}")))?;
                        if side {
                            super::side::filter_tools(&mut tools);
                        } else if workspace.is_some() {
                            super::delegation_tools::add(&mut tools);
                        }
                        let registration = self.mcp.register(Arc::downgrade(self), tools).await;
                        self.mcp.bind(&registration.key, id).await;
                        servers.push(registration.server());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        if workspace.is_some() && !side {
                            let mut tools = Vec::new();
                            super::delegation_tools::add(&mut tools);
                            let registration = self.mcp.register(Arc::downgrade(self), tools).await;
                            self.mcp.bind(&registration.key, id).await;
                            servers.push(registration.server());
                        }
                    }
                    Err(e) => return Err(error(format!("Read session tools: {e}"))),
                }
            }
            servers.extend(self.load_vwork_servers(id, &cwd).await?);
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
            drop(sessions);
            self.restrict_native_delegation(id).await?;
            let mut sessions = self.sessions.lock().await;
            let s = sessions
                .get_mut(id)
                .ok_or_else(|| error("Session unavailable"))?;
            s.loaded = true;
            self.config_event(id, s.config.clone())?;
        }
        let mut sessions = self.sessions.lock().await;
        let s = sessions
            .get_mut(id)
            .ok_or_else(|| error("Session unavailable"))?;
        let mut result = if resume {
            s.response()
        } else {
            json!({"thread":s.thread()})
        };
        if params["includeTurns"] == true {
            result["thread"]["turns"] = json!(
                s.turns
                    .all_turns()
                    .map_err(|_| error("Restore Goose history failed"))?
            );
        }
        drop(sessions);
        self.queue_event(id, self.load_queue(id).await?);
        Ok(result)
    }

    pub(super) async fn history(&self, params: &Value, method: &str) -> Reply {
        let id = session_id(text(params, "threadId"))?;
        let mut sessions = self.sessions.lock().await;
        let s = sessions
            .get_mut(id)
            .filter(|s| s.loaded)
            .ok_or_else(|| error("Load the Goose session before reading history"))?;
        let turns = method == "thread/turns/list";
        let total = if turns {
            s.turns.len()
        } else {
            s.turns.item_count()
        };
        let page = super::history::page(params, total, turns)?;
        let data = if turns {
            s.turns.turns_page(page.offset, page.limit, page.descending)
        } else {
            s.turns
                .items_page(page.offset, page.limit, page.descending)
                .map_err(|_| error("Restore Goose history page failed"))?
        };
        Ok(json!({"data":data,"nextCursor":page.next,"backwardsCursor":null}))
    }

    pub(super) async fn history_window(&self, params: &Value) -> Reply {
        let id = session_id(text(params, "threadId"))?;
        let start = match params.get("start").filter(|value| !value.is_null()) {
            None => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| error("Invalid history position"))?,
            ),
        };
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(id)
            .filter(|session| session.loaded)
            .ok_or_else(|| error("Load the Goose session before browsing history"))?;
        let window = session
            .turns
            .window(start, 100)
            .map_err(|_| error("Restore Goose history window failed"))?;
        serde_json::to_value(window).map_err(|_| error("Encode Goose history window failed"))
    }

    pub(super) async fn set_config(&self, id: &str, config_id: &str, value: &Value) -> Reply {
        let result = self.set_config_raw(id, config_id, value).await?;
        if matches!(config_id, "provider" | "model") {
            if let Some(updated) = self.apply_effort_default(id).await? {
                return Ok(updated);
            }
        }
        Ok(result)
    }

    pub(super) async fn set_config_raw(&self, id: &str, config_id: &str, value: &Value) -> Reply {
        if self.embedded_runtime && config_id == "provider" {
            let requested = value.as_str().ok_or_else(|| error("Choose a provider"))?;
            super::provider_policy::selected(Some(requested.to_owned()))
                .await
                .map_err(|e| error(e.to_string()))?;
        }
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
            if self.embedded_runtime && config_id == "model" {
                if !valid_model_id(value) {
                    return Err(error(
                        "Enter a model ID of at most 1024 bytes without control characters",
                    ));
                }
            } else if !option_values(option).iter().any(|o| o["value"] == *value) {
                return Err(error(format!(
                    "Goose has not offered this {config_id} value for this session"
                )));
            }
        }
        let mut result = self
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
        if self.embedded_runtime {
            result["configOptions"] = json!(super::provider_policy::visible_options(
                config_options(&result)
            ));
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
        let requested_at = std::time::Instant::now();
        let foreground = self.summaries.as_ref().map(|s| s.foreground());
        self.deletion_ready(session_id(text(params, "threadId"))?)
            .await?;
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
            if s.recovery_only {
                return Err(error(
                    "Review the uncertain history in Checkpoints or fork before the interrupted turn before sending new work",
                ));
            }
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
        self.checkpoint_capture(&id, &tid, "before", "Before turn")
            .await
            .map_err(|e| error(format!("Save checkpoint before turn: {e:#}")))?;
        if !self
            .sessions
            .lock()
            .await
            .get(&id)
            .is_some_and(|s| s.ephemeral && !s.binding.side)
        {
            self.claim_dispatch(&id, &tid, params).await?;
        }
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
            s.first_response_since = Some(requested_at);
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
        self.emit(json!({"method":"thread/status/changed","params":{"threadId":ui_id(&id),"status":{"type":"active","activeFlags":[]}}}))?;
        self.changed.notify_waiters();
        self.emit(json!({"method":"item/completed","params":{"threadId":ui_id(&id),"turnId":tid,"item":user}}))?;
        // Enqueue the prompt before acknowledging it; a subsequent Stop cannot overtake it.
        latency::record(latency::Phase::TurnSetup, requested_at);
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
                let _foreground = foreground;
                let reply = call.wait().await;
                adapter.finish_turn(&id, &tid, reply).await;
            }))
            .await
            .map_err(|_| error("Goose stopped before prompt dispatch"))?;
        Ok(json!({"turn":response_turn}))
    }

    pub(super) async fn finish_turn(self: &Arc<Self>, id: &str, tid: &str, reply: Reply) {
        if self.deletion_ready(id).await.is_err() {
            return;
        }
        let completed = reply.as_ref().is_ok_and(|r| r["stopReason"] == "end_turn");
        let user_command = reply.as_ref().is_ok_and(|r| r["userCommand"] == true);
        self.cancel_answers(id).await;
        let capture = if completed {
            self.checkpoint_capture(id, tid, "after", "Completed turn")
                .await
        } else {
            Ok(None)
        };
        let captured = capture.is_ok();
        if let Err(e) = capture {
            let _=self.emit(json!({"method":"error","params":{"threadId":ui_id(id),"turnId":tid,"willRetry":false,"error":{"message":format!("Save completed-turn checkpoint: {e:#}")}}}));
        }
        let mut sessions = self.sessions.lock().await;
        let Some(s) = sessions.get_mut(id) else {
            return;
        };
        if s.active.as_deref() != Some(tid) {
            return;
        }
        s.info["updatedAt"] = json!(chrono::Utc::now().to_rfc3339());
        s.info["_meta"]["lastMessageAt"] = s.info["updatedAt"].clone();
        let summarize = completed && !s.ephemeral && !user_command;
        let delegated = s.binding.delegation.is_some();
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
        let completion =
            json!({"method":"turn/completed","params":{"threadId":ui_id(id),"turn":t}});
        let outcome = delegated.then(|| super::delegation::outcome(t));
        drop(sessions);
        let mut saved = captured;
        if let Some(outcome) = outcome {
            if let Err(e) = self.save_delegate_outcome(id, outcome).await {
                saved = false;
                let _=self.emit(json!({"method":"error","params":{"threadId":ui_id(id),"turnId":tid,"willRetry":false,"error":{"message":e.message}}}));
            }
        }
        self.settle_dispatch(id, tid, completed && saved).await;
        // Queue summary replacement before publishing Idle. Metadata operations
        // such as title generation must not observe an obsolete summary as ready.
        if summarize {
            if let Some(service) = &self.summaries {
                service.completed(id.to_owned()).await;
            }
        }
        {
            // Keep restore/start blocked through durable completion. Publish its
            // events while holding the same state lock that admits idle work.
            let mut sessions = self.sessions.lock().await;
            let Some(s) = sessions
                .get_mut(id)
                .filter(|s| s.active.as_deref() == Some(tid))
            else {
                return;
            };
            s.active = None;
            s.first_response_since = None;
            s.run_id = None;
            s.cancel_requested = false;
            s.echo_user = None;
            let _=self.emit(json!({"method":"thread/status/changed","params":{"threadId":ui_id(id),"status":{"type":"idle"}}}));
            if let Err(error) = self.emit(completion) {
                tracing::error!(error = %error.message, "could not deliver turn completion");
            }
        }
        self.changed.notify_waiters();
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
            "thread/name/set" | "thread/setName" => "rename",
            "thread/archive" => "archive",
            "thread/unarchive" => "unarchive",
            _ => "compact",
        };
        // Compaction mutates the same public conversation storage as rewind.
        // Serialize it with turns/restores and leave uncertain history intact.
        let compact_gate = if matches!(suffix, "compact" | "rename") {
            Some(self.gate(id).await)
        } else {
            None
        };
        let _compact_lock = match &compact_gate {
            Some(gate) => Some(gate.lock().await),
            None => None,
        };
        if suffix == "compact" {
            let sessions = self.sessions.lock().await;
            let session = sessions
                .get(id)
                .ok_or_else(|| error("Open the conversation before compacting"))?;
            if session.active.is_some() || session.recovery_only {
                return Err(error(
                    "Wait for the turn or resolve recovery before compacting",
                ));
            }
            drop(sessions);
            self.checkpoint_ready(id)
                .await
                .map_err(|e| error(e.to_string()))?;
        }
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
        let shell = self
            .sessions
            .lock()
            .await
            .get(id)
            .and_then(|s| s.shell.clone());
        if let Some(shell) = shell {
            shell.stop().await?;
        }
        let ephemeral = self.sessions.lock().await.get(id).is_some_and(|s| {
            s.ephemeral && !(s.binding.side && params["preserveTemporary"] == true)
        });
        if ephemeral && self.embedded_runtime {
            self.begin_temporary_deletion(id).await?;
            self.rpc
                .call("session/close", json!({"sessionId":id}))
                .await?;
            self.cancel_answers(id).await;
            self.finish_temporary_deletion(id).await?;
            return Ok(json!({"status":"unsubscribed"}));
        }
        let method = if ephemeral {
            if self.embedded_runtime {
                let manager = goose_runtime::session::SessionManager::new(
                    goose_runtime::config::paths::Paths::data_dir(),
                );
                super::id_reservation::before_delete(&manager, id)
                    .await
                    .map_err(|_| {
                        error("Could not reserve a safe conversation identity before deletion")
                    })?;
            }
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
        self.pause_closed_queue(id).await?;
        Ok(json!({"status":"unsubscribed"}))
    }
}

pub(super) fn valid_model_id(value: &Value) -> bool {
    value.as_str().is_some_and(|id| {
        !id.is_empty() && id.len() <= 1024 && id.trim() == id && !id.chars().any(char::is_control)
    })
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

#[cfg(test)]
mod completion_tests {
    use super::*;
    #[tokio::test]
    async fn v63_turn_remains_busy_until_completion_is_durable_and_emitted() {
        let temp = tempfile::tempdir().unwrap();
        let (rpc, _incoming, tasks) = Rpc::stream(tokio::io::empty(), tokio::io::sink());
        let _tasks = TaskGuard(tasks);
        let (events, mut receive) = mpsc::channel(32);
        let (jobs, _jobs) = mpsc::channel(4);
        let mut session = Session::new("session", "C:\\fixture");
        session.loaded = true;
        session.active = Some("turn".into());
        let mut t = turn("turn", "inProgress");
        t["items"] = json!([{"type":"agentMessage","id":"answer","text":"done"}]);
        session.turns.push(t);
        let adapter = Arc::new(Adapter {
            rpc,
            events,
            sessions: Mutex::new(HashMap::from([("session".into(), session)])),
            gates: Mutex::new(HashMap::new()),
            answers: Mutex::new(HashMap::new()),
            jobs,
            capabilities: Mutex::new(Value::Null),
            defaults: Mutex::new(Value::Null),
            cwd: "C:\\fixture".into(),
            mcp: mcp::Bridge::new().await.unwrap(),
            queue: Mutex::new(HashMap::new()),
            queue_root: temp.path().join("queue"),
            deletions: Mutex::new(Default::default()),
            vwork_paths: fastrock_workspaces::vwork::DiscoveryPaths {
                user_home: temp.path().into(),
                configs: Vec::new(),
                search: Vec::new(),
                default_vwork_home: temp.path().join("vwork"),
            },
            embedded_runtime: false,
            summaries: None,
            changed: tokio::sync::Notify::new(),
            schedules: Mutex::new(schedule::model::State::open(
                temp.path().join("schedule.json"),
            )),
            schedule_gate: Mutex::new(()),
            webhook_listener: Mutex::new(Default::default()),
        });
        // Stop completion at the durable queue boundary without timing sleeps.
        let queue_guard = adapter.queue.lock().await;
        let worker = adapter.clone();
        let finish = tokio::spawn(async move {
            worker
                .finish_turn("session", "turn", Ok(json!({"stopReason":"end_turn"})))
                .await;
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), receive.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            adapter.sessions.lock().await["session"].active.as_deref(),
            Some("turn"),
            "Restore must not see idle while completion is still settling"
        );
        drop(queue_guard);
        finish.await.unwrap();
        assert!(adapter.sessions.lock().await["session"].active.is_none());
        let mut completed = false;
        while let Ok(event) = receive.try_recv() {
            if let AppServerEvent::ServerNotification(note) = event {
                completed |= json!(note).to_string().contains("turn/completed");
            }
        }
        assert!(
            completed,
            "Idle requires an already-emitted completion event"
        );
    }
}
