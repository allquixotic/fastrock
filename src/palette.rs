//! Native T3-style command/project/chat launcher; cached data only on the UI thread.
use crate::{
    app::AppController,
    ui::{AppState, CommandPaletteRow, CommandPaletteState},
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

const RESULT_LIMIT: usize = 100;
const CANDIDATE_LIMIT: usize = 5000;

#[derive(Clone)]
struct Item {
    kind: &'static str,
    target: String,
    title: String,
    detail: String,
    shortcut: String,
    enabled: bool,
}

/// Linear subsequence scoring; input and candidate lengths are bounded. Exact
/// substring matches rank before fuzzy matches, with stable source-order ties.
fn score(query: &str, title: &str, detail: &str) -> Option<usize> {
    let title = title.chars().take(512).collect::<String>().to_lowercase();
    let detail = detail.chars().take(512).collect::<String>().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    if let Some(at) = title.find(query) {
        return Some(at);
    }
    if detail.contains(query) {
        return Some(1024);
    }
    let mut wanted = query.chars();
    let mut next = wanted.next();
    let mut gap = 0;
    for ch in title.chars() {
        if Some(ch) == next {
            next = wanted.next();
            if next.is_none() {
                return Some(2048 + gap);
            }
        } else {
            gap += 1;
        }
    }
    None
}

fn filter(items: Vec<Item>, query: &str) -> Vec<Item> {
    let query = query.chars().take(128).collect::<String>().to_lowercase();
    let commands = query.trim_start().starts_with('>');
    let query = query.trim().trim_start_matches('>').trim();
    let mut ranked = items
        .into_iter()
        .take(CANDIDATE_LIMIT)
        .filter(|item| !commands || item.kind == "Command")
        .filter_map(|item| score(query, &item.title, &item.detail).map(|rank| (rank, item)))
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked
        .into_iter()
        .take(RESULT_LIMIT)
        .map(|(_, item)| item)
        .collect()
}

impl AppController {
    pub(crate) fn palette_bind(&mut self) {
        let state = self.window.global::<CommandPaletteState>();
        state.on_toggle_key(crate::ui_thread::scoped_callback!(
            |text, primary, shift, alt, secondary| {
                let press = crate::shortcuts::KeyPress {
                    text: text.to_string(),
                    primary,
                    shift,
                    alt,
                    secondary,
                };
                let mut handled = false;
                crate::ui_thread::with_app_now(|app| {
                    handled = app.keymap.lookup(&press) == Some("command-palette")
                });
                if handled {
                    crate::ui_thread::with_app(AppController::palette_close);
                }
                handled
            }
        ));
        state.on_search(crate::ui_thread::scoped_callback!(|| {
            crate::ui_thread::with_app(AppController::palette_refresh);
        }));
        state.on_execute(crate::ui_thread::scoped_callback!(|index| {
            crate::ui_thread::with_app(move |app| app.palette_execute(index));
        }));
        state.on_close(crate::ui_thread::scoped_callback!(|| {
            crate::ui_thread::with_app(AppController::palette_close);
        }));
    }
    pub(crate) fn palette_toggle(&mut self) {
        let state = self.window.global::<CommandPaletteState>();
        if state.get_open() {
            self.palette_close();
            return;
        }
        if self.window.global::<AppState>().get_dialog_open() {
            return;
        }
        state.set_query("".into());
        self.palette_refresh();
        self.window.global::<CommandPaletteState>().set_open(true);
    }
    fn palette_close(&mut self) {
        let state = self.window.global::<CommandPaletteState>();
        state.set_open(false);
        state.set_rows(ModelRc::default());
        self.restore_focus();
    }
    fn palette_refresh(&mut self) {
        let active = self.active_thread_index().and_then(|i| self.thread_tab(i));
        let idle_thread = active.is_some_and(|t| !t.is_busy());
        let mut items = Vec::new();
        for (target, title, detail, enabled) in [
            (
                "new-tab",
                "New conversation",
                "Choose a project or folder",
                true,
            ),
            (
                "search",
                "Search conversations",
                "Search titles and current summaries",
                true,
            ),
            ("add-project", "Add project", "Register a repository", true),
            (
                "workspaces",
                "Browse workspace files",
                "Files, changes and explicit integration",
                true,
            ),
            ("open-file", "Open file", "Native file viewer", true),
            (
                "rally",
                "Open Rally",
                "Independent Rally documents and boards",
                true,
            ),
            (
                "schedules",
                "Schedules and webhooks",
                "Manage automated conversations",
                true,
            ),
            (
                "providers",
                "Provider profiles",
                "Goose models, endpoints and credentials",
                true,
            ),
            ("settings", "Open settings", "Application preferences", true),
            (
                "keyboard",
                "Keyboard shortcuts",
                "Customize shortcut bindings",
                true,
            ),
            (
                "toggle-sidebar",
                "Toggle sidebar",
                "Projects and conversations",
                true,
            ),
            (
                "toggle-info",
                "Toggle info pane",
                "Conversation details",
                active.is_some(),
            ),
            (
                "fork",
                "Fork conversation",
                "Create an isolated workspace",
                idle_thread,
            ),
            (
                "compact",
                "Compact conversation",
                "Compact model context",
                idle_thread,
            ),
            (
                "export",
                "Export conversation",
                "Save complete history as Markdown",
                active.is_some(),
            ),
            (
                "escape",
                "Stop current response",
                "Cancel this conversation and descendants",
                active.is_some_and(|t| t.is_busy()),
            ),
            (
                "theme-system",
                "Theme: System",
                "Follow the system appearance",
                true,
            ),
            ("theme-light", "Theme: Light", "Use light appearance", true),
            ("theme-dark", "Theme: Dark", "Use dark appearance", true),
        ] {
            let binding = self
                .prefs
                .keymap
                .get(target)
                .map(String::as_str)
                .or_else(|| crate::shortcuts::default_binding(target))
                .unwrap_or("");
            items.push(Item {
                kind: "Command",
                target: target.into(),
                title: title.into(),
                detail: detail.into(),
                shortcut: if binding.is_empty() {
                    String::new()
                } else {
                    crate::shortcuts::display_binding(binding)
                },
                enabled,
            });
        }
        for project in self.sidebar.projects().iter().take(CANDIDATE_LIMIT / 2) {
            items.push(Item {
                kind: "Project",
                target: project.root.to_string_lossy().into_owned(),
                title: crate::app::folder_label(&project.root),
                detail: project.root.display().to_string(),
                shortcut: String::new(),
                enabled: true,
            });
        }
        let mut seen = std::collections::HashSet::new();
        for thread in self
            .tabs
            .iter()
            .filter_map(|tab| tab.thread())
            .take(CANDIDATE_LIMIT / 2)
        {
            let Some(id) = thread.thread_id.as_ref() else {
                continue;
            };
            if !seen.insert(id.clone()) {
                continue;
            }
            items.push(Item {
                kind: "Conversation",
                target: id.clone(),
                title: thread.title(),
                detail: thread.cwd.display().to_string(),
                shortcut: String::new(),
                enabled: true,
            });
        }
        for thread in self
            .sidebar
            .recent_threads()
            .iter()
            .take(CANDIDATE_LIMIT / 2)
        {
            if !seen.insert(thread.id.clone()) {
                continue;
            }
            items.push(Item {
                kind: "Conversation",
                target: thread.id.clone(),
                title: thread.title.clone(),
                detail: thread.group_root().display().to_string(),
                shortcut: String::new(),
                enabled: true,
            });
        }
        let state = self.window.global::<CommandPaletteState>();
        let rows = filter(items, state.get_query().as_str())
            .into_iter()
            .map(|item| CommandPaletteRow {
                kind: item.kind.into(),
                target: item.target.into(),
                title: item.title.into(),
                detail: item.detail.into(),
                shortcut: item.shortcut.into(),
                enabled: item.enabled,
            })
            .collect::<Vec<_>>();
        state.set_rows(ModelRc::new(VecModel::from(rows)));
        state.set_selected(0);
    }
    fn palette_execute(&mut self, index: i32) {
        let state = self.window.global::<CommandPaletteState>();
        if !state.get_open() {
            return;
        }
        let Some(row) = state
            .get_rows()
            .row_data(index as usize)
            .filter(|r| r.enabled)
        else {
            return;
        };
        self.palette_close();
        match row.kind.as_str() {
            "Conversation" => self.open_thread(row.target.to_string(), None),
            "Project" => self.start_thread_checked(std::path::PathBuf::from(row.target.as_str())),
            "Command" => match row.target.as_str() {
                "rally" => self.rally_open_project(),
                "search" => self.sidebar_focus_search(),
                "add-project" => self.newtab_pick_folder(),
                "workspaces" => self
                    .window
                    .global::<AppState>()
                    .invoke_open_virtual_workspaces(),
                "schedules" => self.open_settings(Some("schedules")),
                "providers" => self.open_settings(Some("goose")),
                "theme-system" | "theme-light" | "theme-dark" => {
                    self.prefs.theme = match row.target.as_str() {
                        "theme-light" => crate::prefs::ThemeChoice::Light,
                        "theme-dark" => crate::prefs::ThemeChoice::Dark,
                        _ => crate::prefs::ThemeChoice::System,
                    };
                    self.apply_theme();
                    self.save_prefs();
                }
                target => self.menu_action(target),
            },
            _ => {}
        }
    }
    pub(crate) fn palette_automation(&mut self, args: &[String]) {
        let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
        let state = self.window.global::<CommandPaletteState>();
        match arg(0) {
            "query" => {
                state.set_query(arg(1).into());
                state.invoke_search();
            }
            "dump" => {
                let rows = state.get_rows().iter().map(|r| serde_json::json!({"kind":r.kind.as_str(),"title":r.title.as_str(),"target":r.target.as_str(),"enabled":r.enabled})).collect::<Vec<_>>();
                let value = serde_json::json!({"open":state.get_open(),"selected":state.get_selected(),"query":state.get_query().as_str(),"rows":rows, "busy":self.active_thread_index().and_then(|i| self.thread_tab(i)).is_some_and(|t| t.is_busy()), "draft":self.window.global::<crate::ui::ComposerState>().get_text().as_str()});
                let path = std::path::PathBuf::from(arg(1));
                self.backend.spawn(async move {
                    let _ = tokio::fs::write(path, value.to_string()).await;
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v60_palette_bounds_results_and_ranks_substrings_before_fuzzy() {
        assert!(score("prv", "Provider profiles", "").is_some());
        assert!(
            score("PROVIDER".to_lowercase().as_str(), "Provider profiles", "").unwrap()
                < score("prv", "Provider profiles", "").unwrap()
        );
        assert_eq!(score("absent", "Provider profiles", ""), None);
        let item = Item {
            kind: "Conversation",
            target: "thread".into(),
            title: "Thread".into(),
            detail: String::new(),
            shortcut: String::new(),
            enabled: true,
        };
        assert_eq!(
            filter(vec![item.clone(); CANDIDATE_LIMIT + 1], "").len(),
            RESULT_LIMIT
        );
        assert!(filter(vec![item], "> thread").is_empty());
        assert!(score("λ", "Λ workspace", "").is_some());
    }
}
