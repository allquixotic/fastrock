//! ACP data projected into the existing native transcript's presentation types.
use serde_json::{Value, json};

pub(super) fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
pub(super) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
pub(super) fn ui_id(id: &str) -> String {
    format!("goose:{id}")
}
pub(super) fn session_id(id: &str) -> Result<&str, codex_app_server_protocol::JSONRPCErrorError> {
    id.strip_prefix("goose:")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            super::error(
                "This conversation belongs to another backend. Select its connection to open it.",
            )
        })
}
fn timestamp(value: &Value) -> i64 {
    value
        .as_str()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|v| v.timestamp())
        .unwrap_or(0)
}

#[derive(Default)]
pub(super) struct Session {
    pub binding: super::workspace::BindingRecord,
    pub info: Value,
    pub config: Vec<Value>,
    pub turns: fastrock_workspaces::scrollback::History,
    pub active: Option<String>,
    pub run_id: Option<String>,
    pub cancel_requested: bool,
    pub replaying: bool,
    pub loaded: bool,
    pub ephemeral: bool,
    pub last_chunk: String,
    pub echo_user: Option<String>,
    pub user_clients: std::collections::HashMap<String, Value>,
}

impl Session {
    pub fn new(id: &str, cwd: &str) -> Self {
        Self {
            info: json!({"sessionId":id,"cwd":cwd,"_meta":{"createdAt":chrono::Utc::now().to_rfc3339()}}),
            ..Self::default()
        }
    }
    pub fn config_value(&self, id: &str) -> &str {
        self.config
            .iter()
            .find(|o| o["id"] == id)
            .and_then(|o| o["currentValue"].as_str())
            .unwrap_or("")
    }
    pub fn thread(&self) -> Value {
        let id = ui_id(text(&self.info, "sessionId"));
        let meta = &self.info["_meta"];
        let model = self.config_value("model");
        let model = if model.is_empty() {
            text(meta, "modelId")
        } else {
            model
        };
        let provider = self.config_value("provider");
        let provider = if provider.is_empty() {
            text(meta, "providerId")
        } else {
            provider
        };
        json!({"id":id,"sessionId":id,"name":self.info["title"],"preview":meta["lastMessageSnippet"].as_str().unwrap_or(""),
            "forkedFromId":self.binding.forked_from.as_deref().map(ui_id),
            "ephemeral":self.ephemeral,"modelProvider":if provider.is_empty(){"goose"}else{provider},"model":model,
            "createdAt":timestamp(&meta["createdAt"]),"updatedAt":timestamp(&self.info["updatedAt"]),
            "recencyAt":timestamp(&meta["lastMessageAt"]),"cwd":self.info["cwd"],"cliVersion":"Goose ACP",
            "source":"appServer","historyMode":"paginated","status":if self.active.is_some(){json!({"type":"active","activeFlags":[]})}else{json!({"type":"idle"})},"turns":[]})
    }
    pub fn response(&self) -> Value {
        let thread = self.thread();
        json!({"thread":thread,"model":thread["model"],"modelProvider":thread["modelProvider"],
            "cwd":thread["cwd"],"approvalPolicy":"on-request","approvalsReviewer":"user",
            // ACP mode controls tools; Fastrock does not claim to provide an OS sandbox.
            "sandbox":{"type":"dangerFullAccess"},"reasoningEffort":null})
    }
    pub fn turn_mut(&mut self) -> &mut Value {
        if self.turns.is_empty() {
            self.turns.push(turn("history", "completed"));
        }
        self.turns.last_mut().unwrap_or_else(|| unreachable!())
    }

