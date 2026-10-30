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
    side_parent: Option<crate::threads::SideParent>,
    #[serde(default)]
    last_error: Option<String>,
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
            side_parent: thread.extras.side_parent.clone(),
            last_error: thread.last_error.clone(),
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
        thread.extras.side_parent = self.side_parent;
        thread.last_error = self.last_error;
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
    #[serde(default)]
    size: Option<[f32; 2]>,
    #[serde(default)]
    position: Option<[i32; 2]>,
    #[serde(default)]
    pub(crate) rally: Option<RallyWindow>,
    #[serde(default)]
    active_document: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct RallyWindow {
    pub(crate) root: PathBuf,
    pub(crate) id: String,
}
impl RallyWindow {
    pub(crate) fn store(&self) -> crate::rally::store::Store {
        crate::rally::store::Store {
            directory: self.root.join("windows").join(&self.id),
        }
    }
}
pub(crate) enum Active {
    Chat(crate::app::TabId),
    Rally(String),
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
    validate(&snapshot)?;
    Ok(snapshot)
}
fn validate(snapshot: &Snapshot) -> Result<(), String> {
    let mut registries = std::collections::BTreeSet::new();
    let mut chats = std::collections::BTreeSet::new();
    if snapshot.windows.len() > 128 {
        return Err("Too many saved windows to restore safely; recovery file preserved".into());
    }
    for (owner, window) in &snapshot.windows {
        if window.size.is_some_and(|s| {
            s.iter()
                .any(|v| !v.is_finite() || !(400.0..=10000.0).contains(v))
        }) {
            return Err("Invalid saved window size".into());
        }
        if let Some(rally) = &window.rally {
            if *owner == crate::ui_thread::MAIN
                || !rally.root.is_absolute()
                || uuid::Uuid::parse_str(&rally.id).is_err()
                || !registries.insert((rally.root.clone(), rally.id.clone()))
            {
                return Err("Invalid or duplicate saved Rally window identity".into());
            }
        }
        for chat in &window.chats {
            if let Some(id) = &chat.thread_id {
                if !chats.insert(id) {
                    return Err(
                        "A saved conversation belongs to multiple windows; recovery file preserved"
                            .into(),
                    );
                }
            }
        }
    }
    Ok(())
}
/// Claim every child registry before the main window can recover retired Rally
/// documents. All filesystem work happens on the blocking worker.
fn prepare_windows(mut snapshot: Snapshot) -> Result<Snapshot, String> {
    validate(&snapshot)?;
    for (owner, window) in &mut snapshot.windows {
        if *owner == crate::ui_thread::MAIN {
            continue;
        }
        let window = Arc::make_mut(window);
        if window.rally.is_none() {
            let parent = crate::rally::store::Store::new();
            let mut prefs = parent.load().map_err(|e| e.to_string())?;
            prefs.documents.clear();
            prefs.open_rally_tabs.clear();
            let registry = RallyWindow {
                root: parent.directory,
                id: uuid::Uuid::new_v4().to_string(),
            };
            registry.store().save(&prefs).map_err(|e| e.to_string())?;
            window.rally = Some(registry);
        }
        let rally = window.rally.as_ref().unwrap();
        // Validate the store before claiming it; corrupt drafts stay untouched.
        rally.store().load().map_err(|e| e.to_string())?;
        crate::rally::windows::WindowHub::restore_identity(
            &rally.store(),
            rally.root.clone(),
            rally.id.clone(),
        )
        .map_err(|e| e.to_string())?;
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
    fn adopt(&self, snapshot: Snapshot) {
        self.updates.send_modify(|save| {
            save.snapshot.windows.extend(snapshot.windows);
            save.revision += 1;
        });
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
        let size = self.window.window().size();
        let position = self.window.window().position();
        let scale = self.window.window().scale_factor().max(0.1);
        let mut window = Window {
            size: Some([
                (size.width as f32 / scale).clamp(400., 10000.),
                (size.height as f32 / scale).clamp(400., 10000.),
            ]),
            position: Some([position.x, position.y]),
            rally: self.rally_saved_window(),
            active_document: match &self.session_restore_active {
                Some(Active::Rally(id)) => Some(id.clone()),
                Some(Active::Chat(_)) => None,
                None => self.rally_active_document_identity(),
            },
            ..Default::default()
        };
        for (index, tab) in self.tabs.iter().enumerate() {
            let Some(thread) = tab.thread() else {
                continue;
            };
            let active = match &self.session_restore_active {
                Some(Active::Chat(id)) => *id == tab.id,
                Some(Active::Rally(_)) => false,
                None => self.active == Some(index),
            };
            if active {
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
        if let Some(window) = self.session_restore_pending.take() {
            self.restore_saved_window(window);
            return;
        }
        if self.backend.window_id() != crate::ui_thread::MAIN {
            self.session_restore_ready = true;
            return;
        }
        let Some(home) = self.codex_home.clone() else {
            return;
        };
        self.backend.spawn(async move {
            let path = home.clone();
            let loaded = tokio::task::spawn_blocking(move || read(&path).and_then(prepare_windows)).await.map_err(|e|e.to_string()).and_then(|v|v);
            crate::ui_thread::post(move |app| {
                let snapshot = match loaded {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        app.push_warning(format!("Open windows could not be restored: {error}. The original recovery file is preserved; draft saving is disabled for this launch."));
                        return;
                    }
                };
                // Runtime IDs may have changed, or the user may already have
                // opened another window. Reserve fresh owners for each child.
                let windows: BTreeMap<_,_> = snapshot.windows.into_iter().map(|(old, window)| {
                    (if old == crate::ui_thread::MAIN { old } else { crate::ui_thread::reserve() }, window)
                }).collect();
                app.backend.sessions().adopt(Snapshot { version:1, windows:windows.clone() });
                for (owner, window) in &windows {
                    if *owner == crate::ui_thread::MAIN { continue; }
                    if let Err(error) = app.restore_native_window(*owner, window.as_ref().clone()) {
                        app.push_warning(format!("A saved window could not open: {error}. Its drafts remain saved for the next launch."));
                    }
                }
                app.restore_saved_window(windows.get(&crate::ui_thread::MAIN).map(|w|w.as_ref().clone()).unwrap_or_default());
                app.backend.sessions().enable(home);
            });
        });
    }
    fn restore_saved_window(&mut self, window: Window) {
        if let Some([width, height]) = window.size {
            self.window
                .window()
                .set_size(slint::LogicalSize::new(width, height));
        }
        if let Some([x, y]) = window.position {
            // A removed monitor must not strand the recovered window offscreen.
            use slint::winit_030::WinitWindowAccessor;
            self.window.window().with_winit_window(|native| {
                let reachable = native.available_monitors().any(|monitor| {
                    let p = monitor.position();
                    let s = monitor.size();
                    (x as i64) >= p.x as i64
                        && (x as i64) < p.x as i64 + s.width as i64
                        && (y as i64) >= p.y as i64
                        && (y as i64) < p.y as i64 + s.height as i64
                });
                if reachable {
                    native.set_outer_position(slint::winit_030::winit::dpi::PhysicalPosition::new(
                        x, y,
                    ));
                }
            });
        }
        let mut active = None;
        for (position, chat) in window.chats.into_iter().enumerate() {
            if chat
                .thread_id
                .as_ref()
                .is_some_and(|id| !id.starts_with("goose:"))
            {
                continue;
            }
            let existing = chat
                .thread_id
                .as_ref()
                .and_then(|id| self.tab_index_for_thread(id));
            let restored = chat.restore();
            let index = if let Some(index) = existing {
                let thread = self.thread_tab_mut(index).unwrap();
                if thread.composer.text.is_empty() {
                    thread.composer = restored.composer;
                }
                thread.recovered_sends.extend(restored.recovered_sends);
                thread.recovery_paused = true;
                index
            } else {
                self.push_tab(TabKind::Thread(Box::new(restored)), true)
            };
            if window.active == Some(position) {
                active = Some(Active::Chat(self.tabs[index].id));
            }
        }
        self.session_restore_active = window.active_document.map(Active::Rally).or(active);
        self.session_restore_ready = true;
        self.finish_restored_selection();
        self.save_open_chats();
        if self.backend.is_ready() {
            self.reattach_recovered_chats();
        }
        self.composer_refresh();
    }
    pub(crate) fn finish_restored_selection(&mut self) {
        if !self.session_restore_ready || !self.rally_recovery_ready() {
            return;
        }
        match self.session_restore_active.take() {
            Some(Active::Chat(id)) => {
                if let Some(index) = self.tab_index_by_id(id) {
                    self.activate_tab(index);
                }
            }
            Some(Active::Rally(id)) => {
                self.rally_activate_recovered_document(&id);
            }
            None => {}
        }
        self.save_open_chats();
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
    fn v76_snapshot_preserves_groups_geometry_and_unique_ownership() {
        let mut snapshot = Snapshot::default();
        for owner in 1..=2 {
            let mut thread = ThreadTab::new(PathBuf::from("fixture"));
            thread.thread_id = Some(format!("goose:window-{owner}"));
            thread.composer.text = format!("draft {owner}");
            snapshot.windows.insert(
                owner,
                Arc::new(Window {
                    chats: vec![Chat::capture(&thread, None)],
                    active: Some(0),
                    size: Some([1100., 800.]),
                    position: Some([100, 200]),
                    ..Default::default()
                }),
            );
        }
        let temp = tempfile::tempdir().unwrap();
        write(temp.path(), &snapshot).unwrap();
        let saved = read(temp.path()).unwrap();
        assert_eq!(saved.windows.len(), 2);
        assert_eq!(saved.windows[&2].chats[0].draft.text, "draft 2");
        assert_eq!(saved.windows[&2].size, Some([1100., 800.]));
        assert_eq!(saved.windows[&2].position, Some([100, 200]));
        Arc::make_mut(snapshot.windows.get_mut(&2).unwrap()).chats[0].thread_id =
            Some("goose:window-1".into());
        assert!(validate(&snapshot).is_err());
        let legacy: Window = serde_json::from_value(json!({"chats":[],"active":null})).unwrap();
        assert!(legacy.size.is_none() && legacy.rally.is_none());
    }
    #[tokio::test]
    async fn v76_preclaim_and_writer_preserve_child_documents_and_failed_window() {
        use crate::rally::{
            store::{Document, Preferences, Store},
            windows::WindowHub,
        };
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().join("rally"),
        };
        let parent = WindowHub::new(&store).unwrap();
        let prefs = Preferences {
            documents: vec![Document {
                identity: "child-document".into(),
                assistant_draft: "retained Rally draft".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let (id, child) = parent.prepare_child(&prefs).unwrap();
        std::fs::remove_file(child.directory.join("launching")).unwrap();
        assert_eq!(parent.retired_documents().unwrap().len(), 1);
        let mut snapshot = Snapshot::default();
        let mut thread = ThreadTab::new(PathBuf::from("child"));
        thread.composer.text = "child chat draft".into();
        snapshot.windows.insert(
            2,
            Arc::new(Window {
                chats: vec![Chat::capture(&thread, None)],
                rally: Some(RallyWindow {
                    root: parent.root.clone(),
                    id,
                }),
                active_document: Some("child-document".into()),
                ..Default::default()
            }),
        );
        let prepared = prepare_windows(snapshot).unwrap();
        assert!(parent.retired_documents().unwrap().is_empty());
        assert_eq!(
            child.load().unwrap().documents[0].assistant_draft,
            "retained Rally draft"
        );
        let writer = Writer::new(&tokio::runtime::Handle::current());
        writer.adopt(prepared);
        writer.save(1, Window::default()); // The child could not be created.
        writer.enable(temp.path().into());
        writer.flush().await.unwrap();
        let saved = read(temp.path()).unwrap();
        assert_eq!(saved.windows[&2].chats[0].draft.text, "child chat draft");
        assert_eq!(
            saved.windows[&2].active_document.as_deref(),
            Some("child-document")
        );
        let mut duplicate = saved.clone();
        duplicate.windows.insert(3, saved.windows[&2].clone());
        assert!(validate(&duplicate).is_err());
    }
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
    #[test]
    fn v75_side_snapshot_retains_identity_draft_and_error_without_replay() {
        let mut thread = ThreadTab::new(PathBuf::from("isolated"));
        thread.thread_id = Some("goose:side".into());
        thread.extras.side_parent = Some(crate::threads::SideParent {
            thread_id: "goose:parent".into(),
            title: "Parent".into(),
        });
        thread.last_error = Some("Interrupted setup".into());
        thread.composer.text = "Unsent side question".into();
        thread.xtab_enabled = false;
        let bytes = serde_json::to_vec(&Chat::capture(&thread, None)).unwrap();
        let restored = serde_json::from_slice::<Chat>(&bytes).unwrap().restore();
        assert_eq!(restored.extras.side_parent, thread.extras.side_parent);
        assert_eq!(restored.last_error, thread.last_error);
        assert_eq!(restored.composer.text, thread.composer.text);
        assert!(restored.recovery_paused && restored.pending_inputs.is_empty());
        assert!(!restored.xtab_enabled);
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
                    ..Default::default()
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
