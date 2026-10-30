use base64::Engine as _;
use fastrock_workspaces::{
    embedded::Connection,
    integration::{self, MergeResult},
};
use serde_json::json;

struct Fixture {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    connection: Connection,
}

impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let repo = git2::Repository::init(&root).unwrap();
        std::fs::write(root.join("file.txt"), "original\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("file.txt")).unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "Fixture", &tree, &[])
            .unwrap();
        let connection = Connection::new(temp.path().join("state"));
        connection
            .call("repo_register".into(), json!({"repo":"repo","path":root}))
            .await
            .unwrap();
        connection
            .call(
                "workspace_create".into(),
                json!({"repo":"repo","workspace":"parent"}),
            )
            .await
            .unwrap();
        connection
            .call(
                "workspace_fork".into(),
                json!({"workspace":"parent","name":"child"}),
            )
            .await
            .unwrap();
        Self {
            _temp: temp,
            root,
            connection,
        }
    }

    async fn head(&self, workspace: &str) -> String {
        self.connection
            .call("workspace_info".into(), json!({"workspace":workspace}))
            .await
            .unwrap()["revision"]
            .as_str()
            .unwrap()
            .into()
    }

    async fn write(&self, workspace: &str, text: &str, mode: &str) {
        self.connection.call("files_write".into(), json!({"workspace":workspace,"path":"file.txt","content":text,"mode":mode,"expected_revision":self.head(workspace).await})).await.unwrap();
    }

    async fn read(&self, workspace: &str) -> String {
        let value = self
            .connection
            .call(
                "files_read".into(),
                json!({"workspace":workspace,"path":"file.txt","binary":true}),
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
}

#[tokio::test]
async fn v49_review_is_read_only_and_merge_preserves_source_and_checkout() {
    let fixture = Fixture::new().await;
    fixture.write("child", "λ — child\r\n", "100755").await;
    let parent = fixture.head("parent").await;
    let child = fixture.head("child").await;
    let review = integration::review(&fixture.connection, "parent".into(), "child".into())
        .await
        .unwrap();
    assert!(review.complete);
    assert!(review.report.contains("λ — child"));
    assert!(review.report.contains("100644 → 100755"));
    assert_eq!(fixture.head("parent").await, parent);
    assert_eq!(fixture.read("parent").await, "original\n");
    let result = integration::apply(&fixture.connection, review.clone())
        .await
        .unwrap();
    assert!(matches!(result, MergeResult::Merged { .. }));
    assert_eq!(fixture.head("child").await, child);
    assert_eq!(fixture.read("parent").await, "λ — child\r\n");
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("file.txt")).unwrap(),
        "original\n"
    );
    assert!(
        integration::apply(&fixture.connection, review)
            .await
            .unwrap_err()
            .to_string()
            .contains("changed after review")
    );
}

#[tokio::test]
async fn v49_either_changed_head_rejects_merge_without_mutation() {
    for changed in ["parent", "child"] {
        let fixture = Fixture::new().await;
        fixture.write("child", "reviewed\n", "100644").await;
        let review = integration::review(&fixture.connection, "parent".into(), "child".into())
            .await
            .unwrap();
        fixture.write(changed, "unreviewed\n", "100644").await;
        let parent = fixture.head("parent").await;
        let child = fixture.head("child").await;
        assert!(
            integration::apply(&fixture.connection, review)
                .await
                .unwrap_err()
                .to_string()
                .contains("changed after review")
        );
        assert_eq!(fixture.head("parent").await, parent);
        assert_eq!(fixture.head("child").await, child);
    }
}

#[tokio::test]
async fn v49_conflicts_leave_both_workspaces_unchanged() {
    let fixture = Fixture::new().await;
    fixture.write("parent", "parent change\n", "100644").await;
    fixture.write("child", "child change\n", "100644").await;
    let parent = fixture.head("parent").await;
    let child = fixture.head("child").await;
    let review = integration::review(&fixture.connection, "parent".into(), "child".into())
        .await
        .unwrap();
    assert!(review.report.contains("parent change") && review.report.contains("child change"));
    assert_eq!(
        integration::apply(&fixture.connection, review)
            .await
            .unwrap(),
        MergeResult::Conflicts {
            paths: vec!["file.txt".into()]
        }
    );
    assert_eq!(fixture.head("parent").await, parent);
    assert_eq!(fixture.head("child").await, child);
}

