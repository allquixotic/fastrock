//! Local deterministic search. Model work belongs to background summaries only.
use super::ThreadSummary;
use crate::{app::AppController, backend::Backend};
use codex_app_server_protocol::{ClientRequest, Model, ThreadListResponse};
use fastrock_workspaces::{
    embedded::Connection,
    search::{Phase, Query},
    summaries::Summaries,
};
use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::oneshot;

pub(super) struct SearchRun {
    cancel: Option<oneshot::Sender<()>>,
    stopped: Arc<AtomicBool>,
}
impl Drop for SearchRun {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
    }
}

fn publish(generation: u64, hits: Vec<ThreadSummary>) {
    if hits.is_empty() {
        return;
    }
    crate::ui_thread::post(move |app| {
        if app.sidebar.generation == generation {
            super::merge_page(&mut app.sidebar.history_hits, hits);
            app.sidebar_render();
        }
    });
}

async fn search_history(
    backend: &Backend,
    generation: u64,
    query: String,
    archived: bool,
    stopped: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let matcher = Arc::new(Query::new(&query)?);
    let mut candidates = HashMap::new();
    let mut cursor = None;
    let mut seen = HashSet::new();
    loop {
        let page: ThreadListResponse = backend
            .request(ClientRequest::ThreadList {
                request_id: backend.next_request_id(),
                params: super::list_params(cursor, 100, None, archived, true),
            })
            .await?;
        let mut hits = Vec::new();
        for thread in page.data.into_iter().filter(|t| !t.ephemeral) {
            let summary = ThreadSummary::from_thread(&thread);
            if matcher.matches(thread.name.as_deref().unwrap_or(""), Phase::Exact, &stopped)
                || matcher.matches(&thread.preview, Phase::Exact, &stopped)
            {
                hits.push(summary.clone());
            }
            candidates.insert(thread.id, summary);
        }
        publish(generation, hits);
        cursor = page.next_cursor;
        if let Some(next) = &cursor {
            anyhow::ensure!(seen.insert(next.clone()), "Repeated thread cursor");
        } else {
            break;
        }
    }
    let paths = fastrock_workspaces::vwork::DiscoveryPaths::current();
    let mut stores = fastrock_workspaces::vwork::discover(&paths).await?;
    if let Some(home) = crate::goose::summary_store() {
        if !stores.iter().any(|store| store.home == home) {
            stores.push(fastrock_workspaces::vwork::Installation {
                id: home.to_string_lossy().into(),
                label: "Search summaries".into(),
                home,
            });
        }
    }
    // All literal summary matches are published before fuzzy continuation.
    for phase in [Phase::Exact, Phase::Fuzzy] {
        for store in &stores {
            let summaries = Summaries(Connection::new(store.home.clone()));
            let mut after = None;
            loop {
                if stopped.load(Ordering::Relaxed) {
                    return Ok(());
                }
                let rows = summaries.page(after.clone(), 16).await?;
                if rows.is_empty() {
                    break;
                }
                after = rows.last().map(|(id, _)| id.clone());
                let matcher = matcher.clone();
                let cancelled = stopped.clone();
                let ids = tokio::task::spawn_blocking(move || {
                    fastrock_workspaces::search::matching_ids(&matcher, &rows, phase, &cancelled)
                })
                .await??;
                publish(
                    generation,
                    ids.into_iter()
                        .filter_map(|id| candidates.get(&id).cloned())
                        .collect(),
                );
            }
        }
    }
    Ok(())
}

impl AppController {
    pub(super) fn sidebar_start_history_search(&mut self) {
        self.sidebar.search_run = None;
        self.sidebar.history_hits.clear();
        if self.sidebar.search_term.is_empty() {
            return;
        }
        let generation = self.sidebar.generation;
        let query = self.sidebar.search_term.clone();
        let archived = self.sidebar.archived;
        let (cancel, cancelled) = oneshot::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        self.sidebar.search_run = Some(SearchRun {
            cancel: Some(cancel),
            stopped: stopped.clone(),
        });
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = tokio::select! {
                _ = cancelled => return,
                result = search_history(&backend, generation, query, archived, stopped) => result,
            };
            crate::ui_thread::post(move |app| {
                if app.sidebar.generation != generation {
                    return;
                }
                app.sidebar.search_run = None;
                if result.is_err() {
                    app.sidebar.error =
                        Some("Summary search unavailable; title matches are shown.".into());
                }
                app.sidebar_render();
            });
        });
    }
}

