//! Review target picker (`/review`, tab menu › Review changes, GUI.md §6):
//! uncommitted changes, a base branch, a commit, or custom instructions, like
//! the TUI's review popup (`tui/src/chatwidget/review_popups.rs`).
//!
//! Branches and commits come from the embedded repository owner. Source remains
//! compressed until the bounded review context is prepared off the UI thread.

use codex_app_server_protocol::{
    ClientRequest, ReviewDelivery, ReviewStartParams, ReviewStartResponse, ReviewTarget,
};
use fastrock_workspaces::review::{Catalog, Commit as CommitLogEntry};

use super::picker::PickerButton;
use super::picker::PickerEvent;
use super::picker::PickerFlow;
use super::picker::PickerInputView;
use super::picker::PickerListView;
use super::picker::PickerOutcome;
use super::picker::PickerRowView;
use super::picker::PickerView;
use crate::app::AppController;
use crate::app::TabId;
use crate::backend::BackendError;
use crate::transcript::NoticeKind;

/// The review picker of one thread tab.
pub(crate) struct ReviewPicker {
    tab_id: TabId,
    thread_id: String,
    step: ReviewStep,
    /// Text of the step's field: the list filter, or custom instructions.
    text: String,
}

#[derive(Clone, Debug)]
enum ReviewStep {
    Presets,
    Branches(GitList<BranchList>),
    Commits(GitList<Vec<CommitLogEntry>>),
    Custom,
}

#[derive(Clone, Debug)]
enum GitList<T> {
    Loading,
    Ready(T),
    Failed(String),
}

/// Local branches (default branch first) and the checked-out one.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct BranchList {
    pub(crate) branches: Vec<String>,
    /// `None` on a detached HEAD.
    pub(crate) current: Option<String>,
}

/// The first step's choices, in display order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReviewPreset {
    Uncommitted,
    BaseBranch,
    Commit,
    Custom,
}

const PRESETS: [(ReviewPreset, &str, &str); 4] = [
    (
        ReviewPreset::Uncommitted,
        "Uncommitted changes",
        "Staged, unstaged, and untracked files",
    ),
    (
        ReviewPreset::BaseBranch,
        "Against a base branch",
        "Everything the current branch changed since it split off, like a pull request",
    ),
    (
        ReviewPreset::Commit,
        "A commit",
        "The changes one recent commit introduced",
    ),
    (
        ReviewPreset::Custom,
        "Custom instructions",
        "Tell the reviewer what to look at",
    ),
];

/// Case-insensitive substring match of every word of `filter`.
fn matches_filter(haystack: &str, filter: &str) -> bool {
    let haystack = haystack.to_lowercase();
    filter
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

fn filtered_branches<'a>(list: &'a BranchList, filter: &str) -> Vec<&'a str> {
    list.branches
        .iter()
        .filter(|branch| matches_filter(branch, filter))
        .map(String::as_str)
        .collect()
}

fn filtered_commits<'a>(commits: &'a [CommitLogEntry], filter: &str) -> Vec<&'a CommitLogEntry> {
    commits
        .iter()
        .filter(|commit| matches_filter(&format!("{} {}", commit.subject, commit.sha), filter))
        .collect()
}

fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// Review target for custom instructions, or `None` when they are blank.
pub(crate) fn custom_target(text: &str) -> Option<ReviewTarget> {
    let instructions = text.trim();
    (!instructions.is_empty()).then(|| ReviewTarget::Custom {
        instructions: instructions.to_string(),
    })
}

/// One-line description of a review target for notices.
fn target_label(target: &ReviewTarget) -> String {
    match target {
        ReviewTarget::UncommittedChanges => "uncommitted changes".to_string(),
        ReviewTarget::BaseBranch { branch } => format!("changes against {branch}"),
        ReviewTarget::Commit { sha, title } => match title {
            Some(title) if !title.is_empty() => format!("commit {} ({title})", short_sha(sha)),
            _ => format!("commit {}", short_sha(sha)),
        },
        ReviewTarget::Custom { .. } => "custom instructions".to_string(),
    }
}

