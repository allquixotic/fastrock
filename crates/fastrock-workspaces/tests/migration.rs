use anyhow::Result;
use fastrock_workspaces::{
    embedded::Connection,
    memories::{Memories, Snapshot},
    migration,
    summaries::{Summaries, Summary},
};
use redb::TableDefinition;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn v96_migration_progress_and_manual_destination() -> Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.redb");
    let summary = Summary {
        revision: 7,
        text: "Terminology and unique keywords".into(),
        profile: "cheap-fixture".into(),
        ..Default::default()
    };
    let memory = Snapshot {
        revision: 3,
        learning: false,
        recall: true,
        ..Default::default()
    };
    let db = redb::Database::create(&source)?;
    let tx = db.begin_write()?;
    tx.open_table(TableDefinition::<&str, &[u8]>::new("records_v1"))?
        .insert("schema", b"2".as_slice())?;
    tx.open_table(TableDefinition::<&str, &[u8]>::new(
        "fastrock_search_current_v1",
    ))?
    .insert(
        "thread",
        zstd::encode_all(serde_json::to_vec(&summary)?.as_slice(), 3)?.as_slice(),
    )?;
    tx.open_table(TableDefinition::<&str, u64>::new(
        "fastrock_search_requested_v1",
    ))?
    .insert("thread", 7)?;
    tx.open_table(TableDefinition::<&str, &[u8]>::new(
        "fastrock_memory_state_v1",
    ))?
    .insert(
        "current",
        zstd::encode_all(serde_json::to_vec(&memory)?.as_slice(), 3)?.as_slice(),
    )?;
    tx.commit()?;
    drop(db);
    let before = std::fs::read(&source)?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let capture = events.clone();
    let selected = source.canonicalize()?;
    migration::observe(move |p| {
        if p.source == selected {
            capture.lock().unwrap().push(p.clone());
        }
        p.active
    });
    let home = root.path().join("manual-import");
    let from = source.clone();
    let to = home.clone();
    tokio::task::spawn_blocking(move || migration::migrate(&from, &to)).await??;
    assert_eq!(std::fs::read(&source)?, before);
    let events = events.lock().unwrap();
    assert!(events.iter().any(|p| p.active && p.total > 0));
    assert!(events.iter().all(|p| p.completed <= p.total));
    assert!(
        events
            .last()
            .is_some_and(|p| !p.active && p.error.is_none() && p.completed == p.total)
    );
    drop(events);
    let connection = Connection::new(home);
    let summaries = Summaries(connection.clone());
    assert_eq!(summaries.get("thread".into()).await?, Some(summary));
    assert_eq!(summaries.advance("thread".into()).await?, 8);
    let saved = Memories(connection).load().await?;
    assert_eq!(saved.revision, 3);
    assert!(!saved.learning);
    assert!(saved.recall);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v95_parallel_conversation_summaries_do_not_conflict() -> Result<()> {
    let home = tempfile::tempdir()?;
    let summaries = Summaries(Connection::new(home.path().into()));
    let mut jobs = Vec::new();
    for i in 0..8 {
        let store = summaries.clone();
        jobs.push(tokio::spawn(async move {
            let id = format!("thread-{i}");
            for generation in 1..=16 {
                assert_eq!(store.advance(id.clone()).await?, generation);
                assert!(
                    store
                        .replace(
                            id.clone(),
                            Summary {
                                revision: generation,
                                text: format!("saved {i} {generation}"),
                                ..Default::default()
                            }
                        )
                        .await?
                );
            }
            Ok::<_, anyhow::Error>(())
        }));
    }
    for job in jobs {
        job.await??;
    }
    assert_eq!(summaries.page(None, 128).await?.len(), 8);
    Ok(())
}
