//! Deterministic local summary matching; no model calls or unbounded edit matrix.
use rayon::prelude::*;
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

const MAX_QUERY_CHARS: usize = 256;
const FUZZY_CHAR_BUDGET: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Exact,
    Fuzzy,
}

pub struct Query {
    lower: String,
    chars: Vec<char>,
}
impl Query {
    pub fn new(text: &str) -> anyhow::Result<Self> {
        let lower = text.trim().to_lowercase();
        let chars: Vec<_> = lower.chars().take(MAX_QUERY_CHARS + 1).collect();
        anyhow::ensure!(
            !chars.is_empty() && chars.len() <= MAX_QUERY_CHARS,
            "Search query must contain 1–256 characters"
        );
        Ok(Self { lower, chars })
    }

    pub fn matches(&self, text: &str, phase: Phase, cancelled: &AtomicBool) -> bool {
        if cancelled.load(Ordering::Relaxed) {
            return false;
        }
        if phase == Phase::Exact {
            return text.to_lowercase().contains(&self.lower);
        }
        if self.chars.len() < 3 {
            return false;
        }
        // A greedy subsequence pass is linear with a hard character budget.
        // Reset after long gaps so scattered letters across an entire summary
        // do not masquerade as a relevant match.
        let mut next = 0;
        let mut gap = 0;
        for (i, c) in text
            .chars()
            .flat_map(char::to_lowercase)
            .take(FUZZY_CHAR_BUDGET)
            .enumerate()
        {
            if i % 256 == 0 && cancelled.load(Ordering::Relaxed) {
                return false;
            }
            if c == self.chars[next] {
                next += 1;
                gap = 0;
                if next == self.chars.len() {
                    return true;
                }
            } else if next > 0 {
                gap += 1;
                if gap > 16 {
                    next = 0;
                    gap = 0;
                }
            }
        }
        false
    }
}

/// Keep a small dedicated pool; repeated queries reuse its workers. Indexed
/// collection preserves source order regardless of thread scheduling.
pub fn matching_ids(
    query: &Query,
    rows: &[(String, crate::summaries::Summary)],
    phase: Phase,
    cancelled: &AtomicBool,
) -> anyhow::Result<Vec<String>> {
    static POOL: OnceLock<Result<rayon::ThreadPool, rayon::ThreadPoolBuildError>> = OnceLock::new();
    let pool = POOL
        .get_or_init(|| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(
                    std::thread::available_parallelism()
                        .map_or(2, usize::from)
                        .min(4),
                )
                .thread_name(|i| format!("fastrock-search-{i}"))
                .build()
        })
        .as_ref()
        .map_err(|_| anyhow::anyhow!("Search workers unavailable"))?;
    Ok(pool
        .install(|| {
            rows.par_iter()
                .map(|(id, summary)| {
                    query
                        .matches(&summary.text, phase, cancelled)
                        .then(|| id.clone())
                })
                .collect::<Vec<_>>()
        })
        .into_iter()
        .flatten()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v44_exact_is_literal_case_insensitive_and_fuzzy_is_bounded() {
        let cancel = AtomicBool::new(false);
        let q = Query::new("Λ.*[API]").unwrap();
        assert!(q.matches("keyword λ.*[api] here", Phase::Exact, &cancel));
        assert!(!q.matches("λotherapi", Phase::Exact, &cancel));
        let q = Query::new("cfg").unwrap();
        assert!(q.matches("configuration", Phase::Fuzzy, &cancel));
        assert!(!q.matches(
            &format!("{}configuration", "x".repeat(FUZZY_CHAR_BUDGET)),
            Phase::Fuzzy,
            &cancel
        ));
        assert!(Query::new(&"x".repeat(MAX_QUERY_CHARS + 1)).is_err());
    }
    #[test]
    fn v44_parallel_search_is_ordered_and_cancelled_work_is_empty() {
        let rows: Vec<_> = (0..100)
            .map(|n| {
                (
                    n.to_string(),
                    crate::summaries::Summary {
                        revision: 1,
                        profile: "cheap".into(),
                        text: "Unique keyword".into(),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let cancel = AtomicBool::new(false);
        let query = Query::new("KEYWORD").unwrap();
        let ids = matching_ids(&query, &rows, Phase::Exact, &cancel).unwrap();
        assert_eq!(
            ids,
            rows.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>()
        );
        cancel.store(true, Ordering::Relaxed);
        assert!(
            matching_ids(&query, &rows, Phase::Exact, &cancel)
                .unwrap()
                .is_empty()
        );
    }
}
