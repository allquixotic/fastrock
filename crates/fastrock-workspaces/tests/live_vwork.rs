//! Explicit opt-in contract test against a separately compiled vwork release.
use fastrock_workspaces::{vwork::*, *};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn call(binary: &Path, home: &Path, op: &str, args: Value) -> Value {
    let output = Command::new(binary)
        .arg("--home")
        .arg(home)
        .args(["call", op, "--json"])
        .arg(args.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
#[ignore = "set FASTROCK_TEST_VWORK to a compiled vwork 0.2+ binary and run --ignored"]
async fn v23_live_vwork_roundtrip_does_not_export_files() {
    let binary = std::env::var_os("FASTROCK_TEST_VWORK").expect("FASTROCK_TEST_VWORK is required");
    let binary = Path::new(&binary);
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("source");
    std::fs::create_dir(&repo).unwrap();
    let original = "\u{feff}from git\r\n";
    std::fs::write(repo.join("file.txt"), original).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "fixture"]);
    let home = dir.path().join("state");
    let _daemon = Daemon(
        Command::new(binary)
            .arg("--home")
            .arg(&home)
            .arg("serve")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    for _ in 0..100 {
        if home.join("connection.json").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    call(
        binary,
        &home,
        "repo_register",
        json!({"repo":"mono","path":repo}),
    );
    let provider = VworkProvider::new(&Installation {
        id: "live".into(),
        label: "live".into(),
        home: home.clone(),
    })
    .unwrap();
    let Reply::Created(ws) = provider
        .execute(Request::Create {
            repo: "mono".into(),
            name: "gui".into(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(ws.id, "gui");
    let Reply::Catalog {
        repositories,
        workspaces,
    } = provider.execute(Request::Catalog).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(repositories[0].id, "mono");
    assert_eq!(workspaces[0].id, "gui");
    let Reply::Files(page) = provider
        .execute(Request::Files {
            workspace: "gui".into(),
            prefix: "".into(),
            offset: 0,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(page.entries.len(), 1);
    let Reply::Document(mut doc) = provider
        .execute(Request::Read {
            workspace: "gui".into(),
            file: page.entries[0].clone(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(doc.text, original);
    doc.text = "\u{feff}edited in Fastrock\r\n".into();
    let stale = doc.clone();
    assert!(matches!(
        provider
            .execute(Request::Write {
                workspace: "gui".into(),
                document: doc
            })
            .await
            .unwrap(),
        Reply::Saved { .. }
    ));
    assert!(
        provider
            .execute(Request::Write {
                workspace: "gui".into(),
                document: stale
            })
            .await
            .is_err()
    );
    let Reply::Document(readback) = provider
        .execute(Request::Read {
            workspace: "gui".into(),
            file: page.entries[0].clone(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(readback.text, "\u{feff}edited in Fastrock\r\n");
    assert!(matches!(
        provider
            .execute(Request::Fork {
                workspace: "gui".into(),
                name: "fork".into()
            })
            .await
            .unwrap(),
        Reply::Created(_)
    ));
    let Reply::Report(diff) = provider
        .execute(Request::Diff {
            workspace: "gui".into(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(diff.contains("edited in Fastrock"));
    assert!(matches!(
        provider
            .execute(Request::Size {
                workspace: "gui".into()
            })
            .await
            .unwrap(),
        Reply::Report(_)
    ));
    assert_eq!(
        std::fs::read(repo.join("file.txt")).unwrap(),
        original.as_bytes()
    );
    // No materialization/build operation was called: source remains untouched.
    assert!(!home.join("build").exists());
}
