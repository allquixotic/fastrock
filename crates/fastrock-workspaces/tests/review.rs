use fastrock_workspaces::{
    embedded::Connection,
    review::{self, Target},
    session::Selection,
};
use serde_json::json;
use std::path::Path;

fn commit(repo: &git2::Repository, message: &str) -> git2::Oid {
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let parent = repo.head().ok().map(|r| r.peel_to_commit().unwrap());
    let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        message,
        &tree,
        &parent.iter().collect::<Vec<_>>(),
    )
    .unwrap()
}
#[tokio::test]
async fn v72_review_local_targets_include_dirty_untracked_and_root_commit() {
    let temp = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init_opts(
        temp.path(),
        git2::RepositoryInitOptions::new().initial_head("feature"),
    )
    .unwrap();
    std::fs::write(temp.path().join("a.txt"), "original\n").unwrap();
    let first = commit(&repo, "First");
    repo.branch("main", &repo.find_commit(first).unwrap(), true)
        .unwrap();
    std::fs::write(temp.path().join("a.txt"), "committed\n").unwrap();
    let second = commit(&repo, "Second");
    std::fs::write(temp.path().join("a.txt"), "dirty\n").unwrap();
    std::fs::write(temp.path().join("extra.txt"), "untracked\n").unwrap();
    let cwd = temp.path().to_owned();
    let catalog = review::list(None, cwd.clone()).await.unwrap();
    assert_eq!(catalog.branches[0], "main");
    assert_eq!(catalog.commits[0].sha, second.to_string());
    let dirty = review::context(None, cwd.clone(), Target::UncommittedChanges)
        .await
        .unwrap();
    assert!(
        dirty.contains("-committed") && dirty.contains("+dirty") && dirty.contains("+untracked"),
        "{dirty}"
    );
    let base = review::context(
        None,
        cwd.clone(),
        Target::BaseBranch {
            branch: "main".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        base.contains("-original") && base.contains("+dirty"),
        "{base}"
    );
    let selected = review::context(
        None,
        cwd.clone(),
        Target::Commit {
            sha: second.to_string(),
        },
    )
    .await
    .unwrap();
    assert!(
        selected.contains("-original")
            && selected.contains("+committed")
            && !selected.contains("+dirty"),
        "{selected}"
    );
    let root = review::context(
        None,
        cwd.clone(),
        Target::Commit {
            sha: first.to_string(),
        },
    )
    .await
    .unwrap();
    assert!(
        root.contains("empty tree") && root.contains("+original"),
        "{root}"
    );
    assert!(
        review::context(None, cwd.clone(), Target::Commit { sha: "HEAD".into() })
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(cwd.join("a.txt")).unwrap(),
        "dirty\n"
    );
    std::fs::write(cwd.join("huge"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
    assert!(
        review::context(None, cwd, Target::UncommittedChanges)
            .await
            .unwrap_err()
            .to_string()
            .contains("1 MiB")
    );
}

#[tokio::test]
async fn v72_review_virtual_targets_use_compressed_revision_without_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    let repo = git2::Repository::init_opts(
        &root,
        git2::RepositoryInitOptions::new().initial_head("feature"),
    )
    .unwrap();
    std::fs::write(root.join("a.txt"), "original\n").unwrap();
    let first = commit(&repo, "First");
    repo.branch("main", &repo.find_commit(first).unwrap(), true)
        .unwrap();
    std::fs::write(root.join("a.txt"), "committed\n").unwrap();
    let second = commit(&repo, "Second");
    let home = temp.path().join("store");
    let connection = Connection::new(home.clone());
    connection
        .call("repo_register".into(), json!({"repo":"review","path":root}))
        .await
        .unwrap();
    connection
        .call(
            "workspace_create".into(),
            json!({"repo":"review","workspace":"chat"}),
        )
        .await
        .unwrap();
    let head = connection
        .call("workspace_info".into(), json!({"workspace":"chat"}))
        .await
        .unwrap()["revision"]
        .clone();
    connection.call("files_write".into(), json!({"workspace":"chat","path":"a.txt","content":"virtual only\n","expected_revision":head})).await.unwrap();
    let head = connection
        .call("workspace_info".into(), json!({"workspace":"chat"}))
        .await
        .unwrap()["revision"]
        .clone();
    let selection = Some(Selection {
        home: home.clone(),
        workspace: "chat".into(),
    });
    let catalog = review::list(selection.clone(), root.clone()).await.unwrap();
    assert_eq!(catalog.branches[0], "main");
    assert!(catalog.current.is_none());
    assert_eq!(catalog.commits[0].sha, second.to_string());
    let dirty = review::context(selection.clone(), root.clone(), Target::UncommittedChanges)
        .await
        .unwrap();
    assert!(
        dirty.contains("-committed")
            && dirty.contains("+virtual only")
            && dirty.contains(head.as_str().unwrap()),
        "{dirty}"
    );
    let base = review::context(
        selection.clone(),
        root.clone(),
        Target::BaseBranch {
            branch: "main".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        base.contains("-original") && base.contains("+virtual only"),
        "{base}"
    );
    let selected = review::context(
        selection.clone(),
        root.clone(),
        Target::Commit {
            sha: second.to_string(),
        },
    )
    .await
    .unwrap();
    assert!(
        selected.contains("-original")
            && selected.contains("+committed")
            && !selected.contains("+virtual only"),
        "{selected}"
    );
    assert!(
        review::context(
            selection,
            root.clone(),
            Target::BaseBranch {
                branch: "missing".into()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        connection
            .call("workspace_info".into(), json!({"workspace":"chat"}))
            .await
            .unwrap()["revision"],
        head
    );
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "committed\n"
    );
    assert!(!home.join("fastrock-agent-workdirs").exists());
    assert!(!Path::new(&home).join("checkouts").exists());
}

#[tokio::test]
async fn v72_review_limits_fail_without_silent_truncation() {
    let temp = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init_opts(
        temp.path(),
        git2::RepositoryInitOptions::new().initial_head("feature"),
    )
    .unwrap();
    std::fs::write(temp.path().join("a"), "initial\n").unwrap();
    commit(&repo, "First");
    let cwd = temp.path().to_owned();
    for i in 0..101 {
        std::fs::write(cwd.join(format!("untracked-{i}")), "new\n").unwrap();
    }
    assert!(
        review::context(None, cwd.clone(), Target::UncommittedChanges)
            .await
            .unwrap_err()
            .to_string()
            .contains("100 changed")
    );
    for i in 0..101 {
        std::fs::remove_file(cwd.join(format!("untracked-{i}"))).unwrap();
    }
    std::fs::write(cwd.join("a"), "a new line\n".repeat(30_000)).unwrap();
    assert!(
        review::context(None, cwd.clone(), Target::UncommittedChanges)
            .await
            .unwrap_err()
            .to_string()
            .contains("256 KiB")
    );
    assert!(
        review::context(
            None,
            cwd.clone(),
            Target::Custom {
                instructions: " ".into()
            }
        )
        .await
        .is_err()
    );
    assert!(
        review::context(
            None,
            cwd.clone(),
            Target::Custom {
                instructions: "x".repeat(65537)
            }
        )
        .await
        .is_err()
    );
    std::fs::write(cwd.join("a"), b"binary\0data").unwrap();
    assert!(
        review::context(None, cwd, Target::UncommittedChanges)
            .await
            .unwrap()
            .contains("Binary contents changed")
    );
}
