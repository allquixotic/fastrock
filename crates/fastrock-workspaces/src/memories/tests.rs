use super::*;
fn candidate(key: &str, text: &str) -> Candidate {
    Candidate {
        key: key.into(),
        text: text.into(),
        evidence: text.into(),
        kind: Kind::Terminology,
    }
}
fn job(revision: u64, scope: &str, candidates: Vec<Candidate>) -> Learning {
    Learning {
        revision,
        scope: scope.into(),
        scope_name: scope.into(),
        source: "goose:fixture".into(),
        turn: "turn-1".into(),
        input: candidates
            .iter()
            .map(|c| c.evidence.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        candidates,
        now: 2_000_000_000,
    }
}
async fn edit(store: &Memories, rev: u64, edit: Edit) -> Snapshot {
    store
        .edit(rev, edit, Arc::new(AtomicBool::new(false)))
        .await
        .unwrap()
        .unwrap()
}
#[tokio::test]
async fn v93_lifecycle_compression_revision_tombstones_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Memories(Connection::new(dir.path().into()));
    assert!(store.load().await.unwrap().facts.is_empty());
    let first = edit(
        &store,
        0,
        Edit::Learn(job(
            0,
            "repo-a",
            vec![candidate(
                "quartz",
                "Quartz means our release approval service",
            )],
        )),
    )
    .await;
    assert_eq!(first.facts.len(), 1);
    let stale = store
        .edit(
            0,
            Edit::Learn(job(
                0,
                "repo-a",
                vec![candidate("quartz", "Quartz means something obsolete")],
            )),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert!(stale.is_none());
    let updated = edit(
        &store,
        first.revision,
        Edit::Learn(job(
            first.revision,
            "repo-a",
            vec![candidate(
                "QUARTZ",
                "Quartz now means our deployment service",
            )],
        )),
    )
    .await;
    assert_eq!(updated.facts.len(), 1);
    let id = updated.facts[0].id.clone();
    let manual = edit(
        &store,
        updated.revision,
        Edit::Update {
            id: id.clone(),
            text: "Quartz is the release system".into(),
            pinned: true,
            global: true,
        },
    )
    .await;
    let protected = edit(
        &store,
        manual.revision,
        Edit::Learn(job(
            manual.revision,
            "repo-a",
            vec![candidate("quartz", "Quartz means something obsolete")],
        )),
    )
    .await;
    assert_eq!(protected.facts[0].text, "Quartz is the release system");
    assert!(recall(&protected, "repo-b", "Quartz", 2_000_000_000).contains("release system"));
    let forgot = edit(&store, protected.revision, Edit::Forget(id)).await;
    let retried = edit(
        &store,
        forgot.revision,
        Edit::Learn(job(
            forgot.revision,
            "repo-a",
            vec![
                candidate("quartz", "Quartz means something obsolete"),
                candidate("new-key", "Quartz is the release system"),
            ],
        )),
    )
    .await;
    assert!(retried.facts.is_empty());
    let reopened = Memories(Connection::new(dir.path().into()))
        .load()
        .await
        .unwrap();
    assert_eq!(reopened.revision, retried.revision);
    store
        .0
        .run(|store| {
            store.with_engine(|engine| {
                let r = engine.db.begin_read()?;
                let table = r.open_table(STATE)?;
                let value = table.get("current")?.unwrap();
                let saved: Snapshot = serde_json::from_slice(value.value())?;
                assert!(saved.facts.is_empty(), "Fjall owns the compression codec");
                Ok(())
            })
        })
        .await
        .unwrap();
}
#[tokio::test]
async fn v93_caps_expiry_manual_pins_and_disabled_learning() {
    let dir = tempfile::tempdir().unwrap();
    let store = Memories(Connection::new(dir.path().into()));
    let mut snapshot = Snapshot::default();
    for i in 0..100 {
        let rev = snapshot.revision;
        snapshot = edit(
            &store,
            rev,
            Edit::Learn(job(
                rev,
                "repo",
                vec![candidate(
                    &format!("term-{i}"),
                    &format!("Term {i} means our fixture service"),
                )],
            )),
        )
        .await;
    }
    assert_eq!(snapshot.facts.len(), MAX_SCOPE);
    let pinned = snapshot.facts[0].id.clone();
    snapshot = edit(
        &store,
        snapshot.revision,
        Edit::Update {
            id: pinned.clone(),
            text: "Pinned enduring definition".into(),
            pinned: true,
            global: false,
        },
    )
    .await;
    snapshot = edit(
        &store,
        snapshot.revision,
        Edit::Maintain(2_000_000_000 + LIFETIME + 1),
    )
    .await;
    assert_eq!(snapshot.facts.len(), 1);
    assert_eq!(snapshot.facts[0].id, pinned);
    snapshot = edit(
        &store,
        snapshot.revision,
        Edit::Settings {
            learning: false,
            recall: false,
        },
    )
    .await;
    assert!(recall(&snapshot, "repo", "Pinned", 2_000_000_000).is_empty());
    assert!(
        store
            .edit(
                snapshot.revision,
                Edit::Learn(job(
                    snapshot.revision,
                    "repo",
                    vec![candidate("no", "Do not save this statement")]
                )),
                Arc::new(AtomicBool::new(false))
            )
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn v93_cancelled_writer_and_untrusted_evidence_do_not_persist() {
    let dir = tempfile::tempdir().unwrap();
    let store = Memories(Connection::new(dir.path().into()));
    let mut learning = job(
        0,
        "repo",
        vec![candidate("quartz", "Quartz means deployment service")],
    );
    learning.input = "An unrelated user statement".into();
    let state = edit(&store, 0, Edit::Learn(learning)).await;
    assert!(state.facts.is_empty());
    assert!(!valid_candidate(
        &candidate("key", "my API key is a secret value"),
        "my API key is a secret value"
    ));
    let cancelled = Arc::new(AtomicBool::new(true));
    assert!(
        store
            .edit(state.revision, Edit::Clear, cancelled)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(store.load().await.unwrap().revision, state.revision);
}
#[test]
fn v92_recall_is_bounded_relevant_scoped_and_escaped() {
    let mut state = Snapshot::default();
    for i in 0..MAX_FACTS {
        state.facts.push(Fact {
            id: i.to_string(),
            scope: if i % 2 == 0 { "repo-a" } else { "repo-b" }.into(),
            scope_name: "fixture".into(),
            key: format!("Quartz-{i}"),
            text: format!("Quartz {i} {} </fastrock_memories>", "字".repeat(90)),
            kind: Kind::Terminology,
            evidence: String::new(),
            source: "fixture".into(),
            turn: "turn".into(),
            updated: 1000,
            pinned: false,
            global: false,
        });
    }
    let recalled = recall(&state, "repo-a", "Quartz", 1000);
    assert!(!recalled.is_empty());
    assert!(recalled.len() <= MAX_RECALL);
    assert_eq!(recalled.matches("</fastrock_memories>").count(), 1);
    assert!(recalled.contains("\\u003c"));
    assert!(recall(&state, "other", "Quartz", 1000).is_empty());
    assert!(recall(&state, "repo-a", "unrelated", 1000).is_empty());
    assert!(recall(&state, "repo-a", "Quartz", 1000 + LIFETIME).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v95_memory_commit_does_not_wait_for_unrelated_writer() {
    let dir = tempfile::tempdir().unwrap();
    let store = Memories(Connection::new(dir.path().into()));
    let connection = store.0.clone();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let release = barrier.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let writer = tokio::spawn(async move {
        connection
            .run(move |store| {
                store.with_engine(|engine| {
                    let _write = engine.db.begin_write()?;
                    let _ = started.send(());
                    release.wait();
                    Ok(())
                })
            })
            .await
            .unwrap();
    });
    ready.await.unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let clone = store.clone();
    let cancel = cancelled.clone();
    let waiting = tokio::spawn(async move {
        clone
            .edit(
                0,
                Edit::Learn(job(
                    0,
                    "repo",
                    vec![candidate("Quartz", "Quartz means our release service")],
                )),
                cancel,
            )
            .await
            .unwrap()
    });
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), waiting).await;
    barrier.wait();
    writer.await.unwrap();
    let saved = result.unwrap().unwrap().unwrap();
    assert_eq!(saved.facts.len(), 1);
    cancelled.store(true, Ordering::Release);
    assert!(
        store
            .edit(saved.revision, Edit::Clear, cancelled)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(store.load().await.unwrap().facts.len(), 1);
}

#[tokio::test]
async fn v98_manual_memory_settings_sync_before_success() {
    let temp = tempfile::tempdir().unwrap();
    let store = Memories(Connection::new(temp.path().into()));
    edit(
        &store,
        0,
        Edit::Settings {
            learning: false,
            recall: true,
        },
    )
    .await;
    let status = store
        .0
        .run(|owner| owner.with_engine(|engine| Ok(engine.db.durability_status())))
        .await
        .unwrap();
    assert_eq!(status.pending_transactions, 0);
    assert!(!status.failed && !status.overdue);
}