impl ReviewPicker {
    fn new(tab_id: TabId, thread_id: String) -> Self {
        Self {
            tab_id,
            thread_id,
            step: ReviewStep::Presets,
            text: String::new(),
        }
    }

    pub(crate) fn view(&self) -> PickerView {
        let filter_list = |placeholder: &str, rows: Vec<PickerRowView>, empty: String| {
            (
                Some(PickerInputView {
                    label: String::new(),
                    placeholder: placeholder.to_string(),
                    text: self.text.clone(),
                }),
                Some(PickerListView {
                    placeholder: if rows.is_empty() {
                        empty
                    } else {
                        String::new()
                    },
                    rows,
                    accept_needs_selection: true,
                    select_first: true,
                    ..PickerListView::default()
                }),
            )
        };
        match &self.step {
            ReviewStep::Presets => PickerView {
                title: "Review".to_string(),
                message: "What should the reviewer look at? The review runs in this thread and reports its findings here.".to_string(),
                list: Some(PickerListView {
                    rows: PRESETS
                        .iter()
                        .map(|(preset, title, detail)| {
                            let row = PickerRowView::new(*title, *detail);
                            if *preset == ReviewPreset::Uncommitted {
                                row
                            } else {
                                row.trailing("›")
                            }
                        })
                        .collect(),
                    click_activates: true,
                    ..PickerListView::default()
                }),
                cancel_label: "Cancel".to_string(),
                ..PickerView::default()
            },
            ReviewStep::Branches(list) => {
                let (message, rows, empty) = match list {
                    GitList::Loading => (String::new(), Vec::new(), "Loading branches…".to_string()),
                    GitList::Failed(error) => (
                        String::new(),
                        Vec::new(),
                        format!("Could not list branches: {error}"),
                    ),
                    GitList::Ready(list) => {
                        let message = match &list.current {
                            Some(current) => format!(
                                "Reviews what {current} changed since it split off the branch you pick."
                            ),
                            None => "Reviews changes since the shared ancestor with the branch you pick, including current workspace edits.".to_string(),
                        };
                        let rows = filtered_branches(list, &self.text)
                            .into_iter()
                            .map(|branch| {
                                let row = PickerRowView::new(branch, "");
                                if list.current.as_deref() == Some(branch) {
                                    row.trailing("current")
                                } else {
                                    row
                                }
                            })
                            .collect();
                        let empty = if list.branches.is_empty() {
                            "No local branches found. Is this folder in a Git repository?".to_string()
                        } else {
                            format!("No branch matches “{}”.", self.text.trim())
                        };
                        (message, rows, empty)
                    }
                };
                let (input, list) = filter_list("Filter branches", rows, empty);
                PickerView {
                    title: "Review against a base branch".to_string(),
                    message,
                    input,
                    list,
                    back_label: "Back".to_string(),
                    cancel_label: "Cancel".to_string(),
                    accept_label: "Start review".to_string(),
                    accept_enabled: true,
                    ..PickerView::default()
                }
            }
            ReviewStep::Commits(list) => {
                let (rows, empty) = match list {
                    GitList::Loading => (Vec::new(), "Loading commits…".to_string()),
                    GitList::Failed(error) => (Vec::new(), format!("Could not list commits: {error}")),
                    GitList::Ready(commits) => {
                        let now = crate::sidebar::unix_now();
                        let rows = filtered_commits(commits, &self.text)
                            .into_iter()
                            .map(|commit| {
                                let subject = if commit.subject.is_empty() {
                                    "(no message)"
                                } else {
                                    commit.subject.as_str()
                                };
                                PickerRowView::new(subject, short_sha(&commit.sha)).trailing(
                                    crate::sidebar::relative_time(now, commit.timestamp),
                                )
                            })
                            .collect();
                        let empty = if commits.is_empty() {
                            "No commits found. Is this folder in a Git repository?".to_string()
                        } else {
                            format!("No commit matches “{}”.", self.text.trim())
                        };
                        (rows, empty)
                    }
                };
                let (input, list) = filter_list("Filter by message or SHA", rows, empty);
                PickerView {
                    title: "Review a commit".to_string(),
                    message: "The latest 100 commits reachable from this workspace’s base or local HEAD.".into(),
                    input,
                    list,
                    back_label: "Back".to_string(),
                    cancel_label: "Cancel".to_string(),
                    accept_label: "Start review".to_string(),
                    accept_enabled: true,
                    ..PickerView::default()
                }
            }
            ReviewStep::Custom => PickerView {
                title: "Custom review".to_string(),
                message: "The reviewer follows your instructions instead of a fixed set of changes.".to_string(),
                input: Some(PickerInputView {
                    label: String::new(),
                    placeholder: "What should the review focus on?".to_string(),
                    text: self.text.clone(),
                }),
                back_label: "Back".to_string(),
                cancel_label: "Cancel".to_string(),
                accept_label: "Start review".to_string(),
                accept_enabled: custom_target(&self.text).is_some(),
                ..PickerView::default()
            },
        }
    }

