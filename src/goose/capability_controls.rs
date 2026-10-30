use crate::{
    app::AppController,
    ui::{CapabilityRow, CapabilityState},
};
use serde_json::{Value, json};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

#[derive(Default)]
pub(crate) struct Controller {
    snapshot: Value,
    thread: String,
    hook: Value,
}
impl AppController {
    pub(super) fn capabilities_install(&mut self) {
        let state = self.window.global::<CapabilityState>();
        state.on_action(crate::ui_thread::scoped_callback!(|action| {
            let action = action.to_string();
            crate::ui_thread::with_app(move |app| app.capabilities_action(&action));
        }));
        state.on_select(crate::ui_thread::scoped_callback!(|key| {
            let key = key.to_string();
            crate::ui_thread::with_app(move |app| app.capabilities_select(&key));
        }));
        state.on_choose_section(crate::ui_thread::scoped_callback!(|section| {
            crate::ui_thread::with_app(move |app| {
                let state = app.window.global::<CapabilityState>();
                if state.get_busy() || state.get_dirty() {
                    return;
                }
                state.set_section(section);
                state.set_adding(false);
                state.set_selected("".into());
                state.set_description("".into());
                state.set_editing_hooks(false);
                app.capabilities_publish();
            });
        }));
        state.on_choose_session(crate::ui_thread::scoped_callback!(|index| {
            crate::ui_thread::with_app(move |app| {
                let state = app.window.global::<CapabilityState>();
                if state.get_busy() || state.get_dirty() {
                    return;
                }
                app.settings.capabilities.thread = app.settings.capabilities.snapshot["sessions"]
                    .as_array()
                    .and_then(|a| a.get(index as usize))
                    .and_then(|s| s["id"].as_str())
                    .unwrap_or("")
                    .to_owned();
                app.capabilities_load();
            });
        }));
        state.on_remove_extension(crate::ui_thread::scoped_callback!(|key| {
            let key = key.to_string();
            crate::ui_thread::with_app(move |app| {
                app.capabilities_request("session-remove", json!({"key":key}))
            });
        }));
    }
    pub(crate) fn capabilities_load(&mut self) {
        if self.window.global::<CapabilityState>().get_dirty() {
            return;
        }
        self.capabilities_request("list", json!({}));
    }
    fn capability_items(&self) -> Vec<Value> {
        let value = &self.settings.capabilities.snapshot;
        let rows = match self.window.global::<CapabilityState>().get_section() {
            0 => &value["catalog"]["extensions"],
            1 => &value["tools"],
            _ => &value["catalog"]["hooks"],
        };
        rows.as_array().cloned().unwrap_or_default()
    }
    fn capabilities_publish(&mut self) {
        let state = self.window.global::<CapabilityState>();
        let section = state.get_section();
        let rows = self
            .capability_items()
            .iter()
            .map(|row| CapabilityRow {
                key: row[if section == 1 { "name" } else { "key" }]
                    .as_str()
                    .unwrap_or("")
                    .into(),
                title: row["name"].as_str().unwrap_or("").into(),
                detail: if section == 0 {
                    format!(
                        "{} · {}{}",
                        row["kind"].as_str().unwrap_or(""),
                        if row["enabled"] == true {
                            "Enabled for new chats"
                        } else {
                            "Disabled default"
                        },
                        if row["configured"] == true {
                            ""
                        } else {
                            " · Built in"
                        }
                    )
                } else if section == 1 {
                    format!(
                        "{} · {}",
                        row["permission"].as_str().unwrap_or("Goose mode default"),
                        row["description"]
                            .as_str()
                            .unwrap_or("")
                            .lines()
                            .next()
                            .unwrap_or("")
                    )
                } else {
                    format!(
                        "{} · {}",
                        if row["active"] == true {
                            "Active"
                        } else {
                            "Inactive"
                        },
                        row["key"].as_str().unwrap_or("")
                    )
                }
                .into(),
                enabled: row["enabled"] == true,
                protected: row["protected"] == true,
                configured: row["configured"] == true,
            })
            .collect::<Vec<_>>();
        state.set_rows(ModelRc::new(VecModel::from(rows)));
        let value = &self.settings.capabilities.snapshot;
        state.set_sessions(ModelRc::new(VecModel::from(
            value["sessions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| SharedString::from(s["title"].as_str().unwrap_or("Conversation")))
                .collect::<Vec<_>>(),
        )));
        state.set_session_index(
            value["sessions"]
                .as_array()
                .and_then(|a| {
                    a.iter()
                        .position(|s| s["id"] == self.settings.capabilities.thread)
                })
                .map_or(-1, |i| i as i32),
        );
        state.set_session_extensions(ModelRc::new(VecModel::from(
            value["extensions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| CapabilityRow {
                    key: e["key"].as_str().unwrap_or("").into(),
                    title: e["name"].as_str().unwrap_or("").into(),
                    protected: e["protected"] == true,
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        )));
        state.set_code_mode(
            value["extensions"]
                .as_array()
                .is_some_and(|a| a.iter().any(|e| e["codeMode"] == true)),
        );
        let selected = state.get_selected().to_string();
        self.capabilities_select(&selected);
    }
    fn capabilities_select(&mut self, key: &str) {
        let state = self.window.global::<CapabilityState>();
        if state.get_busy() || state.get_dirty() {
            return;
        }
        let section = state.get_section();
        let row = self
            .capability_items()
            .into_iter()
            .find(|r| r[if section == 1 { "name" } else { "key" }] == key);
        state.set_editing_hooks(false);
        self.settings.capabilities.hook = Value::Null;
        if let Some(row) = row {
            state.set_selected(key.into());
            state.set_selected_enabled(row["enabled"] == true);
            state.set_selected_protected(row["protected"] == true);
            state.set_description(
                if row["protected"] == true {
                    "Managed by Fastrock."
                } else if section == 1 {
                    row["description"].as_str().unwrap_or("")
                } else {
                    key
                }
                .into(),
            );
        } else {
            state.set_selected("".into());
            state.set_description("".into());
        }
    }
    fn capabilities_action(&mut self, action: &str) {
        let state = self.window.global::<CapabilityState>();
        if state.get_busy() {
            return;
        }
        let key = state.get_selected().to_string();
        let catalog = &self.settings.capabilities.snapshot["catalog"];
        match action {
            "refresh"=>self.capabilities_load(),
            "toggle"=>{let plugin=state.get_section()==2;self.capabilities_request(if plugin{"toggle-plugin"}else{"toggle-default"},json!({"key":key,"revision":catalog[if plugin{"pluginRevision"}else{"revision"}],"enabled":!state.get_selected_enabled()}));},
            "add"=>self.capabilities_request(action,json!({"revision":catalog["revision"],"form":{"name":state.get_name().as_str(),"kind":if state.get_transport()==0{"stdio"}else{"http"},"address":state.get_address().as_str(),"args":state.get_args().as_str(),"keys":state.get_keys().as_str()}})),
            "plugin-create"=>self.capabilities_request(action,json!({"revision":catalog["pluginRevision"],"name":state.get_name().trim()})),
            "code-mode"=>self.capabilities_request(action,json!({"enabled":!state.get_code_mode()})),
            "always_allow"|"ask_before"|"never_allow"=>{
                let previous=self.capability_items().into_iter().find(|t|t["name"]==key).unwrap_or(Value::Null)["permission"].clone();
                self.capabilities_request("permission",json!({"key":key,"previous":previous,"permission":action}));
            },
            "hooks-read"=>self.capabilities_request(action,json!({"key":key})),
            "hooks-save"=>self.capabilities_request(action,json!({"key":key,"revision":self.settings.capabilities.hook["revision"],"content":state.get_hook_text().as_str()})),
            "hooks-discard"=>{state.set_dirty(false);state.set_editing_hooks(false);state.set_hook_text("".into());self.settings.capabilities.hook=Value::Null;},
            _=>{},
        }
    }
    fn capabilities_request(&mut self, action: &str, mut params: Value) {
        let state = self.window.global::<CapabilityState>();
        if state.get_busy() {
            return;
        }
        if state.get_dirty() && action != "hooks-save" {
            state.set_status("Save or discard hook edits first.".into());
            return;
        }
        params["threadId"] = json!(self.settings.capabilities.thread);
        state.set_busy(true);
        state.set_status("".into());
        let backend = self.backend.clone();
        let action = action.to_owned();
        self.backend.spawn(async move{
            let result=backend.goose_request(&format!("fastrock/capability/{action}"),params).await;
            crate::ui_thread::post(move|app|{
                let state=app.window.global::<CapabilityState>();state.set_busy(false);
                match result {
                    Err(e)=>state.set_status(e.into()),
                    Ok(value)=>match action.as_str(){
                        "list"=>{
                            let first=value["sessions"].as_array().and_then(|a|a.first()).and_then(|s|s["id"].as_str()).map(str::to_owned);
                            app.settings.capabilities.thread=value["threadId"].as_str().unwrap_or("").to_owned();let choose=app.settings.capabilities.thread.is_empty();app.settings.capabilities.snapshot=value;app.capabilities_publish();
                            if choose {if let Some(id)=first {app.settings.capabilities.thread=id;app.capabilities_load();}}
                        },
                        "hooks-read"=>{state.set_hook_text(value["text"].as_str().unwrap_or("").into());state.set_dirty(false);state.set_editing_hooks(true);app.settings.capabilities.hook=value;},
                        _=>{
                            if action=="hooks-save"{state.set_dirty(false);state.set_editing_hooks(false);state.set_hook_text("".into());}
                            if action=="add" || action=="plugin-create"{state.set_adding(false);state.set_name("".into());state.set_address("".into());state.set_args("[]".into());state.set_keys("".into());}
                            app.capabilities_load();app.window.global::<CapabilityState>().set_status(if matches!(action.as_str(),"toggle-default"|"toggle-plugin"|"hooks-save"|"add"|"plugin-create"){"Saved. Defaults and hooks apply to newly activated conversations."}else{"Conversation tools updated."}.into());
                        }
                    }
                }
            });
        });
    }
    pub(crate) fn capabilities_automation(&mut self, args: &[String]) {
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        let state = self.window.global::<CapabilityState>();
        match arg(0) {
            "section" => state.invoke_choose_section(arg(1).parse().unwrap_or(0)),
            "session" => state.invoke_choose_session(arg(1).parse().unwrap_or(0)),
            "select" => state.invoke_select(arg(1).into()),
            "field" => match arg(1) {
                "name" => state.set_name(arg(2).into()),
                "address" => state.set_address(arg(2).into()),
                "args" => state.set_args(arg(2).into()),
                "keys" => state.set_keys(arg(2).into()),
                "transport" => state.set_transport(arg(2).parse().unwrap_or(0)),
                "hooks" => {
                    state.set_hook_text(arg(2).into());
                    state.set_dirty(true);
                }
                _ => {}
            },
            "dump" => {
                let mut value = self.settings.capabilities.snapshot.clone();
                value["status"] = json!(state.get_status().as_str());
                value["busy"] = json!(state.get_busy());
                value["selected"] = json!(state.get_selected().as_str());
                value["dirty"] = json!(state.get_dirty());
                value["codeMode"] = json!(state.get_code_mode());
                let path = std::path::PathBuf::from(arg(1));
                self.backend.spawn(async move {
                    let _ = tokio::fs::write(path, value.to_string()).await;
                });
            }
            _ => state.invoke_action(arg(0).into()),
        }
    }
}
