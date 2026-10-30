//! User-requested native PTYs. Core services never use this path.
mod archive;
pub(crate) mod command;
mod input;
mod lease;
mod process;
mod runtime;
mod screen;
use crate::{
    app::AppController,
    ui::{TerminalCell, TerminalState, TerminalTab},
};
pub(crate) use lease::ContextInfo as CommandContext;
pub(crate) use runtime::shutdown;
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Default)]
pub(crate) struct Controller {
    entries: Vec<Entry>,
    selected: HashMap<String, i32>,
    visible: bool,
    next_id: i32,
    cells: Rc<VecModel<TerminalCell>>,
    shown: Option<i32>,
    rows: u16,
    cols: u16,
}
struct Entry {
    id: i32,
    thread: String,
    frame: screen::Frame,
    handle: Option<runtime::Handle>,
    paths: String,
}
impl AppController {
    pub(crate) fn terminal_show_existing(&mut self) {
        if self.terminal_entry().is_none() {
            self.toast("This conversation has no open terminals.");
        } else {
            self.terminal.visible = true;
            self.terminal_show();
        }
    }
    pub(crate) fn terminal_stop_owned(&mut self) {
        let Some(thread) = self.terminal_thread() else {
            return;
        };
        if !self
            .terminal
            .entries
            .iter()
            .any(|e| e.thread == thread && e.handle.is_some())
        {
            self.toast("This conversation has no open terminals.");
            return;
        }
        self.show_dialog(crate::app::DialogRequest::confirm("Close conversation terminals?", "Stop this conversation's terminal processes? Unimported workspace changes are retained for recovery.").accept_label("Close terminals"), Box::new(move |app, accepted| {
            if accepted.is_some() {
                for entry in app.terminal.entries.iter().filter(|e|e.thread == thread) {
                    if let Some(handle) = &entry.handle { handle.close(Vec::new()); }
                }
                app.terminal_show();
            }
        }));
    }
    fn terminal_thread(&self) -> Option<String> {
        self.active_thread_index()
            .and_then(|i| self.thread_tab(i))
            .and_then(|t| t.thread_id.clone())
    }
    fn terminal_entry(&self) -> Option<&Entry> {
        let thread = self.terminal_thread()?;
        let id = self.terminal.selected.get(&thread)?;
        self.terminal.entries.iter().find(|e| e.id == *id)
    }
    pub(crate) fn terminal_bind(&mut self) {
        let state = self.window.global::<TerminalState>();
        state.set_cells(ModelRc::from(self.terminal.cells.clone()));
        state.on_action(crate::ui_thread::scoped_callback!(|action| {
            crate::ui_thread::with_app(move |app| app.terminal_action(&action));
        }));
        state.on_select(crate::ui_thread::scoped_callback!(|id| {
            crate::ui_thread::with_app(move |app| {
                if let Some(entry) = app.terminal.entries.iter().find(|e| e.id == id) {
                    app.terminal.selected.insert(entry.thread.clone(), id);
                    app.terminal_show();
                }
            });
        }));
        state.on_resize(crate::ui_thread::scoped_callback!(|rows, cols| {
            crate::ui_thread::with_app(move |app| {
                app.terminal.rows = rows.clamp(2, 80) as u16;
                app.terminal.cols = cols.clamp(10, 200) as u16;
                if let Some(handle) = app.terminal_entry().and_then(|e| e.handle.as_ref()) {
                    handle.resize(app.terminal.rows, app.terminal.cols);
                }
            });
        }));
        state.on_scroll(crate::ui_thread::scoped_callback!(|rows| {
            crate::ui_thread::with_app(move |app| {
                if let Some(handle) = app.terminal_entry().and_then(|e| e.handle.as_ref()) {
                    handle.scroll(rows);
                }
            });
        }));
        state.on_key(crate::ui_thread::scoped_callback!(
            |text, primary, shift, alt, secondary| {
                crate::ui_thread::with_app(move |app| {
                    let copy_modifier = if cfg!(target_os = "macos") {
                        primary
                    } else {
                        primary && shift
                    };
                    if copy_modifier && text.eq_ignore_ascii_case("c") {
                        app.terminal_action("copy");
                        return;
                    }
                    if copy_modifier && text.eq_ignore_ascii_case("v") {
                        app.terminal_action("paste");
                        return;
                    }
                    let control = if cfg!(target_os = "macos") {
                        secondary
                    } else {
                        primary
                    };
                    let application = app
                        .terminal_entry()
                        .is_some_and(|e| e.frame.application_cursor);
                    app.terminal_input(input::key(&text, control, shift, alt, application), false);
                });
            }
        ));
    }
    pub(crate) fn terminal_toggle(&mut self) {
        if self.terminal_thread().is_none() {
            self.toast("Open a conversation to use its terminal");
            return;
        }
        if self.terminal_entry().is_none() {
            self.terminal_new();
            return;
        }
        self.terminal.visible = !self.terminal.visible;
        self.terminal_show();
        if !self.terminal.visible {
            self.restore_focus();
        }
    }
    pub(crate) fn terminal_delete_guard(&self, id: &str) -> bool {
        self.terminal
            .entries
            .iter()
            .any(|e| e.thread == id && !e.frame.finished)
    }
    fn terminal_new(&mut self) {
        if self
            .active_thread_index()
            .and_then(|i| self.thread_tab(i))
            .is_some_and(|t| t.extras.deleting)
        {
            self.toast("Deletion is pending. Terminal was not opened.");
            return;
        }
        let Some(thread) = self.terminal_thread() else {
            return;
        };
        if self.terminal.entries.len() >= 16 {
            self.toast("Close an existing terminal before opening another (16 per window)");
            return;
        }
        self.terminal.next_id += 1;
        let id = self.terminal.next_id;
        self.terminal.entries.push(Entry {
            id,
            thread: thread.clone(),
            frame: screen::Frame {
                status: "Preparing terminal…".into(),
                ..Default::default()
            },
            handle: None,
            paths: String::new(),
        });
        self.terminal.selected.insert(thread.clone(), id);
        self.terminal.visible = true;
        self.terminal_show();
        let backend = self.backend.clone();
        let rows = self.terminal.rows.max(12);
        let cols = self.terminal.cols.max(80);
        self.backend.spawn(async move {
            let result = async {
                let value = backend
                    .goose_request(
                        "fastrock/goose/terminalContext",
                        serde_json::json!({"threadId":thread}),
                    )
                    .await
                    .map_err(anyhow::Error::msg)?;
                let context = serde_json::from_value(value)?;
                runtime::open(context, rows, cols).await
            }
            .await;
            crate::ui_thread::post(move |app| match result {
                Ok(handle) => {
                    let Some(entry) = app.terminal.entries.iter_mut().find(|e| e.id == id) else {
                        return;
                    };
                    let mut frames = handle.frames.clone();
                    entry.frame = frames.borrow().clone();
                    handle.resize(app.terminal.rows.max(2), app.terminal.cols.max(10));
                    entry.handle = Some(handle);
                    app.terminal_show();
                    let pending = Arc::new(AtomicBool::new(false));
                    app.backend.spawn(async move {
                        loop {
                            let latest = frames.clone();
                            let posted = pending.clone();
                            if !pending.swap(true, Ordering::AcqRel)
                                && !crate::ui_thread::post(move |app| {
                                    posted.store(false, Ordering::Release);
                                    if let Some(entry) =
                                        app.terminal.entries.iter_mut().find(|e| e.id == id)
                                    {
                                        entry.frame = latest.borrow().clone();
                                        app.terminal_show();
                                    }
                                })
                            {
                                break;
                            }
                            if frames.changed().await.is_err() {
                                break;
                            }
                        }
                    });
                }
                Err(error) => {
                    if let Some(entry) = app.terminal.entries.iter_mut().find(|e| e.id == id) {
                        entry.frame.finished = true;
                        entry.frame.status = format!("Terminal failed: {error:#}");
                    }
                    app.terminal_show();
                }
            });
        });
    }
    fn terminal_input(&mut self, data: Vec<u8>, paste: bool) {
        if data.is_empty() {
            return;
        }
        let result = self
            .terminal_entry()
            .and_then(|e| e.handle.as_ref())
            .map(|h| h.input(data, paste));
        if let Some(Err(e)) = result {
            self.toast(&e.to_string());
        }
        let state = self.window.global::<TerminalState>();
        state.set_selection_start(-1);
        state.set_selection_end(-1);
        state.set_focus_generation(state.get_focus_generation().wrapping_add(1));
    }
    pub(crate) fn terminal_action(&mut self, action: &str) {
        match action {
            "new" => self.terminal_new(),
            "hide" => {
                self.terminal.visible = false;
                self.terminal_show();
                self.restore_focus();
            }
            "paste" => {
                if let Some(text) = self.clipboard_text() {
                    self.terminal_input(text.into_bytes(), true);
                }
            }
            "copy" => {
                if let Some(entry) = self.terminal_entry() {
                    let state = self.window.global::<TerminalState>();
                    let text = selected_text(
                        &entry.frame,
                        state.get_selection_start(),
                        state.get_selection_end(),
                    );
                    self.copy_to_clipboard(&text);
                }
            }
            "close" | "import" => {
                let paths = if action == "import" {
                    match lease::import_paths(&self.window.global::<TerminalState>().get_paths()) {
                        Ok(paths) => paths,
                        Err(e) => {
                            self.toast(&e.to_string());
                            return;
                        }
                    }
                } else {
                    Vec::new()
                };
                let Some(id) = self.terminal_entry().map(|e| e.id) else {
                    return;
                };
                let entry = self
                    .terminal
                    .entries
                    .iter_mut()
                    .find(|e| e.id == id)
                    .unwrap();
                if entry.frame.finished || entry.handle.is_none() {
                    let thread = entry.thread.clone();
                    self.terminal.entries.retain(|e| e.id != id);
                    if let Some(e) = self
                        .terminal
                        .entries
                        .iter()
                        .rev()
                        .find(|e| e.thread == thread)
                    {
                        self.terminal.selected.insert(thread, e.id);
                    } else {
                        self.terminal.selected.remove(&thread);
                        self.terminal.visible = false;
                    }
                } else {
                    if let Some(handle) = &entry.handle {
                        handle.close(paths);
                    }
                    entry.frame.status = "Stopping terminal and preserving source edits…".into();
                }
                self.terminal_show();
            }
            _ => {}
        }
    }
    pub(crate) fn terminal_show(&mut self) {
        let state = self.window.global::<TerminalState>();
        if let Some(old) = self
            .terminal
            .shown
            .and_then(|id| self.terminal.entries.iter_mut().find(|e| e.id == id))
        {
            old.paths = state.get_paths().to_string();
        }
        let thread = self.terminal_thread();
        let entry = self.terminal_entry();
        let id = entry.map(|e| e.id);
        let frame = entry.map(|e| e.frame.clone()).unwrap_or_default();
        let paths = entry.map(|e| e.paths.clone()).unwrap_or_default();
        let visible = self.terminal.visible && thread.is_some() && id.is_some();
        state.set_visible(visible);
        let switched = self.terminal.shown != id;
        self.terminal.shown = id;
        if !visible {
            state.set_focused(false);
            return;
        }
        if switched {
            state.set_paths(paths.into());
            state.set_selection_start(-1);
            state.set_selection_end(-1);
            state.set_focus_generation(state.get_focus_generation().wrapping_add(1));
        }
        let tabs = self
            .terminal
            .entries
            .iter()
            .filter(|e| Some(&e.thread) == thread.as_ref())
            .map(|e| TerminalTab {
                id: e.id,
                title: format!(
                    "Shell {}{}",
                    e.id,
                    if e.frame.finished { " · closed" } else { "" }
                )
                .into(),
                selected: Some(e.id) == id,
            })
            .collect::<Vec<_>>();
        let current = state.get_tabs();
        if current.row_count() != tabs.len()
            || tabs
                .iter()
                .enumerate()
                .any(|(i, row)| current.row_data(i).as_ref() != Some(row))
        {
            state.set_tabs(ModelRc::new(VecModel::from(tabs)));
        }
        state.set_rows(frame.rows.max(2).into());
        state.set_cols(frame.cols.max(10).into());
        state.set_scrollback(frame.scrollback as i32);
        state.set_history(frame.history as i32);
        state.set_cursor_row(frame.cursor.map_or(-1, |v| v.0.into()));
        state.set_cursor_col(frame.cursor.map_or(-1, |v| v.1.into()));
        state.set_running(
            !frame.finished
                && frame.pid != 0
                && self
                    .terminal_entry()
                    .and_then(|e| e.handle.as_ref())
                    .is_some_and(|h| !h.stopping()),
        );
        state.set_virtual_copy(frame.virtual_copy);
        state.set_status(frame.status.into());
        state.set_cwd(frame.cwd.into());
        let cells = frame
            .cells
            .iter()
            .map(|c| TerminalCell {
                text: c.text.as_str().into(),
                foreground: slint::Color::from_argb_encoded(0xff000000 | c.fg),
                background: slint::Color::from_argb_encoded(0xff000000 | c.bg),
                bold: c.bold,
                italic: c.italic,
                underline: c.underline,
                span: i32::from(c.width),
            })
            .collect::<Vec<_>>();
        if self.terminal.cells.row_count() != cells.len() {
            self.terminal.cells.set_vec(cells);
        } else {
            for (i, cell) in cells.into_iter().enumerate() {
                if self.terminal.cells.row_data(i).as_ref() != Some(&cell) {
                    self.terminal.cells.set_row_data(i, cell);
                }
            }
        }
    }
    pub(crate) fn terminal_close_thread(&mut self, thread: &str) {
        self.terminal.entries.retain(|e| e.thread != thread);
        self.terminal.selected.remove(thread);
    }
    pub(crate) fn terminal_automation(&mut self, args: &[String]) {
        match args.first().map(String::as_str) {
            Some("clipboard") => {
                if let Some(text) = args.get(1) {
                    self.copy_to_clipboard(text);
                }
            }
            Some("input") => {
                if let Some(text) = args.get(1) {
                    self.terminal_input(text.as_bytes().to_vec(), false);
                }
            }
            Some("paths") => {
                if let Some(text) = args.get(1) {
                    self.window
                        .global::<TerminalState>()
                        .set_paths(text.as_str().into());
                }
            }
            Some("scroll") => {
                if let Some(handle) = self.terminal_entry().and_then(|e| e.handle.as_ref()) {
                    handle.scroll(args.get(1).and_then(|v| v.parse().ok()).unwrap_or(0));
                }
            }
            Some("resize") => {
                if let Some(handle) = self.terminal_entry().and_then(|e| e.handle.as_ref()) {
                    handle.resize(
                        args.get(1).and_then(|v| v.parse().ok()).unwrap_or(24),
                        args.get(2).and_then(|v| v.parse().ok()).unwrap_or(80),
                    );
                }
            }
            Some("dump") => {
                if let Some(path) = args.get(1) {
                    let state = self.window.global::<TerminalState>();
                    let entry = self.terminal_entry();
                    let value = serde_json::json!({"visible":state.get_visible(),"focused":state.get_focused(),"text":entry.map(|e|e.frame.text.as_str()),"status":state.get_status().as_str(),"pid":entry.map(|e|e.frame.pid),"finished":entry.is_some_and(|e|e.frame.finished),"cwd":state.get_cwd().as_str(),"virtualCopy":state.get_virtual_copy(),"rows":state.get_rows(),"cols":state.get_cols(),"history":state.get_history(),"scrollback":state.get_scrollback(),"terminals":self.terminal.entries.len()});
                    let path = path.clone();
                    self.backend.spawn(async move {
                        let _ = tokio::fs::write(path, value.to_string()).await;
                    });
                }
            }
            Some(action) => self.terminal_action(action),
            None => {}
        }
    }
}
fn selected_text(frame: &screen::Frame, start: i32, end: i32) -> String {
    if start < 0 || end < 0 || frame.cols == 0 {
        return frame.text.clone();
    }
    let first = start.min(end) as usize;
    let last = start.max(end) as usize;
    let mut text = String::new();
    for i in first..=last.min(frame.cells.len().saturating_sub(1)) {
        if i > first && i % usize::from(frame.cols) == 0 {
            while text.ends_with(' ') {
                text.pop();
            }
            text.push('\n');
        }
        if let Some(c) = frame.cells.get(i) {
            if c.width > 0 {
                text.push_str(if c.text.is_empty() { " " } else { &c.text });
            }
        }
    }
    text.trim_end_matches(' ').into()
}