    /// Target of row `row` of the current list step.
    fn target_at(&self, row: usize) -> Option<ReviewTarget> {
        match &self.step {
            ReviewStep::Branches(GitList::Ready(list)) => filtered_branches(list, &self.text)
                .get(row)
                .map(|branch| ReviewTarget::BaseBranch {
                    branch: (*branch).to_string(),
                }),
            ReviewStep::Commits(GitList::Ready(commits)) => filtered_commits(commits, &self.text)
                .get(row)
                .map(|commit| ReviewTarget::Commit {
                    sha: commit.sha.clone(),
                    title: (!commit.subject.is_empty()).then(|| commit.subject.clone()),
                }),
            _ => None,
        }
    }
}

/// Runs `git args` in `cwd` on the server and returns its stdout.
impl AppController {
    /// Opens the review picker for tab `index`. Non-empty `instructions`
    /// (`/review <text>`) start a custom review right away, like the TUI.
    pub(crate) fn review_open(&mut self, index: usize, instructions: &str) {
        let Some(thread) = self.thread_tab(index) else {
            return;
        };
        let Some(thread_id) = thread.thread_id.clone() else {
            self.toast("Wait for the thread to start before asking for a review");
            return;
        };
        let tab_id = self.tabs[index].id;
        if let Some(target) = custom_target(instructions) {
            self.review_start(tab_id, thread_id, target);
            return;
        }
        self.picker_open(PickerFlow::Review(ReviewPicker::new(tab_id, thread_id)));
    }

