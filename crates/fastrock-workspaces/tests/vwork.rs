use base64::Engine;
use fastrock_workspaces::{vwork::*, *};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn v40_discovery_uses_existing_store_or_default_without_installed_vwork() {
    let dir = tempfile::tempdir().unwrap();
    let executable = dir
        .path()
        .join(if cfg!(windows) { "vwork.exe" } else { "vwork" });
    std::fs::write(&executable, "never execute this file").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let config = dir.path().join("config.yaml");
    let paths = DiscoveryPaths {
        user_home: dir.path().into(),
        configs: vec![config.clone()],
        search: vec![dir.path().into()],
        default_vwork_home: dir.path().join("state"),
    };
    assert_eq!(
        discover(&paths).await.unwrap()[0].home,
        paths.default_vwork_home
    );
    for enabled in [false, true] {
        std::fs::write(&config, serde_yaml::to_string(&json!({"extensions":{"virtual":{"enabled":enabled,"type":"stdio","cmd":executable,"args":["--home",dir.path().join("other-state"),"mcp","--workspace","one"]}}})).unwrap()).unwrap();
        let found = discover(&paths).await.unwrap();
        assert_eq!(found.len(), 1);
        if enabled {
            assert_eq!(found[0].home, dir.path().join("other-state"));
        }
    }
    std::fs::remove_file(executable).unwrap();
    assert_eq!(discover(&paths).await.unwrap().len(), 1);
}

#[test]
fn v22_loopback_rejects_token_leak_destinations() {
    assert!(validate_loopback("http://127.0.0.1:12345/").is_ok());
    for url in [
        "https://127.0.0.1:123/",
        "http://127.0.0.1:123@evil.test/",
        "http://evil.test/",
        "http://127.0.0.1:0/",
        "http://127.0.0.1:123/path",
    ] {
        assert!(validate_loopback(url).is_err(), "{url}");
    }
}

async fn fixture(
    responses: Vec<Value>,
) -> (
    tempfile::TempDir,
    Installation,
    Arc<Mutex<Vec<Value>>>,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    std::fs::write(
        dir.path().join("connection.json"),
        json!({"url":url,"token":"fixture-only-token"}).to_string(),
    )
    .unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let task = tokio::spawn(async move {
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let split = loop {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break split + 4;
                }
            };
            let header = String::from_utf8_lossy(&bytes[..split]).to_lowercase();
            assert!(header.contains("authorization: bearer fixture-only-token"));
            let len = header
                .lines()
                .find_map(|s| s.strip_prefix("content-length: "))
                .unwrap()
                .trim()
                .parse::<usize>()
                .unwrap();
            while bytes.len() < split + len {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            recorded
                .lock()
                .unwrap()
                .push(serde_json::from_slice(&bytes[split..split + len]).unwrap());
            let body = response.to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
        }
    });
    let installation = Installation {
        id: "test".into(),
        label: "test".into(),
        home: dir.path().into(),
    };
    (dir, installation, requests, task)
}

