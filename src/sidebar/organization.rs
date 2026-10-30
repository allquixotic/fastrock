//! Native T3-style groups over a shared durable metadata snapshot.
use super::organization_store::{Action, Mutation, State};
use super::*;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Controller {
    state: Arc<State>,
    loaded: bool,
    busy: usize,
    error: String,
    project: String,
    choices: Vec<String>,
    labels: Rc<VecModel<slint::SharedString>>,
    selecting: bool,
    selected: HashSet<String>,
    wake_timer: slint::Timer,
    retained: Vec<ThreadSummary>,
    requested: HashSet<String>,
    removed: HashSet<String>,
    dragged: Option<String>,
    auto_deadline: Option<i64>,
}
pub(super) fn working(status: SidebarThreadStatus) -> bool {
    matches!(
        status,
        SidebarThreadStatus::Running | SidebarThreadStatus::Starting
    )
}
pub(super) fn attention(status: SidebarThreadStatus) -> bool {
    matches!(
        status,
        SidebarThreadStatus::Waiting | SidebarThreadStatus::Error
    )
}
fn section_title(section: &str) -> &'static str {
    match section {
        "pinned" => "Pinned",
        "working" => "Working",
        "snoozed" => "Snoozed",
        "settled" => "Settled",
        _ => "Active",
    }
}
fn snooze_time(text: &str) -> Result<i64, String> {
    let date = chrono::NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M")
        .map_err(|_| "Use local date and time: YYYY-MM-DD HH:MM")?;
    use chrono::TimeZone;
    chrono::Local
        .from_local_datetime(&date)
        .single()
        .map(|v| v.timestamp())
        .ok_or_else(|| "That local time is missing or ambiguous; choose another time".into())
}
impl AppController {
    pub(super) fn organization_filter_project(&mut self, project: Option<String>) {
        if let Some(project) = project {
            self.sidebar.organization.project = project;
            self.sidebar.organization.selected.clear();
            self.sidebar_render();
        } else {
            self.toast("Register this repository using Project settings first.");
        }
    }
    pub(super) fn organization_bind(&mut self) {
        let ui = self.window.global::<SidebarState>();
        ui.set_project_choices(ModelRc::from(self.sidebar.organization.labels.clone()));
        ui.on_policy_changed(crate::ui_thread::scoped_callback!(|key, value| {
            let key = key.to_string();
            crate::ui_thread::with_app(move |app| {
                app.organization_request(Some(Mutation::Policy { key, value }))
            });
        }));
        ui.on_drag_begin(crate::ui_thread::scoped_callback!(|id| {
            let id = id.to_string();
            crate::ui_thread::with_app(move |app| app.organization_drag_begin(id));
        }));
        ui.on_drag_move(crate::ui_thread::scoped_callback!(|x, y| {
            crate::ui_thread::with_app(move |app| app.organization_drag_move(x, y));
        }));
        ui.on_drag_end(crate::ui_thread::scoped_callback!(|x, y, commit| {
            crate::ui_thread::with_app(move |app| app.organization_drag_end(x, y, commit));
        }));
        ui.on_organization_action(crate::ui_thread::scoped_callback!(|action| {
            let action = action.to_string();
            crate::ui_thread::with_app(move |app| app.organization_action(&action));
        }));
        ui.on_project_selected(crate::ui_thread::scoped_callback!(|index| {
            crate::ui_thread::with_app(move |app| {
                if let Some(id) = app.sidebar.organization.choices.get(index.max(0) as usize) {
                    app.sidebar.organization.project = id.clone();
                    app.sidebar.organization.selected.clear();
                    app.sidebar_render();
                }
            });
        }));
        ui.on_section_toggle(crate::ui_thread::scoped_callback!(|id| {
            let section = id.to_string();
            crate::ui_thread::with_app(move |app| {
                let collapsed = !app.sidebar.organization.state.collapsed.contains(&section);
                app.organization_request(Some(Mutation::Collapse { section, collapsed }));
            });
        }));
        self.organization_request(None);
    }
    pub(super) fn organization_request(&mut self, mutation: Option<Mutation>) {
        let Some(home) = self.codex_home.clone() else {
            return;
        };
        if mutation.is_some()
            && !matches!(&mutation, Some(Mutation::Forget(_)))
            && !self.sidebar.organization.loaded
        {
            self.toast("Sidebar organization is still loading or unavailable");
            return;
        }
        self.sidebar.organization.busy += 1;
        self.organization_render();
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = backend.organization().request(home, mutation).await;
            if let Ok(state) = &result {
                let state = state.clone();
                crate::ui_thread::post_all(move |app| {
                    if state.revision >= app.sidebar.organization.state.revision {
                        app.sidebar.organization.state = state.clone();
                        app.sidebar.organization.loaded = true;
                        app.sidebar.organization.error.clear();
                        app.organization_fetch_retained();
                        app.sidebar_render();
                        app.refresh_tabs();
                    }
                });
            }
            crate::ui_thread::post(move |app| {
                app.sidebar.organization.busy = app.sidebar.organization.busy.saturating_sub(1);
                if let Err(error) = result {
                    app.sidebar.organization.error = error;
                }
                app.organization_render();
            });
        });
    }
    pub(super) fn organization_rows(
        &self,
        threads: &[ThreadSummary],
        marks: &HashMap<String, ThreadMark>,
        more: bool,
        now: i64,
    ) -> Vec<RowSpec> {
        let org = &self.sidebar.organization;
        let threads: Vec<_> = threads
            .iter()
            .filter(|t| org.project.is_empty() || t.project_id.as_deref() == Some(&org.project))
            .cloned()
            .collect();
        let projects: Vec<_> = self
            .sidebar
            .projects
            .iter()
            .filter(|p| org.project.is_empty() || org.project == p.id)
            .cloned()
            .collect();
        if self.sidebar.archived {
            return projects::rows(&threads, &projects, &self.sidebar.collapsed, more, false);
        }
        let order: HashMap<_, _> = org
            .state
            .order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        let mut rows = vec![];
        for section in ["pinned", "active", "working", "snoozed", "settled"] {
            let subset: Vec<_> = threads
                .iter()
                .filter(|t| {
                    org.state.section_at(
                        &t.id,
                        marks.get(&t.id).is_some_and(|m| working(m.status)),
                        marks.get(&t.id).is_some_and(|m| attention(m.status)),
                        t.updated_at,
                        now,
                    ) == section
                })
                .cloned()
                .collect();
            let empty_collapsed = HashSet::new();
            let mut contents = projects::rows(
                &subset,
                &projects,
                if org.state.policy.group_projects {
                    &self.sidebar.collapsed
                } else {
                    &empty_collapsed
                },
                false,
                section == "active" && self.sidebar.is_unfiltered(),
            );
            if !org.state.policy.group_projects {
                contents.retain(|row| !matches!(row, RowSpec::Folder { .. }));
            }
            if contents.is_empty() {
                continue;
            }
            // Keep repository groups contiguous while applying manual order within them.
            let mut start = 0;
            while start < contents.len() {
                if !matches!(contents[start], RowSpec::Thread { .. }) {
                    start += 1;
                    continue;
                }
                let end = (start..contents.len())
                    .find(|&i| !matches!(contents[i], RowSpec::Thread { .. }))
                    .unwrap_or(contents.len());
                contents[start..end].sort_by_key(|row| {
                    let RowSpec::Thread { id, updated_at, .. } = row else {
                        unreachable!()
                    };
                    match section {
                        "snoozed" => (
                            0,
                            org.state
                                .entries
                                .get(id)
                                .and_then(|e| e.snoozed_until)
                                .unwrap_or(i64::MAX),
                        ),
                        "settled" => (
                            0,
                            -org.state
                                .entries
                                .get(id)
                                .and_then(|e| e.settled_at)
                                .unwrap_or(*updated_at),
                        ),
                        "working" => (0, -*updated_at),
                        _ => (
                            order.get(id.as_str()).copied().unwrap_or(usize::MAX),
                            -*updated_at,
                        ),
                    }
                });
                start = end;
            }
            // An existing project represented in another group need not appear as an empty active folder.
            if section == "active" {
                contents.retain(|row| !matches!(row,RowSpec::Folder{path,count:0,..} if threads.iter().any(|t|t.group_root()==path)));
                if contents.is_empty() {
                    continue;
                }
            }
            let collapsed =
                org.state.collapsed.contains(section) && self.sidebar.search_term.is_empty();
            rows.push(RowSpec::Section {
                id: section.into(),
                title: section_title(section).into(),
                count: subset.len(),
                collapsed,
            });
            if !collapsed {
                rows.extend(contents);
            }
        }
        if more {
            rows.push(RowSpec::More);
        }
        rows
    }

    pub(super) fn organization_retained(&self) -> Vec<ThreadSummary> {
        if self.sidebar.archived {
            return vec![];
        }
        let query = self.sidebar.search_term.to_lowercase();
        self.sidebar
            .organization
            .retained
            .iter()
            .filter(|t| query.is_empty() || t.title.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    fn organization_fetch_retained(&mut self) {
        let org = &mut self.sidebar.organization;
        let ids: Vec<_> = org
            .state
            .entries
            .iter()
            .filter(|(_, e)| e.pinned || e.snoozed_until.is_some_and(|at| at > unix_now()))
            .map(|(id, _)| id.clone())
            .filter(|id| org.requested.insert(id.clone()))
            .collect();
        if ids.is_empty() {
            return;
        }
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            for chunk in ids.chunks(128) {
                let result = backend
                    .goose_request("fastrock/thread/metadata", serde_json::json!({"ids":chunk}))
                    .await;
                let Ok(value) = result else {
                    continue;
                };
                let rows: Vec<Thread> =
                    serde_json::from_value(value["data"].clone()).unwrap_or_default();
                crate::ui_thread::post(move |app| {
                    let rows = rows
                        .iter()
                        .filter(|t| {
                            !t.ephemeral
                                && t.parent_thread_id.is_none()
                                && !app.sidebar.organization.removed.contains(&t.id)
                        })
                        .map(ThreadSummary::from_thread)
                        .collect();
                    merge_page(&mut app.sidebar.organization.retained, rows);
                    app.sidebar_render();
                });
            }
        });
    }

    pub(super) fn organization_deselect(&mut self, id: &str) {
        self.sidebar.organization.selected.remove(id);
    }
    pub(super) fn organization_forget(&mut self, id: &str) -> bool {
        self.sidebar.organization.removed.insert(id.into());
        let before = self.sidebar.organization.retained.len();
        self.sidebar.organization.retained.retain(|t| t.id != id);
        before != self.sidebar.organization.retained.len()
    }

    pub(super) fn organization_restore(&mut self, id: &str) {
        self.sidebar.organization.removed.remove(id);
        self.sidebar.organization.requested.remove(id);
        self.organization_fetch_retained();
    }
    pub(super) fn organization_set_title(&mut self, id: &str, name: &str) {
        for thread in &mut self.sidebar.organization.retained {
            if thread.id == id {
                thread.title = name.into();
            }
        }
    }
    pub(super) fn organization_observe_activity(
        &mut self,
        threads: &[ThreadSummary],
        marks: &HashMap<String, ThreadMark>,
        now: i64,
    ) {
        self.sidebar.organization.auto_deadline = threads
            .iter()
            .filter(|t| {
                !marks
                    .get(&t.id)
                    .is_some_and(|m| working(m.status) || attention(m.status))
            })
            .filter_map(|t| {
                self.sidebar
                    .organization
                    .state
                    .auto_settle_at(&t.id, t.updated_at)
            })
            .filter(|at| *at > now)
            .min();
    }
    pub(super) fn organization_decorate(&self, row: &mut SidebarRow, activity: i64, now: i64) {
        let org = &self.sidebar.organization;
        row.section = org
            .state
            .section_at(
                row.id.as_str(),
                working(row.status),
                attention(row.status),
                activity,
                now,
            )
            .into();
        row.auto_settle_enabled = !org
            .state
            .entries
            .get(row.id.as_str())
            .is_some_and(|e| e.auto_settle_disabled);
        row.pinned = org
            .state
            .entries
            .get(row.id.as_str())
            .is_some_and(|e| e.pinned);
        row.checked = org.selected.contains(row.id.as_str());
        row.regenerating_title = self.sidebar.titles.pending.contains(row.id.as_str());
        if row.section == "snoozed" {
            if let Some(until) = org
                .state
                .entries
                .get(row.id.as_str())
                .and_then(|e| e.snoozed_until)
                .and_then(|v| chrono::DateTime::from_timestamp(v, 0))
            {
                let label = until
                    .with_timezone(&chrono::Local)
                    .format("%a %H:%M")
                    .to_string();
                row.detail = label.clone().into();
                row.tooltip = format!("Snoozed until {label}\n{}", row.tooltip).into();
            }
        }
    }
    pub(super) fn organization_render(&mut self) {
        let visible: HashSet<_> = self.sidebar.row_index.keys().cloned().collect();
        let org = &mut self.sidebar.organization;
        org.selected.retain(|id| visible.contains(id));
        let mut labels = vec![slint::SharedString::from("All projects")];
        org.choices = vec![String::new()];
        for project in &self.sidebar.projects {
            org.choices.push(project.id.clone());
            labels.push(
                org.state
                    .project_names
                    .get(&project.id)
                    .unwrap_or(&project.name)
                    .as_str()
                    .into(),
            );
        }
        let ui = self.window.global::<SidebarState>();
        sync_model(&org.labels, labels);
        ui.set_project_index(
            org.choices
                .iter()
                .position(|id| id == &org.project)
                .unwrap_or(0) as i32,
        );
        ui.set_organization_ready(org.loaded && org.busy == 0);
        ui.set_organization_error(org.error.as_str().into());
        ui.set_selecting(org.selecting);
        ui.set_selection_count(org.selected.len() as i32);
        ui.set_regenerating_titles(
            self.sidebar
                .titles
                .pending
                .len()
                .max(usize::from(self.sidebar.titles.running)) as i32,
        );
        ui.set_title_generation_status(self.sidebar.titles.status.as_str().into());
        ui.set_group_projects(org.state.policy.group_projects);
        ui.set_working_section(org.state.policy.working_section);
        ui.set_auto_settle_days(org.state.policy.auto_settle_days.unwrap_or(0) as i32);
        let now = unix_now();
        org.wake_timer.stop();
        if let Some(at) = org
            .state
            .next_wake(now)
            .into_iter()
            .chain(org.auto_deadline)
            .min()
        {
            org.wake_timer.start(
                slint::TimerMode::SingleShot,
                Duration::from_secs(at.saturating_sub(now) as u64),
                crate::ui_thread::scoped_callback!(|| {
                    crate::ui_thread::with_app(AppController::sidebar_render);
                }),
            );
        }
    }
    pub(crate) fn sidebar_project_name<'a>(
        &'a self,
        project: &'a fastrock_workspaces::projects::Project,
    ) -> &'a str {
        self.sidebar
            .organization
            .state
            .project_names
            .get(&project.id)
            .unwrap_or(&project.name)
    }
    pub(super) fn organization_rename_project(&mut self, path: &Path) {
        let Some(project) = self
            .sidebar
            .projects
            .iter()
            .find(|p| dunce::simplified(&p.root) == path)
            .cloned()
        else {
            return;
        };
        let current = self.sidebar_project_name(&project).to_owned();
        self.show_dialog(
            DialogRequest::prompt("Project display name", current).accept_label("Save"),
            Box::new(move |app, value| {
                if let Some(name) = value {
                    app.organization_request(Some(Mutation::ProjectName {
                        id: project.id,
                        name,
                    }));
                }
            }),
        );
    }
    pub(crate) fn organization_wake_input(&mut self, id: &str) {
        if self
            .sidebar
            .organization
            .state
            .entries
            .get(id)
            .is_some_and(|e| e.settled_at.is_some() || e.snoozed_until.is_some())
        {
            self.organization_request(Some(Mutation::Threads {
                ids: vec![id.into()],
                action: Action::Wake,
            }));
        }
    }
    pub(super) fn organization_thread_action(&mut self, id: &str, action: &str) -> bool {
        match action {
            "regenerate-title" => self.sidebar_regenerate_titles(vec![id.into()]),
            "select" => {
                self.sidebar.organization.selecting = true;
                if !self.sidebar.organization.selected.remove(id) {
                    self.sidebar.organization.selected.insert(id.into());
                }
                self.sidebar_render();
            }
            "pin" | "unpin" | "settle" | "unsettle" | "wake" | "snooze-hour" | "snooze-day"
            | "snooze-custom" | "mark-unread" | "mark-read" | "auto-settle-on"
            | "auto-settle-off" => self.organization_apply(vec![id.into()], action),
            "move-up" | "move-down" => {
                let Some(row) = self
                    .sidebar
                    .row_index
                    .get(id)
                    .and_then(|i| self.sidebar.rows.row_data(*i))
                else {
                    return true;
                };
                let mut ids: Vec<String> = self
                    .sidebar
                    .rows
                    .iter()
                    .filter(|r| {
                        r.kind == SidebarRowKind::Thread
                            && r.section == row.section
                            && (!self.sidebar.organization.state.policy.group_projects
                                || r.project == row.project)
                    })
                    .map(|r| r.id.to_string())
                    .collect();
                if let Some(index) = ids.iter().position(|v| v == id) {
                    let other = if action == "move-up" {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(ids.len() - 1)
                    };
                    ids.swap(index, other);
                    self.organization_request(Some(Mutation::Order(ids)));
                }
            }
            _ => return false,
        }
        true
    }
    fn organization_action(&mut self, action: &str) {
        match action {
            "delete" => self
                .sidebar_delete_many(self.sidebar.organization.selected.iter().cloned().collect()),
            "regenerate-title" => self.sidebar_regenerate_titles(
                self.sidebar.organization.selected.iter().cloned().collect(),
            ),
            "cancel-titles" => self.sidebar_cancel_titles(),
            "clear-title-status" => {
                self.sidebar.titles.status.clear();
                self.organization_render();
            }
            "select" => {
                self.sidebar.organization.selecting = !self.sidebar.organization.selecting;
                self.sidebar.organization.selected.clear();
                self.sidebar_render();
            }
            "select-all" => {
                self.sidebar.organization.selected =
                    self.sidebar.row_index.keys().cloned().collect();
                self.sidebar_render();
            }
            "clear" => {
                self.sidebar.organization.selected.clear();
                self.sidebar_render();
            }
            "retry" => self.organization_request(None),
            _ => {
                let ids = self.sidebar.organization.selected.iter().cloned().collect();
                self.organization_apply(ids, action);
            }
        }
    }

    fn organization_drag_begin(&mut self, id: String) {
        if self.sidebar.archived
            || !self.sidebar.organization.loaded
            || self.sidebar.organization.busy > 0
            || !self.sidebar.row_index.contains_key(&id)
        {
            return;
        }
        self.sidebar.organization.dragged = Some(id);
        let ui = self.window.global::<SidebarState>();
        ui.set_dragging(true);
        ui.set_drop_target("".into());
        ui.set_tooltip_text("".into());
    }
    pub(crate) fn organization_cancel_drag(&mut self) -> bool {
        let cancelled = self.sidebar.organization.dragged.take().is_some();
        let ui = self.window.global::<SidebarState>();
        ui.set_dragging(false);
        ui.set_drop_target("".into());
        cancelled
    }
    fn organization_drop_target(&self, x: f32, y: f32) -> Option<(String, Option<String>)> {
        let ui = self.window.global::<SidebarState>();
        if !(0.0..self.prefs.sidebar_width).contains(&x) {
            return None;
        }
        let zone = ui.get_drop_zone_top();
        if y >= zone && y < zone + 30.0 {
            let index = ((x.max(12.0) - 12.0) / ((self.prefs.sidebar_width - 24.0) / 3.0))
                .floor()
                .clamp(0.0, 2.0) as usize;
            return Some((["pinned", "active", "settled"][index].into(), None));
        }
        let top = ui.get_list_top();
        if y < top || y > top + ui.get_viewport_height() {
            return None;
        }
        let offset = y - top + ui.get_viewport_offset();
        let mut cursor = 0.0;
        let mut section = String::new();
        for row in self.sidebar.rows.iter() {
            if row.kind == SidebarRowKind::Section {
                section = row.id.to_string();
            }
            let height = match row.kind {
                SidebarRowKind::Folder => 30.0,
                SidebarRowKind::Thread => 28.0,
                _ => 32.0,
            };
            if offset >= cursor && offset < cursor + height {
                if !["pinned", "active", "settled"].contains(&section.as_str()) {
                    return None;
                }
                return Some((
                    section,
                    (row.kind == SidebarRowKind::Thread).then(|| row.id.to_string()),
                ));
            }
            cursor += height;
        }
        None
    }
    fn organization_drag_move(&mut self, x: f32, y: f32) {
        if self.sidebar.organization.dragged.is_none() {
            return;
        }
        let target = self
            .organization_drop_target(x, y)
            .map(|(section, before)| before.unwrap_or(section))
            .unwrap_or_default();
        self.window
            .global::<SidebarState>()
            .set_drop_target(target.into());
    }
    fn organization_drag_end(&mut self, x: f32, y: f32, commit: bool) {
        let target = commit
            .then(|| self.organization_drop_target(x, y))
            .flatten();
        let dragged = self.sidebar.organization.dragged.clone();
        self.organization_cancel_drag();
        let (Some(id), Some((section, before))) = (dragged, target) else {
            return;
        };
        let Some(source) = self
            .sidebar
            .row_index
            .get(&id)
            .and_then(|i| self.sidebar.rows.row_data(*i))
        else {
            return;
        };
        if before.as_deref() == Some(id.as_str()) && source.section.as_str() == section {
            return;
        }
        let mut order: Vec<_> = self
            .sidebar
            .rows
            .iter()
            .filter(|r| {
                r.kind == SidebarRowKind::Thread
                    && r.section.as_str() == section
                    && (!self.sidebar.organization.state.policy.group_projects
                        || r.project == source.project)
                    && r.id.as_str() != id
            })
            .map(|r| r.id.to_string())
            .collect();
        let index = before
            .as_ref()
            .and_then(|before| order.iter().position(|id| id == before))
            .unwrap_or(order.len());
        order.insert(index, id.clone());
        self.organization_request(Some(Mutation::Place { id, section, order }));
    }
    fn organization_apply(&mut self, ids: Vec<String>, action: &str) {
        if ids.is_empty() {
            return;
        }
        if action.starts_with("snooze")
            && ids.iter().any(|id| {
                self.sidebar_marks()
                    .get(id)
                    .is_some_and(|m| m.status == SidebarThreadStatus::Waiting)
            })
        {
            self.toast("Answer pending questions or approvals before snoozing these conversations");
            return;
        }
        let mutation = match action {
            "pin" => Action::Pin(true),
            "unpin" => Action::Pin(false),
            "settle" => Action::Settle(true),
            "unsettle" => Action::Settle(false),
            "wake" => Action::Wake,
            "auto-settle-on" => Action::AutoSettle(true),
            "auto-settle-off" => Action::AutoSettle(false),
            "snooze-hour" => Action::Snooze(unix_now() + 3600),
            "snooze-day" => Action::Snooze(unix_now() + 86400),
            "snooze-custom" => {
                let default = (chrono::Local::now() + chrono::Duration::hours(1))
                    .format("%Y-%m-%d %H:%M")
                    .to_string();
                self.show_dialog(
                    DialogRequest::prompt("Snooze until (local YYYY-MM-DD HH:MM)", default)
                        .accept_label("Snooze"),
                    Box::new(move |app, value| {
                        if let Some(value) = value {
                            match snooze_time(&value) {
                                Ok(until) => app.organization_request(Some(Mutation::Threads {
                                    ids,
                                    action: Action::Snooze(until),
                                })),
                                Err(error) => app.toast(error),
                            }
                        }
                    }),
                );
                return;
            }
            "mark-unread" | "mark-read" => {
                self.activity_mark(&ids, action == "mark-unread");
                self.sidebar_render();
                return;
            }
            _ => return,
        };
        self.organization_request(Some(Mutation::Threads {
            ids,
            action: mutation,
        }));
    }
}
