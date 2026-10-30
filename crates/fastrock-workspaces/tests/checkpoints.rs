use base64::Engine as _;
use fastrock_workspaces::{
    checkpoints::{self, Capture},
    embedded::Connection,
};
use serde_json::json;
use std::path::PathBuf;
struct Fixture {
    temp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    connection: Connection,
}
impl Fixture {
    async fn new(limit: usize) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let repo = git2::Repository::init(&root).unwrap();
        // Model the CI host's global policy without reading/changing the user's
        // Git files or process-wide libgit2 search paths (tests run in parallel).
        let global_path = temp.path().join("global.gitconfig");
        std::fs::write(&global_path, "[core]\n    autocrlf = true\n").unwrap();
        let mut config = git2::Config::new().unwrap();
        config
            .add_file(&global_path, git2::ConfigLevel::Global, false)
            .unwrap();
        config
            .add_file(&repo.path().join("config"), git2::ConfigLevel::Local, false)
            .unwrap();
        repo.set_config(&config).unwrap();
        assert!(repo.config().unwrap().get_bool("core.autocrlf").unwrap());
        // Preserve the fixture's exact CRLF bytes even when the host's global
        // Git configuration enables autocrlf (as on GitHub's Windows runners).
        repo.config()
            .unwrap()
            .set_bool("core.autocrlf", false)
            .unwrap();
        std::fs::write(root.join("file.txt"), b"original\r\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("file.txt")).unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let original = tree.get_name("file.txt").unwrap();
        assert_eq!(
            repo.find_blob(original.id()).unwrap().content(),
            b"original\r\n"
        );
        let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "Fixture", &tree, &[])
            .unwrap();
        let home = temp.path().join("state");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(
            home.join("config.toml"),
            format!("max_output_bytes = {limit}\n"),
        )
        .unwrap();
        let connection = Connection::new(home.clone());
        connection
            .call("repo_register".into(), json!({"repo":"repo","path":root}))
            .await
            .unwrap();
        connection
            .call(
                "workspace_create".into(),
                json!({"repo":"repo","workspace":"chat"}),
            )
            .await
            .unwrap();
        Self {
            temp,
            root,
            home,
            connection,
        }
    }
    async fn head(&self) -> String {
        head(&self.connection).await
    }
    async fn write(&self, text: &str, mode: &str) {
        self.connection.call("files_write".into(),json!({"workspace":"chat","path":"file.txt","content":text,"mode":mode,"expected_revision":self.head().await})).await.unwrap();
    }
}
async fn head(connection: &Connection) -> String {
    connection
        .call("workspace_info".into(), json!({"workspace":"chat"}))
        .await
        .unwrap()["revision"]
        .as_str()
        .unwrap()
        .into()
}
async fn read(connection: &Connection) -> String {
    let value = connection
        .call(
            "files_read".into(),
            json!({"workspace":"chat","path":"file.txt","binary":true}),
        )
        .await
        .unwrap();
    String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(value["base64"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap()
}
fn point(id: &str) -> Capture {
    Capture {
        id: id.into(),
        session: "session".into(),
        created_ms: 1,
        turn: "turn".into(),
        label: "Before turn".into(),
        phase: "before".into(),
        boundary: json!({"lastMessage":"message","count":3,"digest":"fixture"}),
    }
}
#[tokio::test]
async fn v62_checkpoint_reopens_survives_squash_gc_and_restores_exact_source() {
    let f = Fixture::new(1024 * 1024).await;
    f.write("checkpoint λ\r\n", "100755").await;
    let saved = checkpoints::capture(&f.connection, Some("chat".into()), point("0001"))
        .await
        .unwrap();
    assert_eq!(
        checkpoints::capture(&f.connection, Some("chat".into()), point("0001"))
            .await
            .unwrap(),
        saved
    );
    f.write("later\n", "100644").await;
    f.connection
        .call(
            "workspace_squash".into(),
            json!({"workspace":"chat","expected_revision":f.head().await}),
        )
        .await
        .unwrap();
    f.connection.call("gc".into(), json!({})).await.unwrap();
    let Fixture {
        temp,
        root,
        home,
        connection,
    } = f;
    drop(connection);
    let connection = Connection::new(home);
    assert_eq!(
        checkpoints::get(&connection, "session".into(), "0001".into())
            .await
            .unwrap(),
        saved
    );
    let review = checkpoints::review(
        &connection,
        "session".into(),
        "0001".into(),
        Some("chat".into()),
    )
    .await
    .unwrap();
    assert!(review.complete && review.diff.contains("checkpoint λ"));
    assert_eq!(review.files[0].checkpoint_mode, "100755");
    assert_eq!(review.files[0].current_mode, "100644");
    let previous = head(&connection).await;
    let restored = checkpoints::restore_files(&connection, review)
        .await
        .unwrap();
    assert_ne!(restored, previous);
    assert_ne!(Some(restored), saved.revision);
    assert_eq!(read(&connection).await, "checkpoint λ\r\n");
    let history = connection
        .call("workspace_history".into(), json!({"workspace":"chat"}))
        .await
        .unwrap();
    assert!(history.to_string().contains(&previous));
    assert_eq!(
        std::fs::read(root.join("file.txt")).unwrap(),
        b"original\r\n"
    );
    let repo = git2::Repository::open(&root).unwrap();
    assert!(
        !repo
            .references()
            .unwrap()
            .filter_map(Result::ok)
            .any(|r| r.name().is_ok_and(|n| n.starts_with("refs/vwork/")))
    );
    drop(connection);
    drop(temp);
}
#[tokio::test]
async fn v62_checkpoint_rejects_stale_incomplete_and_wrong_workspace_reviews() {
    let f = Fixture::new(1024).await;
    checkpoints::capture(&f.connection, Some("chat".into()), point("0001"))
        .await
        .unwrap();
    f.write("changed\n", "100644").await;
    let review = checkpoints::review(
        &f.connection,
        "session".into(),
        "0001".into(),
        Some("chat".into()),
    )
    .await
    .unwrap();
    f.write("concurrent\n", "100644").await;
    assert!(
        checkpoints::restore_files(&f.connection, review)
            .await
            .is_err()
    );
    assert_eq!(read(&f.connection).await, "concurrent\n");
    assert!(
        checkpoints::review(
            &f.connection,
            "session".into(),
            "0001".into(),
            Some("other".into())
        )
        .await
        .is_err()
    );
    f.write(&"large changed line\n".repeat(1000), "100644")
        .await;
    let mut review = checkpoints::review(
        &f.connection,
        "session".into(),
        "0001".into(),
        Some("chat".into()),
    )
    .await
    .unwrap();
    assert!(!review.complete);
    let head = f.head().await;
    assert!(
        checkpoints::restore_files(&f.connection, review.clone())
            .await
            .is_err()
    );
    review.complete = true;
    assert!(
        checkpoints::restore_files(&f.connection, review)
            .await
            .is_err()
    );
    assert_eq!(f.head().await, head);
}
#[tokio::test]
async fn v62_checkpoint_pages_and_identities_are_bounded_and_immutable() {
    let f = Fixture::new(1024 * 1024).await;
    for id in ["0001", "0002", "0003"] {
        checkpoints::capture(&f.connection, None, point(id))
            .await
            .unwrap();
    }
    let page = checkpoints::list(&f.connection, "session".into(), None, 2)
        .await
        .unwrap();
    assert_eq!(
        page.points
            .iter()
            .map(|p| p.capture.id.as_str())
            .collect::<Vec<_>>(),
        ["0003", "0002"]
    );
    let page = checkpoints::list(&f.connection, "session".into(), page.before, 2)
        .await
        .unwrap();
    assert_eq!(page.points[0].capture.id, "0001");
    assert!(page.before.is_none());
    assert!(
        checkpoints::list(&f.connection, "other".into(), None, 100)
            .await
            .unwrap()
            .points
            .is_empty()
    );
    let mut changed = point("0001");
    changed.label = "different".into();
    assert!(
        checkpoints::capture(&f.connection, None, changed)
            .await
            .is_err()
    );
    let mut huge = point("0004");
    huge.boundary = json!("x".repeat(64 * 1024));
    assert!(
        checkpoints::capture(&f.connection, None, huge)
            .await
            .is_err()
    );
    assert!(
        checkpoints::get(&f.connection, "session".into(), "0004".into())
            .await
            .is_err()
    );
    let review = checkpoints::review(&f.connection, "session".into(), "0001".into(), None)
        .await
        .unwrap();
    assert!(review.complete && review.files.is_empty());
    assert!(
        checkpoints::restore_files(&f.connection, review)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn v63_interrupted_restore_retains_backup_blocks_repeat_and_recovers_after_reopen() {
    use checkpoints::journal::{self, Journal};
    let f = Fixture::new(1024 * 1024).await;
    checkpoints::capture(&f.connection, Some("chat".into()), point("0001"))
        .await
        .unwrap();
    f.write("original before restore\n", "100644").await;
    let mut c = point("0002");
    c.phase = "restore-backup".into();
    let backup = checkpoints::capture(&f.connection, Some("chat".into()), c)
        .await
        .unwrap();
    let review = checkpoints::review(
        &f.connection,
        "session".into(),
        "0001".into(),
        Some("chat".into()),
    )
    .await
    .unwrap();
    let original = json!([{"id":"same-second-last","text":"retained conversation"}]);
    let record = Journal {
        backup: backup.clone(),
        target: review.checkpoint.clone(),
        files: true,
        conversation: true,
        original: original.clone(),
        status: "pending".into(),
    };
    assert!(
        journal::apply(
            &f.connection,
            review.clone(),
            record.clone(),
            || anyhow::bail!("simulated provider write failure")
        )
        .await
        .is_err()
    );
    assert_eq!(read(&f.connection).await, "original\r\n");
    assert!(
        journal::apply(&f.connection, review, record, || Ok(()))
            .await
            .is_err()
    );
    let Fixture {
        temp,
        root: _,
        home,
        connection,
    } = f;
    drop(connection);
    let connection = Connection::new(home);
    let pending = journal::pending(&connection, "session".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.original, original);
    assert_eq!(pending.backup, backup);
    let recovery = checkpoints::review(
        &connection,
        "session".into(),
        "0002".into(),
        Some("chat".into()),
    )
    .await
    .unwrap();
    // Another failure keeps the same original backup, even after restoring files.
    assert!(
        journal::resolve(
            &connection,
            recovery,
            "0002".into(),
            true,
            |_| anyhow::bail!("second interrupted write")
        )
        .await
        .is_err()
    );
    assert!(
        journal::pending(&connection, "session".into())
            .await
            .unwrap()
            .is_some()
    );
    let recovery = checkpoints::review(
        &connection,
        "session".into(),
        "0002".into(),
        Some("chat".into()),
    )
    .await
    .unwrap();
    journal::resolve(&connection, recovery, "0002".into(), true, move |j| {
        assert_eq!(j.original, original);
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(read(&connection).await, "original before restore\n");
    assert!(
        journal::pending(&connection, "session".into())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        journal::get(&connection, "session".into(), "0002".into())
            .await
            .unwrap()
            .status,
        "rolled-back"
    );
    drop(connection);
    drop(temp);
}

#[tokio::test]
async fn v63_restore_rejects_active_leases_and_keeps_reviewed_partial_state() {
    use checkpoints::journal::{self, Journal};
    let f = Fixture::new(1024 * 1024).await;
    checkpoints::capture(&f.connection, Some("chat".into()), point("0001"))
        .await
        .unwrap();
    f.write("before\n", "100644").await;
    let mut c = point("0002");
    c.phase = "restore-backup".into();
    let backup = checkpoints::capture(&f.connection, Some("chat".into()), c)
        .await
        .unwrap();
    let review = checkpoints::review(
        &f.connection,
        "session".into(),
        "0001".into(),
        Some("chat".into()),
    )
    .await
    .unwrap();
    let record = Journal {
        backup,
        target: review.checkpoint.clone(),
        files: true,
        conversation: true,
        original: json!("saved"),
        status: "pending".into(),
    };
    let lease = f
        .connection
        .call(
            "workspace_temporary".into(),
            json!({"workspace":"chat","purpose":"checkpoint test","expected_revision":f.head().await}),
        )
        .await
        .unwrap();
    assert!(
        journal::apply(&f.connection, review.clone(), record.clone(), || Ok(()))
            .await
            .is_err()
    );
    assert_eq!(read(&f.connection).await, "before\n");
    assert!(
        journal::pending(&f.connection, "session".into())
            .await
            .unwrap()
            .is_none()
    );
    f.connection
        .call("temporary_release".into(), json!({"lease":lease["id"]}))
        .await
        .unwrap();
    assert!(
        journal::apply(&f.connection, review, record, || anyhow::bail!(
            "provider failed"
        ))
        .await
        .is_err()
    );
    let recovery = checkpoints::review(
        &f.connection,
        "session".into(),
        "0002".into(),
        Some("chat".into()),
    )
    .await
    .unwrap();
    f.write("new reviewed content\n", "100644").await;
    assert!(
        journal::resolve(&f.connection, recovery, "0002".into(), false, |_| Ok(()))
            .await
            .is_err()
    );
    let recovery = checkpoints::review(
        &f.connection,
        "session".into(),
        "0002".into(),
        Some("chat".into()),
    )
    .await
    .unwrap();
    journal::resolve(&f.connection, recovery, "0002".into(), false, |_| Ok(()))
        .await
        .unwrap();
    assert_eq!(read(&f.connection).await, "new reviewed content\n");
    assert_eq!(
        journal::get(&f.connection, "session".into(), "0002".into())
            .await
            .unwrap()
            .status,
        "kept-current"
    );
}

#[tokio::test]
async fn v62_concurrent_turn_captures_share_the_store_without_exclusive_lock_failures() {
    let f = Fixture::new(1024 * 1024).await;
    let mut jobs = Vec::new();
    for i in 0..8 {
        let connection = f.connection.clone();
        jobs.push(tokio::spawn(async move {
            checkpoints::capture(&connection, Some("chat".into()), point(&format!("{i:04}"))).await
        }));
    }
    for job in jobs {
        job.await.unwrap().unwrap();
    }
    assert_eq!(
        checkpoints::list(&f.connection, "session".into(), None, 100)
            .await
            .unwrap()
            .points
            .len(),
        8
    );
}

#[tokio::test]
async fn v77_delete_checkpoints_preserves_pending_restore_and_workspace_source() {
    use checkpoints::journal::{self, Journal};
    let f = Fixture::new(1024 * 1024).await;
    let target = checkpoints::capture(&f.connection, Some("chat".into()), point("target"))
        .await
        .unwrap();
    let mut neighbor = point("neighbor");
    neighbor.session = "session-other".into();
    let neighbor = checkpoints::capture(&f.connection, Some("chat".into()), neighbor)
        .await
        .unwrap();
    f.write("keep this source\n", "100644").await;
    let backup = checkpoints::capture(&f.connection, Some("chat".into()), point("backup"))
        .await
        .unwrap();
    let review = checkpoints::review(
        &f.connection,
        "session".into(),
        "target".into(),
        Some("chat".into()),
    )
    .await
    .unwrap();
    let record = Journal {
        backup,
        target,
        files: false,
        conversation: true,
        original: json!("private history"),
        status: "pending".into(),
    };
    assert!(
        journal::apply(&f.connection, review, record, || anyhow::bail!(
            "interrupted provider"
        ))
        .await
        .is_err()
    );
    assert!(
        checkpoints::forget(&f.connection, "session".into())
            .await
            .is_err()
    );
    assert!(
        checkpoints::get(&f.connection, "session".into(), "target".into())
            .await
            .is_ok()
    );
    assert!(
        journal::pending(&f.connection, "session".into())
            .await
            .unwrap()
            .is_some()
    );
    let recovery = checkpoints::review(
        &f.connection,
        "session".into(),
        "backup".into(),
        Some("chat".into()),
    )
    .await
    .unwrap();
    journal::resolve(&f.connection, recovery, "backup".into(), false, |_| Ok(()))
        .await
        .unwrap();
    let revision = f.head().await;
    checkpoints::forget(&f.connection, "session".into())
        .await
        .unwrap();
    checkpoints::forget(&f.connection, "session".into())
        .await
        .unwrap();
    assert!(
        checkpoints::get(&f.connection, "session".into(), "target".into())
            .await
            .is_err()
    );
    assert!(
        checkpoints::get(&f.connection, "session".into(), "backup".into())
            .await
            .is_err()
    );
    assert!(
        journal::get(&f.connection, "session".into(), "backup".into())
            .await
            .is_err()
    );
    assert_eq!(
        checkpoints::get(&f.connection, "session-other".into(), "neighbor".into())
            .await
            .unwrap(),
        neighbor
    );
    assert_eq!(f.head().await, revision);
    assert_eq!(read(&f.connection).await, "keep this source\n");
    assert_eq!(
        std::fs::read(f.root.join("file.txt")).unwrap(),
        b"original\r\n"
    );
}