#[tokio::test]
async fn v49_mode_only_changes_are_reviewed_and_large_reviews_cannot_apply() {
    let fixture = Fixture::new().await;
    fixture.write("child", "original\n", "100755").await;
    let review = integration::review(&fixture.connection, "parent".into(), "child".into())
        .await
        .unwrap();
    assert!(review.complete && review.report.contains("100644 → 100755"));
    fixture
        .write("child", &"λ large change\n".repeat(12000), "100755")
        .await;
    let review = integration::review(&fixture.connection, "parent".into(), "child".into())
        .await
        .unwrap();
    assert!(!review.complete);
    assert!(review.report.len() < 129 * 1024);
    assert!(
        integration::apply(&fixture.connection, review)
            .await
            .is_err()
    );
    assert_eq!(fixture.read("parent").await, "original\n");
}

#[tokio::test]
async fn v49_review_excludes_active_operations_and_rejects_unrelated_workspaces() {
    let fixture = Fixture::new().await;
    assert!(
        integration::review(&fixture.connection, "parent".into(), "parent".into())
            .await
            .is_err()
    );
    fixture
        .connection
        .call(
            "workspace_create".into(),
            json!({"repo":"repo","workspace":"unrelated"}),
        )
        .await
        .unwrap();
    assert!(
        integration::review(&fixture.connection, "parent".into(), "unrelated".into())
            .await
            .unwrap_err()
            .to_string()
            .contains("No shared")
    );
    let active = fixture.connection.clone();
    fixture
        .connection
        .store()
        .await
        .unwrap()
        .with_engine(|_| {
            let result = std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(integration::review(
                        &active,
                        "parent".into(),
                        "child".into(),
                    ))
            })
            .join()
            .unwrap();
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("operation is active")
            );
            Ok(())
        })
        .unwrap();
}

#[tokio::test]
async fn v50_workspace_workdirs_are_stable_separate_and_preserve_checkout() {
    use fastrock_workspaces::session::Selection;
    let f = Fixture::new().await;
    let selected = |name: &str| Selection {
        home: f._temp.path().join("state"),
        workspace: name.into(),
    };
    let parent = selected("parent").resolve().await.unwrap();
    let child = selected("child").resolve().await.unwrap();
    assert_eq!(parent, selected("parent").resolve().await.unwrap());
    assert_ne!(parent.cwd, child.cwd);
    assert!(parent.cwd.starts_with(&parent.selection.home));
    assert!(!parent.cwd.starts_with(&f.root));
    std::fs::write(child.cwd.join("scratch.txt"), "child only").unwrap();
    assert!(!parent.cwd.join("scratch.txt").exists());
    assert_eq!(
        std::fs::read_to_string(f.root.join("file.txt")).unwrap(),
        "original\n"
    );
    assert!(selected("missing").resolve().await.is_err());
}

#[tokio::test]
async fn v51_register_reuses_repository_and_imports_head_without_dirty_files() {
    use fastrock_workspaces::projects;
    let f = Fixture::new().await;
    std::fs::write(f.root.join("file.txt"), "uncommitted source").unwrap();
    std::fs::write(f.root.join("untracked.txt"), "not committed").unwrap();
    let root = projects::root(f.root.clone()).await.unwrap().unwrap();
    let selected = projects::open(&f.connection, root.clone(), "new-chat".into())
        .await
        .unwrap();
    assert_eq!(selected.repo, "repo");
    assert_eq!(f.read("new-chat").await, "original\n");
    assert_eq!(
        std::fs::read_to_string(f.root.join("file.txt")).unwrap(),
        "uncommitted source"
    );
    assert_eq!(projects::list(&f.connection).await.unwrap().len(), 1);
    let fresh = Connection::new(f._temp.path().join("fresh"));
    let first = projects::open(&fresh, root.clone(), "first".into())
        .await
        .unwrap();
    let second = projects::open(&fresh, root, "second".into()).await.unwrap();
    assert_eq!(first.repo, second.repo);
    assert_ne!(first.cwd, second.cwd);
    let registered = projects::list(&fresh).await.unwrap();
    assert_eq!(registered.len(), 1);
    assert_eq!(
        registered[0].id,
        projects::id(&first.selection.home, &first.repo)
    );
    assert!(
        projects::root(f._temp.path().join("fresh"))
            .await
            .unwrap()
            .is_none()
    );
}