/// Keep the default model's provider namespace and geographic profile.
/// Catalog entries from a different region are deliberately excluded.
pub(crate) fn search_model(default: Option<&str>, models: &[Model]) -> Option<String> {
    let default = default.or_else(|| {
        models
            .iter()
            .find(|m| m.is_default)
            .map(|m| m.model.as_str())
    });
    let scope = default.and_then(|slug| slug.find("gpt-").map(|i| &slug[..i]));
    let gpt_default = scope.is_some() || default.is_none();
    let scope = scope.unwrap_or(match default {
        Some(slug) if slug.starts_with("us.") => "us.openai.",
        Some(slug) if slug.starts_with("global.") => "global.openai.",
        _ => "",
    });
    let suffix = default
        .and_then(|slug| slug.strip_prefix(scope))
        .and_then(|slug| slug.find(':').map(|i| &slug[i..]))
        .unwrap_or("");
    models
        .iter()
        .filter(|m| {
            m.model.strip_prefix(scope).is_some_and(|slug| {
                slug.starts_with("gpt-") && slug.ends_with(&format!("-luna{suffix}"))
            }) && luna_version(&m.model) >= vec![6]
        })
        .max_by_key(|m| luna_version(&m.model))
        .map(|m| m.model.clone())
        .or_else(|| {
            if gpt_default {
                Some(format!("{scope}gpt-6-luna{suffix}"))
            } else {
                default.map(str::to_owned)
            }
        })
}

fn luna_version(model: &str) -> Vec<u32> {
    model
        .rsplit("gpt-")
        .next()
        .unwrap_or_default()
        .split("-luna")
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn model_selection_preserves_provider_scope() {
        assert_eq!(
            search_model(Some("us.openai.gpt-6.1-sol"), &[]).as_deref(),
            Some("us.openai.gpt-6-luna")
        );
        assert_eq!(
            search_model(Some("global.openai.gpt-6-sol"), &[]).as_deref(),
            Some("global.openai.gpt-6-luna")
        );
        assert_eq!(
            search_model(Some("gpt-6.1-sol"), &[]).as_deref(),
            Some("gpt-6-luna")
        );
        assert_eq!(
            search_model(Some("claude-custom"), &[]).as_deref(),
            Some("claude-custom")
        );
    }
    #[test]
    fn catalog_luna_retains_region_even_for_a_different_model_family() {
        fn model(slug: &str) -> Model {
            serde_json::from_value(json!({"id": slug, "model": slug, "displayName": slug, "description": "", "hidden": false, "supportedReasoningEfforts": [], "defaultReasoningEffort": "low", "isDefault": false})).expect("model")
        }
        let catalog = vec![
            model("global.openai.gpt-6-luna"),
            model("us.openai.gpt-6-luna"),
            model("us.openai.gpt-6.1-luna"),
        ];
        assert_eq!(
            search_model(Some("us.anthropic.claude"), &catalog).as_deref(),
            Some("us.openai.gpt-6.1-luna")
        );
        assert_eq!(
            search_model(Some("global.openai.gpt-6-sol"), &catalog).as_deref(),
            Some("global.openai.gpt-6-luna")
        );
    }

    #[test]
    fn model_selection_preserves_arn_and_revision() {
        let profile = "arn:aws:bedrock:us-east-1:1234:inference-profile/us.openai.gpt-6-sol:0";
        assert_eq!(
            search_model(Some(profile), &[]).as_deref(),
            Some("arn:aws:bedrock:us-east-1:1234:inference-profile/us.openai.gpt-6-luna:0")
        );
        assert_eq!(luna_version("us.openai.gpt-6-luna:0"), vec![6]);
    }

    #[test]
    fn latest_luna_uses_numeric_versions() {
        assert!(luna_version("gpt-6-luna") > luna_version("gpt-5.6-luna"));
        assert!(luna_version("gpt-6.10-luna") > luna_version("gpt-6.9-luna"));
    }
}
