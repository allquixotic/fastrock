use fastrock_workspaces::embedded::{Connection, cancel_shutdown, prepare_shutdown};
use serde_json::json;
use vwork_engine::kv::TableDefinition;

const DATA: TableDefinition<&str, &str> = TableDefinition::new("shutdown_fixture");

// Separate test executable: shutdown deliberately quiesces all process stores.
#[tokio::test]
async fn v98_shutdown_drains_inflight_writes_and_syncs_every_store() {
    let temp = tempfile::tempdir().unwrap();
    let first = Connection::new(temp.path().join("one"));
    let second = Connection::new(temp.path().join("two"));
    second
        .run(|owner| {
            owner.with_engine(|engine| {
                let tx = engine.db.begin_write()?;
                tx.open_table(DATA)?.insert("key", "second")?;
                tx.commit()
            })
        })
        .await
        .unwrap();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let writer = first.clone();
    let writing = tokio::spawn(async move {
        writer
            .run(move |owner| {
                owner.with_engine(|engine| {
                    let _ = entered.send(());
                    wait.recv_timeout(std::time::Duration::from_secs(5))?;
                    let tx = engine.db.begin_write()?;
                    tx.open_table(DATA)?.insert("key", "first")?;
                    tx.commit()
                })
            })
            .await
    });
    ready.await.unwrap();
    let mut closing = tokio::task::spawn_blocking(prepare_shutdown);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut closing)
            .await
            .is_err()
    );
    release.send(()).unwrap();
    writing.await.unwrap().unwrap();
    closing.await.unwrap().unwrap();
    let states = vwork_engine::kv::durability_statuses();
    assert_eq!(states.len(), 2);
    assert!(
        states
            .iter()
            .all(|s| s.pending_transactions == 0 && !s.failed && !s.overdue)
    );
    assert!(
        first
            .call("database_compact".into(), json!({}))
            .await
            .is_err()
    );
    let late = Connection::new(temp.path().join("late"));
    assert!(late.call("ping".into(), json!({})).await.is_err());
    cancel_shutdown();
    for (connection, expected) in [(first, "first"), (second, "second")] {
        let actual = connection
            .run(|owner| {
                owner.with_engine(|engine| {
                    let tx = engine.db.begin_read()?;
                    Ok(tx.open_table(DATA)?.get("key")?.unwrap().value().to_owned())
                })
            })
            .await
            .unwrap();
        assert_eq!(actual, expected);
    }
}