    /// Handles the review picker's events.
    pub(crate) fn review_event(
        &mut self,
        picker: &mut ReviewPicker,
        id: u64,
        event: PickerEvent,
    ) -> PickerOutcome {
        if let PickerEvent::Button { button, .. } = &event {
            match button {
                PickerButton::Cancel => return PickerOutcome::Close,
                PickerButton::Back => {
                    picker.step = ReviewStep::Presets;
                    picker.text.clear();
                    return PickerOutcome::NewStep;
                }
                PickerButton::Accept | PickerButton::Secondary => {}
            }
        }
        if let PickerEvent::InputEdited(text) = &event {
            picker.text.clone_from(text);
            return PickerOutcome::Refresh;
        }
        let target = match (&picker.step, event) {
            (ReviewStep::Presets, PickerEvent::Activated(row))
            | (
                ReviewStep::Presets,
                PickerEvent::Button {
                    selected: Some(row),
                    ..
                },
            ) => {
                let Some((preset, ..)) = PRESETS.get(row) else {
                    return PickerOutcome::Unchanged;
                };
                match preset {
                    ReviewPreset::Uncommitted => Some(ReviewTarget::UncommittedChanges),
                    ReviewPreset::BaseBranch => {
                        picker.step = ReviewStep::Branches(GitList::Loading);
                        self.review_load(id, picker.thread_id.clone(), *preset);
                        return PickerOutcome::NewStep;
                    }
                    ReviewPreset::Commit => {
                        picker.step = ReviewStep::Commits(GitList::Loading);
                        self.review_load(id, picker.thread_id.clone(), *preset);
                        return PickerOutcome::NewStep;
                    }
                    ReviewPreset::Custom => {
                        picker.step = ReviewStep::Custom;
                        picker.text.clear();
                        return PickerOutcome::NewStep;
                    }
                }
            }
            (ReviewStep::Custom, PickerEvent::InputAccepted { text, .. })
            | (ReviewStep::Custom, PickerEvent::Button { input: text, .. }) => custom_target(&text),
            (_, PickerEvent::Activated(row))
            | (
                _,
                PickerEvent::InputAccepted {
                    selected: Some(row),
                    ..
                },
            )
            | (
                _,
                PickerEvent::Button {
                    selected: Some(row),
                    ..
                },
            ) => picker.target_at(row),
            _ => None,
        };
        let Some(target) = target else {
            return PickerOutcome::Unchanged;
        };
        self.review_start(picker.tab_id, picker.thread_id.clone(), target);
        PickerOutcome::Close
    }

