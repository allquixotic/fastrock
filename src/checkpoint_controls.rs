use crate::{
    app::AppController,
    ui::{CheckpointRow, CheckpointState},
};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
#[derive(Default)]
pub(crate) struct Controller {
    epoch: u64,
    thread: Option<String>,
    review: Option<Value>,
    before: Option<String>,
    pending: Option<String>,
}
impl AppController {
    pub(crate) fn checkpoints_install(&mut self) {
        let state = self.window.global::<CheckpointState>();
        state.on_action(crate::ui_thread::scoped_callback!(|action| {
            let action = action.to_string();
            crate::ui_thread::with_app(move |app| app.checkpoints_action(&action));
        }));
        state.on_select(crate::ui_thread::scoped_callback!(|id| {
            let id = id.to_string();
            crate::ui_thread::with_app(move |app| {
                app.checkpoints_request("review", json!({"id":id}))
            });
        }));
    }
    pub(crate) fn checkpoints_context_changed(&mut self) {
        let current = self
            .active_thread_index()
            .and_then(|i| self.thread_tab(i))
            .and_then(|t| t.thread_id.as_ref());
        if self.checkpoints.thread.as_ref() != current {
            self.checkpoints_close();
        }
    }
    fn checkpoints_close(&mut self) {
        self.checkpoints.epoch = self.checkpoints.epoch.wrapping_add(1);
        self.checkpoints.review = None;
        self.checkpoints.thread = None;
        let state = self.window.global::<CheckpointState>();
        state.set_open(false);
        state.set_rows(ModelRc::default());
        state.set_lines(ModelRc::default());
        state.set_busy(false);
    }
    pub(crate) fn checkpoints_open(&mut self) {
        let Some(id) = self
            .active_thread_index()
            .and_then(|i| self.thread_tab(i))
            .and_then(|t| t.thread_id.clone())
        else {
            return;
        };
        self.checkpoints_close();
        self.checkpoints.thread = Some(id);
        let state = self.window.global::<CheckpointState>();
        state.set_open(true);
        state.set_label("".into());
        state.set_status("".into());
        state.set_description("Choose a checkpoint to review.".into());
        state.set_blocked(false);
        state.set_recovery(false);
        state.set_reviewed(false);
        state.set_selected("".into());
        state.set_files(false);
        state.set_conversation(false);
        self.checkpoints_request("list", json!({}));
    }
    fn checkpoints_action(&mut self, action: &str) {
        let state = self.window.global::<CheckpointState>();
        if action == "close" {
            self.checkpoints_close();
            self.restore_focus();
            return;
        }
        if state.get_busy() {
            return;
        }
        match action {
            "refresh" => self.checkpoints_request("list", json!({})),
            "older" => self.checkpoints_request("list", json!({"before":self.checkpoints.before})),
            "capture" => {
                self.checkpoints_request(action, json!({"label":state.get_label().to_string()}))
            }
            "restore" | "rollback" | "keep" => {
                if let Some(review) = self.checkpoints.review.clone() {
                    self.checkpoints_request(action,json!({"review":review["review"],"current":review["current"],"files":state.get_files(),"conversation":state.get_conversation()}));
                }
            }
            "copy" => {
                if let Some(review) = &self.checkpoints.review {
                    self.copy_to_clipboard(&format!(
                        "{}\n{}",
                        state.get_description(),
                        review["review"]["diff"].as_str().unwrap_or("")
                    ));
                }
            }
            _ => {}
        }
    }
    fn checkpoints_request(&mut self, action: &str, mut params: Value) {
        let Some(id) = self.checkpoints.thread.clone() else {
            return;
        };
        let state = self.window.global::<CheckpointState>();
        if state.get_busy() {
            return;
        }
        params["threadId"] = json!(id);
        let epoch = self.checkpoints.epoch;
        let action = action.to_owned();
        let backend = self.backend.clone();
        state.set_busy(true);
        state.set_status("".into());
        if action == "review" || action == "list" {
            self.checkpoints.review = None;
            state.set_reviewed(false);
            state.set_recovery(false);
        }
        self.backend.spawn(async move{
            let result=backend.goose_request(&format!("fastrock/checkpoint/{action}"),params).await;
            crate::ui_thread::post(move|app|{
                // Restore completion must refresh any surviving tab even if its
                // panel was closed/switched while the worker was running.
                if result.as_ref().is_ok_and(|v|v["reload"]==true){if let Some(index)=app.tab_index_for_thread(&id){app.resume_thread_in_tab(index,id.clone(),false);}}
                if app.checkpoints.epoch!=epoch || app.checkpoints.thread.as_ref()!=Some(&id){return}
                let state=app.window.global::<CheckpointState>();state.set_busy(false);
                match result {
                    Err(e)=>{state.set_status(e.into());state.set_reviewed(false);app.checkpoints.review=None;},
                    Ok(value)=>match action.as_str(){
                        "list"=>{
                            app.checkpoints.before=value["page"]["before"].as_str().map(str::to_owned);
                            app.checkpoints.pending=value["pending"]["id"].as_str().map(str::to_owned);
                            state.set_blocked(app.checkpoints.pending.is_some());state.set_more(app.checkpoints.before.is_some());
                            let rows=value["page"]["points"].as_array().into_iter().flatten().map(|p|{
                                let time=chrono::DateTime::from_timestamp_millis(p["created_ms"].as_i64().unwrap_or_default()).map(|t|t.with_timezone(&chrono::Local).format("%b %d %H:%M:%S").to_string()).unwrap_or_default();
                                CheckpointRow{id:p["id"].as_str().unwrap_or("").into(),title:p["label"].as_str().unwrap_or("Checkpoint").into(),detail:format!("{time} · {} messages{}",p["boundary"]["count"],if p["workspace"].is_null(){""}else{" · files"}).into()}
                            }).collect::<Vec<_>>();state.set_rows(ModelRc::new(VecModel::from(rows)));
                            if let Some(id)=app.checkpoints.pending.clone(){app.checkpoints_request("review",json!({"id":id}));}
                        },
                        "review"=>{
                            let review=&value["review"];let point=&review["checkpoint"];
                            state.set_selected(point["id"].as_str().unwrap_or("").into());
                            let files=review["complete"]==true && !point["workspace"].is_null();let conversation=value["targetCount"].is_number();
                            state.set_can_files(files);state.set_can_conversation(conversation);state.set_files(files);state.set_conversation(conversation);state.set_reviewed(true);
                            state.set_recovery(app.checkpoints.pending.as_deref()==point["id"].as_str());
                            let mut description=format!("{}\n{} file changes. Conversation: {} → {} messages.",point["label"].as_str().unwrap_or("Checkpoint"),review["files"].as_array().map_or(0,Vec::len),value["current"]["count"],value["targetCount"].as_u64().map_or("unavailable".into(),|n|n.to_string()));
                            if review["complete"]!=true{description.push_str("\nFile review exceeds its limit; file restore is disabled.");}
                            if let Some(reason)=value["conversationError"].as_str().filter(|s|!s.is_empty()){description.push('\n');description.push_str(reason);}
                            state.set_description(description.into());
                            let file_headers=review["files"].as_array().into_iter().flatten().map(|f|SharedString::from(format!("{}  {} → {}  {}",f["kind"].as_str().unwrap_or(""),f["checkpoint_mode"].as_str().unwrap_or(""),f["current_mode"].as_str().unwrap_or(""),f["path"].as_str().unwrap_or(""))));
                            state.set_lines(ModelRc::new(VecModel::from(file_headers.chain(review["diff"].as_str().unwrap_or("").lines().map(SharedString::from)).collect::<Vec<_>>())));
                            app.checkpoints.review=Some(value);
                        },
                        _=>{app.checkpoints_request("list",json!({}));app.window.global::<CheckpointState>().set_status(if action=="capture"{"Checkpoint saved."}else{"Restore saved. Queued work remains paused; review it before choosing Resume."}.into());}
                    }
                }
            });
        });
    }
    pub(crate) fn checkpoints_automation(&mut self, args: &[String]) {
        let state = self.window.global::<CheckpointState>();
        match args.first().map(String::as_str) {
            Some("select") => {
                if let Some(row) = args
                    .get(1)
                    .and_then(|s| s.parse::<usize>().ok())
                    .and_then(|i| state.get_rows().row_data(i))
                {
                    state.invoke_select(row.id);
                }
            }
            Some("label") => state.set_label(args.get(1).map(String::as_str).unwrap_or("").into()),
            Some("files") => state.set_files(args.get(1).is_some_and(|s| s == "true")),
            Some("conversation") => {
                state.set_conversation(args.get(1).is_some_and(|s| s == "true"))
            }
            Some("dump") => {
                if let Some(path) = args.get(1) {
                    let value = json!({"open":state.get_open(),"busy":state.get_busy(),"status":state.get_status().to_string(),"description":state.get_description().to_string(),"selected":state.get_selected().to_string(),"reviewed":state.get_reviewed(),"blocked":state.get_blocked(),"recovery":state.get_recovery(),"canFiles":state.get_can_files(),"canConversation":state.get_can_conversation(),"review":self.checkpoints.review,"rows":state.get_rows().iter().map(|r|json!({"id":r.id.to_string(),"title":r.title.to_string(),"detail":r.detail.to_string()})).collect::<Vec<_>>()});
                    let path = path.clone();
                    self.backend.spawn(async move {
                        let _ = tokio::fs::write(path, value.to_string()).await;
                    });
                }
            }
            Some(action) => self.checkpoints_action(action),
            None => {}
        }
    }
}