#[tokio::test]
async fn v23_exact_bytes_modes_pagination_and_conflict() {
    let text = "\u{feff}one\r\ntwo\r\n";
    let entry = json!({"path":"script.rs","size":text.len(),"mode":"100755"});
    let (_dir, installation, requests, task)=fixture(vec![
        json!({"result":{"revision":"r1","entries":[entry.clone()],"total":100000,"next_offset":200}}),
        json!({"result":{"revision":"r1","entries":[entry]}}),
        json!({"result":{"revision":"r1","base64":base64::engine::general_purpose::STANDARD.encode(text)}}),
        json!({"error":"revision conflict"}),
    ]).await;
    let provider = VworkProvider::fixture(&installation).unwrap();
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
    assert_eq!(page.total, 100000);
    assert_eq!(page.next, Some(200));
    let Reply::Document(document) = provider
        .execute(Request::Read {
            workspace: "one".into(),
            file: page.entries[0].clone(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(document.text, text);
    assert_eq!(document.mode, "100755");
    let error = provider
        .execute(Request::Write {
            workspace: "one".into(),
            document,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("revision conflict"));
    task.await.unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests[0]["arguments"]["limit"], PAGE_SIZE);
    assert_eq!(requests[3]["arguments"]["expected_revision"], "r1");
    assert_eq!(requests[3]["arguments"]["mode"], "100755");
    assert_eq!(
        requests[3]["arguments"]["base64"],
        base64::engine::general_purpose::STANDARD.encode(text)
    );
}

#[tokio::test]
async fn v23_rejects_nontext_unsafe_paths_and_changed_reads() {
    for (size, mode) in [(MAX_TEXT_BYTES as u64 + 1, "100644"), (3, "120000")] {
        let (_dir, installation, _, task) = fixture(vec![
            json!({"result":{"revision":"r1","entries":[{"path":"file","size":size,"mode":mode}]}}),
        ])
        .await;
        let provider = VworkProvider::fixture(&installation).unwrap();
        let result = provider
            .execute(Request::Read {
                workspace: "one".into(),
                file: FileEntry {
                    path: "file".into(),
                    size: 0,
                    mode: "100644".into(),
                },
            })
            .await;
        assert!(matches!(result, Err(WorkspaceError::NotEditable)));
        task.await.unwrap();
    }
    for raw in [b"a\0b".as_slice(), b"\xff\xfe".as_slice()] {
        let (_dir,installation,_,task)=fixture(vec![
            json!({"result":{"revision":"r1","entries":[{"path":"file","size":raw.len(),"mode":"100644"}]}}),
            json!({"result":{"revision":"r1","base64":base64::engine::general_purpose::STANDARD.encode(raw)}}),
        ]).await;
        let result = VworkProvider::fixture(&installation)
            .unwrap()
            .execute(Request::Read {
                workspace: "one".into(),
                file: FileEntry {
                    path: "file".into(),
                    size: 0,
                    mode: "100644".into(),
                },
            })
            .await;
        assert!(matches!(result, Err(WorkspaceError::NotEditable)));
        task.await.unwrap();
    }
    let (_dir, installation, _, task) = fixture(vec![
        json!({"result":{"revision":"r1","entries":[{"path":"file","size":1,"mode":"100644"}]}}),
        json!({"result":{"revision":"r2","base64":"YQ=="}}),
    ])
    .await;
    let result = VworkProvider::fixture(&installation)
        .unwrap()
        .execute(Request::Read {
            workspace: "one".into(),
            file: FileEntry {
                path: "file".into(),
                size: 1,
                mode: "100644".into(),
            },
        })
        .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("changed while opening")
    );
    task.await.unwrap();
    for path in [
        "../file",
        "/etc/passwd",
        "C:/file",
        "src\\file",
        ".git/config",
    ] {
        assert!(validate_path(path, false).is_err());
    }
}

#[tokio::test]
async fn v22_discovery_deduplicates_stores_and_ignores_other_commands() {
    let dir = tempfile::tempdir().unwrap();
    let executable = dir
        .path()
        .join(if cfg!(windows) { "vwork.exe" } else { "vwork" });
    std::fs::write(&executable, "never execute").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let home = dir.path().join("state");
    let valid = json!({"enabled":true,"type":"stdio","cmd":executable,"args":[format!("--home={}",home.display()),"mcp"]});
    let config = dir.path().join("config.yaml");
    std::fs::write(&config,serde_yaml::to_string(&json!({"extensions":{
        "one":valid,"same-store":valid,
        "wrong-command":{"enabled":true,"type":"stdio","cmd":"sh","args":["mcp"]},
        "not-mcp":{"enabled":true,"type":"stdio","cmd":executable,"args":["serve"]},
        "another-store":{"enabled":true,"type":"stdio","cmd":executable,"args":["--home",dir.path().join("other"),"mcp"]}
    }})).unwrap()).unwrap();
    let paths = DiscoveryPaths {
        user_home: dir.path().into(),
        configs: vec![config],
        search: vec![],
        default_vwork_home: home,
    };
    let found = discover(&paths).await.unwrap();
    assert_eq!(found.len(), 2);
    assert_ne!(found[0].home, found[1].home);
}

#[tokio::test]
async fn v30_compaction_rpc_reports_skips_success_and_errors() {
    let (_dir, installation, requests, task) = fixture(vec![
        json!({"result":{"reason":"busy"}}),
        json!({"result":{"reason":"below_threshold","threshold_bytes":1_000_000_000}}),
        json!({"result":{"reason":"compacted","reclaimed_bytes":1_200_000_000,"after_bytes":680_000_000}}),
        json!({"error":"database maintenance failed; restart vwork for recovery"}),
    ]).await;
    let provider = VworkProvider::fixture(&installation).unwrap();
    for expected in ["active", "threshold", "1.20 GB"] {
        let Reply::Report(report) = provider.execute(Request::Compact).await.unwrap() else {
            panic!()
        };
        assert!(report.contains(expected), "{report}");
    }
    assert!(
        provider
            .execute(Request::Compact)
            .await
            .unwrap_err()
            .to_string()
            .contains("recovery")
    );
    task.await.unwrap();
    for request in requests.lock().unwrap().iter() {
        assert_eq!(request["operation"], "database_compact");
        assert_eq!(request["arguments"], json!({}));
    }
}

#[tokio::test]
async fn v38_mcp_discovery_rejects_external_vwork_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let executable = dir
        .path()
        .join(if cfg!(windows) { "vwork.exe" } else { "vwork" });
    std::fs::write(&executable, "never execute").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let config = dir.path().join("config.yaml");
    let paths = DiscoveryPaths {
        user_home: dir.path().into(),
        configs: vec![config.clone()],
        search: vec![],
        default_vwork_home: dir.path().join("state"),
    };
    let valid = json!({"enabled":true,"type":"stdio","name":"Goose.vwork","cmd":executable,"args":["mcp","--workspace=one","--no-auto-start"],"timeout":900});
    for (field, value) in [
        ("envs", json!({"VWORK_HOME":"custom"})),
        ("env_keys", json!(["KEY"])),
        ("available_tools", json!(["workspace_info"])),
        ("args", json!(["mcp", "--unknown"])),
    ] {
        let mut extension = valid.clone();
        extension[field] = value;
        std::fs::write(
            &config,
            json!({"extensions":{"vwork":extension}}).to_string(),
        )
        .unwrap();
        assert!(discover_mcp(&paths).await.is_err(), "{field}");
    }
    std::fs::write(config, json!({"extensions":{"vwork":valid}}).to_string()).unwrap();
    let bindings = discover_mcp(&paths).await.unwrap();
    assert_eq!(bindings.len(), 1);
    let b = &bindings[0];
    assert_eq!(b.name, "Goose.vwork");
    assert_eq!(b.workspace.as_deref(), Some("one"));
    assert!(!b.auto_start);
    assert_eq!(b.timeout_seconds, 900);
    assert!(b.matches_stdio(&json!({"name":b.name,"command":b.command,"args":b.args,"env":[]})));
    assert!(!b.matches_stdio(
        &json!({"name":b.name,"command":b.command,"args":["mcp","--workspace=two"],"env":[]})
    ));
}
