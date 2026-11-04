use super::*;

#[tokio::test]
async fn v94_tool_discovery_does_not_open_a_locked_store() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("store");
    std::fs::create_dir(&home).unwrap();
    // Model an existing tray process, with its actual exclusive Fjall lock.
    let owner =
        vwork_engine::engine::Engine::open_embedded(&home, vwork_engine::model::Config::default())
            .unwrap();
    let provider = VworkProvider::new(&Installation {
        id: "test".into(),
        label: "test".into(),
        home: home.clone(),
    })
    .unwrap();
    assert!(provider.shared_mcp_supported().await.unwrap());
    for workspace in [None, Some("bound-workspace")] {
        for method in ["initialize", "tools/list", "prompts/list", "ping"] {
            let request = json!({"jsonrpc":"2.0","id":1,"method":method});
            let response = tokio::time::timeout(
                Duration::from_secs(1),
                provider.mcp_request(workspace, request, Duration::from_secs(1)),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(response.get("result").is_some(), "{response}");
        }
    }
    assert!(
        !home.join("workspaces.fjall.migrating").exists(),
        "Discovery opened storage"
    );
    let request = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"vwork_repo_list","arguments":{}}});
    let error = provider
        .mcp_request(None, request.clone(), Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("another running instance"),
        "{error}"
    );
    drop(owner);
    let response = provider
        .mcp_request(None, request, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(response["result"]["isError"], false, "{response}");
}

#[tokio::test(start_paused = true)]
async fn v100_native_workspace_waits_for_completion_past_old_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let provider = VworkProvider::new(&Installation {
        id: "slow".into(),
        label: "slow".into(),
        home: temp.path().join("store"),
    })
    .unwrap();
    let store = provider.embedded.store().await.unwrap();
    let paused = store.block_workers().await;
    let task = tokio::spawn(async move { provider.call("repo_list", json!({})).await });
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(3600)).await;
    tokio::task::yield_now().await;
    let abandoned = task.is_finished();
    drop(paused);
    let result = task.await.unwrap();
    assert!(
        !abandoned,
        "Native operation abandoned an active workspace worker"
    );
    result.unwrap();
}