    /// Fetches branches or commits for review picker `id`.
    fn review_load(&mut self, id: u64, thread_id: String, preset: ReviewPreset) {
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = backend
                .goose_request(
                    "fastrock/review/catalog",
                    serde_json::json!({"threadId":thread_id}),
                )
                .await
                .and_then(|value| {
                    serde_json::from_value::<Catalog>(value).map_err(|e| e.to_string())
                });
            let step = match (preset, result) {
                (ReviewPreset::BaseBranch, Ok(catalog)) => {
                    ReviewStep::Branches(GitList::Ready(BranchList {
                        branches: catalog.branches,
                        current: catalog.current,
                    }))
                }
                (ReviewPreset::Commit, Ok(catalog)) => {
                    ReviewStep::Commits(GitList::Ready(catalog.commits))
                }
                (ReviewPreset::BaseBranch, Err(error)) => {
                    ReviewStep::Branches(GitList::Failed(error))
                }
                (ReviewPreset::Commit, Err(error)) => ReviewStep::Commits(GitList::Failed(error)),
                _ => return,
            };
            crate::ui_thread::post(move |app| {
                app.picker_update(id, move |_, flow| {
                    let PickerFlow::Review(picker) = flow else {
                        return PickerOutcome::Unchanged;
                    };
                    // Only into the step that asked (not after Back).
                    let same_step = matches!(
                        (&picker.step, &step),
                        (
                            ReviewStep::Branches(GitList::Loading),
                            ReviewStep::Branches(_)
                        ) | (
                            ReviewStep::Commits(GitList::Loading),
                            ReviewStep::Commits(_)
                        )
                    );
                    if !same_step {
                        return PickerOutcome::Unchanged;
                    }
                    picker.step = step;
                    PickerOutcome::Refresh
                });
            });
        });
    }

    /// Starts a review of `target` in the thread of tab `tab_id`.
    fn review_start(&mut self, tab_id: TabId, thread_id: String, target: ReviewTarget) {
        if self.tab_index_by_id(tab_id).is_none() {
            self.toast("The thread was closed before the review started");
            return;
        }
        let label = target_label(&target);
        self.backend.call(
            |request_id| ClientRequest::ReviewStart {
                request_id,
                params: ReviewStartParams {
                    thread_id,
                    target,
                    delivery: Some(ReviewDelivery::Inline),
                },
            },
            move |app, result: Result<ReviewStartResponse, BackendError>| {
                let Some(index) = app.tab_index_by_id(tab_id) else {
                    return;
                };
                match result {
                    // Turn events can finish before this reply reaches the UI.
                    // They alone own the running state, just as for normal sends.
                    Ok(_) => {}
                    Err(err) => app.transcript_push_notice(
                        index,
                        NoticeKind::Error,
                        format!(
                            "Could not start a review of {label}: {}",
                            err.user_message()
                        ),
                    ),
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn picker(step: ReviewStep, text: &str) -> ReviewPicker {
        ReviewPicker {
            tab_id: 1,
            thread_id: "t".to_string(),
            step,
            text: text.to_string(),
        }
    }

    fn commit(sha: &str, subject: &str) -> CommitLogEntry {
        CommitLogEntry {
            sha: sha.to_string(),
            timestamp: 100,
            subject: subject.to_string(),
        }
    }

    #[test]
    fn filters_match_every_word_case_insensitively() {
        assert!(matches_filter("feature/Auth-Login", "auth login"));
        assert!(matches_filter("anything", "  "));
        assert!(!matches_filter("feature/auth", "billing"));
    }

    #[test]
    fn rows_map_to_review_targets() {
        let branches = picker(
            ReviewStep::Branches(GitList::Ready(BranchList {
                branches: vec!["main".to_string(), "feature/auth".to_string()],
                current: Some("feature/auth".to_string()),
            })),
            "auth",
        );
        assert_eq!(
            branches.target_at(0),
            Some(ReviewTarget::BaseBranch {
                branch: "feature/auth".to_string()
            })
        );
        assert_eq!(branches.target_at(1), None);

        let commits = picker(
            ReviewStep::Commits(GitList::Ready(vec![
                commit("aaa1111", "Add login"),
                commit("bbb2222", ""),
            ])),
            "",
        );
        assert_eq!(
            commits.target_at(0),
            Some(ReviewTarget::Commit {
                sha: "aaa1111".to_string(),
                title: Some("Add login".to_string()),
            })
        );
        assert_eq!(
            commits.target_at(1),
            Some(ReviewTarget::Commit {
                sha: "bbb2222".to_string(),
                title: None,
            })
        );
        // Lists still loading have no targets.
        assert_eq!(
            picker(ReviewStep::Commits(GitList::Loading), "").target_at(0),
            None
        );
    }

    #[test]
    fn custom_instructions_must_not_be_blank() {
        assert_eq!(custom_target("  "), None);
        assert_eq!(
            custom_target(" check auth "),
            Some(ReviewTarget::Custom {
                instructions: "check auth".to_string()
            })
        );
    }

    #[test]
    fn views_follow_the_step() {
        let presets = picker(ReviewStep::Presets, "").view();
        let list = presets.list.unwrap_or_default();
        assert_eq!(list.rows.len(), PRESETS.len());
        assert!(list.click_activates);
        assert_eq!(presets.input, None);

        let loading = picker(ReviewStep::Branches(GitList::Loading), "").view();
        let list = loading.list.unwrap_or_default();
        assert_eq!(list.placeholder, "Loading branches…");
        assert!(list.accept_needs_selection);
        assert_eq!(loading.back_label, "Back");

        let empty_custom = picker(ReviewStep::Custom, " ").view();
        assert!(!empty_custom.accept_enabled);
        assert!(picker(ReviewStep::Custom, "auth").view().accept_enabled);

        let no_match = picker(
            ReviewStep::Branches(GitList::Ready(BranchList {
                branches: vec!["main".to_string()],
                current: None,
            })),
            "zzz",
        )
        .view();
        assert_eq!(
            no_match.list.unwrap_or_default().placeholder,
            "No branch matches “zzz”."
        );
    }

    #[test]
    fn target_labels_describe_the_review() {
        assert_eq!(
            target_label(&ReviewTarget::Commit {
                sha: "0123456789".to_string(),
                title: Some("Fix".to_string()),
            }),
            "commit 0123456 (Fix)"
        );
        assert_eq!(
            target_label(&ReviewTarget::BaseBranch {
                branch: "main".to_string()
            }),
            "changes against main"
        );
    }
}
