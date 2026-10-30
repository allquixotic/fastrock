//! Search compressed workspace path metadata without exporting or reading blobs.
use super::{Backend, FileHit};
use anyhow::{Result, ensure};
use fastrock_workspaces::{embedded::Connection, session::Selection};
use std::{
    collections::{BTreeSet, BinaryHeap},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::watch;

const MAX_PATHS: usize = 250_000;
const MAX_PATH_BYTES: usize = 32 * 1024 * 1024;
const MAX_RESULTS: usize = 100;

struct Candidate {
    path: String,
    lower: String,
    directory: bool,
}
#[derive(Clone, Default)]
struct Cache {
    revision: String,
    paths: Arc<Vec<Candidate>>,
}

pub(super) struct Session {
    queries: watch::Sender<(u64, String)>,
    epoch: Arc<AtomicU64>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.epoch.store(u64::MAX, Ordering::Release);
    }
}
impl Session {
    pub fn start(backend: &Backend, selection: Selection, generation: u64) -> Self {
        let (queries, mut receiver) = watch::channel((0, String::new()));
        let epoch = Arc::new(AtomicU64::new(0));
        let current = epoch.clone();
        backend.spawn(async move {
            let connection = Connection::new(selection.home);
            let mut cache = Cache::default();
            while receiver.changed().await.is_ok() {
                let (version, query) = receiver.borrow_and_update().clone();
                let wanted = current.clone();
                let workspace = selection.workspace.clone();
                let text = query.clone();
                let previous = cache.clone();
                let result = find(&connection, workspace, previous, text, wanted, version).await;
                if current.load(Ordering::Acquire) != version {
                    continue;
                }
                let (hits, error) = match result {
                    Ok((updated, hits)) => {
                        cache = updated;
                        (hits, None)
                    }
                    Err(error) => (
                        Vec::new(),
                        Some(format!("Could not search workspace files: {error}")),
                    ),
                };
                crate::ui_thread::post(move |app| {
                    if !app
                        .composer_shared
                        .file_search
                        .accept_results(generation, &query)
                    {
                        return;
                    }
                    app.composer_on_file_matches(generation, &query, hits, true);
                    if let Some(error) = error {
                        app.toast(error);
                    }
                });
            }
        });
        Self { queries, epoch }
    }
    pub fn update(&self, query: &str) {
        let version = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.queries.send_replace((version, query.to_owned()));
    }
}

async fn find(
    connection: &Connection,
    workspace: String,
    previous: Cache,
    text: String,
    wanted: Arc<AtomicU64>,
    version: u64,
) -> Result<(Cache, Vec<FileHit>)> {
    connection
        .run(move |store| {
            let cached = store.with_engine(|engine| {
                let workspace = engine.workspace(&workspace)?;
                if previous.revision == workspace.head {
                    return Ok(previous);
                }
                let (_, tree) = engine.snapshot(&workspace)?;
                ensure!(
                    tree.len() <= MAX_PATHS,
                    "Workspace file search supports at most 250,000 paths"
                );
                let mut bytes = 0usize;
                let mut paths = Vec::with_capacity(tree.len().min(MAX_PATHS));
                let mut directories = BTreeSet::new();
                for (path, _) in tree {
                    if wanted.load(Ordering::Acquire) != version {
                        return Ok(Cache::default());
                    }
                    bytes = bytes.saturating_add(path.len());
                    ensure!(
                        bytes <= MAX_PATH_BYTES && path.len() <= 16 * 1024,
                        "Workspace path metadata exceeds the file-search limit"
                    );
                    for (end, _) in path.match_indices('/') {
                        let directory = &path[..end];
                        if !directories.contains(directory) {
                            bytes = bytes.saturating_add(directory.len());
                            ensure!(
                                bytes <= MAX_PATH_BYTES
                                    && paths.len() + directories.len() + 1 < MAX_PATHS,
                                "Workspace path metadata exceeds the file-search limit"
                            );
                            directories.insert(directory.to_owned());
                        }
                    }
                    paths.push(Candidate {
                        lower: path.to_lowercase(),
                        path,
                        directory: false,
                    });
                }
                ensure!(
                    paths.len() + directories.len() <= MAX_PATHS,
                    "Workspace file search supports at most 250,000 paths"
                );
                for path in directories {
                    paths.push(Candidate {
                        lower: path.to_lowercase(),
                        path,
                        directory: true,
                    });
                }
                Ok(Cache {
                    revision: workspace.head,
                    paths: Arc::new(paths),
                })
            })?;
            let hits = search(&cached.paths, &text, &wanted, version)?;
            Ok((cached, hits))
        })
        .await
}

/// Linear subsequence matching with original-character indices even when Unicode
/// case folding expands a character. Exact substring matches rank before fuzzy.
fn matched(candidate: &Candidate, lower: &str, wanted: &[char]) -> Option<(u8, Vec<u32>)> {
    let exact = candidate.lower.find(lower);
    let mut indices = Vec::with_capacity(wanted.len());
    let mut next = 0;
    let mut byte = 0;
    for (index, ch) in candidate.path.chars().enumerate() {
        for folded in ch.to_lowercase() {
            let matches = if let Some(start) = exact {
                byte >= start && byte < start + lower.len()
            } else {
                next < wanted.len() && folded == wanted[next]
            };
            if matches {
                if indices.last() != Some(&(index as u32)) {
                    indices.push(index as u32);
                }
                next += 1;
            }
            byte += folded.len_utf8();
        }
        if next == wanted.len() {
            break;
        }
    }
    (next == wanted.len()).then_some((
        match exact {
            Some(0) => 0,
            Some(_) => 1,
            None => 2,
        },
        indices,
    ))
}

