use super::capability_config as config;
use super::*;
use goose_runtime::config::Config;

impl Adapter {
    pub(super) async fn capability_request(
        self: &Arc<Self>,
        params: &Value,
        action: &str,
    ) -> Reply {
        if !self.embedded_runtime {
            return Err(unsupported("Goose capabilities"));
        }
        // Config/hook writes from different native windows share one owner.
        static SETTINGS: Mutex<()> = Mutex::const_new(());
        let _settings = SETTINGS.lock().await;
        if matches!(
            action,
            "toggle-default"
                | "add"
                | "toggle-plugin"
                | "hooks-read"
                | "hooks-save"
                | "plugin-create"
        ) {
            let p = params.clone();
            let action = action.to_owned();
            return tokio::task::spawn_blocking(move || {
                let c = Config::global();
                let key = text(&p, "key");
                let expected = text(&p, "revision");
                let result = match action.as_str() {
                    "toggle-default" => {
                        config::toggle(c, "extensions", key, expected, p["enabled"] == true)
                    }
                    "toggle-plugin" => {
                        config::toggle(c, "plugins", key, expected, p["enabled"] == true)
                    }
                    "add" => config::add(c, expected, &p["form"]),
                    "plugin-create" => config::create_plugin(
                        c,
                        &goose_runtime::plugins::plugin_install_dir(),
                        expected,
                        text(&p, "name"),
                    ),
                    "hooks-read" => {
                        return config::open_hooks(c, key).map_err(|e| error(e.to_string()));
                    }
                    "hooks-save" => config::save_hooks(c, key, expected, text(&p, "content")),
                    _ => unreachable!(),
                };
                result.map(|_| json!({})).map_err(|e| error(e.to_string()))
            })
            .await
            .map_err(|_| error("Goose capability settings worker stopped"))?;
        }
        let mut id = params["threadId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(session_id)
            .transpose()?;
        let gate = if let Some(id) = id {
            Some(self.gate(id).await)
        } else {
            None
        };
        let _lock = if let Some(gate) = &gate {
            Some(gate.lock().await)
        } else {
            None
        };
        if action == "list"
            && let Some(selected) = id
        {
            if !self
                .sessions
                .lock()
                .await
                .get(selected)
                .is_some_and(|s| s.loaded && !s.ephemeral && !s.recovery_only)
            {
                // Closing a chat must not strand the settings page on its old ID.
                id = None;
            }
        }
        let cwd = if let Some(id) = id {
            let sessions = self.sessions.lock().await;
            let s = sessions
                .get(id)
                .filter(|s| s.loaded && !s.ephemeral && !s.recovery_only)
                .ok_or_else(|| error("Open a conversation that does not need recovery first"))?;
            if action != "list" && s.active.is_some() {
                return Err(error(
                    "Wait for this conversation to finish before changing its tools",
                ));
            }
            s.binding
                .workspace
                .as_ref()
                .map(|workspace| workspace.source_root.to_string_lossy().into_owned())
                .unwrap_or_else(|| text(&s.info, "cwd").to_owned())
        } else {
            self.cwd.clone()
        };
        let bindings: Vec<fastrock_workspaces::vwork::McpBinding> = if let Some(id) = id {
            super::fork::read_saved(
                &self.queue_path(id).with_extension("vwork.json"),
                1024 * 1024,
            )
            .await?
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        match action {
            "list" => {
                let catalog = tokio::task::spawn_blocking(move || {
                    config::snapshot(std::path::Path::new(&cwd))
                })
                .await
                .map_err(|_| error("Capability catalog worker stopped"))?
                .map_err(|e| error(e.to_string()))?;
                let (extensions, tools) = if let Some(id) = id {
                    let extensions = self
                        .rpc
                        .call(
                            "_goose/unstable/session/extensions/list",
                            json!({"sessionId":id}),
                        )
                        .await?;
                    let tools = self
                        .rpc
                        .call("_goose/unstable/tools/list", json!({"sessionId":id}))
                        .await?;
                    (extensions, tools)
                } else {
                    (json!({"extensions":[]}), json!({"tools":[]}))
                };
                let tool_rows = tools["tools"]
                    .as_array()
                    .ok_or_else(|| error("Goose returned an invalid tool catalog"))?;
                if tool_rows.len() > 4096 {
                    return Err(error("Goose tool catalog exceeds 4096 entries"));
                }
                let tools:Vec<_>=tool_rows.iter().map(|t|json!({"name":t["name"],"permission":t["permission"],"description":text(t,"description").chars().take(4096).collect::<String>(),"protected":config::managed_session_name(text(t,"name"), &bindings, true)})).collect();
                let extensions:Vec<_>=extensions["extensions"].as_array().into_iter().flatten().map(|entry|{
                    let ext=&entry["extension"];let name=ext["name"].as_str().or(ext["server"]["name"].as_str()).unwrap_or("");
                    json!({"key":entry["extensionKey"],"name":name,"protected":config::managed_session_name(name, &bindings, false),"codeMode":ext["type"]=="platform" && name=="code_execution"})
                }).collect();
                let sessions = self.sessions.lock().await;
                let mut choices:Vec<_>=sessions.iter().filter(|(_,s)|s.loaded && !s.ephemeral && !s.recovery_only).map(|(id,s)|json!({"id":ui_id(id),"title":s.info["title"].as_str().filter(|s|!s.is_empty()).unwrap_or(id),"busy":s.active.is_some()})).collect();
                choices.sort_by_key(|s| text(s, "id").to_owned());
                Ok(
                    json!({"catalog":catalog,"sessions":choices,"extensions":extensions,"tools":tools,"threadId":id.map(ui_id)}),
                )
            }
            "code-mode" | "session-remove" | "permission" => {
                let id = id.ok_or_else(|| error("Choose an open conversation first"))?;
                self.checkpoint_ready(id)
                    .await
                    .map_err(|e| error(e.to_string()))?;
                if action == "permission" {
                    let name = text(params, "key");
                    let permission = text(params, "permission");
                    if config::managed_session_name(name, &bindings, true)
                        || !matches!(permission, "always_allow" | "ask_before" | "never_allow")
                    {
                        return Err(error("Invalid tool permission change"));
                    }
                    let tools = self
                        .rpc
                        .call("_goose/unstable/tools/list", json!({"sessionId":id}))
                        .await?;
                    let tool = tools["tools"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|t| t["name"] == name)
                        .ok_or_else(|| error("Tool no longer available; refresh"))?;
                    if tool["permission"] != params["previous"] {
                        return Err(error("Tool permission changed; refresh before saving"));
                    }
                    self.rpc
                        .call(
                            "_goose/unstable/tools/permissions/set",
                            json!({"toolPermissions":[{"toolName":name,"permission":permission}]}),
                        )
                        .await?;
                } else {
                    let list = self
                        .rpc
                        .call(
                            "_goose/unstable/session/extensions/list",
                            json!({"sessionId":id}),
                        )
                        .await?;
                    let rows = list["extensions"]
                        .as_array()
                        .ok_or_else(|| error("Cannot inspect active extensions"))?;
                    if action == "code-mode" {
                        let current = rows.iter().find(|e| {
                            e["extension"]["type"] == "platform"
                                && e["extension"]["name"] == "code_execution"
                        });
                        if params["enabled"] == true {
                            if current.is_none() {
                                self.rpc.call("_goose/unstable/session/extensions/add",json!({"sessionId":id,"extension":{"type":"platform","name":"code_execution"}})).await?;
                            }
                        } else if let Some(entry) = current {
                            self.rpc
                                .call(
                                    "_goose/unstable/session/extensions/remove",
                                    json!({"sessionId":id,"extensionKey":entry["extensionKey"]}),
                                )
                                .await?;
                        }
                    } else {
                        let entry = rows
                            .iter()
                            .find(|e| e["extensionKey"] == params["key"])
                            .ok_or_else(|| error("Extension no longer active; refresh"))?;
                        let ext = &entry["extension"];
                        let name = ext["name"]
                            .as_str()
                            .or(ext["server"]["name"].as_str())
                            .unwrap_or("");
                        if config::managed_session_name(name, &bindings, false) {
                            return Err(error(
                                "Fastrock's workspace tools cannot be detached here",
                            ));
                        }
                        self.rpc
                            .call(
                                "_goose/unstable/session/extensions/remove",
                                json!({"sessionId":id,"extensionKey":entry["extensionKey"]}),
                            )
                            .await?;
                    }
                }
                Ok(json!({}))
            }
            _ => Err(unsupported(action)),
        }
    }
}
