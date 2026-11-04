use super::*;

#[tokio::test]
async fn v94_failed_activation_preserves_read_only_history_and_retry() {
    let temp = tempfile::tempdir().unwrap();
    let (mut rpc, _incoming, tasks) = Rpc::stream(tokio::io::empty(), tokio::io::sink());
    let _tasks = TaskGuard(tasks);
    let _original_writer = rpc.writer.clone();
    let (writer, mut requests) = mpsc::channel::<Value>(4);
    rpc.writer = writer;
    let replies = rpc.clone();
    let responder = tokio::spawn(async move {
        let mut first = true;
        while let Some(request) = requests.recv().await {
            assert_eq!(request["method"], "session/load");
            let response = if first {
                first = false;
                json!({"id":request["id"],"error":{"code":-32000,"message":"Fixture activation failed"}})
            } else {
                json!({"id":request["id"],"result":{}})
            };
            replies.complete(&response).unwrap();
        }
    });
    let _responder = TaskGuard(vec![responder]);
    let (events, _events) = mpsc::channel(32);
    let (jobs, _jobs) = mpsc::channel(4);
    let mut session = Session::new("session", "C:\\fixture");
    session.loaded = true;
    session.recovery_only = true;
    session.storage_pending = true;
    let mut history = turn("old", "completed");
    history["items"] = json!([{"type":"agentMessage","id":"saved","text":"Saved answer survives"}]);
    session.turns.push(history);
    let adapter = Arc::new(Adapter {
        stores: Default::default(),
        rpc,
        events,
        sessions: Mutex::new(HashMap::from([("session".into(), session)])),
        gates: Mutex::new(HashMap::new()),
        answers: Mutex::new(HashMap::new()),
        jobs,
        capabilities: Mutex::new(json!({"loadSession":true})),
        defaults: Mutex::new(Value::Null),
        cwd: "C:\\fixture".into(),
        mcp: mcp::Bridge::new().await.unwrap(),
        queue: Mutex::new(HashMap::new()),
        queue_root: temp.path().join("queue"),
        deletions: Mutex::new(Default::default()),
        vwork_paths: fastrock_workspaces::vwork::DiscoveryPaths {
            user_home: temp.path().into(),
            configs: vec![],
            search: vec![],
            default_vwork_home: temp.path().join("vwork"),
        },
        embedded_runtime: false,
        summaries: None,
        changed: tokio::sync::Notify::new(),
        schedules: Mutex::new(schedule::model::State::open(
            temp.path().join("schedule.json"),
        )),
        schedule_gate: Mutex::new(()),
        webhook_listener: Mutex::new(Default::default()),
    });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        adapter.retry_storage_history("session"),
    )
    .await
    .unwrap();
    assert!(
        result
            .unwrap_err()
            .message
            .contains("Fixture activation failed")
    );
    {
        let mut sessions = adapter.sessions.lock().await;
        let session = sessions.get_mut("session").unwrap();
        assert!(session.loaded && session.recovery_only && session.storage_pending);
        assert!(!session.replaying);
        assert!(
            json!(session.turns.all_turns().unwrap())
                .to_string()
                .contains("Saved answer survives")
        );
    }
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            adapter.retry_storage_history("session")
        )
        .await
        .unwrap()
        .unwrap()
    );
    let sessions = adapter.sessions.lock().await;
    assert!(sessions["session"].loaded);
    assert!(!sessions["session"].recovery_only && !sessions["session"].storage_pending);
}
