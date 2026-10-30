use super::*;
use std::time::Duration;

async fn fixture() -> (AppServerClient, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let client = fixture_at(temp.path()).await;
    (client, temp)
}
async fn fixture_at(root: &std::path::Path) -> AppServerClient {
    let script = root.join("goose.py");
    std::fs::write(&script, include_str!("../../dev/external-goose-fixture.py")).unwrap();
    let python = std::env::var_os("FASTROCK_TEST_PYTHON").unwrap_or_else(|| {
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        }
    });
    let mut command = tokio::process::Command::new(python);
    command
        .arg(script)
        .arg("acp")
        .env_remove("FASTROCK_FIXTURE_ARTIFACT");
    let cwd = dunce::canonicalize(root).unwrap();
    tokio::time::timeout(
        Duration::from_secs(15),
        connect_command(command, cwd, root.join("queues")),
    )
    .await
    .unwrap()
    .unwrap()
}
async fn call(client: &AppServerClient, method: &str, params: Value) -> Reply {
    tokio::time::timeout(
        Duration::from_secs(10),
        client
            .request_handle()
            .raw(json!({"id":uuid::Uuid::new_v4().to_string(),"method":method,"params":params})),
    )
    .await
    .unwrap()
    .unwrap()
}
async fn new(client: &AppServerClient, cwd: &std::path::Path) -> String {
    call(
        client,
        "thread/start",
        json!({"cwd":dunce::canonicalize(cwd).unwrap(),"model":"fixture-model"}),
    )
    .await
    .unwrap()["thread"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}
#[tokio::test]
async fn v69_native_typed_rename_reaches_public_goose_api() {
    use codex_app_server_protocol::{
        ClientRequest, RequestId, ThreadSetNameParams, ThreadSetNameResponse,
    };
    let (client, temp) = fixture().await;
    let id = new(&client, temp.path()).await;
    let request = serde_json::to_value(ClientRequest::ThreadSetName {
        request_id: RequestId::String("rename".into()),
        params: ThreadSetNameParams {
            thread_id: id.clone(),
            name: "Human title wins".into(),
        },
    })
    .unwrap();
    let result = call(
        &client,
        request["method"].as_str().unwrap(),
        request["params"].clone(),
    )
    .await
    .unwrap();
    let _: ThreadSetNameResponse = serde_json::from_value(result).unwrap();
    let read = call(&client, "thread/read", json!({"threadId":id}))
        .await
        .unwrap();
    assert_eq!(read["thread"]["name"], "Human title wins");
    client.shutdown().await.unwrap();
}
async fn send(client: &AppServerClient, id: &str, message: &str) -> Value {
    call(client,"turn/start",json!({"threadId":id,"clientUserMessageId":uuid::Uuid::new_v4().to_string(),"input":[{"type":"text","text":message,"textElements":[]}]})).await.unwrap()
}
async fn event(client: &mut AppServerClient) -> AppServerEvent {
    tokio::time::timeout(Duration::from_secs(10), client.next_event())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn concurrent_sessions_and_cancel() {
    use codex_app_server_protocol::ServerNotification;
    let (mut client, temp) = fixture().await;
    let a = new(&client, temp.path()).await;
    let b = new(&client, temp.path()).await;
    send(&client, &a, "hold first").await;
    send(&client, &b, "second").await;
    let mut saw_a = false;
    loop {
        if let AppServerEvent::ServerNotification(n) = event(&mut client).await {
            match *n {
                ServerNotification::AgentMessageDelta(n) if n.thread_id == a => saw_a = true,
                ServerNotification::TurnCompleted(n) if n.thread_id == b => {
                    assert_eq!(serde_json::to_value(n.turn.status).unwrap(), "completed");
                    break;
                }
                ServerNotification::TurnCompleted(n) if n.thread_id == a => {
                    panic!("Other prompt ended early: {:?}", n.turn.status)
                }
                _ => {}
            }
        }
    }
    assert!(saw_a, "A must stream while B completes in the same process");
    call(&client, "turn/interrupt", json!({"threadId":a}))
        .await
        .unwrap();
    loop {
        if let AppServerEvent::ServerNotification(n) = event(&mut client).await {
            if let ServerNotification::TurnCompleted(n) = *n {
                assert_eq!(n.thread_id, a);
                assert_eq!(serde_json::to_value(n.turn.status).unwrap(), "interrupted");
                break;
            }
        }
    }
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn permission_routes_to_own_session_and_unknown_choice_cancels() {
    let (mut client, temp) = fixture().await;
    let a = new(&client, temp.path()).await;
    let b = new(&client, temp.path()).await;
    send(&client, &a, "permission").await;
    send(&client, &b, "permission").await;
    let mut answered = 0;
    let mut replies = HashMap::new();
    let mut completed = 0;
    while completed < 2 {
        match event(&mut client).await {
            AppServerEvent::ServerRequest(r) => {
                let value = serde_json::to_value(r).unwrap();
                let sid = text(&value["params"], "threadId");
                let label = if sid == a {
                    value["params"]["questions"][0]["options"][0]["label"].clone()
                } else {
                    json!("unoffered value")
                };
                let id = serde_json::from_value(value["id"].clone()).unwrap();
                client
                    .resolve_server_request(
                        id,
                        json!({"answers":{"permission":{"answers":[label]}}}),
                    )
                    .await
                    .unwrap();
                answered += 1;
            }
            AppServerEvent::ServerNotification(n) => match *n {
                codex_app_server_protocol::ServerNotification::AgentMessageDelta(n)
                    if n.item_id.starts_with("permission-result-") =>
                {
                    replies.insert(n.thread_id, n.delta);
                }
                codex_app_server_protocol::ServerNotification::TurnCompleted(_) => completed += 1,
                _ => {}
            },
            _ => {}
        }
    }
    assert_eq!(answered, 2);
    assert!(replies[&a].contains("allow-fixture-1"));
    assert!(replies[&b].contains("cancelled"));
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn load_history_and_future_config_values() {
    let (client, temp) = fixture().await;
    let id = new(&client, temp.path()).await;
    call(&client, "thread/unsubscribe", json!({"threadId":id}))
        .await
        .unwrap();
    call(&client, "thread/resume", json!({"threadId":id}))
        .await
        .unwrap();
    let page = call(&client, "thread/items/list", json!({"threadId":id}))
        .await
        .unwrap();
    assert_eq!(page["data"].as_array().unwrap().len(), 2);
    assert_eq!(page["data"][0]["item"]["text"], "Previous answer");
    call(&client, "thread/resume", json!({"threadId":id}))
        .await
        .unwrap();
    assert_eq!(
        call(&client, "thread/items/list", json!({"threadId":id}))
            .await
            .unwrap(),
        page
    );
    let config = call(
        &client,
        "fastrock/goose/setConfig",
        json!({"threadId":id,"configId":"thinking_effort","value":"future-effort"}),
    )
    .await
    .unwrap();
    assert_eq!(config["configOptions"][3]["currentValue"], "future-effort");
    assert!(
        call(
            &client,
            "fastrock/goose/setConfig",
            json!({"threadId":id,"configId":"thinking_effort","value":"invented"})
        )
        .await
        .is_err()
    );
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn disconnect_fails_pending() {
    let (mut client, temp) = fixture().await;
    let id = new(&client, temp.path()).await;
    send(&client, &id, "disconnect").await;
    loop {
        if matches!(
            event(&mut client).await,
            AppServerEvent::Disconnected { .. }
        ) {
            break;
        }
    }
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn queued_edits_are_atomic_and_wait_for_turn_end() {
    let (mut client, temp) = fixture().await;
    let id = new(&client, temp.path()).await;
    send(&client, &id, "permission").await;
    let added=call(&client,"thread/queue/add",json!({"threadId":id,"clientUserMessageId":"queued-client","input":[{"type":"text","text":"before","textElements":[]}]})).await.unwrap();
    let qid = added["queuedSubmission"]["id"].clone();
    call(&client,"thread/queue/update",json!({"threadId":id,"queuedSubmissionId":qid,"input":[{"type":"text","text":"after","textElements":[]}]})).await.unwrap();
    assert_eq!(
        call(&client, "thread/queue/list", json!({"threadId":id}))
            .await
            .unwrap()["data"][0]["input"][0]["text"],
        "after"
    );
    let mut completed = 0;
    while completed < 2 {
        match event(&mut client).await {
            AppServerEvent::ServerRequest(r) => {
                let v = serde_json::to_value(r).unwrap();
                client
                    .resolve_server_request(
                        serde_json::from_value(v["id"].clone()).unwrap(),
                        json!({"answers":{}}),
                    )
                    .await
                    .unwrap();
            }
            AppServerEvent::ServerNotification(n) => {
                if matches!(
                    *n,
                    codex_app_server_protocol::ServerNotification::TurnCompleted(_)
                ) {
                    completed += 1
                }
            }
            _ => {}
        }
    }
    assert!(
        call(
            &client,
            "thread/queue/update",
            json!({"threadId":id,"queuedSubmissionId":qid,"input":[]})
        )
        .await
        .is_err()
    );
    assert_eq!(
        call(&client, "thread/queue/list", json!({"threadId":id}))
            .await
            .unwrap()["data"],
        json!([])
    );
    client.shutdown().await.unwrap();
}

#[test]
fn backend_ids_cannot_cross_connections() {
    assert_eq!(session_id("goose:session-1").unwrap(), "session-1");
    assert!(session_id("codex-thread").is_err());
    assert!(session_id("goose:").is_err());
}

#[test]
fn replay_groups_messages_and_preserves_tool_output() {
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut session = Session::new("session-1", &cwd);
    session.replaying = true;
    for update in [
        json!({"sessionUpdate":"user_message_chunk","messageId":"u1","content":{"type":"text","text":"First"}}),
        json!({"sessionUpdate":"agent_message_chunk","messageId":"a1","content":{"type":"text","text":"Hello "}}),
        json!({"sessionUpdate":"agent_message_chunk","messageId":"a1","content":{"type":"text","text":"world"}}),
        json!({"sessionUpdate":"tool_call","toolCallId":"tool-1","title":"Read file","status":"in_progress","rawInput":{"path":"README.md"}}),
        json!({"sessionUpdate":"tool_call_update","toolCallId":"tool-1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"file text"}}]}),
        json!({"sessionUpdate":"user_message_chunk","messageId":"u2","content":{"type":"text","text":"Next"}}),
    ] {
        assert!(session.update(&update).is_empty());
    }
    assert_eq!(session.turns.len(), 2);
    let restored = session.turns.all_turns().unwrap();
    assert_eq!(restored[0]["items"][1]["text"], "Hello world");
    assert_eq!(
        restored[0]["items"][2]["result"]["content"][0]["text"],
        "file text"
    );
    let _: codex_app_server_protocol::ThreadStartResponse =
        serde_json::from_value(session.response()).unwrap();
    for turn in restored {
        let _: codex_app_server_protocol::Turn = serde_json::from_value(turn).unwrap();
    }
}

#[test]
fn live_projection_satisfies_ui_lifecycle_schema() {
    let mut session = Session::new("s", "/tmp");
    session.active = Some("turn".into());
    session.turns.push(turn("turn", "inProgress"));
    for update in [
        json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"Hello"}}),
        json!({"sessionUpdate":"agent_message_chunk","messageId":"shared-message","content":{"type":"text","text":"Response"}}),
        json!({"sessionUpdate":"agent_thought_chunk","messageId":"shared-message","content":{"type":"text","text":"Thinking"}}),
        json!({"sessionUpdate":"tool_call","toolCallId":"tool","title":"Read","status":"in_progress"}),
        json!({"sessionUpdate":"tool_call_update","toolCallId":"tool","status":"completed"}),
        json!({"sessionUpdate":"plan","entries":[{"content":"Do work","status":"in_progress"}]}),
    ] {
        for mut event in session.update(&update) {
            stamp_item_event(&mut event);
            let _: codex_app_server_protocol::ServerNotification =
                serde_json::from_value(event).unwrap();
        }
    }
    let items = session.turns.last().unwrap()["items"].as_array().unwrap();
    assert!(
        items
            .iter()
            .any(|i| i["type"] == "reasoning" && i["content"][0] == "Thinking")
    );
    assert!(
        items
            .iter()
            .any(|i| i["type"] == "agentMessage" && i["text"] == "Response")
    );
}

#[tokio::test]
async fn restricted_mcp_routes_namespaced_tool_to_its_session() {
    let (mut client, temp) = fixture().await;
    let started=call(&client,"thread/start",json!({"cwd":dunce::canonicalize(temp.path()).unwrap(),"sandbox":"read-only","developerInstructions":"Use only the fixture tool.","dynamicTools":[{"type":"namespace","name":"fixture","tools":[{"type":"function","name":"echo","description":"Echo fixture","inputSchema":{"type":"object"}}]}]})).await.unwrap();
    let id = text(&started["thread"], "id");
    call(&client, "thread/unsubscribe", json!({"threadId":id}))
        .await
        .unwrap();
    call(&client, "thread/resume", json!({"threadId":id}))
        .await
        .unwrap();
    send(&client, id, "mcp").await;
    let mut called = false;
    loop {
        match event(&mut client).await {
            AppServerEvent::ServerRequest(r) => {
                let value = serde_json::to_value(r).unwrap();
                assert_eq!(value["method"], "item/tool/call");
                assert_eq!(value["params"]["threadId"], id);
                assert_eq!(value["params"]["namespace"], "fixture");
                assert_eq!(value["params"]["tool"], "echo");
                client.resolve_server_request(serde_json::from_value(value["id"].clone()).unwrap(),json!({"contentItems":[{"type":"inputText","text":"tool accepted"}],"success":true})).await.unwrap();
                called = true;
            }
            AppServerEvent::ServerNotification(n)
                if matches!(
                    *n,
                    codex_app_server_protocol::ServerNotification::TurnCompleted(_)
                ) =>
            {
                break;
            }
            _ => {}
        }
    }
    assert!(called);
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn v68_sidebar_metadata_never_activates_closed_sessions() {
    let (client, temp) = fixture().await;
    let id = new(&client, temp.path()).await;
    call(&client, "thread/unsubscribe", json!({"threadId":id}))
        .await
        .unwrap();
    let before = call(&client, "thread/loaded/list", json!({}))
        .await
        .unwrap();
    let result = call(&client, "fastrock/thread/metadata", json!({"ids":[id]}))
        .await
        .unwrap();
    assert_eq!(result["data"][0]["id"], id);
    assert_eq!(
        call(&client, "thread/loaded/list", json!({}))
            .await
            .unwrap(),
        before
    );
    assert!(
        call(
            &client,
            "fastrock/thread/metadata",
            json!({"ids":vec![id;129]})
        )
        .await
        .is_err()
    );
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn immediate_cancel_is_not_lost() {
    let (mut client, temp) = fixture().await;
    let id = new(&client, temp.path()).await;
    send(&client, &id, "hold immediate").await;
    call(&client, "turn/interrupt", json!({"threadId":id}))
        .await
        .unwrap();
    loop {
        if let AppServerEvent::ServerNotification(n) = event(&mut client).await {
            if let codex_app_server_protocol::ServerNotification::TurnCompleted(n) = *n {
                assert_eq!(serde_json::to_value(n.turn.status).unwrap(), "interrupted");
                break;
            }
        }
    }
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn queued_drafts_are_persisted_before_acknowledgement() {
    let (client, temp) = fixture().await;
    let id = new(&client, temp.path()).await;
    send(&client, &id, "hold queue").await;
    call(&client,"thread/queue/add",json!({"threadId":id,"clientUserMessageId":"durable","input":[{"type":"text","text":"retained draft","textElements":[]}]})).await.unwrap();
    client.shutdown().await.unwrap();
    let files = std::fs::read_dir(temp.path().join("queues"))
        .unwrap()
        .filter(|entry| {
            entry.as_ref().is_ok_and(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                !name.ends_with(".vwork.json") && name != "schedules.json"
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(files.len(), 1);
    let queue: Value = serde_json::from_slice(&std::fs::read(files[0].path()).unwrap()).unwrap();
    assert_eq!(queue["pending"][0]["input"][0]["text"], "retained draft");
}

#[tokio::test]
async fn v41_restart_requires_resume_and_never_replays_uncertain_send() {
    let (client, temp) = fixture().await;
    let id = new(&client, temp.path()).await;
    send(&client, &id, "hold uncertain").await;
    let add = |client_id: &str| {
        json!({"threadId":id,"clientUserMessageId":client_id,
        "input":[{"type":"text","text":"safe pending message","textElements":[]}]})
    };
    call(&client, "thread/queue/add", add("pending-one"))
        .await
        .unwrap();
    client.shutdown().await.unwrap();

    let mut client = fixture_at(temp.path()).await;
    assert_eq!(new(&client, temp.path()).await, id); // Fixture restores its stable native ID.
    call(&client, "thread/resume", json!({"threadId":id}))
        .await
        .unwrap();
    let recovery = call(&client, "fastrock/goose/recovery", json!({"threadId":id}))
        .await
        .unwrap();
    assert_eq!(recovery["paused"], true);
    assert_eq!(recovery["pending"], 1);
    assert_eq!(recovery["uncertain"].as_array().unwrap().len(), 1);
    assert_eq!(
        recovery["uncertain"][0]["input"][0]["text"],
        "hold uncertain"
    );
    call(&client, "thread/queue/add", add("pending-two"))
        .await
        .unwrap();
    assert!(
        call(
            &client,
            "turn/start",
            json!({"threadId":id,"input":[{"type":"text","text":"must wait"}]})
        )
        .await
        .is_err()
    );
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        call(&client, "thread/read", json!({"threadId":id}))
            .await
            .unwrap()["thread"]["status"]["type"],
        "idle"
    );
    call(
        &client,
        "fastrock/goose/resumeQueue",
        json!({"threadId":id}),
    )
    .await
    .unwrap();
    let mut started = vec![];
    let mut completed = 0;
    while completed < 2 {
        if let AppServerEvent::ServerNotification(n) = event(&mut client).await {
            match *n {
                codex_app_server_protocol::ServerNotification::TurnStarted(n) => {
                    let value = serde_json::to_value(n.turn).unwrap();
                    started.push(value["items"][0]["clientId"].clone());
                }
                codex_app_server_protocol::ServerNotification::TurnCompleted(_) => completed += 1,
                _ => {}
            }
        }
    }
    assert_eq!(started, vec![json!("pending-one"), json!("pending-two")]);
    let recovery = call(&client, "fastrock/goose/recovery", json!({"threadId":id}))
        .await
        .unwrap();
    assert_eq!(recovery["pending"], 0);
    assert_eq!(recovery["uncertain"].as_array().unwrap().len(), 1);
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn v41_close_reopen_requires_resume_and_preserves_queued_edits() {
    let (client, temp) = fixture().await;
    let id = new(&client, temp.path()).await;
    send(&client, &id, "hold uncertain close").await;
    let added = call(
        &client,
        "thread/queue/add",
        json!({"threadId":id,"input":[{"type":"text","text":"original"}]}),
    )
    .await
    .unwrap();
    let queued_id = &added["queuedSubmission"]["id"];
    let input =
        json!([{"type":"text","text":"edited é"},{"type":"localImage","path":"retained.png"}]);
    call(
        &client,
        "thread/queue/update",
        json!({"threadId":id,"queuedSubmissionId":queued_id,"input":input}),
    )
    .await
    .unwrap();
    call(&client, "thread/unsubscribe", json!({"threadId":id}))
        .await
        .unwrap();
    call(&client, "thread/resume", json!({"threadId":id}))
        .await
        .unwrap();
    let recovery = call(&client, "fastrock/goose/recovery", json!({"threadId":id}))
        .await
        .unwrap();
    assert_eq!(recovery["paused"], true);
    assert_eq!(recovery["pending"], 1);
    assert_eq!(recovery["uncertain"].as_array().unwrap().len(), 1);
    let pending = call(&client, "thread/queue/list", json!({"threadId":id}))
        .await
        .unwrap();
    assert_eq!(pending["data"][0]["input"], input);
    call(
        &client,
        "fastrock/goose/dismissRecovery",
        json!({"threadId":id,"ids":["not-reviewed"]}),
    )
    .await
    .unwrap();
    assert_eq!(
        call(&client, "fastrock/goose/recovery", json!({"threadId":id}))
            .await
            .unwrap()["uncertain"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    call(
        &client,
        "fastrock/goose/dismissRecovery",
        json!({"threadId":id,"ids":[recovery["uncertain"][0]["id"]]}),
    )
    .await
    .unwrap();
    assert!(
        call(&client, "fastrock/goose/recovery", json!({"threadId":id}))
            .await
            .unwrap()["uncertain"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn v41_failed_completion_save_remains_reviewable_and_resumable() {
    use base64::Engine;
    let (mut client, temp) = fixture().await;
    let id = new(&client, temp.path()).await;
    send(&client, &id, "hold failed completion save").await;
    let path = temp.path().join("queues").join(format!(
        "{}.json",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(session_id(&id).unwrap())
    ));
    let original = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap(); // Force atomic replacement to fail on every platform.
    let failure = call(&client, "turn/interrupt", json!({"threadId":id}))
        .await
        .unwrap_err();
    assert!(failure.message.contains("Save queued drafts"));
    // Stop still cancels the running prompt despite the durable pause failure.
    loop {
        if let AppServerEvent::ServerNotification(notification) = event(&mut client).await {
            if matches!(
                *notification,
                codex_app_server_protocol::ServerNotification::TurnCompleted(_)
            ) {
                break;
            }
        }
    }
    let recovered = call(&client, "fastrock/goose/recovery", json!({"threadId":id}))
        .await
        .unwrap();
    assert_eq!(recovered["paused"], true);
    assert_eq!(recovered["uncertain"].as_array().unwrap().len(), 1);
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, original).unwrap();
    call(
        &client,
        "fastrock/goose/resumeQueue",
        json!({"threadId":id}),
    )
    .await
    .unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["dispatch"], Value::Null);
    assert_eq!(saved["uncertain"].as_array().unwrap().len(), 1);
    send(
        &client,
        &id,
        "An explicit new send is possible after storage recovers",
    )
    .await;
    client.shutdown().await.unwrap();
}