fn search(
    paths: &[Candidate],
    query: &str,
    epoch: &AtomicU64,
    version: u64,
) -> Result<Vec<FileHit>> {
    ensure!(query.len() <= 4096, "File search query is too long");
    let lower = query.replace('\\', "/").to_lowercase();
    let wanted = lower.chars().collect::<Vec<_>>();
    ensure!(
        !wanted.is_empty() && wanted.len() <= 128,
        "File search accepts 1–128 characters"
    );
    let mut best = BinaryHeap::new();
    for (index, path) in paths.iter().enumerate() {
        if index % 128 == 0 && epoch.load(Ordering::Acquire) != version {
            return Ok(Vec::new());
        }
        if let Some((rank, indices)) = matched(path, &lower, &wanted) {
            let span = indices.last().copied().unwrap_or(0) - indices.first().copied().unwrap_or(0);
            let hit = (
                rank,
                span,
                path.path.len(),
                path.path.clone(),
                indices,
                path.directory,
            );
            if best.len() < MAX_RESULTS {
                best.push(hit);
            } else if best.peek().is_some_and(|old| &hit < old) {
                best.pop();
                best.push(hit);
            }
        }
    }
    Ok(best
        .into_sorted_vec()
        .into_iter()
        .map(|(_, _, _, path, indices, is_dir)| FileHit {
            path,
            is_dir,
            indices,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn v71_virtual_file_cache_tracks_revision_without_source_export() {
        use serde_json::json;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let repo = git2::Repository::init(&root).unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "original\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("src/main.rs")).unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "Fixture", &tree, &[])
            .unwrap();
        let home = temp.path().join("store");
        let connection = Connection::new(home.clone());
        connection
            .call("repo_register".into(), json!({"repo":"repo","path":root}))
            .await
            .unwrap();
        connection
            .call(
                "workspace_create".into(),
                json!({"repo":"repo","workspace":"main"}),
            )
            .await
            .unwrap();
        let selection = Selection {
            home,
            workspace: "main".into(),
        }
        .resolve()
        .await
        .unwrap();
        let epoch = Arc::new(AtomicU64::new(1));
        let (first, hits) = find(
            &connection,
            "main".into(),
            Cache::default(),
            "src".into(),
            epoch.clone(),
            1,
        )
        .await
        .unwrap();
        assert!(hits.iter().any(|h| h.path == "src" && h.is_dir));
        assert!(hits.iter().any(|h| h.path == "src/main.rs" && !h.is_dir));
        let (same, _) = find(
            &connection,
            "main".into(),
            first.clone(),
            "main".into(),
            epoch.clone(),
            1,
        )
        .await
        .unwrap();
        assert!(Arc::ptr_eq(&same.paths, &first.paths));
        connection.call("files_write".into(), json!({"workspace":"main","path":"src/virtual-only.rs","content":"compressed\n","expected_revision":first.revision})).await.unwrap();
        let (changed, hits) = find(
            &connection,
            "main".into(),
            first,
            "virtual-only".into(),
            epoch.clone(),
            1,
        )
        .await
        .unwrap();
        assert_eq!(hits[0].path, "src/virtual-only.rs");
        assert!(!root.join("src/virtual-only.rs").exists());
        assert!(!selection.cwd.join("src").exists());
        connection.call("files_delete".into(), json!({"workspace":"main","path":"src/main.rs","expected_revision":changed.revision})).await.unwrap();
        let (_, hits) = find(
            &connection,
            "main".into(),
            changed,
            "src/main".into(),
            epoch,
            1,
        )
        .await
        .unwrap();
        assert!(hits.is_empty());
        assert_eq!(
            std::fs::read_to_string(root.join("src/main.rs")).unwrap(),
            "original\n"
        );
    }
    fn paths(values: &[&str]) -> Vec<Candidate> {
        values
            .iter()
            .map(|path| Candidate {
                path: path.to_string(),
                lower: path.to_lowercase(),
                directory: false,
            })
            .collect()
    }
    #[test]
    fn v71_virtual_file_matches_rank_bound_and_preserve_unicode_indices() {
        let epoch = AtomicU64::new(7);
        let files = paths(&[
            "src/config.rs",
            "config/main.rs",
            "src/c_f_g.rs",
            "İmage/石.rs",
        ]);
        let hits = search(&files, "config", &epoch, 7).unwrap();
        assert_eq!(
            hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(),
            ["config/main.rs", "src/config.rs"]
        );
        assert!(
            search(&files, "cfg", &epoch, 7)
                .unwrap()
                .iter()
                .any(|h| h.path == "src/c_f_g.rs")
        );
        let unicode = search(&files, "i\u{307}mage/石", &epoch, 7).unwrap();
        assert_eq!(unicode[0].indices, [0, 1, 2, 3, 4, 5, 6]);
        assert!(search(&files, "config", &epoch, 6).unwrap().is_empty());
        assert!(search(&files, &"x".repeat(129), &epoch, 7).is_err());
        let many = (0..1000)
            .map(|i| format!("src/file-{i:04}.rs"))
            .collect::<Vec<_>>();
        let many = paths(&many.iter().map(String::as_str).collect::<Vec<_>>());
        let hits = search(&many, "file", &epoch, 7).unwrap();
        assert_eq!(hits.len(), MAX_RESULTS);
        assert_eq!(hits[0].path, "src/file-0000.rs");
        assert_eq!(hits[99].path, "src/file-0099.rs");
    }
}
