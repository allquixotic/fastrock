//! Bounded historical viewports over Goose's indexed compressed history.
//! The backend continues collecting live events while the user reads a snapshot.
use super::history::BrowseRange;
use super::{HistoryStart, ThreadHistoryMode};
use crate::app::{AppController, TabId};
use crate::ui::TranscriptState;
use codex_app_server_protocol::{ThreadItemEntry, Turn};
use serde::Deserialize;
use slint::ComponentHandle;
use std::time::Instant;

#[derive(Deserialize)]
struct Window {
    start: usize,
    total: usize,
    items: Vec<ThreadItemEntry>,
    turns: Vec<Turn>,
}
impl Window {
    fn range(&self) -> Result<BrowseRange, String> {
        let end = self.start.saturating_add(self.items.len());
        if self.items.len() > 100
            || self.turns.len() > 100
            || self.start > self.total
            || end > self.total
        {
            return Err("Invalid bounded history response".into());
        }
        Ok(BrowseRange {
            start: self.start,
            end,
            total: self.total,
        })
    }
}

impl AppController {
    pub(super) fn history_navigation_bind(&mut self) {
        self.window.global::<TranscriptState>().on_history_action(
            crate::ui_thread::scoped_callback!(|action| {
                let action = action.to_string();
                crate::ui_thread::with_app(move |app| app.history_navigation_action(&action));
            }),
        );
    }

    fn history_navigation_action(&mut self, action: &str) {
        let Some(index) = self.active_thread_index() else {
            return;
        };
        if action == "latest" {
            self.history_return_to_latest(index);
            let state = self.window.global::<TranscriptState>();
            state.set_follow_tail(true);
            state.set_scroll_request(state.get_scroll_request().wrapping_add(1));
            return;
        }
        let range = self
            .thread_tab(index)
            .and_then(|thread| thread.transcript.history.browsing.as_ref());
        let start = match action {
            "tail" => None,
            "first" => Some(0),
            "previous" => range.map(|range| range.start.saturating_sub(100)),
            "next" => range.map(|range| range.end),
            "jump" => match self
                .window
                .global::<TranscriptState>()
                .get_history_position()
                .trim()
                .parse::<usize>()
            {
                Ok(position) if position > 0 => Some(position - 1),
                _ => {
                    self.toast("Enter a history position starting at 1");
                    return;
                }
            },
            _ => return,
        };
        self.history_navigate(index, start);
    }

    pub(super) fn history_navigation_refresh(&self) {
        let state = self.window.global::<TranscriptState>();
        let thread = self
            .active_thread_index()
            .and_then(|index| self.thread_tab(index));
        let history = thread.map(|thread| &thread.transcript.history);
        let range = history.and_then(|history| history.browsing.as_ref());
        state.set_history_available(thread.is_some_and(|thread| {
            thread
                .thread_id
                .as_ref()
                .is_some_and(|id| id.starts_with("goose:"))
        }));
        state.set_history_browsing(range.is_some());
        state.set_history_loading(history.is_some_and(|history| history.browse_loading));
        state.set_history_previous(range.is_some_and(|range| range.start > 0));
        state.set_history_next(range.is_some_and(|range| range.end < range.total));
        state.set_history_label(
            range
                .map(|range| {
                    format!(
                        "{}–{} of {}{}",
                        if range.total == 0 { 0 } else { range.start + 1 },
                        range.end,
                        range.total,
                        if history.is_some_and(|history| history.browse_activity) {
                            " · New activity"
                        } else {
                            ""
                        }
                    )
                })
                .unwrap_or_else(|| "Loading history…".into())
                .into(),
        );
    }

    pub(super) fn history_navigate(&mut self, index: usize, start: Option<usize>) {
        let Some(tab_id) = self.tabs.get(index).map(|tab| tab.id) else {
            return;
        };
        let Some(thread) = self.thread_tab_mut(index) else {
            return;
        };
        let Some(id) = thread
            .thread_id
            .clone()
            .filter(|id| id.starts_with("goose:"))
        else {
            return;
        };
        let history = &mut thread.transcript.history;
        if history.loading_initial && !history.browse_loading {
            return;
        }
        history.generation += 1;
        let generation = history.generation;
        history.loading_initial = true;
        history.loading_older = false;
        history.browse_loading = true;
        let backend = self.backend.clone();
        let started = Instant::now();
        self.backend.spawn(async move {
            let result = backend
                .goose_request(
                    "fastrock/history/window",
                    serde_json::json!({"threadId":id,"start":start}),
                )
                .await
                .and_then(|value| {
                    serde_json::from_value::<Window>(value)
                        .map_err(|_| "Invalid history response".to_string())
                });
            crate::ui_thread::post(move |app| {
                app.history_apply_window(tab_id, generation, result, started)
            });
        });
        if self.active == Some(index) {
            self.transcript_refresh_state();
        }
    }

