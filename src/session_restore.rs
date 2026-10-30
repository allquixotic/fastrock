//! Durable UI drafts, separate from Goose history and its atomic dispatch journal.
//! A recovered local send is evidence, never an instruction to replay a tool.
use crate::{
    app::{AppController, TabKind, ThreadPhase, ThreadTab},
    composer::{ComposerDraft, attachments::Attachment},
    session::TurnOverrides,
    threads::PendingInput,
    ui::ComposerState,
};
use codex_protocol::{config_types::ModeKind, openai_models::ReasoningEffort};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use slint::ComponentHandle;
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

const FILE: &str = "fastrock-open-chats.json";
const MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Default, Serialize, Deserialize)]
struct Draft {
    text: String,
    attachments: Vec<PathBuf>,
    mode: ModeKind,
    pre_plan_effort: Option<Option<ReasoningEffort>>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Chat {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    workspace: Option<fastrock_workspaces::session::Selection>,
    thread_id: Option<String>,
    cwd: PathBuf,
    name: Option<String>,
    preview: String,
    model: Option<String>,
    model_provider: Option<String>,
    effort: Option<ReasoningEffort>,
    overrides: TurnOverrides,
    xtab_enabled: bool,
    draft: Draft,
    pending: Vec<PendingInput>,
    recovered: Vec<Value>,
}
impl Chat {
    fn capture(thread: &ThreadTab, text: Option<String>) -> Self {
        let mut recovered = thread.recovered_sends.clone();
        if let Some(preparing) = &thread.composer.preparing {
            let mut input: Vec<_> = preparing
                .attachments
                .iter()
                .map(|a| json!({"type":"localImage","path":a.path}))
                .collect();
            input.push(json!({"type":"text","text":preparing.text,"text_elements":[]}));
            recovered.push(json!({"id":"local-preparing","input":input,"mode":preparing.mode,"recoveryReason":"Image preparation was interrupted. Review and send manually."}));
        }
        Self {
            project_id: thread.extras.project_id.clone(),
            workspace: thread.extras.workspace.clone(),
            thread_id: thread.thread_id.clone(),
            cwd: thread.cwd.clone(),
            name: thread.name.clone(),
            preview: thread.preview.clone(),
            model: thread.model.clone(),
            model_provider: thread.model_provider.clone(),
            effort: thread.effort.clone(),
            overrides: thread.turn_overrides.clone(),
            xtab_enabled: thread.xtab_enabled,
            draft: Draft {
                text: text.unwrap_or_else(|| thread.composer.text.clone()),
                attachments: thread
                    .composer
                    .attachments
                    .iter()
                    .map(|a| a.path.clone())
                    .collect(),
                mode: thread.composer.mode,
                pre_plan_effort: thread.composer.pre_plan_effort.clone(),
            },
            pending: thread.pending_inputs.clone(),
            recovered,
        }
    }
    fn restore(self) -> ThreadTab {
        let mut thread = ThreadTab::new(self.cwd);
        thread.extras.workspace = self.workspace;
        thread.extras.project_id = self.project_id;
        thread.thread_id = self.thread_id;
        thread.name = self.name;
        thread.preview = self.preview;
        thread.model = self.model;
        thread.model_provider = self.model_provider;
        thread.effort = self.effort;
        thread.turn_overrides = self.overrides;
        thread.xtab_enabled = self.xtab_enabled;
        thread.phase = ThreadPhase::Idle;
        thread.recovery_paused = true;
        thread.composer = ComposerDraft {
            text: self.draft.text,
            attachments: self
                .draft
                .attachments
                .into_iter()
                .map(Attachment::new)
                .collect(),
            mode: self.draft.mode,
            pre_plan_effort: self.draft.pre_plan_effort,
            ..Default::default()
        };
        thread.recovered_sends = self.recovered;
        for pending in self.pending {
            // The latest UI snapshot can predate the backend accepting this send.
            // Only the backend dispatch journal can prove whether replay is safe.
            let id = format!("local:{}", pending.client_id);
            if !thread.recovered_sends.iter().any(|v| v["id"] == id) {
                thread.recovered_sends.push(json!({"id":id,"clientUserMessageId":pending.client_id,"input":pending.input,"mode":pending.mode,"recoveryReason":"Saved before Fastrock stopped; delivery is unknown. Review conversation history before sending manually."}));
            }
        }
        thread
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Window {
    chats: Vec<Chat>,
    active: Option<usize>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    windows: BTreeMap<u64, Arc<Window>>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: 1,
            windows: BTreeMap::new(),
        }
    }
}
fn read(home: &Path) -> Result<Snapshot, String> {
    let file = match std::fs::File::open(home.join(FILE)) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Snapshot::default()),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Open-chat snapshot exceeds the recovery size limit".into());
    }
    let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if snapshot.version != 1 {
        return Err("Unknown open-chat snapshot version".into());
    }
    Ok(snapshot)
}
fn write(home: &Path, snapshot: &Snapshot) -> Result<(), String> {
    let bytes = serde_json::to_vec(snapshot).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Open-chat snapshot exceeds the recovery size limit".into());
    }
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    crate::rally::store::atomic_write(&home.join(FILE), &bytes).map_err(|e| e.to_string())
}
#[derive(Clone, Default)]
struct Update {
    revision: u64,
    enabled: bool,
    home: PathBuf,
    snapshot: Snapshot,
}
pub(crate) struct Writer {
    updates: tokio::sync::watch::Sender<Update>,
    finished: tokio::sync::watch::Receiver<(u64, Option<String>)>,
}
impl Writer {
    pub(crate) fn new(runtime: &tokio::runtime::Handle) -> Self {
        let (updates, mut pending) = tokio::sync::watch::channel(Update::default());
        let (done, finished) = tokio::sync::watch::channel((0, None));
        runtime.spawn(async move {
            while pending.changed().await.is_ok() {
                // Coalesce typing and streaming updates; no polling when idle.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let save = pending.borrow_and_update().clone();
                if !save.enabled {
                    continue;
                }
                let revision = save.revision;
                let result = tokio::task::spawn_blocking(move || write(&save.home, &save.snapshot))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|v| v);
                if let Err(error) = &result {
                    tracing::warn!(%error, "Could not save open chat drafts");
                    let message = format!("Could not save open chat drafts: {error}");
                    crate::ui_thread::post_for(crate::ui_thread::MAIN, move |app| {
                        app.push_warning(message)
                    });
                }
                done.send_replace((revision, result.err()));
            }
        });
        Self { updates, finished }
    }
    fn enable(&self, home: PathBuf) {
        self.updates.send_modify(|save| {
            save.enabled = true;
            save.home = home;
            save.revision += 1;
        });
    }
    fn save(&self, owner: u64, window: Window) {
        self.updates.send_modify(|save| {
            save.snapshot.windows.insert(owner, Arc::new(window));
            save.revision += 1;
        });
    }
    pub(crate) async fn flush(&self) -> Result<(), String> {
        let (enabled, revision) = {
            let save = self.updates.borrow();
            (save.enabled, save.revision)
        };
        if !enabled {
            return Ok(());
        }
        let mut finished = self.finished.clone();
        let result = finished
            .wait_for(|(done, _)| *done >= revision)
            .await
            .map_err(|e| e.to_string())?;
        result.1.clone().map_or(Ok(()), Err)
    }
}
impl AppController {
    pub(crate) fn save_open_chats(&self) {
        if !self.session_restore_ready {
            return;
        }
        let shown = self.shown_composer_tab();
        let mut window = Window::default();
        for (index, tab) in self.tabs.iter().enumerate() {
            let Some(thread) = tab.thread() else {
                continue;
            };
            // Temporary side sessions have no durable Goose history.
            if thread.extras.side_parent.is_some() {
                continue;
            }
            if self.active == Some(index) {
                window.active = Some(window.chats.len());
            }
            let text = (shown == Some(tab.id))
                .then(|| self.window.global::<ComposerState>().get_text().to_string());
            window.chats.push(Chat::capture(thread, text));
        }
        self.backend
            .sessions()
            .save(self.backend.window_id(), window);
    }
    pub(crate) fn restore_open_chats(&mut self) {
        if self.backend.window_id() != crate::ui_thread::MAIN {
            self.session_restore_ready = true;
            return;
        }
        let Some(home) = self.codex_home.clone() else {
            return;
        };
        self.backend.spawn(async move {
            let path = home.clone();
            let loaded = tokio::task::spawn_blocking(move || read(&path)).await.map_err(|e| e.to_string()).and_then(|v| v);
            crate::ui_thread::post(move |app| {
                let snapshot = match loaded {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        app.push_warning(format!("Open chats could not be restored: {error}. The original recovery file is preserved; draft saving is disabled for this launch."));
                        return;
                    }
                };
                let mut active = None;
                for window in snapshot.windows.values() {
                    for (position, chat) in window.chats.iter().cloned().enumerate() {
                        if chat.thread_id.as_ref().is_some_and(|id| !id.starts_with("goose:")) { continue; }
                        let existing = chat.thread_id.as_ref().and_then(|id| app.tab_index_for_thread(id));
                        let restored = chat.restore();
                        let index = if let Some(index) = existing {
                            let thread = app.thread_tab_mut(index).unwrap();
                            if thread.composer.text.is_empty() { thread.composer = restored.composer; }
                            thread.recovered_sends.extend(restored.recovered_sends);
                            thread.recovery_paused = true;
                            index
                        } else { app.push_tab(TabKind::Thread(Box::new(restored)), true) };
                        if window.active == Some(position) && active.is_none() { active = Some(index); }
                    }
                }
                app.session_restore_ready = true;
                if let Some(index) = active { app.activate_tab(index); }
                app.save_open_chats();
                app.backend.sessions().enable(home);
                if app.backend.is_ready() { app.reattach_recovered_chats(); }
                app.composer_refresh();
            });
        });
    }
    pub(crate) fn reattach_recovered_chats(&mut self) {
        let targets: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| {
                let thread = tab.thread()?;
                (thread.recovery_paused && thread.extras.open_generation == 0)
                    .then(|| thread.thread_id.clone().map(|id| (index, id)))
                    .flatten()
            })
            .collect();
        for (index, id) in targets {
            self.resume_thread_in_tab(index, id, false);
        }
    }
    pub(crate) fn pause_local_inputs(&mut self) {
        for tab in &mut self.tabs {
            if let TabKind::Thread(thread) = &mut tab.kind {
                if !thread.pending_inputs.is_empty() || thread.composer.preparing.is_some() {
                    thread.recovery_paused = true;
                }
            }
        }
        self.save_open_chats();
    }
    pub(crate) fn resume_local_inputs(&mut self, tab_id: crate::app::TabId) {
        let Some(index) = self.tab_index_by_id(tab_id) else {
            return;
        };
        let Some(thread) = self.thread_tab_mut(index) else {
            return;
        };
        thread.recovery_paused = false;
        if thread.thread_id.is_none() {
            self.start_thread_in_tab(index);
        } else {
            self.dispatch_pending_inputs(index);
        }
        self.save_open_chats();
        self.composer_refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v47_snapshot_retains_drafts_and_quarantines_local_sends() {
        let mut thread = ThreadTab::new(PathBuf::from("fixture"));
        thread.thread_id = Some("goose:fixture".into());
        thread.composer.text = "unsent é 🔭".into();
        thread.effort = Some(ReasoningEffort::High);
        thread.turn_overrides.model = Some("saved-model".into());
        thread.composer.mode = ModeKind::Plan;
        thread.composer.preparing = Some(crate::composer::PreparingMessage {
            text: "preparing images".into(),
            attachments: vec![Attachment::new(PathBuf::from("pending.png"))],
            mode: crate::prefs::BusyInput::Queue,
        });
        thread
            .composer
            .attachments
            .push(Attachment::new(PathBuf::from("image.png")));
        thread.pending_inputs.push(PendingInput {
            input: vec![codex_app_server_protocol::UserInput::Text {
                text: "possibly delivered".into(),
                text_elements: vec![],
            }],
            client_id: "send-1".into(),
            mode: crate::prefs::BusyInput::Steer,
        });
        let bytes = serde_json::to_vec(&Chat::capture(&thread, None)).unwrap();
        let restored = serde_json::from_slice::<Chat>(&bytes).unwrap().restore();
        assert!(restored.recovery_paused);
        assert!(restored.pending_inputs.is_empty());
        assert_eq!(restored.composer.text, "unsent é 🔭");
        assert_eq!(
            restored.composer.attachments[0].path,
            PathBuf::from("image.png")
        );
        assert_eq!(restored.effort, Some(ReasoningEffort::High));
        assert_eq!(
            restored.turn_overrides.model.as_deref(),
            Some("saved-model")
        );
        assert_eq!(restored.composer.mode, ModeKind::Plan);
        assert!(restored.composer.preparing.is_none());
        assert_eq!(restored.recovered_sends.len(), 2);
        assert_eq!(
            restored.recovered_sends[0]["input"][0]["path"],
            "pending.png"
        );
        assert_eq!(restored.recovered_sends[1]["clientUserMessageId"], "send-1");
        assert_eq!(
            Chat::capture(&restored, None)
                .restore()
                .recovered_sends
                .len(),
            2
        );
    }
    #[tokio::test]
    async fn v47_writer_flushes_all_windows_and_closed_tabs() {
        let temp = tempfile::tempdir().unwrap();
        let writer = Writer::new(&tokio::runtime::Handle::current());
        writer.enable(temp.path().into());
        for owner in 1..=3 {
            let mut thread = ThreadTab::new(PathBuf::from("fixture"));
            thread.composer.text = format!("window {owner}");
            writer.save(
                owner,
                Window {
                    chats: vec![Chat::capture(&thread, None)],
                    active: Some(0),
                },
            );
        }
        writer.flush().await.unwrap();
        let saved = read(temp.path()).unwrap();
        assert_eq!(saved.windows.len(), 3);
        assert_eq!(saved.windows[&2].chats[0].draft.text, "window 2");
        writer.save(2, Window::default());
        writer.flush().await.unwrap();
        assert!(read(temp.path()).unwrap().windows[&2].chats.is_empty());
    }
    #[tokio::test]
    async fn v47_writer_reports_failed_save_and_recovers() {
        let temp = tempfile::tempdir().unwrap();
        let blocked = temp.path().join("file");
        std::fs::write(&blocked, b"preserve").unwrap();
        let writer = Writer::new(&tokio::runtime::Handle::current());
        writer.enable(blocked.clone());
        writer.save(1, Window::default());
        assert!(writer.flush().await.is_err());
        assert_eq!(std::fs::read(blocked).unwrap(), b"preserve");
        writer.enable(temp.path().into());
        writer.flush().await.unwrap();
        assert_eq!(read(temp.path()).unwrap().windows.len(), 1);
    }

    #[test]
    fn v47_invalid_snapshot_is_not_replaced() {
        let temp = tempfile::tempdir().unwrap();
        for bytes in [b"{invalid".as_slice(), br#"{"version":99,"windows":{}}"#] {
            std::fs::write(temp.path().join(FILE), bytes).unwrap();
            assert!(read(temp.path()).is_err());
            assert_eq!(std::fs::read(temp.path().join(FILE)).unwrap(), bytes);
        }
    }
}