    /// Returns presentation events. During load, updates populate history only.
    pub fn update(&mut self, update: &Value) -> Vec<Value> {
        let kind = text(update, "sessionUpdate");
        if kind == "config_option_update" {
            self.config = update["configOptions"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            return vec![];
        }
        if kind == "session_info_update" {
            for key in ["title", "updatedAt"] {
                if !update[key].is_null() {
                    self.info[key] = update[key].clone();
                }
            }
            return vec![];
        }
        if kind == "current_mode_update" {
            if let Some(option) = self.config.iter_mut().find(|o| o["id"] == "mode") {
                option["currentValue"] = update["currentModeId"].clone();
            }
            return vec![];
        }
        let mut events = Vec::new();
        let sid = ui_id(text(&self.info, "sessionId"));
        match kind {
            "user_message_chunk" | "agent_message_chunk" | "agent_thought_chunk" => {
                let user = kind == "user_message_chunk";
                let thought = kind == "agent_thought_chunk";
                let supplied_id = update["messageId"]
                    .as_str()
                    .or_else(|| update["_meta"]["goose"]["messageId"].as_str());
                let message_id = supplied_id
                    .map(|id| {
                        if thought {
                            format!("{id}:thought")
                        } else {
                            id.to_string()
                        }
                    })
                    .unwrap_or_else(|| {
                        if self.last_chunk == kind {
                            self.turns
                                .last()
                                .and_then(|t| t["items"].as_array())
                                .and_then(|a| a.last())
                                .map(|i| text(i, "id").to_string())
                                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
                        } else {
                            uuid::Uuid::new_v4().to_string()
                        }
                    });
                let content = &update["content"];
                if user && !self.replaying && self.echo_user.as_deref() == content["text"].as_str()
                {
                    if let Some(id) = supplied_id {
                        self.turn_mut()["gooseMessageId"] = json!(id);
                    }
                    self.echo_user = None;
                    return events;
                }
                let exists = self
                    .turns
                    .last()
                    .and_then(|t| t["items"].as_array())
                    .is_some_and(|items| items.iter().any(|i| i["id"] == message_id));
                if user && self.replaying && !exists && update["_meta"]["goose"]["steer"] != true {
                    self.turns
                        .push(turn(&format!("history-{message_id}"), "completed"));
                }
                if user
                    && supplied_id.is_some()
                    && update["_meta"]["goose"]["steer"] != true
                    && self.turn_mut()["gooseMessageId"].is_null()
                {
                    self.turn_mut()["gooseMessageId"] = json!(message_id);
                }
                let client_id = self
                    .user_clients
                    .get(&message_id)
                    .cloned()
                    .unwrap_or(Value::Null);
                let tid = text(self.turn_mut(), "id").to_string();
                let items = self.turn_mut()["items"]
                    .as_array_mut()
                    .unwrap_or_else(|| unreachable!());
                let index = items.iter().position(|i| i["id"] == message_id).unwrap_or_else(|| {
                    items.push(if user { json!({"type":"userMessage","id":message_id,"clientId":client_id,"content":[]}) }
                    else if thought { json!({"type":"reasoning","id":message_id,"summary":[],"content":[""]}) }
                    else { json!({"type":"agentMessage","id":message_id,"text":""}) });
                    events.push(json!({"method":"item/started","params":{"threadId":sid,"turnId":tid,"item":items.last()}}));
                    items.len()-1
                });
                let item = &mut items[index];
                if user {
                    let input = match text(content, "type") {
                        "text" => json!({"type":"text","text":content["text"],"textElements":[]}),
                        "image" => {
                            json!({"type":"image","url":format!("data:{};base64,{}",text(content,"mimeType"),text(content,"data"))})
                        }
                        _ => {
                            json!({"type":"text","text":render_content(content),"textElements":[]})
                        }
                    };
                    if let Some(contents) = item["content"].as_array_mut() {
                        contents.push(input);
                    }
                    events.push(json!({"method":"item/completed","params":{"threadId":sid,"turnId":tid,"item":item}}));
                } else {
                    let delta = render_content(content);
                    if thought {
                        let old = item["content"][0].as_str().unwrap_or("");
                        item["content"][0] = json!(format!("{old}{delta}"));
                    } else {
                        let old = text(item, "text");
                        item["text"] = json!(format!("{old}{delta}"));
                    }
                    events.push(json!({"method":if thought{"item/reasoning/textDelta"}else{"item/agentMessage/delta"},"params":{"threadId":sid,"turnId":tid,"itemId":message_id,"delta":delta,"contentIndex":0}}));
                }
                self.last_chunk = kind.into();
            }
            "tool_call" | "tool_call_update" => {
                let id = text(update, "toolCallId");
                if id.is_empty() {
                    return events;
                }
                let tid = text(self.turn_mut(), "id").to_string();
                let items = self.turn_mut()["items"]
                    .as_array_mut()
                    .unwrap_or_else(|| unreachable!());
                let index = items.iter().position(|i| i["id"] == id).unwrap_or_else(|| {
                    items.push(json!({"type":"mcpToolCall","id":id,"server":"Goose","tool":"Tool","arguments":{},"status":"inProgress"}));
                    items.len()-1
                });
                let item = &mut items[index];
                if !update["title"].is_null() {
                    item["tool"] = update["title"].clone();
                }
                if !update["rawInput"].is_null() {
                    item["arguments"] = update["rawInput"].clone();
                }
                let done = matches!(text(update, "status"), "completed" | "failed");
                item["status"] = json!(if done {
                    if update["status"] == "failed" {
                        "failed"
                    } else {
                        "completed"
                    }
                } else {
                    "inProgress"
                });
                if let Some(contents) = update["content"].as_array() {
                    let output: Vec<Value> = contents
                        .iter()
                        .map(|c| json!({"type":"text","text":render_tool_content(c)}))
                        .collect();
                    item["result"] = json!({"content":output});
                } else if !update["rawOutput"].is_null() {
                    item["result"] =
                        json!({"content":[{"type":"text","text":update["rawOutput"].to_string()}]});
                }
                events.push(json!({"method":if done{"item/completed"}else{"item/started"},"params":{"threadId":sid,"turnId":tid,"item":item}}));
                self.last_chunk.clear();
            }
            "plan" => {
                let plan: Vec<Value> = update["entries"].as_array().into_iter().flatten().map(|e| json!({"step":e["content"],"status":match text(e,"status") {"in_progress"=>"inProgress","completed"=>"completed",_=>"pending"}})).collect();
                let tid = text(self.turn_mut(), "id").to_string();
                events.push(json!({"method":"turn/plan/updated","params":{"threadId":sid,"turnId":tid,"plan":plan}}));
            }
            _ => {}
        }
        if self.replaying {
            events.clear();
        }
        events
    }
}

pub(super) fn turn(id: &str, status: &str) -> Value {
    json!({"id":id,"items":[],"status":status,"error":null,"startedAt":now()})
}
pub(super) fn render_content(content: &Value) -> String {
    match text(content, "type") {
        "text" => text(content, "text").to_string(),
        "image" => format!(
            "\n![Image](data:{};base64,{})\n",
            text(content, "mimeType"),
            text(content, "data")
        ),
        "resource_link" => format!("\n[{}]({})\n", text(content, "name"), text(content, "uri")),
        "resource" => content["resource"]["text"]
            .as_str()
            .unwrap_or("[Embedded resource]")
            .to_string(),
        _ => "[Unsupported ACP content]".into(),
    }
}
fn render_tool_content(content: &Value) -> String {
    match text(content, "type") {
        "content" => render_content(&content["content"]),
        "diff" => format!(
            "{}\n```diff\n--- before\n{}\n+++ after\n{}\n```",
            text(content, "path"),
            text(content, "oldText"),
            text(content, "newText")
        ),
        "terminal" => format!("Terminal {}", text(content, "terminalId")),
        _ => content.to_string(),
    }
}