    fn history_apply_window(
        &mut self,
        tab_id: TabId,
        generation: u64,
        result: Result<Window, String>,
        started: Instant,
    ) {
        let Some(index) = self.tab_index_by_id(tab_id) else {
            return;
        };
        let Some(thread) = self.thread_tab(index) else {
            return;
        };
        if thread.transcript.history.generation != generation {
            return;
        }
        let result = result.and_then(|window| window.range().map(|range| (window, range)));
        let (window, range) = match result {
            Ok(result) => result,
            Err(error) => {
                if let Some(thread) = self.thread_tab_mut(index) {
                    thread.transcript.history.loading_initial = false;
                    thread.transcript.history.browse_loading = false;
                }
                self.toast(format!("Could not browse history: {error}"));
                self.transcript_refresh_state();
                return;
            }
        };
        let position = (range.start + 1).to_string();
        if self.active == Some(index) {
            self.selection_clear();
        }
        self.with_transcript(index, |transcript, ctx, _| {
            transcript.prepare_goose_reload(ctx);
            let history = &mut transcript.history;
            history.loading_initial = false;
            history.loading_older = false;
            history.browse_loading = false;
            history.browse_activity = false;
            history.browsing = Some(range);
            history.turns = window
                .turns
                .into_iter()
                .map(|turn| (turn.id.clone(), turn))
                .collect();
            history.error = None;
            let turns = history.turns.clone();
            transcript.prepend_history(
                window
                    .items
                    .into_iter()
                    .map(|entry| (entry.turn_id, entry.item))
                    .collect(),
                &turns,
                ctx,
            );
            transcript.history.browse_fetch_ms = started.elapsed().as_secs_f64() * 1000.0;
        });
        if self.active == Some(index) {
            let state = self.window.global::<TranscriptState>();
            state.set_history_position(position.into());
            state.set_follow_tail(false);
            state.set_scroll_top_request(state.get_scroll_top_request().wrapping_add(1));
            self.transcript_refresh_state();
        }
    }

    pub(super) fn history_return_to_latest(&mut self, index: usize) {
        let Some(thread) = self.thread_tab(index) else {
            return;
        };
        let history = &thread.transcript.history;
        if history.browsing.is_none() && !history.browse_loading {
            return;
        }
        let Some(id) = thread.thread_id.clone() else {
            return;
        };
        if self.active == Some(index) {
            self.selection_clear();
        }
        // Clear first, then accept live notifications during the async tail
        // fetch. The ordinary loader deduplicates its snapshot against them.
        self.transcript_load_history(
            index,
            HistoryStart {
                thread_id: id,
                history_mode: ThreadHistoryMode::Paginated,
                turns: Vec::new(),
                turns_cursor: None,
                items_cursor: None,
            },
        );
    }

    pub(super) fn history_navigation_dump(&self, index: usize, path: &str) {
        let Some(thread) = self.thread_tab(index) else {
            return;
        };
        let history = &thread.transcript.history;
        let value = serde_json::json!({"browsing":history.browsing.is_some(), "loading":history.browse_loading,
            "start":history.browsing.as_ref().map(|r|r.start), "end":history.browsing.as_ref().map(|r|r.end),
            "total":history.browsing.as_ref().map(|r|r.total), "activity":history.browse_activity,
            "fetch_ms":history.browse_fetch_ms,"rows":thread.transcript.rows(),"bytes":thread.transcript.bytes()});
        let path = std::path::PathBuf::from(path);
        self.backend.spawn(async move {
            if let Ok(bytes) = serde_json::to_vec_pretty(&value) {
                let _ = tokio::fs::write(path, bytes).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v53_browsing_disables_older_autoload_and_reset_cancels_stale_results() {
        let mut history = super::super::history::HistoryState::default();
        history.reset("goose:fixture".into(), ThreadHistoryMode::Paginated);
        history.older = super::super::history::Older::Paged {
            cursor: None,
            include_from: None,
        };
        assert!(history.has_older());
        history.browse_loading = true;
        assert!(!history.has_older());
        history.browse_loading = false;
        history.browsing = Some(BrowseRange {
            start: 0,
            end: 100,
            total: 10000,
        });
        assert!(!history.has_older());
        let generation = history.generation;
        history.reset("goose:fixture".into(), ThreadHistoryMode::Paginated);
        assert!(history.browsing.is_none() && !history.browse_loading);
        assert_ne!(generation, history.generation);
        assert!(
            Window {
                start: 1,
                total: 0,
                items: Vec::new(),
                turns: Vec::new()
            }
            .range()
            .is_err()
        );
    }
}
