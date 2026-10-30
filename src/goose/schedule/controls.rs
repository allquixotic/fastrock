use super::{
    model::{Pattern, Task},
    webhook_model::{Filter, Hook},
};
use crate::{
    app::AppController,
    ui::{ScheduleDay, ScheduleRow, ScheduleState, SettingsState, WebhookFilter, WebhookReceipt},
};
use chrono::{Local, NaiveDateTime, TimeZone};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

impl AppController {
    pub(crate) fn schedules_install(&mut self) {
        let state = self.window.global::<ScheduleState>();
        state.on_refresh(crate::ui_thread::scoped_callback!(|| {
            crate::ui_thread::with_app(AppController::schedules_refresh)
        }));
        state.on_new_task(crate::ui_thread::scoped_callback!(|| {
            crate::ui_thread::with_app(|app| app.schedules_select(""))
        }));
        state.on_select(crate::ui_thread::scoped_callback!(|id| {
            let id = id.to_string();
            crate::ui_thread::with_app(move |app| app.schedules_select(&id));
        }));
        state.on_save(crate::ui_thread::scoped_callback!(|| {
            crate::ui_thread::with_app(AppController::schedules_save)
        }));
        state.on_action(crate::ui_thread::scoped_callback!(|id, action| {
            let (id, action) = (id.to_string(), action.to_string());
            crate::ui_thread::with_app(move |app| app.schedules_action(&id, &action));
        }));
        state.on_open_run(crate::ui_thread::scoped_callback!(|id| {
            let id = id.to_string();
            crate::ui_thread::with_app(move |app| app.open_thread(id, None));
        }));
        state.on_edit_filter(crate::ui_thread::scoped_callback!(|index| {
            crate::ui_thread::with_app(move |app| {
                let state = app.window.global::<ScheduleState>();
                let mut filters: Vec<_> = state.get_filters().iter().collect();
                if index < 0 && filters.len() < 16 {
                    filters.push(WebhookFilter {
                        path: "body.action".into(),
                        op: 0,
                        value: "".into(),
                    });
                } else if index >= 0 && (index as usize) < filters.len() {
                    filters.remove(index as usize);
                }
                state.set_filters(ModelRc::new(VecModel::from(filters)));
            });
        }));
        self.schedules_select("");
    }
    pub(crate) fn schedules_changed(&mut self) {
        if self.window.global::<SettingsState>().get_page() == "schedules" {
            self.settings.schedules_stale = true;
            self.schedules_refresh();
        }
    }
    pub(crate) fn schedules_refresh(&mut self) {
        let state = self.window.global::<ScheduleState>();
        if state.get_loading() {
            self.settings.schedules_stale = true;
            return;
        }
        self.settings.schedules_stale = false;
        state.set_loading(true);
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = backend
                .goose_request("fastrock/schedule/list", json!({}))
                .await;
            crate::ui_thread::post(move |app| {
                let state = app.window.global::<ScheduleState>();
                state.set_loading(false);
                match result {
                    Ok(value) => {
                        let old_source = state
                            .get_source_ids()
                            .row_data(state.get_source_index().max(0) as usize);
                        let mut sources = value["sources"].as_array().cloned().unwrap_or_default();
                        // Restored sources may not be loaded in the session
                        // cache yet. Editing must retain their exact identity.
                        for task in value["tasks"].as_array().into_iter().flatten() {
                            if let Some(source) = task["source"].as_str() {
                                let id = format!("goose:{source}");
                                if !sources.iter().any(|s| s["id"] == id) {
                                    sources.push(json!({"id":id,"name":format!("Source conversation {source}")}));
                                }
                            }
                        }
                        if let Some(id) = &old_source {
                            if !sources.iter().any(|s| s["id"] == id.as_str()) {
                                sources.push(json!({"id":id.as_str(),"name":"Previously selected conversation"}));
                            }
                        }
                        state.set_source_names(ModelRc::new(VecModel::from(
                            sources
                                .iter()
                                .map(|s| {
                                    SharedString::from(s["name"].as_str().unwrap_or("Conversation"))
                                })
                                .collect::<Vec<_>>(),
                        )));
                        state.set_source_ids(ModelRc::new(VecModel::from(
                            sources
                                .iter()
                                .map(|s| SharedString::from(s["id"].as_str().unwrap_or("")))
                                .collect::<Vec<_>>(),
                        )));
                        state.set_source_index(
                            old_source
                                .and_then(|id| sources.iter().position(|s| s["id"] == id.as_str()))
                                .unwrap_or(0) as i32,
                        );
                        let tasks: Vec<Task> =
                            serde_json::from_value(value["tasks"].clone()).unwrap_or_default();
                        state.set_webhook_address(value["webhookAddress"].as_str().unwrap_or("").into());
                        state.set_webhook_error(value["webhookError"].as_str().unwrap_or("").into());
                        state.set_receipts(receipts(tasks.iter().find(|t|t.id == state.get_edit_id().as_str())));
                        state.set_rows(ModelRc::new(VecModel::from(
                            tasks.iter().map(row).collect::<Vec<_>>(),
                        )));
                        if let Some(problem) = value["error"].as_str() {
                            state.set_status(problem.into());
                        }
                        app.settings.schedules = value;
                    }
                    Err(error) => state.set_status(error.into()),
                }
                if app.settings.schedules_stale {
                    app.schedules_refresh();
                }
            });
        });
    }
    fn schedules_select(&mut self, id: &str) {
        let state = self.window.global::<ScheduleState>();
        let task = self.settings.schedules["tasks"]
            .as_array()
            .and_then(|tasks| tasks.iter().find(|t| t["id"] == id))
            .and_then(|value| serde_json::from_value::<Task>(value.clone()).ok());
        state.set_edit_id(id.into());
        if id.is_empty() {
            state.set_create_id(uuid::Uuid::new_v4().to_string().into());
        }
        state.set_edit_revision(
            task.as_ref()
                .map(|t| t.revision.to_string())
                .unwrap_or_default()
                .into(),
        );
        state.set_title(task.as_ref().map(|t| t.title.as_str()).unwrap_or("").into());
        state.set_prompt(
            task.as_ref()
                .map(|t| t.prompt.as_str())
                .unwrap_or("")
                .into(),
        );
        state.set_status("".into());
        let pattern = task
            .as_ref()
            .map(|t| t.pattern.clone())
            .unwrap_or(Pattern::Interval { minutes: 60 });
        state.set_kind(match pattern {
            Pattern::Interval { .. } => 0,
            Pattern::Daily { .. } => 1,
            Pattern::Once { .. } => 2,
            Pattern::Webhook { .. } => 3,
        });
        let hook = match &pattern {
            Pattern::Webhook { hook } => hook.clone(),
            _ => Hook::default(),
        };
        state.set_signature_header(hook.header.into());
        state.set_signature_prefix(hook.prefix.into());
        state.set_signature_encoding(if hook.base64 { 1 } else { 0 });
        state.set_replay_key(hook.replay_key.into());
        state.set_coalesce_key(hook.coalesce_key.into());
        state.set_coalesce_seconds(hook.coalesce_seconds.to_string().into());
        state.set_max_age(hook.max_age_minutes.to_string().into());
        state.set_filters(ModelRc::new(VecModel::from(
            hook.filters
                .iter()
                .map(|f| WebhookFilter {
                    path: f.path.as_str().into(),
                    op: FILTER_OPS.iter().position(|op| *op == f.op).unwrap_or(0) as i32,
                    value: f.value.as_str().into(),
                })
                .collect::<Vec<_>>(),
        )));
        state.set_receipts(receipts(task.as_ref()));
        if let Some(task) = &task {
            if let Some(index) = state
                .get_source_ids()
                .iter()
                .position(|id| id == format!("goose:{}", task.source))
            {
                state.set_source_index(index as i32);
            }
        }
        state.set_minutes(
            match pattern {
                Pattern::Interval { minutes } => minutes.to_string(),
                _ => "60".into(),
            }
            .into(),
        );
        state.set_time(
            match pattern {
                Pattern::Daily { hour, minute, .. } => format!("{hour:02}:{minute:02}"),
                _ => "09:00".into(),
            }
            .into(),
        );
        let at = match pattern {
            Pattern::Once { at } => at,
            _ => chrono::Utc::now().timestamp() + 3600,
        };
        state.set_once(display_time(at).into());
        state.set_days(ModelRc::new(VecModel::from(
            ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
                .iter()
                .enumerate()
                .map(|(i, label)| ScheduleDay {
                    label: (*label).into(),
                    enabled: match &pattern {
                        Pattern::Daily { weekdays, .. } => weekdays.contains(&(i as u32)),
                        _ => true,
                    },
                })
                .collect::<Vec<_>>(),
        )));
    }
    fn schedules_save(&mut self) {
        let state = self.window.global::<ScheduleState>();
        if state.get_saving() {
            return;
        }
        let parsed = (|| -> Result<Pattern, String> {
            Ok(match state.get_kind() {
                0 => Pattern::Interval {
                    minutes: state
                        .get_minutes()
                        .trim()
                        .parse()
                        .map_err(|_| "Enter a whole number of minutes")?,
                },
                1 => {
                    let value = state.get_time();
                    let (h, m) = value
                        .trim()
                        .split_once(':')
                        .ok_or("Enter a local time as HH:MM")?;
                    Pattern::Daily {
                        hour: h.parse().map_err(|_| "Invalid hour")?,
                        minute: m.parse().map_err(|_| "Invalid minute")?,
                        weekdays: state
                            .get_days()
                            .iter()
                            .enumerate()
                            .filter_map(|(i, d)| d.enabled.then_some(i as u32))
                            .collect(),
                    }
                }
                3 => Pattern::Webhook {
                    hook: Hook {
                        header: state.get_signature_header().trim().to_ascii_lowercase(),
                        prefix: state.get_signature_prefix().to_string(),
                        base64: state.get_signature_encoding() == 1,
                        replay_key: state.get_replay_key().to_string(),
                        coalesce_key: state.get_coalesce_key().to_string(),
                        coalesce_seconds: state
                            .get_coalesce_seconds()
                            .parse()
                            .map_err(|_| "Enter whole seconds for coalescing")?,
                        max_age_minutes: state
                            .get_max_age()
                            .parse()
                            .map_err(|_| "Enter whole minutes for delivery age")?,
                        filters: state
                            .get_filters()
                            .iter()
                            .map(|f| Filter {
                                path: f.path.to_string(),
                                op: FILTER_OPS
                                    .get(f.op.max(0) as usize)
                                    .unwrap_or(&"equals")
                                    .to_string(),
                                value: f.value.to_string(),
                            })
                            .collect(),
                    },
                },
                _ => {
                    let date =
                        NaiveDateTime::parse_from_str(state.get_once().trim(), "%Y-%m-%d %H:%M:%S")
                            .map_err(|_| "Enter a local date/time as YYYY-MM-DD HH:MM:SS")?;
                    let at = Local.from_local_datetime(&date).single().ok_or("This local time is missing or repeated by a clock change; choose an unambiguous time")?;
                    Pattern::Once { at: at.timestamp() }
                }
            })
        })();
        let pattern = match parsed {
            Ok(p) => p,
            Err(e) => {
                state.set_status(e.into());
                return;
            }
        };
        if let Err(e) = pattern.validate() {
            state.set_status(e.message.into());
            return;
        }
        let source = state
            .get_source_ids()
            .row_data(state.get_source_index().max(0) as usize)
            .unwrap_or_default();
        let params = json!({"id":state.get_edit_id().as_str(),"createId":state.get_create_id().as_str(),"revision":state.get_edit_revision().parse::<u64>().unwrap_or(0),
            "title":state.get_title().as_str(),"prompt":state.get_prompt().as_str(),"source":source.as_str(),"pattern":pattern});
        self.schedules_request("save", params);
    }
    fn schedules_action(&mut self, id: &str, action: &str) {
        if action == "copy-url" {
            let address = self.settings.schedules["webhookAddress"]
                .as_str()
                .unwrap_or("");
            self.copy_to_clipboard(&format!("{address}/hooks/{id}"));
            return;
        }
        let task = self.settings.schedules["tasks"]
            .as_array()
            .and_then(|tasks| tasks.iter().find(|t| t["id"] == id));
        let Some(task) = task else {
            return;
        };
        self.schedules_request(
            action,
            json!({"id":id,"revision":task["revision"],"runId":task["active"]["id"]}),
        );
    }
    fn schedules_request(&mut self, action: &str, params: Value) {
        let state = self.window.global::<ScheduleState>();
        if state.get_saving() {
            return;
        }
        state.set_saving(true);
        state.set_status("".into());
        let method = format!("fastrock/schedule/{action}");
        let action = action.to_owned();
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = backend.goose_request(&method, params).await;
            crate::ui_thread::post(move |app| {
                if action == "delete" && result.is_ok() {
                    app.schedules_select("");
                }
                if action == "copy-key" {
                    if let Ok(reply) = &result {
                        if let Some(key) = reply["key"].as_str() { app.copy_to_clipboard(key); }
                    }
                }
                let state = app.window.global::<ScheduleState>();
                state.set_saving(false);
                match result {
                    Ok(reply) => {
                        if action == "save" {
                            state.set_edit_id(reply["task"]["id"].as_str().unwrap_or("").into());
                            state.set_edit_revision(
                                reply["task"]["revision"]
                                    .as_u64()
                                    .unwrap_or(0)
                                    .to_string()
                                    .into(),
                            );
                        }
                        state.set_status(
                            match action.as_str() {
                                "save" => "Schedule saved",
                                "pause" => "Future runs paused; a current run continues",
                                "resume" => "Schedule resumed; queued webhooks may run; missed timed occurrences skipped",
                                "copy-key" => "Signing key copied to clipboard",
                                "rotate-key" => "Signing key rotated; update the sender with the new key",
                                "discard-pending" => "Pending deliveries discarded; replay receipts retained",
                                "run" => "New occurrence started in an isolated workspace",
                                "stop" => "Run interruption requested",
                                _ => "Schedule deleted; run conversations and workspaces retained",
                            }
                            .into(),
                        );
                    }
                    Err(e) => state.set_status(e.into()),
                }
                app.schedules_refresh();
            });
        });
    }
    pub(crate) fn schedules_automation(&mut self, args: &[String]) {
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        let state = self.window.global::<ScheduleState>();
        let selected = || {
            state
                .get_rows()
                .iter()
                .find(|r| r.title == arg(1))
                .map(|r| r.id.to_string())
                .unwrap_or_else(|| arg(1).into())
        };
        match arg(0) {
            "new" => state.invoke_new_task(),
            "refresh" => state.invoke_refresh(),
            "select" => state.invoke_select(selected().into()),
            "title" => state.set_title(arg(1).into()),
            "prompt" => state.set_prompt(arg(1).into()),
            "kind" => state.set_kind(arg(1).parse().unwrap_or(0)),
            "minutes" => state.set_minutes(arg(1).into()),
            "once" => state.set_once(arg(1).into()),
            "time" => state.set_time(arg(1).into()),
            "source" => state.set_source_index(arg(1).parse().unwrap_or(0)),
            "signature-header" => state.set_signature_header(arg(1).into()),
            "signature-prefix" => state.set_signature_prefix(arg(1).into()),
            "signature-encoding" => state.set_signature_encoding(arg(1).parse().unwrap_or(0)),
            "replay-key" => state.set_replay_key(arg(1).into()),
            "coalesce-key" => state.set_coalesce_key(arg(1).into()),
            "coalesce-seconds" => state.set_coalesce_seconds(arg(1).into()),
            "max-age" => state.set_max_age(arg(1).into()),
            "filter" => {
                let mut filters: Vec<_> = state.get_filters().iter().collect();
                filters.push(WebhookFilter {
                    path: arg(1).into(),
                    op: arg(2).parse().unwrap_or(0),
                    value: arg(3).into(),
                });
                state.set_filters(ModelRc::new(VecModel::from(filters)));
            }
            "save" => state.invoke_save(),
            // Simulate a lost creation receipt without changing its identity.
            "retry-create" => {
                state.set_edit_id("".into());
                state.invoke_save();
            }
            "run" | "pause" | "resume" | "stop" | "delete" | "rotate-key" | "discard-pending" => {
                state.invoke_action(selected().into(), arg(0).into())
            }
            "dump" => {
                let value = json!({"data":self.settings.schedules,"status":state.get_status().as_str(),"loading":state.get_loading(),"saving":state.get_saving(),
                    "rows":state.get_rows().iter().map(|r|json!({"id":r.id.as_str(),"title":r.title.as_str(),"detail":r.detail.as_str(),"child":r.child.as_str(),"paused":r.paused,"running":r.running})).collect::<Vec<_>>()});
                let path = std::path::PathBuf::from(arg(1));
                self.backend.spawn(async move {
                    let _ = tokio::fs::write(path, value.to_string()).await;
                });
            }
            _ => {}
        }
    }
}
fn display_time(at: i64) -> String {
    Local
        .timestamp_opt(at, 0)
        .single()
        .map(|at| at.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}
fn row(task: &Task) -> ScheduleRow {
    let run = task.active.as_ref().or(task.last.as_ref());
    let mut detail = if task.paused { "Paused" } else { "Enabled" }.to_string();
    if let Some(run) = run {
        detail.push_str(&format!(" · {}", run.status));
        if !run.detail.is_empty() {
            detail.push_str(&format!(" · {}", run.detail));
        }
    }
    ScheduleRow {
        id: task.id.as_str().into(),
        title: task.title.as_str().into(),
        detail: detail.into(),
        next: task
            .next_at
            .map(|at| format!("Next: {} (local)", display_time(at)))
            .unwrap_or_else(|| {
                if matches!(task.pattern, Pattern::Webhook { .. }) {
                    format!(
                        "Webhook · {} pending deliveries",
                        task.deliveries
                            .iter()
                            .filter(|d| d.status == "pending")
                            .count()
                    )
                } else {
                    "No future occurrence".into()
                }
            })
            .into(),
        paused: task.paused,
        running: task.active.is_some(),
        webhook: matches!(task.pattern, Pattern::Webhook { .. }),
        child: run
            .and_then(|r| r.child.as_ref())
            .map(|id| format!("goose:{id}"))
            .unwrap_or_default()
            .into(),
    }
}

const FILTER_OPS: [&str; 5] = ["equals", "notEquals", "startsWith", "in", "exists"];
fn receipts(task: Option<&Task>) -> ModelRc<WebhookReceipt> {
    ModelRc::new(VecModel::from(
        task.into_iter()
            .flat_map(|t| t.deliveries.iter().rev().take(100))
            .map(|d| WebhookReceipt {
                detail: format!("{} · {} · {}", display_time(d.received), d.status, d.id).into(),
                child: d
                    .child
                    .as_ref()
                    .map(|c| format!("goose:{c}"))
                    .unwrap_or_default()
                    .into(),
            })
            .collect::<Vec<_>>(),
    ))
}
