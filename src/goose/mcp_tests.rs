use super::*;

#[tokio::test]
async fn v35_shared_mcp_is_local_bound_and_reusable() {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    std::fs::write(
        dir.path().join("connection.json"),
        json!({"url":format!("http://{address}/"),"token":"fixture-only"}).to_string(),
    )
    .unwrap();
    let fixture = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/", post(|headers: HeaderMap, Json(request): Json<Value>| async move {
            assert_eq!(headers["authorization"], "Bearer fixture-only");
            if request["operation"] == "health" {
                Json(json!({"result":{"capabilities":{"shared_mcp":true}}}))
            } else {
                assert_eq!(request["operation"], "mcp");
                let args = &request["arguments"];
                Json(json!({"result":{"jsonrpc":"2.0","id":args["request"]["id"],"result":{"workspace":args["workspace"]}}}))
            }
        }))).await.unwrap();
    });
    let bridge = Bridge::new().await.unwrap();
    let mut binding = McpBinding {
        name: "vwork".into(),
        executable: dir.path().join("must-not-spawn.exe"),
        command: "vwork".into(),
        args: vec!["mcp".into()],
        home: dir.path().into(),
        workspace: Some("one".into()),
        auto_start: true,
        timeout_seconds: 5,
    };
    let one = bridge.vwork_server(&binding).await.unwrap().unwrap();
    bridge.remove_session("closed-conversation").await;
    assert_eq!(bridge.vwork_server(&binding).await.unwrap().unwrap(), one);
    binding.workspace = Some("two".into());
    let two = bridge.vwork_server(&binding).await.unwrap().unwrap();
    assert_ne!(one["url"], two["url"]);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let request =
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"workspace_info"}});
    let call = |server: Value| {
        let client = client.clone();
        let request = request.clone();
        async move {
            client
                .post(server["url"].as_str().unwrap())
                .json(&request)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let (a, b) = tokio::join!(call(one.clone()), call(two));
    assert_eq!(a["result"]["workspace"], "one");
    assert_eq!(b["result"]["workspace"], "two");
    assert_eq!(a["id"], 7);
    for (header, value) in [("origin", "http://localhost"), ("host", "outside.example")] {
        assert_eq!(
            client
                .post(one["url"].as_str().unwrap())
                .header(header, value)
                .json(&request)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        client
            .post(format!("http://{}/unknown", bridge.state.authority))
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    fixture.abort();
}
