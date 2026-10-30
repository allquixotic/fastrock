use fastrock_workspaces::{
    embedded::Connection,
    summaries::{Summaries, Summary},
    vwork::{Installation, VworkProvider},
    *,
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn v40_hook_uses_original_vwork_policy_without_a_daemon() {
    for (command, decision) in [
        ("git clone https://example.invalid/repo", "block"),
        ("git status", "allow"),
    ] {
        let input =
            json!({"tool_name":"developer__shell","tool_input":{"command":command}}).to_string();
        let mut output = Vec::new();
        fastrock_workspaces::embedded::run_hook(input.as_bytes(), &mut output).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output).unwrap()["decision"],
            decision
        );
    }
}

#[tokio::test]
async fn v40_one_owner_and_no_daemon_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("state");
    let a = Connection::new(home.clone());
    let b = Connection::new(home.clone());
    let (a, b) = tokio::join!(a.store(), b.store());
    assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
    assert!(home.join("workspaces.redb").exists());
    assert!(!home.join("connection.json").exists());
}

#[tokio::test]
async fn v40_summaries_replace_atomically_and_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let connection = Connection::new(temp.path().to_owned());
    let summaries = Summaries(connection.clone());
    let text = "Distinctive keyword λ and unmodified source. ".repeat(2000);
    summaries.request("thread".into(), 1).await.unwrap();
    assert!(
        summaries
            .replace(
                "thread".into(),
                Summary {
                    revision: 1,
                    text: text.clone(),
                    profile: "cheap".into(),
                    ..Summary::default()
                }
            )
            .await
            .unwrap()
    );
    summaries.request("thread".into(), 3).await.unwrap();
    summaries.request("thread".into(), 2).await.unwrap();
    assert!(
        !summaries
            .replace(
                "thread".into(),
                Summary {
                    revision: 2,
                    text: "stale".into(),
                    profile: "cheap".into(),
                    ..Summary::default()
                }
            )
            .await
            .unwrap()
    );
    assert_eq!(
        summaries.get("thread".into()).await.unwrap().unwrap().text,
        text
    );
    assert!(
        summaries
            .replace(
                "thread".into(),
                Summary {
                    revision: 3,
                    text: "latest only".into(),
                    profile: "cheap".into(),
                    ..Summary::default()
                }
            )
            .await
            .unwrap()
    );
    assert_eq!(summaries.page(None, 100).await.unwrap().len(), 1);
    assert_eq!(summaries.advance("thread".into()).await.unwrap(), 4);
    drop(summaries);
    drop(connection);
    let reopened = Summaries(Connection::new(temp.path().to_owned()));
    assert_eq!(reopened.advance("thread".into()).await.unwrap(), 5);
    assert_eq!(
        reopened.get("thread".into()).await.unwrap().unwrap().text,
        "latest only"
    );
    assert_eq!(
        reopened
            .page(Some("thread".into()), 100)
            .await
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn v40_compaction_excludes_readers_and_summaries() {
    let temp = tempfile::tempdir().unwrap();
    let connection = Connection::new(temp.path().to_owned());
    let store = connection.store().await.unwrap();
    let active = connection.clone();
    store
        .with_engine(|_| {
            let result = std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(active.call("database_compact".into(), json!({})))
            })
            .join()
            .unwrap()?;
            assert_eq!(result["reason"], "busy");
            Ok(())
        })
        .unwrap();
    assert_eq!(
        connection
            .call("database_compact".into(), json!({}))
            .await
            .unwrap()["reason"],
        "below_threshold"
    );
}

#[tokio::test]
async fn v40_embedded_workspace_revision_guard_and_utf8() {
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
    let home = temp.path().join("state");
    let connection = Connection::new(home.clone());
    connection
        .call("repo_register".into(), json!({"repo":"repo","path":root}))
        .await
        .unwrap();
    connection
        .call(
            "workspace_create".into(),
            json!({"repo":"repo","workspace":"one"}),
        )
        .await
        .unwrap();
    let provider = VworkProvider::new(&Installation {
        id: "state".into(),
        label: "state".into(),
        home,
    })
    .unwrap();
    let Reply::Files(page) = provider
        .execute(Request::Files {
            workspace: "one".into(),
            prefix: "".into(),
            offset: 0,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let Reply::Document(mut document) = provider
        .execute(Request::Read {
            workspace: "one".into(),
            file: page.entries[0].clone(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    document.text = "λ — edited\n".into();
    provider
        .execute(Request::Write {
            workspace: "one".into(),
            document: document.clone(),
        })
        .await
        .unwrap();
    let error = provider
        .execute(Request::Write {
            workspace: "one".into(),
            document,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("revision"));
    assert_eq!(
        std::fs::read_to_string(root.join("file.txt")).unwrap(),
        "original\n"
    );
    let Reply::Document(saved) = provider
        .execute(Request::Read {
            workspace: "one".into(),
            file: page.entries[0].clone(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(saved.text, "λ — edited\n");
}

#[tokio::test]
async fn v77_deleted_summary_cannot_be_resurrected_by_late_completion() {
    let temp = tempfile::tempdir().unwrap();
    let summaries = Summaries(Connection::new(temp.path().to_owned()));
    let old = Summary {
        revision: 1,
        text: "private history summary".into(),
        ..Default::default()
    };
    for id in ["goose:delete", "goose:keep"] {
        summaries.request(id.into(), 1).await.unwrap();
        assert!(summaries.replace(id.into(), old.clone()).await.unwrap());
    }
    summaries.forget("goose:delete".into()).await.unwrap();
    assert!(
        !summaries
            .replace("goose:delete".into(), old.clone())
            .await
            .unwrap()
    );
    assert!(summaries.advance("goose:delete".into()).await.is_err());
    summaries.request("goose:delete".into(), 2).await.unwrap();
    assert!(
        !summaries
            .replace(
                "goose:delete".into(),
                Summary {
                    revision: 2,
                    ..old.clone()
                }
            )
            .await
            .unwrap()
    );
    assert!(
        summaries
            .get("goose:delete".into())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(summaries.get("goose:keep".into()).await.unwrap(), Some(old));
    assert_eq!(summaries.page(None, 10).await.unwrap().len(), 1);
}
