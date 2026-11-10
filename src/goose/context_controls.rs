use super::context_report::Report;
use crate::{
    app::AppController,
    ui::{ContextReportState, ContextRow},
};
use serde_json::json;
use slint::{ComponentHandle, Model, ModelRc, VecModel};

impl AppController {
    pub(crate) fn context_report_automation(&mut self, args: &[String]) {
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
        let state = self.window.global::<ContextReportState>();
        match arg(0) {
            "snapshot" => {
                let rows: Vec<_> = state.get_rows().iter().map(|r| json!({"title":r.title.as_str(),"detail":r.detail.as_str(),"preview":r.preview.as_str(),"part":r.part})).collect();
                let thinking: Vec<_> = self
                    .active_thread_index()
                    .and_then(|i| self.thread_tab(i))
                    .map(|t| {
                        t.transcript
                            .model()
                            .iter()
                            .filter(|r| r.kind == 7)
                            .map(|r| json!({"id":r.id.as_str(),"expanded":r.expanded}))
                            .collect()
                    })
                    .unwrap_or_default();
                let value = json!({"open":state.get_open(),"busy":state.get_busy(),"thread":state.get_thread().as_str(),"summary":state.get_summary().as_str(),"status":state.get_status().as_str(),"note":state.get_note().as_str(),"rows":rows,"thinking":thinking,"collapseThinking":self.prefs.collapse_thinking});
                let path = arg(1).to_owned();
                self.backend.spawn(async move {
                    let _ = tokio::fs::write(path, value.to_string()).await;
                });
            }
            "select" => state.invoke_select(arg(1).parse().unwrap_or(0)),
            action => state.invoke_action(action.into()),
        }
    }

    pub(crate) fn context_report_install(&mut self) {
        let state = self.window.global::<ContextReportState>();
        state.on_action(crate::ui_thread::scoped_callback!(|action| {
            let action = action.to_string();
            crate::ui_thread::with_app(move |app| app.context_report_action(&action));
        }));
        state.on_select(crate::ui_thread::scoped_callback!(|index| {
            crate::ui_thread::with_app(move |app| {
                let state = app.window.global::<ContextReportState>();
                if let Some(row) = usize::try_from(index)
                    .ok()
                    .and_then(|i| state.get_rows().row_data(i))
                {
                    state.set_selected(index);
                    state.set_preview(row.preview);
                }
            });
        }));
    }

    fn context_thread(&self) -> String {
        self.active_thread_index()
            .and_then(|i| self.thread_tab(i))
            .and_then(|t| t.thread_id.clone())
            .unwrap_or_default()
    }

    pub(crate) fn context_report_sync(&self) {
        let state = self.window.global::<ContextReportState>();
        if state.get_thread().as_str() != self.context_thread() {
            state.set_open(false);
            state.set_generation(state.get_generation().wrapping_add(1));
            state.set_thread("".into());
            state.set_busy(false);
            state.set_rows(ModelRc::default());
            state.set_preview("".into());
        }
    }

    pub(crate) fn context_report_action(&mut self, action: &str) {
        let state = self.window.global::<ContextReportState>();
        if action == "close" {
            state.set_open(false);
            state.set_generation(state.get_generation().wrapping_add(1));
            state.set_busy(false);
            return;
        }
        if !matches!(action, "open" | "refresh") || state.get_busy() {
            return;
        }
        let thread = self.context_thread();
        if !self.backend.is_goose() || thread.is_empty() {
            return;
        }
        let generation = state.get_generation().wrapping_add(1);
        state.set_generation(generation);
        state.set_thread(thread.clone().into());
        state.set_open(true);
        state.set_busy(true);
        state.set_summary("".into());
        state.set_note("".into());
        state.set_status("".into());
        state.set_selected(-1);
        state.set_preview("".into());
        state.set_rows(ModelRc::default());
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = backend.goose_request("fastrock/context/report", json!({"threadId":thread})).await
                .and_then(|v|serde_json::from_value::<Report>(v).map_err(|_|"Invalid context report.".to_owned()));
            crate::ui_thread::post(move |app| {
                let state = app.window.global::<ContextReportState>();
                if !state.get_open() || state.get_generation() != generation || app.context_thread() != thread { return; }
                state.set_busy(false);
                match result {
                    Err(message) => state.set_status(message.into()),
                    Ok(report) => {
                        state.set_summary(if report.managed {
                            format!("{} · Context is managed by the provider", report.model)
                        } else if report.limit == 0 {
                            format!("{} · {} tokens · Context limit unknown", report.model, report.total)
                        } else {
                            format!("{} · {} / {} tokens ({:.1}%)", report.model, report.total, report.limit, report.total as f64 / report.limit as f64 * 100.0)
                        }.into());
                        state.set_note(format!("Local tokenizer estimate of the next request at refresh time; provider accounting may differ. Component totals include their indented parts.{}{}",
                            if report.compaction_pending {" Automatic compaction is pending before the next request."} else {""},
                            if report.truncated {" Some rows or previews are truncated; totals remain complete."} else {""}).into());
                        let rows: Vec<_> = report.rows.into_iter().map(|r| ContextRow {
                            title: format!("{} · {} tokens", r.label, r.tokens).into(),
                            detail: if r.source.is_empty() {r.category} else {format!("{} · {}",r.category,r.source)}.into(),
                            preview: if r.preview.is_empty() {"No text preview is available for this component.".into()} else {r.preview.into()},
                            part: r.part,
                        }).collect();
                        if let Some(first) = rows.first() { state.set_selected(0); state.set_preview(first.preview.clone()); }
                        state.set_rows(ModelRc::new(VecModel::from(rows)));
                        state.set_status("Read-only snapshot. Refresh after a turn or model/tool change.".into());
                    }
                }
            });
        });
    }
}
