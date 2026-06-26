use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fastrock_app::{
    AwsSharedConfigSigV4Signer, BedrockModelRunIo, BedrockModelRunRequest,
    BedrockProfileProbeError, BedrockProfileRequest, FastrockWorkflow, GuiPersistenceWrite,
    GuiRuntimeWork, PersistenceTranscriptStore, ProjectInstructionSnapshotKind,
    RemoteFileOperationKind, aws_static_credentials_from_secret_material, handle_gui_action,
    load_persisted_workflow, persist_gui_action_writes, run_bedrock_model_message_with_transports,
};
use fastrock_bedrock::{
    AwsCredentialSource, AwsSharedConfig, AwsStaticCredentials, MantleClientError,
    MantleHttpRequest, MantleHttpResponse, MantleHttpTransport, MantleSigV4Signer,
    RuntimeHttpRequest, RuntimeHttpResponse, RuntimeSigV4Signer,
    StaticCredentialsMantleSigV4Signer, StaticCredentialsRuntimeSigV4Signer,
};
use fastrock_cache::{
    CacheProviderPlane, CacheUsage, CacheUsageRecord, ContextFragmentInput, ContextFragmentKind,
    MemoryKind, MemoryRecord, MemoryScope, ToolOutputCacheAction, ToolOutputKind,
};
use fastrock_core::{
    AgentMode, CommandPolicy, ConversationActor, ConversationActorConfig, ConversationActorEvent,
    ConversationCommand, ConversationStatus, ThreadGoalStatus, ToolMutationClass,
    ToolRouteDecision, ToolRouteRejectionReason, TranscriptEventKind, TranscriptEventStore,
};
use fastrock_editor::EditorBuffer;
use fastrock_mcp::{
    MCP_OAUTH_TOKEN_SECRET_SERVICE, McpConfigChange, McpConfigScope, McpConfigSet, McpOAuthConfig,
    McpOAuthState, McpOAuthTokenSet, McpRuntimeSnapshot, McpServerConfig, McpServerRuntimeStatus,
    McpTransportConfig, mcp_oauth_token_secret_ref,
};
use fastrock_persistence::{
    ConfigScope, DATABASE_FILE_NAME, PersistenceActorHandle, PersistenceConfig,
    ProjectFolderRecord, SecretRefRecord, SecretStore, SecretStoreError, sqlite_file_contains,
};
use fastrock_projects::{
    ProjectFileEvent, RemoteCommandError, RemoteDiagnosticCheckCode, RemoteDiagnosticStatus,
    SshRemoteTarget, SsmRemoteTarget,
};
use fastrock_rtk::{RtkCommandRequest, RtkRunError, RtkRunnerConfig};
use fastrock_skills::{SkillScope, SkillWriteRequest};
use fastrock_test_support::{
    FakeMantleTransport, FakeRemoteTransport, FakeRuntimeTransport, fake_mcp_config_set,
};
use fastrock_ui::{
    ConversationListFilter, LlmProfileForm, UiLlmProvider, UiMantleApiShape, UiMantleAuthMode,
    UiProjectTargetKind,
};
use tempfile::tempdir;

#[tokio::test]
async fn release_criteria_are_exercised_through_app_workflow() {
    let temp_dir = tempdir().unwrap();
    let fake_rtk = create_fake_rtk(temp_dir.path());
    let mut workflow = FastrockWorkflow::default();

    workflow
        .create_bedrock_profile(profile_request(
            "mantle",
            "Mantle",
            UiLlmProvider::BedrockMantle,
            true,
        ))
        .unwrap();
    workflow
        .create_bedrock_profile(profile_request(
            "runtime",
            "Runtime",
            UiLlmProvider::BedrockRuntime,
            false,
        ))
        .unwrap();
    workflow
        .edit_profile_name("mantle", "Mantle Edited")
        .unwrap();
    workflow
        .create_bedrock_profile(profile_request(
            "delete-me",
            "Delete Me",
            UiLlmProvider::BedrockMantle,
            false,
        ))
        .unwrap();
    assert!(workflow.delete_profile("delete-me"));

    let local = workflow.create_conversation("local", "Local", temp_dir.path().to_str().unwrap());
    let remote = workflow.create_conversation("remote", "Remote", "/srv/app");
    assert!(workflow.switch_visible_conversation(&local));
    assert!(workflow.switch_visible_conversation(&remote));

    workflow
        .run_local_command(
            RtkCommandRequest {
                cwd: temp_dir.path().to_string_lossy().into_owned(),
                command: vec!["status".to_owned()],
                shell: None,
                pty: false,
                timeout_ms: None,
                env: BTreeMap::new(),
                stdin: None,
            },
            RtkRunnerConfig {
                rtk_binary: fake_rtk.to_string_lossy().into_owned(),
                event_channel_capacity: 8,
                max_capture_bytes: 1024,
            },
        )
        .await
        .unwrap();
    assert_eq!(workflow.local_command_transcripts()[0].stdout_bytes, 3);
    assert_eq!(workflow.local_command_transcripts()[0].stderr_bytes, 0);

    let remote_transport = FakeRemoteTransport::default();
    let ssh_target = SshRemoteTarget {
        id: "ssh".to_owned(),
        host_label: "ssh".to_owned(),
        user: None,
        host: "example.test".to_owned(),
        root: "/repo".to_owned(),
        rtk_binary: "rtk".to_owned(),
    };
    let ssm_target = SsmRemoteTarget {
        id: "ssm".to_owned(),
        profile_name: Some("default".to_owned()),
        region: "us-east-1".to_owned(),
        target: "i-test".to_owned(),
        root: "/repo".to_owned(),
        rtk_binary: "rtk".to_owned(),
    };
    let ssh_diagnostics = workflow.diagnose_ssh_remote_target(&ssh_target);
    assert_eq!(
        diagnostic_status(&ssh_diagnostics, RemoteDiagnosticCheckCode::RemoteRtk),
        Some(RemoteDiagnosticStatus::Pass)
    );
    let ssm_diagnostics = workflow.diagnose_ssm_remote_target(&ssm_target);
    assert_eq!(
        diagnostic_status(
            &ssm_diagnostics,
            RemoteDiagnosticCheckCode::SsmIamPermissions
        ),
        Some(RemoteDiagnosticStatus::Warning)
    );
    workflow
        .run_ssh_remote_command(&remote_transport, ssh_target.clone(), rtk_request("."))
        .await
        .unwrap();
    workflow
        .run_ssm_remote_command(&remote_transport, ssm_target.clone(), rtk_request("."))
        .await
        .unwrap();
    assert_eq!(workflow.remote_command_transcripts()[0].stdout_bytes, 2);
    assert_eq!(workflow.remote_command_transcripts()[0].stderr_bytes, 0);
    assert_eq!(workflow.remote_command_transcripts()[1].stdout_bytes, 2);
    let command_snapshot = workflow.shell_snapshot();
    assert!(
        command_snapshot
            .transcript_blocks
            .iter()
            .any(|block| block.speaker == "rtk" && block.body.contains("stdout 3 bytes"))
    );
    assert!(
        command_snapshot
            .transcript_blocks
            .iter()
            .any(|block| block.speaker == "remote rtk" && block.body.contains("stdout 2 bytes"))
    );
    workflow
        .read_ssh_remote_file(&remote_transport, ssh_target.clone(), "src/main.rs")
        .await
        .unwrap();
    workflow
        .write_ssh_remote_file(
            &remote_transport,
            ssh_target,
            "src/lib.rs",
            b"pub fn ok() {}\n".to_vec(),
        )
        .await
        .unwrap();
    workflow
        .read_ssm_remote_file(&remote_transport, ssm_target.clone(), "config/app.toml")
        .await
        .unwrap();
    workflow
        .write_ssm_remote_file(
            &remote_transport,
            ssm_target,
            "config/app.toml",
            b"enabled = true\n".to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(workflow.remote_file_transcripts().len(), 4);
    assert_eq!(workflow.remote_file_transcripts()[0].target_id, "ssh");
    assert_eq!(
        workflow.remote_file_transcripts()[0].kind,
        RemoteFileOperationKind::Read
    );
    assert_eq!(workflow.remote_file_transcripts()[1].target_id, "ssh");
    assert_eq!(
        workflow.remote_file_transcripts()[1].kind,
        RemoteFileOperationKind::Write
    );
    assert_eq!(workflow.remote_file_transcripts()[2].target_id, "ssm");
    assert_eq!(
        workflow.remote_file_transcripts()[2].kind,
        RemoteFileOperationKind::Read
    );
    assert_eq!(workflow.remote_file_transcripts()[3].target_id, "ssm");
    assert_eq!(
        workflow.remote_file_transcripts()[3].kind,
        RemoteFileOperationKind::Write
    );
    assert_eq!(remote_transport.file_reads().len(), 2);
    assert_eq!(remote_transport.file_writes().len(), 2);

    assert_eq!(
        workflow.route_plan_mode_tool(local.clone(), ToolMutationClass::ReadOnly),
        ToolRouteDecision::Allow
    );
    assert!(matches!(
        workflow.route_plan_mode_tool(local.clone(), ToolMutationClass::RunsCommand),
        ToolRouteDecision::Reject {
            reason: ToolRouteRejectionReason::PlanModeMutation,
            ..
        }
    ));
    assert_eq!(
        workflow.route_plan_mode_rtk_command(
            local.clone(),
            &argv(vec!["rtk", "rg", "Fastrock", "SPEC.md"]),
        ),
        ToolRouteDecision::Allow
    );
    assert!(matches!(
        workflow.route_plan_mode_rtk_command(
            local.clone(),
            &argv(vec!["rtk", "cargo", "fmt", "--check"]),
        ),
        ToolRouteDecision::Reject {
            reason: ToolRouteRejectionReason::PlanModeMutation,
            ..
        }
    ));

    workflow.create_goal(local.clone(), "complete release workflow", Some(10_000), 0);
    assert!(workflow.mark_goal_usage_limited(&local, 1));
    assert_eq!(
        workflow.goal_status(&local),
        Some(ThreadGoalStatus::UsageLimited)
    );
    assert!(workflow.resume_goal(&local, 2));
    assert_eq!(workflow.goal_status(&local), Some(ThreadGoalStatus::Active));
    assert!(!workflow.record_goal_blocking_error(&local, "provider-disconnect", 3, 2));
    assert!(workflow.record_goal_blocking_error(&local, "provider-disconnect", 4, 2));
    assert_eq!(
        workflow.goal_status(&local),
        Some(ThreadGoalStatus::Blocked)
    );
    assert!(workflow.resume_goal(&local, 5));
    assert!(workflow.update_goal_status(&local, ThreadGoalStatus::Paused, 1));
    assert!(workflow.update_goal_status(&local, ThreadGoalStatus::Active, 2));
    assert!(workflow.update_goal_status(&local, ThreadGoalStatus::Complete, 3));
    assert_eq!(
        workflow.goal_status(&local),
        Some(ThreadGoalStatus::Complete)
    );

    workflow.configure_mcp(fake_mcp_config_set()).unwrap();
    let mut release_skill = SkillWriteRequest::new("release", "Release skill", "Use RTK.");
    release_skill.mode_slugs = vec!["code".to_owned()];
    workflow
        .create_skill(
            temp_dir.path(),
            SkillScope::Project("project".to_owned()),
            release_skill,
        )
        .unwrap();
    workflow
        .discover_skills(temp_dir.path(), SkillScope::Project("project".to_owned()))
        .unwrap();
    workflow.remember_memory(MemoryRecord::new(
        "release-memory",
        MemoryKind::ProjectFact,
        MemoryScope::Project {
            project_folder_id: "project".to_owned(),
        },
        "Release workflow must keep all commands wrapped with rtk.",
        "release_workflow.rs",
        10,
    ));
    let memory_injections = workflow.memory_injections("rtk commands", Some("project"), 2, 256);
    assert_eq!(memory_injections.len(), 1);
    assert!(
        memory_injections[0]
            .prompt_text
            .contains("release_workflow.rs")
    );

    let file_path = temp_dir.path().join("main.rs");
    std::fs::write(&file_path, "fn main() {}\n").unwrap();
    workflow
        .open_edit_save_file(&file_path, "fn main() { println!(\"ok\"); }\n")
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&file_path).unwrap(),
        "fn main() { println!(\"ok\"); }\n"
    );

    assert!(workflow.release_evidence().all_core_workflows_present());
}

#[tokio::test]
async fn t12_bedrock_settings_probe_mantle_discovery_and_runtime_connection() {
    let shared = AwsSharedConfig::from_ini_documents(
        "[profile default]\nregion = us-west-2\n",
        "[default]\naws_access_key_id = test-key\naws_secret_access_key = test-secret\n",
    );
    let credentials = AwsStaticCredentials::new("test-key", "test-secret", None);
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(profile_request(
            "mantle",
            "Mantle",
            UiLlmProvider::BedrockMantle,
            true,
        ))
        .unwrap();
    workflow
        .create_bedrock_profile(profile_request(
            "runtime",
            "Runtime",
            UiLlmProvider::BedrockRuntime,
            false,
        ))
        .unwrap();

    let mantle_transport = FakeMantleTransport::new(vec![MantleHttpResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: br#"{"data":[{"id":"anthropic.test","owned_by":"aws","supported_api_shapes":["responses","chat_completions"]}]}"#.to_vec(),
    }]);
    let mantle_requests = mantle_transport.clone();
    let discovery = workflow
        .discover_mantle_models(
            "mantle",
            &shared,
            None,
            mantle_transport,
            StaticCredentialsMantleSigV4Signer::new(credentials.clone()),
        )
        .await
        .unwrap();

    assert_eq!(
        discovery.endpoint,
        "https://bedrock-mantle.us-east-1.api.aws/v1"
    );
    assert_eq!(discovery.models[0].id, "anthropic.test");
    assert_eq!(
        discovery.models[0].api_shapes,
        vec![
            UiMantleApiShape::Responses,
            UiMantleApiShape::ChatCompletions
        ]
    );
    assert_eq!(mantle_requests.requests()[0].method, "GET");
    assert!(
        mantle_requests.requests()[0]
            .headers
            .contains_key("authorization")
    );

    let runtime_transport = FakeRuntimeTransport::new(vec![RuntimeHttpResponse {
        status: 200,
        headers: BTreeMap::from([(
            "x-amzn-bedrock-invoked-model-id".to_owned(),
            "anthropic.test:resolved".to_owned(),
        )]),
        body: br#"{"output":{"message":{"content":[{"text":"ok"}]}},"stopReason":"end_turn","usage":{"inputTokens":2,"outputTokens":1,"totalTokens":3}}"#.to_vec(),
    }]);
    let runtime_requests = runtime_transport.clone();
    let connection = workflow
        .test_runtime_connection(
            "runtime",
            &shared,
            runtime_transport,
            StaticCredentialsRuntimeSigV4Signer::new(credentials),
        )
        .await
        .unwrap();

    assert_eq!(
        connection.endpoint,
        "https://bedrock-runtime.us-east-1.amazonaws.com"
    );
    assert_eq!(connection.output_text, "ok");
    assert_eq!(
        connection.invoked_model_id.as_deref(),
        Some("anthropic.test:resolved")
    );
    assert_eq!(connection.usage.unwrap().total_tokens, Some(3));
    assert!(
        runtime_requests.requests()[0]
            .url
            .ends_with("/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/converse")
    );
}

#[tokio::test]
async fn t10_gui_model_work_runs_bedrock_streams_and_appends_transcript() {
    let shared = AwsSharedConfig::from_ini_documents(
        "[profile default]\nregion = us-east-1\n",
        "[default]\naws_access_key_id = test-key\naws_secret_access_key = test-secret\n",
    );
    let credentials = AwsStaticCredentials::new("test-key", "test-secret", None);
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(profile_request(
            "mantle",
            "Mantle",
            UiLlmProvider::BedrockMantle,
            true,
        ))
        .unwrap();
    workflow
        .create_bedrock_profile(profile_request(
            "runtime",
            "Runtime",
            UiLlmProvider::BedrockRuntime,
            false,
        ))
        .unwrap();
    let mut mantle_profile = workflow.profiles().profile("mantle").unwrap().clone();
    mantle_profile.request_tuning.max_output_tokens = Some(1234);
    mantle_profile.request_tuning.temperature_milli = Some(250);
    mantle_profile.request_tuning.top_p_milli = Some(900);
    workflow.save_profile_form(mantle_profile).unwrap();
    let mut runtime_profile = workflow.profiles().profile("runtime").unwrap().clone();
    runtime_profile.runtime_settings.service_tier = Some("priority".to_owned());
    runtime_profile.runtime_settings.prompt_cache_ttl_seconds = Some(3600);
    runtime_profile.runtime_settings.reasoning_budget_tokens = Some(2000);
    runtime_profile.request_tuning.max_output_tokens = Some(2048);
    runtime_profile.request_tuning.temperature_milli = Some(150);
    runtime_profile.request_tuning.top_p_milli = Some(875);
    workflow.save_profile_form(runtime_profile).unwrap();
    let conversation_id = workflow.create_conversation("chat", "Chat", "folder-local");

    let mantle_transport = FakeMantleTransport::new(vec![MantleHttpResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: b"event: response.created\ndata: {\"id\":\"resp-1\"}\n\nevent: response.output_text.delta\ndata: {\"delta\":\"mantle ok\"}\n\nevent: response.completed\ndata: {}\n\n".to_vec(),
    }]);
    let mantle_requests = mantle_transport.clone();
    let mantle_result = run_bedrock_model_message_with_transports(
        BedrockModelRunRequest {
            conversation_id: conversation_id.clone(),
            profile_id: "mantle".to_owned(),
            message: "hello mantle".to_owned(),
            prompt_compression_enabled: false,
            prompt_compression_level: "full".to_owned(),
            mantle_previous_response_id: None,
        },
        workflow.profiles().profile("mantle").unwrap().clone(),
        BedrockModelRunIo {
            shared_config: &shared,
            bearer_api_key: None,
            mantle_transport,
            mantle_signer: StaticCredentialsMantleSigV4Signer::new(credentials.clone()),
            runtime_transport: FakeRuntimeTransport::new(Vec::new()),
            runtime_signer: StaticCredentialsRuntimeSigV4Signer::new(credentials.clone()),
        },
    )
    .await
    .unwrap();

    assert_eq!(mantle_result.metadata.request_kind, "responses.create");
    assert_eq!(mantle_result.events.len(), 3);
    assert_eq!(mantle_requests.requests()[0].method, "POST");
    assert!(mantle_requests.requests()[0].url.ends_with("/responses"));
    let mantle_body: serde_json::Value =
        serde_json::from_slice(&mantle_requests.requests()[0].body).unwrap();
    assert_eq!(mantle_body["input"], "hello mantle");
    assert_eq!(mantle_body["max_output_tokens"], 1234);
    assert_eq!(mantle_body["temperature"], 0.25);
    assert_eq!(mantle_body["top_p"], 0.9);
    // SPEC §V5: Mantle store is permanently disabled — AWS must not retain
    // response state, and no previous_response_id chaining is sent.
    assert_eq!(mantle_body["store"], false);
    assert!(mantle_body.get("previous_response_id").is_none());
    let mantle_writes = workflow.append_model_run_result(mantle_result, 501);
    assert!(mantle_writes.iter().any(
        |write| matches!(write, GuiPersistenceWrite::AppendConversationEvent(event)
            if event.event_type == "transcript.model_request")
    ));
    assert!(
        workflow
            .shell_snapshot()
            .transcript_blocks
            .iter()
            .any(|block| block.speaker == "assistant" && block.body == "mantle ok")
    );

    let runtime_transport = FakeRuntimeTransport::new(vec![
        RuntimeHttpResponse {
            status: 404,
            headers: BTreeMap::new(),
            body: br#"{"message":"converse stream unsupported"}"#.to_vec(),
        },
        RuntimeHttpResponse {
            status: 404,
            headers: BTreeMap::new(),
            body: br#"{"message":"invoke stream unsupported"}"#.to_vec(),
        },
        RuntimeHttpResponse {
            status: 200,
            headers: BTreeMap::from([(
                "x-amzn-bedrock-invoked-model-id".to_owned(),
                "anthropic.test:resolved".to_owned(),
            )]),
            body: br#"{"output":{"message":{"content":[{"text":"runtime fallback ok"}]}},"usage":{"inputTokens":4,"outputTokens":2,"totalTokens":6}}"#.to_vec(),
        },
    ]);
    let runtime_requests = runtime_transport.clone();
    let runtime_result = run_bedrock_model_message_with_transports(
        BedrockModelRunRequest {
            conversation_id: conversation_id.clone(),
            profile_id: "runtime".to_owned(),
            message: "hello runtime".to_owned(),
            prompt_compression_enabled: false,
            prompt_compression_level: "full".to_owned(),
            mantle_previous_response_id: None,
        },
        workflow.profiles().profile("runtime").unwrap().clone(),
        BedrockModelRunIo {
            shared_config: &shared,
            bearer_api_key: None,
            mantle_transport: FakeMantleTransport::new(Vec::new()),
            mantle_signer: StaticCredentialsMantleSigV4Signer::new(credentials.clone()),
            runtime_transport,
            runtime_signer: StaticCredentialsRuntimeSigV4Signer::new(credentials),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        runtime_result.metadata.request_kind,
        "converse_stream_with_invoke_fallback"
    );
    assert_eq!(runtime_result.events.len(), 4);
    let runtime_request_urls = runtime_requests
        .requests()
        .into_iter()
        .map(|request| request.url)
        .collect::<Vec<_>>();
    assert!(runtime_request_urls[0].ends_with("/converse-stream"));
    assert!(runtime_request_urls[1].ends_with("/invoke-with-response-stream"));
    assert!(runtime_request_urls[2].ends_with("/invoke"));
    let runtime_body: serde_json::Value =
        serde_json::from_slice(&runtime_requests.requests()[0].body).unwrap();
    assert_eq!(runtime_body["serviceTier"]["type"], "priority");
    assert_eq!(runtime_body["inferenceConfig"]["maxTokens"], 2048);
    assert_eq!(runtime_body["inferenceConfig"]["temperature"], 0.15);
    assert_eq!(runtime_body["inferenceConfig"]["topP"], 0.875);
    assert_eq!(
        runtime_body["messages"][0]["content"][1]["cachePoint"]["ttl"],
        "1h"
    );
    assert_eq!(
        runtime_body["additionalModelRequestFields"]["thinking"]["budget_tokens"],
        2000
    );
    let runtime_fallback_body: serde_json::Value =
        serde_json::from_slice(&runtime_requests.requests()[2].body).unwrap();
    assert_eq!(runtime_fallback_body["max_tokens"], 2048);
    assert_eq!(runtime_fallback_body["temperature"], 0.15);
    assert_eq!(runtime_fallback_body["top_p"], 0.875);
    workflow.append_model_run_result(runtime_result, 502);
    assert!(
        workflow
            .shell_snapshot()
            .transcript_blocks
            .iter()
            .any(|block| block.speaker == "assistant" && block.body == "runtime fallback ok")
    );
}

#[tokio::test]
async fn t10_model_run_retries_retryable_bedrock_errors() {
    let shared = AwsSharedConfig::from_ini_documents(
        "[profile default]\nregion = us-east-1\n",
        "[default]\naws_access_key_id = test-key\naws_secret_access_key = test-secret\n",
    );
    let credentials = AwsStaticCredentials::new("test-key", "test-secret", None);
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(profile_request(
            "mantle",
            "Mantle",
            UiLlmProvider::BedrockMantle,
            true,
        ))
        .unwrap();
    let mut profile = workflow.profiles().profile("mantle").unwrap().clone();
    profile.request_tuning.retry_max_attempts = Some(2);
    workflow.save_profile_form(profile).unwrap();
    let conversation_id = workflow.create_conversation("retry", "Retry", "folder-local");
    let mantle_transport = FakeMantleTransport::new(vec![
        MantleHttpResponse {
            status: 429,
            headers: BTreeMap::new(),
            body: br#"{"message":"rate limited"}"#.to_vec(),
        },
        MantleHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: b"event: response.created\ndata: {\"id\":\"resp-retry\"}\n\nevent: response.output_text.delta\ndata: {\"delta\":\"ok\"}\n\nevent: response.completed\ndata: {}\n\n".to_vec(),
        },
    ]);
    let requests = mantle_transport.clone();

    let result = run_bedrock_model_message_with_transports(
        BedrockModelRunRequest {
            conversation_id,
            profile_id: "mantle".to_owned(),
            message: "retry me".to_owned(),
            prompt_compression_enabled: false,
            prompt_compression_level: "full".to_owned(),
            mantle_previous_response_id: None,
        },
        workflow.profiles().profile("mantle").unwrap().clone(),
        BedrockModelRunIo {
            shared_config: &shared,
            bearer_api_key: None,
            mantle_transport,
            mantle_signer: StaticCredentialsMantleSigV4Signer::new(credentials.clone()),
            runtime_transport: FakeRuntimeTransport::new(Vec::new()),
            runtime_signer: StaticCredentialsRuntimeSigV4Signer::new(credentials),
        },
    )
    .await
    .unwrap();

    assert_eq!(requests.requests().len(), 2);
    assert!(result
        .events
        .iter()
        .any(|event| matches!(&event.kind, fastrock_core::NormalizedModelStreamEventKind::TextDelta { delta } if delta == "ok")));
}

#[tokio::test]
async fn t10_model_run_timeout_uses_profile_request_tuning() {
    let shared = AwsSharedConfig::from_ini_documents(
        "[profile default]\nregion = us-east-1\n",
        "[default]\naws_access_key_id = test-key\naws_secret_access_key = test-secret\n",
    );
    let credentials = AwsStaticCredentials::new("test-key", "test-secret", None);
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(profile_request(
            "mantle",
            "Mantle",
            UiLlmProvider::BedrockMantle,
            true,
        ))
        .unwrap();
    let mut profile = workflow.profiles().profile("mantle").unwrap().clone();
    profile.request_tuning.timeout_ms = Some(5);
    workflow.save_profile_form(profile).unwrap();
    let conversation_id = workflow.create_conversation("timeout", "Timeout", "folder-local");

    let error = run_bedrock_model_message_with_transports(
        BedrockModelRunRequest {
            conversation_id,
            profile_id: "mantle".to_owned(),
            message: "timeout".to_owned(),
            prompt_compression_enabled: false,
            prompt_compression_level: "full".to_owned(),
            mantle_previous_response_id: None,
        },
        workflow.profiles().profile("mantle").unwrap().clone(),
        BedrockModelRunIo {
            shared_config: &shared,
            bearer_api_key: None,
            mantle_transport: SlowMantleTransport::new(50),
            mantle_signer: StaticCredentialsMantleSigV4Signer::new(credentials.clone()),
            runtime_transport: FakeRuntimeTransport::new(Vec::new()),
            runtime_signer: StaticCredentialsRuntimeSigV4Signer::new(credentials),
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error.to_string(), "Bedrock model run timed out after 5ms");
}

#[tokio::test]
async fn t10_gui_mantle_store_disabled_never_chains_previous_response_id() {
    // SPEC §V5: Mantle store is permanently off. Even across multiple sends in the
    // same conversation and project, every request sends store=false and never
    // chains a previous_response_id, so AWS retains no response state.
    let shared = AwsSharedConfig::from_ini_documents(
        "[profile default]\nregion = us-east-1\n",
        "[default]\naws_access_key_id = test-key\naws_secret_access_key = test-secret\n",
    );
    let credentials = AwsStaticCredentials::new("test-key", "test-secret", None);
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(profile_request(
            "mantle",
            "Mantle",
            UiLlmProvider::BedrockMantle,
            true,
        ))
        .unwrap();
    let mut profile = workflow.profiles().profile("mantle").unwrap().clone();
    profile.mantle_settings.project_id = Some("project-a".to_owned());
    workflow.save_profile_form(profile).unwrap();
    let conversation_id = workflow.create_conversation("chat", "Chat", "folder-local");

    let first_action = handle_gui_action(&mut workflow, "conversation-send:first", 600);
    let first_request = only_run_model_work(&first_action.runtime_work);
    assert_eq!(first_request.conversation_id, conversation_id);
    assert_eq!(first_request.mantle_previous_response_id, None);

    let first_transport = FakeMantleTransport::new(vec![MantleHttpResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: b"event: response.created\ndata: {\"id\":\"resp-1\"}\n\nevent: response.completed\ndata: {}\n\n".to_vec(),
    }]);
    let first_requests = first_transport.clone();
    let first_result = run_bedrock_model_message_with_transports(
        first_request,
        workflow.profiles().profile("mantle").unwrap().clone(),
        BedrockModelRunIo {
            shared_config: &shared,
            bearer_api_key: None,
            mantle_transport: first_transport,
            mantle_signer: StaticCredentialsMantleSigV4Signer::new(credentials.clone()),
            runtime_transport: FakeRuntimeTransport::new(Vec::new()),
            runtime_signer: StaticCredentialsRuntimeSigV4Signer::new(credentials.clone()),
        },
    )
    .await
    .unwrap();
    workflow.append_model_run_result(first_result, 601);

    let first_body: serde_json::Value =
        serde_json::from_slice(&first_requests.requests()[0].body).unwrap();
    assert!(first_body.get("previous_response_id").is_none());
    assert_eq!(first_body["store"], false);
    // Project selection still works even with store disabled.
    assert_eq!(
        first_requests.requests()[0]
            .headers
            .get("openai-project")
            .unwrap(),
        "project-a"
    );

    // A second send in the same project must NOT chain the first response id.
    let second_action = handle_gui_action(&mut workflow, "conversation-send:second", 602);
    let second_request = only_run_model_work(&second_action.runtime_work);
    assert_eq!(second_request.mantle_previous_response_id, None);

    let second_transport = FakeMantleTransport::new(vec![MantleHttpResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: b"event: response.created\ndata: {\"id\":\"resp-2\"}\n\nevent: response.completed\ndata: {}\n\n".to_vec(),
    }]);
    let second_requests = second_transport.clone();
    run_bedrock_model_message_with_transports(
        second_request,
        workflow.profiles().profile("mantle").unwrap().clone(),
        BedrockModelRunIo {
            shared_config: &shared,
            bearer_api_key: None,
            mantle_transport: second_transport,
            mantle_signer: StaticCredentialsMantleSigV4Signer::new(credentials.clone()),
            runtime_transport: FakeRuntimeTransport::new(Vec::new()),
            runtime_signer: StaticCredentialsRuntimeSigV4Signer::new(credentials),
        },
    )
    .await
    .unwrap();

    let second_body: serde_json::Value =
        serde_json::from_slice(&second_requests.requests()[0].body).unwrap();
    assert!(second_body.get("previous_response_id").is_none());
    assert_eq!(second_body["store"], false);
}

#[tokio::test]
async fn t16_named_aws_cli_profile_signs_mantle_and_runtime_model_requests() {
    let shared = AwsSharedConfig::from_ini_documents(
        "[profile named]\nregion = us-west-2\n",
        "[named]\naws_access_key_id = named-key\naws_secret_access_key = named-secret\n",
    );
    let signer = AwsSharedConfigSigV4Signer::new(shared.clone());
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "mantle-named".to_owned(),
            name: "Mantle Named".to_owned(),
            provider: UiLlmProvider::BedrockMantle,
            model_id: "anthropic.claude-test".to_owned(),
            region: "us-west-2".to_owned(),
            aws_profile: "named".to_owned(),
            default_for_new_conversations: true,
        })
        .unwrap();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "runtime-named".to_owned(),
            name: "Runtime Named".to_owned(),
            provider: UiLlmProvider::BedrockRuntime,
            model_id: "anthropic.claude-test".to_owned(),
            region: "us-west-2".to_owned(),
            aws_profile: "named".to_owned(),
            default_for_new_conversations: false,
        })
        .unwrap();
    let conversation_id = workflow.create_conversation("named", "Named", "folder-local");

    let mantle_transport = FakeMantleTransport::new(vec![MantleHttpResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: b"event: response.created\ndata: {\"id\":\"resp-named\"}\n\nevent: response.output_text.delta\ndata: {\"delta\":\"ok\"}\n\nevent: response.completed\ndata: {}\n\n".to_vec(),
    }]);
    let mantle_requests = mantle_transport.clone();
    run_bedrock_model_message_with_transports(
        BedrockModelRunRequest {
            conversation_id: conversation_id.clone(),
            profile_id: "mantle-named".to_owned(),
            message: "hello mantle".to_owned(),
            prompt_compression_enabled: false,
            prompt_compression_level: "full".to_owned(),
            mantle_previous_response_id: None,
        },
        workflow.profiles().profile("mantle-named").unwrap().clone(),
        BedrockModelRunIo {
            shared_config: &shared,
            bearer_api_key: None,
            mantle_transport,
            mantle_signer: signer.clone(),
            runtime_transport: FakeRuntimeTransport::new(Vec::new()),
            runtime_signer: signer.clone(),
        },
    )
    .await
    .unwrap();
    assert!(
        mantle_requests.requests()[0]
            .headers
            .get("authorization")
            .unwrap()
            .contains("Credential=named-key/")
    );

    let runtime_transport = FakeRuntimeTransport::new(vec![
        RuntimeHttpResponse {
            status: 404,
            headers: BTreeMap::new(),
            body: br#"{"message":"converse stream unsupported"}"#.to_vec(),
        },
        RuntimeHttpResponse {
            status: 404,
            headers: BTreeMap::new(),
            body: br#"{"message":"invoke stream unsupported"}"#.to_vec(),
        },
        RuntimeHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"output":{"message":{"content":[{"text":"ok"}]}}}"#.to_vec(),
        },
    ]);
    let runtime_requests = runtime_transport.clone();
    run_bedrock_model_message_with_transports(
        BedrockModelRunRequest {
            conversation_id,
            profile_id: "runtime-named".to_owned(),
            message: "hello runtime".to_owned(),
            prompt_compression_enabled: false,
            prompt_compression_level: "full".to_owned(),
            mantle_previous_response_id: None,
        },
        workflow
            .profiles()
            .profile("runtime-named")
            .unwrap()
            .clone(),
        BedrockModelRunIo {
            shared_config: &shared,
            bearer_api_key: None,
            mantle_transport: FakeMantleTransport::new(Vec::new()),
            mantle_signer: signer.clone(),
            runtime_transport,
            runtime_signer: signer,
        },
    )
    .await
    .unwrap();
    assert!(
        runtime_requests.requests()[0]
            .headers
            .get("authorization")
            .unwrap()
            .contains("Credential=named-key/")
    );
    assert!(
        runtime_requests.requests()[0]
            .url
            .starts_with("https://bedrock-runtime.us-west-2.amazonaws.com")
    );
}

#[tokio::test]
async fn t16_credential_process_profile_signs_mantle_and_runtime_model_requests() {
    let temp_dir = tempdir().unwrap();
    let credential_process = create_fake_credential_process(temp_dir.path());
    let credential_process = format!("\"{}\"", credential_process.display());
    let shared = AwsSharedConfig::from_ini_documents(
        &format!(
            "[profile process]\nregion = us-east-2\ncredential_process = {credential_process}\n"
        ),
        "",
    );
    let signer = AwsSharedConfigSigV4Signer::new(shared.clone());
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "mantle-process".to_owned(),
            name: "Mantle Process".to_owned(),
            provider: UiLlmProvider::BedrockMantle,
            model_id: "anthropic.claude-test".to_owned(),
            region: "us-east-2".to_owned(),
            aws_profile: "process".to_owned(),
            default_for_new_conversations: true,
        })
        .unwrap();
    workflow
        .create_bedrock_profile(BedrockProfileRequest {
            id: "runtime-process".to_owned(),
            name: "Runtime Process".to_owned(),
            provider: UiLlmProvider::BedrockRuntime,
            model_id: "anthropic.claude-test".to_owned(),
            region: "us-east-2".to_owned(),
            aws_profile: "process".to_owned(),
            default_for_new_conversations: false,
        })
        .unwrap();
    let conversation_id = workflow.create_conversation("process", "Process", "folder-local");

    let mantle_transport = FakeMantleTransport::new(vec![MantleHttpResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: b"event: response.created\ndata: {\"id\":\"resp-process\"}\n\nevent: response.completed\ndata: {}\n\n".to_vec(),
    }]);
    let mantle_requests = mantle_transport.clone();
    run_bedrock_model_message_with_transports(
        BedrockModelRunRequest {
            conversation_id: conversation_id.clone(),
            profile_id: "mantle-process".to_owned(),
            message: "hello mantle".to_owned(),
            prompt_compression_enabled: false,
            prompt_compression_level: "full".to_owned(),
            mantle_previous_response_id: None,
        },
        workflow
            .profiles()
            .profile("mantle-process")
            .unwrap()
            .clone(),
        BedrockModelRunIo {
            shared_config: &shared,
            bearer_api_key: None,
            mantle_transport,
            mantle_signer: signer.clone(),
            runtime_transport: FakeRuntimeTransport::new(Vec::new()),
            runtime_signer: signer.clone(),
        },
    )
    .await
    .unwrap();
    assert!(
        mantle_requests.requests()[0]
            .headers
            .get("authorization")
            .unwrap()
            .contains("Credential=process-key/")
    );

    let runtime_transport = FakeRuntimeTransport::new(vec![
        RuntimeHttpResponse {
            status: 404,
            headers: BTreeMap::new(),
            body: br#"{"message":"converse stream unsupported"}"#.to_vec(),
        },
        RuntimeHttpResponse {
            status: 404,
            headers: BTreeMap::new(),
            body: br#"{"message":"invoke stream unsupported"}"#.to_vec(),
        },
        RuntimeHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"output":{"message":{"content":[{"text":"ok"}]}}}"#.to_vec(),
        },
    ]);
    let runtime_requests = runtime_transport.clone();
    run_bedrock_model_message_with_transports(
        BedrockModelRunRequest {
            conversation_id,
            profile_id: "runtime-process".to_owned(),
            message: "hello runtime".to_owned(),
            prompt_compression_enabled: false,
            prompt_compression_level: "full".to_owned(),
            mantle_previous_response_id: None,
        },
        workflow
            .profiles()
            .profile("runtime-process")
            .unwrap()
            .clone(),
        BedrockModelRunIo {
            shared_config: &shared,
            bearer_api_key: None,
            mantle_transport: FakeMantleTransport::new(Vec::new()),
            mantle_signer: signer.clone(),
            runtime_transport,
            runtime_signer: signer,
        },
    )
    .await
    .unwrap();
    assert!(
        runtime_requests.requests()[0]
            .headers
            .get("authorization")
            .unwrap()
            .contains("Credential=process-key/")
    );
}

#[test]
fn t16_aws_cli_export_credentials_signs_sso_and_assume_role_profiles_for_both_bedrock_endpoints() {
    let temp_dir = tempdir().unwrap();
    let fake_aws = create_fake_aws_cli_export_credentials(temp_dir.path());
    let shared = AwsSharedConfig::from_ini_documents(
        r#"
        [profile sso]
        region = us-west-2
        sso_session = corp
        sso_account_id = 123456789012
        sso_role_name = Developer

        [profile role]
        region = eu-west-1
        role_arn = arn:aws:iam::123456789012:role/FastrockBedrock
        source_profile = base
        "#,
        r#"
        [base]
        aws_access_key_id = base-key
        aws_secret_access_key = base-secret
        "#,
    );
    let signer = AwsSharedConfigSigV4Signer::new(shared.clone())
        .with_aws_cli_binary(fake_aws.display().to_string());

    assert_export_profile_signs_both_bedrock_endpoints(&signer, &shared, "sso", "sso-export-key");
    assert_export_profile_signs_both_bedrock_endpoints(&signer, &shared, "role", "role-export-key");
}

#[test]
fn t16_default_chain_uses_aws_cli_export_credentials_for_both_bedrock_endpoints() {
    let temp_dir = tempdir().unwrap();
    let fake_aws = create_fake_aws_cli_export_credentials(temp_dir.path());
    let shared = AwsSharedConfig::default();
    let signer = AwsSharedConfigSigV4Signer::new(shared.clone())
        .with_aws_cli_binary(fake_aws.display().to_string())
        .with_default_chain_environment_enabled(false);
    let auth = shared
        .resolve_bedrock_auth(&AwsCredentialSource::DefaultChain, Some("us-east-1"))
        .unwrap();

    assert_default_or_secret_auth_signs_both_bedrock_endpoints(
        &signer,
        &auth,
        "default-export-key",
    );
}

#[test]
fn t16_explicit_static_secret_ref_signs_both_bedrock_endpoints() {
    let shared = AwsSharedConfig::default();
    let secret_ref = "secret:aws:bedrock-dev";
    let credentials = aws_static_credentials_from_secret_material(
        r#"
        aws_access_key_id = secret-ref-key
        aws_secret_access_key = secret-ref-secret
        aws_session_token = secret-ref-token
        "#,
    )
    .unwrap();
    let signer = AwsSharedConfigSigV4Signer::new(shared.clone())
        .with_static_secret_ref(secret_ref, credentials);
    let auth = shared
        .resolve_bedrock_auth(
            &AwsCredentialSource::ExplicitStaticSecret {
                secret_ref: secret_ref.to_owned(),
            },
            Some("us-east-1"),
        )
        .unwrap();

    assert_default_or_secret_auth_signs_both_bedrock_endpoints(&signer, &auth, "secret-ref-key");
    assert!(
        format!("{signer:?}").contains("<redacted>"),
        "debug output must redact secret key material"
    );
    assert!(
        !format!("{signer:?}").contains("secret-ref-secret"),
        "debug output must not leak keyring secret material"
    );
}

fn assert_export_profile_signs_both_bedrock_endpoints(
    signer: &AwsSharedConfigSigV4Signer,
    shared: &AwsSharedConfig,
    profile_name: &str,
    access_key_id: &str,
) {
    let auth = shared
        .resolve_bedrock_auth(
            &AwsCredentialSource::CliProfile {
                profile_name: profile_name.to_owned(),
            },
            None,
        )
        .unwrap();
    let mut mantle_request = MantleHttpRequest {
        method: "POST".to_owned(),
        url: format!("{}/responses", auth.mantle_sigv4_base_url),
        headers: BTreeMap::new(),
        body: br#"{"model":"anthropic.claude-test"}"#.to_vec(),
    };
    MantleSigV4Signer::sign(signer, &mut mantle_request, &auth).unwrap();
    assert!(
        mantle_request
            .headers
            .get("authorization")
            .unwrap()
            .contains(&format!("Credential={access_key_id}/"))
    );
    assert!(
        mantle_request
            .url
            .starts_with(&format!("https://bedrock-mantle.{}.api.aws", auth.region))
    );

    let mut runtime_request = RuntimeHttpRequest {
        method: "POST".to_owned(),
        url: format!(
            "{}/model/{}/converse-stream",
            auth.runtime_endpoint, "anthropic.claude-test"
        ),
        headers: BTreeMap::new(),
        body: br#"{"messages":[]}"#.to_vec(),
    };
    RuntimeSigV4Signer::sign(signer, &mut runtime_request, &auth).unwrap();
    assert!(
        runtime_request
            .headers
            .get("authorization")
            .unwrap()
            .contains(&format!("Credential={access_key_id}/"))
    );
    assert!(runtime_request.url.starts_with(&format!(
        "https://bedrock-runtime.{}.amazonaws.com",
        auth.region
    )));
}

fn assert_default_or_secret_auth_signs_both_bedrock_endpoints(
    signer: &AwsSharedConfigSigV4Signer,
    auth: &fastrock_bedrock::ResolvedBedrockAuth,
    access_key_id: &str,
) {
    let mut mantle_request = MantleHttpRequest {
        method: "POST".to_owned(),
        url: format!("{}/responses", auth.mantle_sigv4_base_url),
        headers: BTreeMap::new(),
        body: br#"{"model":"anthropic.claude-test"}"#.to_vec(),
    };
    MantleSigV4Signer::sign(signer, &mut mantle_request, auth).unwrap();
    assert!(
        mantle_request
            .headers
            .get("authorization")
            .unwrap()
            .contains(&format!("Credential={access_key_id}/"))
    );
    assert!(
        mantle_request
            .url
            .starts_with(&format!("https://bedrock-mantle.{}.api.aws", auth.region))
    );

    let mut runtime_request = RuntimeHttpRequest {
        method: "POST".to_owned(),
        url: format!(
            "{}/model/{}/converse-stream",
            auth.runtime_endpoint, "anthropic.claude-test"
        ),
        headers: BTreeMap::new(),
        body: br#"{"messages":[]}"#.to_vec(),
    };
    RuntimeSigV4Signer::sign(signer, &mut runtime_request, auth).unwrap();
    assert!(
        runtime_request
            .headers
            .get("authorization")
            .unwrap()
            .contains(&format!("Credential={access_key_id}/"))
    );
    assert!(runtime_request.url.starts_with(&format!(
        "https://bedrock-runtime.{}.amazonaws.com",
        auth.region
    )));
}

#[tokio::test]
async fn t12_bedrock_settings_probe_reports_profile_and_secret_errors() {
    let shared = AwsSharedConfig::default();
    let mut workflow = FastrockWorkflow::default();
    workflow
        .create_bedrock_profile(profile_request(
            "runtime",
            "Runtime",
            UiLlmProvider::BedrockRuntime,
            false,
        ))
        .unwrap();

    let wrong_provider = workflow
        .discover_mantle_models(
            "runtime",
            &shared,
            None,
            FakeMantleTransport::new(Vec::new()),
            StaticCredentialsMantleSigV4Signer::new(AwsStaticCredentials::new(
                "key", "secret", None,
            )),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        wrong_provider,
        BedrockProfileProbeError::WrongProvider {
            expected: UiLlmProvider::BedrockMantle,
            actual: UiLlmProvider::BedrockRuntime,
            ..
        }
    ));

    let mut bearer_profile = LlmProfileForm::new_bedrock_mantle("mantle-bearer");
    bearer_profile.name = "Mantle Bearer".to_owned();
    bearer_profile.model_id = "anthropic.claude-3-5-sonnet-20241022-v2:0".to_owned();
    bearer_profile.mantle_settings.auth_mode = UiMantleAuthMode::BearerApiKey {
        secret_ref: "secret:mantle-api-key".to_owned(),
    };
    workflow.save_profile_form(bearer_profile).unwrap();

    let missing_secret = workflow
        .discover_mantle_models(
            "mantle-bearer",
            &shared,
            None,
            FakeMantleTransport::new(Vec::new()),
            StaticCredentialsMantleSigV4Signer::new(AwsStaticCredentials::new(
                "key", "secret", None,
            )),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        missing_secret,
        BedrockProfileProbeError::MissingBearerApiKey {
            secret_ref,
            ..
        } if secret_ref == "secret:mantle-api-key"
    ));
}

#[tokio::test]
async fn t14_workflow_command_policy_blocks_denied_local_command_before_spawn() {
    let temp_dir = tempdir().unwrap();
    let mut workflow = FastrockWorkflow::default();
    workflow.set_command_policy(CommandPolicy {
        deny_prefixes: vec![argv(vec!["git", "push"])],
        ..CommandPolicy::default()
    });

    let mut request = rtk_request(temp_dir.path().to_str().unwrap());
    request.command = argv(vec!["git", "push", "origin"]);
    let err = workflow
        .run_local_command(request, RtkRunnerConfig::default())
        .await
        .unwrap_err();

    assert!(matches!(err, RtkRunError::PolicyDenied(_)));
    assert!(workflow.local_command_transcripts().is_empty());
}

#[tokio::test]
async fn t14_workflow_command_policy_applies_output_cap_and_labels() {
    let temp_dir = tempdir().unwrap();
    let fake_rtk = create_fake_rtk(temp_dir.path());
    let mut workflow = FastrockWorkflow::default();
    workflow.set_command_policy(CommandPolicy {
        max_output_bytes: Some(2),
        ..CommandPolicy::default()
    });

    let mut request = rtk_request(temp_dir.path().to_str().unwrap());
    request.command = argv(vec!["rm", "../outside.txt"]);
    let transcript = workflow
        .run_local_command(
            request,
            RtkRunnerConfig {
                rtk_binary: fake_rtk.to_string_lossy().into_owned(),
                event_channel_capacity: 8,
                max_capture_bytes: 1024,
            },
        )
        .await
        .unwrap();

    assert_eq!(transcript.stdout_bytes, 3);
    assert_eq!(transcript.stdout, b"ok");
    assert!(transcript.stdout_truncated);
    assert_eq!(
        transcript.policy_labels,
        vec!["modifies outside project: ../outside.txt".to_owned()]
    );
}

#[tokio::test]
async fn t14_workflow_persists_command_policy_for_restart() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let mut workflow = FastrockWorkflow::default();

    workflow
        .set_persisted_command_policy(
            &persistence,
            CommandPolicy {
                deny_exact: vec![argv(vec!["rm", "-rf", "/"])],
                deny_prefixes: vec![argv(vec!["git", "push"])],
                allow_prefixes: vec![argv(vec!["git"]), argv(vec!["cargo"])],
                max_runtime_ms: Some(1_500),
                max_output_bytes: Some(4096),
                max_concurrent_commands: Some(2),
                ..CommandPolicy::default()
            },
            77,
        )
        .await
        .unwrap();

    let stored = persistence
        .load_settings_document(ConfigScope::Global, "command.policy")
        .await
        .unwrap()
        .unwrap();
    assert!(stored.document_json.contains(r#""max_runtime_ms":1500"#));

    let mut hydrated = FastrockWorkflow::default();
    hydrated
        .hydrate_from_persistence(&persistence)
        .await
        .unwrap();
    assert_eq!(hydrated.command_policy().max_output_bytes, Some(4096));
    assert_eq!(hydrated.command_policy().max_concurrent_commands, Some(2));

    let mut denied = rtk_request(temp_dir.path().to_str().unwrap());
    denied.command = argv(vec!["git", "push", "origin"]);
    let err = hydrated
        .run_local_command(denied, RtkRunnerConfig::default())
        .await
        .unwrap_err();
    assert!(matches!(err, RtkRunError::PolicyDenied(_)));

    let settings = hydrated
        .shell_snapshot()
        .settings_rows
        .into_iter()
        .map(|row| (row.name, row.value))
        .collect::<BTreeMap<_, _>>();
    assert!(
        settings["Command policy"].contains("allowlist 2 prefixes"),
        "{}",
        settings["Command policy"]
    );
    assert!(
        settings["Command policy"].contains("output cap 4096 bytes"),
        "{}",
        settings["Command policy"]
    );
}

#[tokio::test]
async fn t14_gui_command_policy_presets_persist_for_restart() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let client = persistence.client();
    let mut workflow = FastrockWorkflow::default();

    let locked = handle_gui_action(&mut workflow, "command-policy-locked", 88);
    assert_eq!(locked.status, "Command policy set: locked down");
    persist_gui_action_writes(&client, locked.writes)
        .await
        .unwrap();

    let stored = persistence
        .load_settings_document(ConfigScope::Global, "command.policy")
        .await
        .unwrap()
        .unwrap();
    assert!(stored.document_json.contains(r#""max_runtime_ms":120000"#));
    assert!(stored.document_json.contains(r#"["rtk","cargo"]"#));

    let hydrated = load_persisted_workflow(&persistence).await.unwrap();
    assert_eq!(
        hydrated.command_policy().allow_prefixes,
        vec![
            argv(vec!["rtk", "cargo"]),
            argv(vec!["rtk", "rg"]),
            argv(vec!["rtk", "git", "status"]),
        ]
    );
    assert_eq!(hydrated.command_policy().max_concurrent_commands, Some(1));

    let settings = hydrated
        .shell_snapshot()
        .settings_rows
        .into_iter()
        .map(|row| (row.name, row.value))
        .collect::<BTreeMap<_, _>>();
    assert!(
        settings["Command policy"].contains("allowlist 3 prefixes"),
        "{}",
        settings["Command policy"]
    );

    let disabled = handle_gui_action(&mut workflow, "command-policy-disabled", 89);
    assert_eq!(disabled.status, "Command policy set: disabled");
    persist_gui_action_writes(&client, disabled.writes)
        .await
        .unwrap();
    let rehydrated = load_persisted_workflow(&persistence).await.unwrap();
    assert!(!rehydrated.command_policy().commands_enabled);

    persistence.shutdown().await.unwrap();
}

#[tokio::test]
async fn t16_workflow_command_policy_blocks_denied_ssh_command_before_transport() {
    let remote_transport = FakeRemoteTransport::default();
    let mut workflow = FastrockWorkflow::default();
    workflow.set_command_policy(CommandPolicy {
        deny_prefixes: vec![argv(vec!["git", "push"])],
        ..CommandPolicy::default()
    });

    let mut request = rtk_request(".");
    request.command = argv(vec!["git", "push", "origin"]);
    let err = workflow
        .run_ssh_remote_command(&remote_transport, test_ssh_target(), request)
        .await
        .unwrap_err();

    assert!(matches!(err, RemoteCommandError::PolicyDenied(_)));
    assert!(remote_transport.requests().is_empty());
    assert!(workflow.remote_command_transcripts().is_empty());
}

#[tokio::test]
async fn t17_workflow_remote_policy_caps_ssm_timeout_output_and_labels() {
    let remote_transport = FakeRemoteTransport::default();
    let mut workflow = FastrockWorkflow::default();
    workflow.set_command_policy(CommandPolicy {
        max_runtime_ms: Some(1_500),
        max_output_bytes: Some(1),
        ..CommandPolicy::default()
    });

    let mut request = rtk_request(".");
    request.command = argv(vec!["rm", "../outside.txt"]);
    request.timeout_ms = Some(5_000);
    let transcript = workflow
        .run_ssm_remote_command(&remote_transport, test_ssm_target(), request)
        .await
        .unwrap();

    assert_eq!(transcript.stdout_bytes, 2);
    assert_eq!(transcript.stdout, b"o");
    assert!(transcript.stdout_truncated);
    assert_eq!(
        transcript.policy_labels,
        vec!["modifies outside project: ../outside.txt".to_owned()]
    );

    let requests = remote_transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].1.timeout_ms, Some(1_500));
}

#[tokio::test]
async fn t11_t21_workflow_persists_conversations_and_goals_for_restart() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let mut workflow = FastrockWorkflow::default();

    workflow
        .create_persisted_bedrock_profile(
            &persistence,
            profile_request(
                "persisted-profile",
                "Persisted Profile",
                UiLlmProvider::BedrockMantle,
                true,
            ),
            90,
        )
        .await
        .unwrap()
        .unwrap();
    workflow
        .edit_persisted_profile_name(&persistence, "persisted-profile", "Edited Profile", 91)
        .await
        .unwrap()
        .unwrap();
    workflow
        .upsert_persisted_project_folder(
            &persistence,
            ProjectFolderRecord {
                id: "folder-local".to_owned(),
                label: "Local Repo".to_owned(),
                target_kind: "local".to_owned(),
                path: "/repo".to_owned(),
                remote_target_id: None,
                document_json: "{\"default_profile_id\":\"persisted-profile\"}".to_owned(),
                updated_at_ms: 92,
            },
        )
        .await
        .unwrap();
    workflow
        .upsert_persisted_project_folder(
            &persistence,
            ProjectFolderRecord {
                id: "folder-ssh".to_owned(),
                label: "SSH Repo".to_owned(),
                target_kind: "ssh".to_owned(),
                path: "/srv/repo".to_owned(),
                remote_target_id: Some("remote-ssh".to_owned()),
                document_json: "{\"default_profile_id\":\"persisted-profile\"}".to_owned(),
                updated_at_ms: 93,
            },
        )
        .await
        .unwrap();
    workflow
        .open_persisted_editor_tab(
            &persistence,
            fastrock_editor::EditorBuffer::from_text("/repo/src/lib.rs", ""),
            None,
            94,
        )
        .await
        .unwrap();
    workflow
        .open_persisted_editor_tab(
            &persistence,
            fastrock_editor::EditorBuffer::from_text("/repo/src/main.rs", ""),
            None,
            95,
        )
        .await
        .unwrap();
    workflow
        .configure_persisted_mcp(&persistence, fake_mcp_config_set(), 96)
        .await
        .unwrap()
        .unwrap();
    workflow
        .record_persisted_cache_usage(
            &persistence,
            CacheUsageRecord {
                provider_plane: CacheProviderPlane::BedrockMantle,
                prompt_tokens: 120,
                cache_read_tokens: 100,
                cache_write_tokens: 0,
                cache_hit: true,
            },
            97,
        )
        .await
        .unwrap();
    workflow
        .record_persisted_cache_usage(
            &persistence,
            CacheUsageRecord {
                provider_plane: CacheProviderPlane::BedrockRuntime,
                prompt_tokens: 300,
                cache_read_tokens: 0,
                cache_write_tokens: 160,
                cache_hit: false,
            },
            98,
        )
        .await
        .unwrap();
    let context_first = workflow
        .remember_persisted_context_fragment(
            &persistence,
            ContextFragmentInput {
                kind: ContextFragmentKind::Instruction,
                source_id: "AGENTS.md".to_owned(),
                bytes: b"Use rtk for every command.".to_vec(),
                estimated_tokens: 6,
            },
            98,
        )
        .await
        .unwrap();
    let context_second = workflow
        .remember_persisted_context_fragment(
            &persistence,
            ContextFragmentInput {
                kind: ContextFragmentKind::Instruction,
                source_id: "AGENTS.md".to_owned(),
                bytes: b"Use rtk for every command.".to_vec(),
                estimated_tokens: 6,
            },
            99,
        )
        .await
        .unwrap();
    assert!(!context_first.hit);
    assert!(context_second.hit);
    let large_output = vec![b'x'; 5000];
    let first_output = workflow
        .remember_persisted_tool_output(
            &persistence,
            "persisted",
            ToolOutputKind::CommandStdout,
            &large_output,
            99,
        )
        .await
        .unwrap();
    let second_output = workflow
        .remember_persisted_tool_output(
            &persistence,
            "persisted",
            ToolOutputKind::CommandStdout,
            &large_output,
            100,
        )
        .await
        .unwrap();
    assert!(matches!(
        first_output.action,
        ToolOutputCacheAction::Stored { .. }
    ));
    assert!(matches!(
        second_output.action,
        ToolOutputCacheAction::Referenced { .. }
    ));
    workflow
        .remember_persisted_memory(
            &persistence,
            MemoryRecord::new(
                "memory-local-state",
                MemoryKind::ProjectFact,
                MemoryScope::Project {
                    project_folder_id: "folder-local".to_owned(),
                },
                "Mantle store=false keeps conversation state local.",
                "SPEC.md:bedrock",
                99,
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        workflow
            .search_memory("store false local", Some("folder-local"), 4)
            .len(),
        1
    );
    assert!(
        workflow
            .set_persisted_memory_enabled(&persistence, "memory-local-state", false, 99)
            .await
            .unwrap()
    );
    assert!(
        workflow
            .search_memory("store false local", Some("folder-local"), 4)
            .is_empty()
    );
    assert!(
        workflow
            .set_persisted_memory_enabled(&persistence, "memory-local-state", true, 100)
            .await
            .unwrap()
    );

    let conversation_id = workflow
        .create_persisted_conversation(&persistence, "persisted", "Persisted", "folder-local", 100)
        .await
        .unwrap();
    workflow
        .create_persisted_goal(
            &persistence,
            conversation_id.clone(),
            "survive restart",
            Some(512),
            101,
        )
        .await
        .unwrap();
    workflow
        .update_persisted_goal_status(
            &persistence,
            &conversation_id,
            ThreadGoalStatus::Paused,
            102,
        )
        .await
        .unwrap();

    let events = persistence
        .load_conversation_events("persisted")
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "conversation.created");
    assert!(events[0].payload_json.contains("Persisted"));
    let stored_mcp = persistence
        .load_settings_document(ConfigScope::Global, "mcp.config")
        .await
        .unwrap()
        .unwrap();
    assert!(stored_mcp.document_json.contains(r#""type":"stdio""#));
    let cache_metadata = persistence.list_cache_metadata().await.unwrap();
    assert_eq!(cache_metadata.len(), 4);
    assert!(
        cache_metadata
            .iter()
            .any(|record| record.cache_key == "usage:bedrock_mantle")
    );
    assert!(
        cache_metadata
            .iter()
            .any(|record| record.cache_key == "usage:bedrock_runtime")
    );
    assert!(
        cache_metadata
            .iter()
            .any(|record| record.cache_key == "context:instruction:AGENTS.md")
    );
    assert!(
        cache_metadata
            .iter()
            .any(|record| record.cache_key.starts_with("tool-output:persisted:"))
    );

    let mut hydrated = FastrockWorkflow::default();
    hydrated
        .hydrate_from_persistence(&persistence)
        .await
        .unwrap();

    assert_eq!(hydrated.scheduler().conversation_count(), 1);
    assert_eq!(
        hydrated.goal_status(&conversation_id),
        Some(ThreadGoalStatus::Paused)
    );
    let mut startup_loaded = load_persisted_workflow(&persistence).await.unwrap();
    assert_eq!(startup_loaded.scheduler().conversation_count(), 1);
    assert_eq!(startup_loaded.profiles().profiles().len(), 1);
    assert_eq!(
        startup_loaded.profiles().profiles()[0].name,
        "Edited Profile"
    );
    assert_eq!(startup_loaded.project_folders().len(), 2);
    assert_eq!(
        startup_loaded
            .project_folders()
            .get("folder-ssh")
            .unwrap()
            .remote_target_id
            .as_deref(),
        Some("remote-ssh")
    );
    let sidebar = startup_loaded.conversation_sidebar_model();
    assert_eq!(sidebar.rows().len(), 1);
    assert_eq!(sidebar.rows()[0].folder_label, "Local Repo");
    assert_eq!(sidebar.rows()[0].target_kind, UiProjectTargetKind::Local);
    assert_eq!(sidebar.rows()[0].profile_name, "Edited Profile");
    assert_eq!(sidebar.rows()[0].goal_status.as_deref(), Some("Paused"));
    assert!(sidebar.rows()[0].selected);
    assert_eq!(
        sidebar
            .filtered_rows(&ConversationListFilter {
                project_folder_id: Some("folder-local".to_owned()),
                ..ConversationListFilter::default()
            })
            .len(),
        1
    );
    assert_eq!(startup_loaded.editor_tab_records().len(), 2);
    assert_eq!(
        startup_loaded.editor_tabs().active().unwrap().path,
        "/repo/src/main.rs"
    );
    assert_eq!(startup_loaded.mcp_config().servers.len(), 1);
    assert_eq!(startup_loaded.mcp_config().servers[0].id, "fake");
    assert_eq!(
        startup_loaded.cache_usage(CacheProviderPlane::BedrockMantle),
        CacheUsage {
            prompt_tokens: 120,
            cache_read_tokens: 100,
            cache_write_tokens: 0,
            cache_hits: 1,
            cache_misses: 0,
        }
    );
    assert_eq!(
        startup_loaded.cache_usage(CacheProviderPlane::BedrockRuntime),
        CacheUsage {
            prompt_tokens: 300,
            cache_read_tokens: 0,
            cache_write_tokens: 160,
            cache_hits: 0,
            cache_misses: 1,
        }
    );
    assert_eq!(startup_loaded.context_fragment_records().len(), 1);
    assert_eq!(
        startup_loaded.context_fragment_records()[0].source_id,
        "AGENTS.md"
    );
    assert_eq!(startup_loaded.tool_output_records().len(), 1);
    assert_eq!(startup_loaded.tool_output_records()[0].occurrences, 2);
    let repeated_after_restart = startup_loaded.remember_tool_output(
        "persisted",
        ToolOutputKind::CommandStdout,
        &large_output,
        101,
    );
    assert!(matches!(
        repeated_after_restart.action,
        ToolOutputCacheAction::Referenced { .. }
    ));
    assert_eq!(startup_loaded.memory_records().len(), 1);
    let injections =
        startup_loaded.memory_injections("mantle local state", Some("folder-local"), 2, 128);
    assert_eq!(injections.len(), 1);
    assert_eq!(injections[0].record_id, "memory-local-state");
    assert!(injections[0].prompt_text.contains("SPEC.md:bedrock"));
    assert!(
        startup_loaded
            .delete_persisted_memory(&persistence, "memory-local-state")
            .await
            .unwrap()
    );
    assert!(startup_loaded.memory_records().is_empty());
    assert!(persistence.list_memory_records().await.unwrap().is_empty());
    assert!(
        startup_loaded
            .clear_persisted_goal(&persistence, &conversation_id)
            .await
            .unwrap()
    );
    assert_eq!(startup_loaded.goal_status(&conversation_id), None);
    assert!(persistence.list_goals().await.unwrap().is_empty());

    let mut deleting = startup_loaded;
    assert!(
        deleting
            .close_persisted_editor_tab(&persistence, "/repo/src/main.rs")
            .await
            .unwrap()
    );
    assert!(
        deleting
            .delete_persisted_project_folder(&persistence, "folder-ssh")
            .await
            .unwrap()
    );
    assert!(
        deleting
            .delete_persisted_profile(&persistence, "persisted-profile")
            .await
            .unwrap()
    );
    assert_eq!(persistence.list_editor_tabs().await.unwrap().len(), 1);
    assert_eq!(persistence.list_project_folders().await.unwrap().len(), 1);
    assert!(persistence.list_llm_profiles().await.unwrap().is_empty());

    persistence.shutdown().await.unwrap();
}

#[tokio::test]
async fn t20_gui_memory_actions_persist_disable_delete_and_inspect_state() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let client = persistence.client();
    let mut workflow = FastrockWorkflow::default();
    workflow
        .remember_persisted_memory(
            &persistence,
            MemoryRecord::new(
                "memory-gui",
                MemoryKind::UserPreference,
                MemoryScope::Global,
                "Prefer rtk wrapped shell commands in Fastrock.",
                "AGENTS.md",
                400,
            ),
        )
        .await
        .unwrap();

    let inspected = handle_gui_action(&mut workflow, "memory-inspect:memory-gui", 401);
    assert_eq!(inspected.status, "Memory opened: memory-gui");
    persist_gui_action_writes(&client, inspected.writes)
        .await
        .unwrap();
    assert!(
        persistence
            .list_editor_tabs()
            .await
            .unwrap()
            .iter()
            .any(|tab| tab.path == "memory://memory-gui")
    );

    let disabled = handle_gui_action(&mut workflow, "memory-toggle:memory-gui", 402);
    assert_eq!(disabled.status, "Memory disabled: memory-gui");
    persist_gui_action_writes(&client, disabled.writes)
        .await
        .unwrap();
    assert!(
        !persistence
            .list_memory_records()
            .await
            .unwrap()
            .first()
            .unwrap()
            .enabled
    );

    let deleted = handle_gui_action(&mut workflow, "memory-delete:memory-gui", 403);
    assert_eq!(deleted.status, "Memory deleted: memory-gui");
    persist_gui_action_writes(&client, deleted.writes)
        .await
        .unwrap();
    assert!(persistence.list_memory_records().await.unwrap().is_empty());
    persistence.shutdown().await.unwrap();
}

#[tokio::test]
async fn v11_workflow_persists_goal_turn_usage_once_and_budget_limit() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let mut workflow = FastrockWorkflow::default();
    let conversation_id = workflow
        .create_persisted_conversation(&persistence, "goal-budget", "Goal Budget", ".", 10)
        .await
        .unwrap();

    workflow
        .create_persisted_goal(
            &persistence,
            conversation_id.clone(),
            "finish within budget",
            Some(100),
            20,
        )
        .await
        .unwrap();
    assert!(
        workflow
            .charge_persisted_goal_turn_usage(
                &persistence,
                &conversation_id,
                "turn-1",
                60,
                1000,
                30
            )
            .await
            .unwrap()
    );
    assert!(
        !workflow
            .charge_persisted_goal_turn_usage(
                &persistence,
                &conversation_id,
                "turn-1",
                60,
                1000,
                40
            )
            .await
            .unwrap()
    );
    assert!(
        workflow
            .charge_persisted_goal_turn_usage(
                &persistence,
                &conversation_id,
                "turn-2",
                40,
                2000,
                50
            )
            .await
            .unwrap()
    );

    let snapshot = workflow.goal_snapshot(&conversation_id).unwrap();
    assert_eq!(snapshot.status, ThreadGoalStatus::BudgetLimited);
    assert_eq!(snapshot.tokens_used, 100);
    assert_eq!(snapshot.elapsed_ms, 3000);

    let records = persistence.list_goals().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, "BudgetLimited");
    assert_eq!(records[0].updated_at_ms, 50);
    let document: serde_json::Value = serde_json::from_str(&records[0].document_json).unwrap();
    assert_eq!(document["tokens_used"], 100);
    assert_eq!(document["elapsed_ms"], 3000);
    assert_eq!(document["created_at_ms"], 20);
    assert_eq!(document["updated_at_ms"], 50);
    assert_eq!(
        document["charged_turn_ids"],
        serde_json::json!(["turn-1", "turn-2"])
    );

    let mut hydrated = FastrockWorkflow::default();
    hydrated
        .hydrate_from_persistence(&persistence)
        .await
        .unwrap();
    let hydrated_snapshot = hydrated.goal_snapshot(&conversation_id).unwrap();
    assert_eq!(hydrated_snapshot.status, ThreadGoalStatus::BudgetLimited);
    assert_eq!(hydrated_snapshot.tokens_used, 100);
    assert_eq!(hydrated_snapshot.elapsed_ms, 3000);
    assert!(!hydrated.charge_goal_turn_usage(&conversation_id, "turn-1", 10, 100, 60));

    persistence.shutdown().await.unwrap();
}

#[tokio::test]
async fn t2_gui_actions_emit_persistence_writes_for_restart() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let client = persistence.client();
    let mut workflow = FastrockWorkflow::default();

    let profile_result = handle_gui_action(&mut workflow, "profile-add", 10);
    assert!(profile_result.status.contains("Mantle profile ready"));
    assert_eq!(profile_result.writes.len(), 1);
    persist_gui_action_writes(&client, profile_result.writes)
        .await
        .unwrap();

    let conversations_result = handle_gui_action(&mut workflow, "conversation-new", 20);
    assert_eq!(conversations_result.status, "New conversation created");
    assert_eq!(conversations_result.writes.len(), 2);
    persist_gui_action_writes(&client, conversations_result.writes)
        .await
        .unwrap();

    let goal_result = handle_gui_action(&mut workflow, "conversation-send", 30);
    assert_eq!(goal_result.status, "Message queued");
    assert!(goal_result.writes.iter().any(
        |write| matches!(write, fastrock_app::GuiPersistenceWrite::AppendConversationEvent(event)
                if event.event_type == "transcript.user_message"
                    && event.payload_json.contains("finish current task"))
    ));
    persist_gui_action_writes(&client, goal_result.writes)
        .await
        .unwrap();

    let message_result = handle_gui_action(
        &mut workflow,
        "conversation-send:finish current SPEC gap",
        35,
    );
    assert_eq!(message_result.status, "Message queued");
    assert!(message_result.writes.iter().any(
        |write| matches!(write, fastrock_app::GuiPersistenceWrite::AppendConversationEvent(event)
                if event.event_type == "transcript.user_message"
                    && event.payload_json.contains("finish current SPEC gap"))
    ));
    persist_gui_action_writes(&client, message_result.writes)
        .await
        .unwrap();

    let plan_result = handle_gui_action(&mut workflow, "plan-toggle", 40);
    assert!(plan_result.status.contains("Plan"));
    assert_eq!(plan_result.writes.len(), 1);
    persist_gui_action_writes(&client, plan_result.writes)
        .await
        .unwrap();

    let plan_created = handle_gui_action(
        &mut workflow,
        "plan-start:write exact implementation checklist",
        41,
    );
    assert_eq!(plan_created.status, "Plan artifact created");
    assert!(plan_created.writes.iter().any(
        |write| matches!(write, fastrock_app::GuiPersistenceWrite::AppendConversationEvent(event)
                if event.event_type == "transcript.plan_created"
                    && event.payload_json.contains("implementation checklist"))
    ));
    persist_gui_action_writes(&client, plan_created.writes)
        .await
        .unwrap();

    let plan_executed = handle_gui_action(&mut workflow, "plan-execute", 42);
    assert!(plan_executed.status.starts_with("Plan executed: plan-"));
    assert!(plan_executed.writes.iter().any(
        |write| matches!(write, fastrock_app::GuiPersistenceWrite::AppendConversationEvent(event)
                if event.event_type == "transcript.plan_executed")
    ));
    persist_gui_action_writes(&client, plan_executed.writes)
        .await
        .unwrap();

    assert_eq!(persistence.list_llm_profiles().await.unwrap().len(), 1);
    assert_eq!(persistence.list_project_folders().await.unwrap().len(), 0);
    assert_eq!(persistence.list_conversations().await.unwrap().len(), 1);
    assert!(persistence.list_goals().await.unwrap().is_empty());
    assert!(persistence.list_editor_tabs().await.unwrap().is_empty());

    let loaded = load_persisted_workflow(&persistence).await.unwrap();
    assert_eq!(loaded.profiles().profiles().len(), 1);
    assert_eq!(loaded.scheduler().conversation_count(), 1);
    assert_eq!(loaded.project_folders().len(), 0);
    assert!(loaded.editor_tab_records().is_empty());
    let loaded_conversation_id = loaded
        .scheduler()
        .visible_conversation_id()
        .expect("loaded selected conversation")
        .clone();
    assert_eq!(loaded.goal_status(&loaded_conversation_id), None);
    assert_eq!(
        loaded.conversation_mode(&loaded_conversation_id),
        AgentMode::Code
    );
    assert!(
        loaded
            .shell_snapshot()
            .transcript_blocks
            .iter()
            .any(|block| block.speaker == "user" && block.body == "finish current SPEC gap")
    );
    assert!(
        loaded
            .shell_snapshot()
            .transcript_blocks
            .iter()
            .any(|block| block.speaker == "plan"
                && block.body.contains("write exact implementation checklist"))
    );
    assert!(
        loaded
            .shell_snapshot()
            .transcript_blocks
            .iter()
            .any(|block| block.speaker == "plan" && block.body.starts_with("Executed plan"))
    );

    persistence.shutdown().await.unwrap();
}

#[tokio::test]
async fn t6_gui_profile_row_actions_persist_enabled_and_default_state() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let client = persistence.client();
    let mut workflow = FastrockWorkflow::default();

    let mantle = handle_gui_action(&mut workflow, "profile-add-mantle", 10);
    persist_gui_action_writes(&client, mantle.writes)
        .await
        .unwrap();
    let runtime = handle_gui_action(&mut workflow, "profile-add-runtime", 11);
    persist_gui_action_writes(&client, runtime.writes)
        .await
        .unwrap();

    let runtime_id = workflow
        .profiles()
        .profiles()
        .iter()
        .find(|profile| profile.provider == UiLlmProvider::BedrockRuntime)
        .unwrap()
        .id
        .clone();
    let disabled = handle_gui_action(&mut workflow, &format!("profile-toggle:{runtime_id}"), 12);
    persist_gui_action_writes(&client, disabled.writes)
        .await
        .unwrap();
    let defaulted = handle_gui_action(&mut workflow, &format!("profile-default:{runtime_id}"), 13);
    persist_gui_action_writes(&client, defaulted.writes)
        .await
        .unwrap();

    let records = persistence.list_llm_profiles().await.unwrap();
    assert_eq!(records.len(), 2);
    let runtime_record = records
        .iter()
        .find(|record| record.id == runtime_id)
        .unwrap();
    assert!(runtime_record.enabled);
    let runtime_document: LlmProfileForm =
        serde_json::from_str(&runtime_record.document_json).unwrap();
    assert!(runtime_document.default_for_new_conversations);

    let hydrated = load_persisted_workflow(&persistence).await.unwrap();
    let snapshot = hydrated.shell_snapshot();
    assert_eq!(snapshot.profile_rows.len(), 2);
    assert!(
        snapshot
            .profile_rows
            .iter()
            .any(|row| row.profile_id == runtime_id
                && row.enabled
                && row.default_for_new_conversations)
    );

    persistence.shutdown().await.unwrap();
}

#[tokio::test]
async fn t21_gui_goal_controls_persist_status_and_clear_goal() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let client = persistence.client();
    let mut workflow = FastrockWorkflow::default();

    let conversation_id = workflow.create_conversation("goal", "Goal", "folder-local");
    workflow.create_goal(
        conversation_id.clone(),
        "finish goal controls",
        Some(1000),
        10,
    );
    let goal_id = conversation_id.0.clone();
    assert_eq!(
        workflow.goal_status(&fastrock_core::ConversationId::new(&goal_id)),
        Some(ThreadGoalStatus::Active)
    );

    let paused = handle_gui_action(&mut workflow, "goal-pause", 20);
    assert_eq!(paused.status, "Goal paused");
    persist_gui_action_writes(&client, paused.writes)
        .await
        .unwrap();
    assert_eq!(persistence.list_goals().await.unwrap()[0].status, "Paused");

    let resumed = handle_gui_action(&mut workflow, "goal-resume", 30);
    assert_eq!(resumed.status, "Goal resumed");
    persist_gui_action_writes(&client, resumed.writes)
        .await
        .unwrap();
    assert_eq!(persistence.list_goals().await.unwrap()[0].status, "Active");

    let completed = handle_gui_action(&mut workflow, "goal-complete", 40);
    assert_eq!(completed.status, "Goal complete");
    persist_gui_action_writes(&client, completed.writes)
        .await
        .unwrap();
    assert_eq!(
        persistence.list_goals().await.unwrap()[0].status,
        "Complete"
    );

    let cleared = handle_gui_action(&mut workflow, "goal-clear", 50);
    assert_eq!(cleared.status, "Goal cleared");
    persist_gui_action_writes(&client, cleared.writes)
        .await
        .unwrap();
    assert!(persistence.list_goals().await.unwrap().is_empty());
    assert_eq!(
        workflow.goal_status(&fastrock_core::ConversationId::new(goal_id)),
        None
    );

    persistence.shutdown().await.unwrap();
}

#[tokio::test]
async fn t22_gui_editor_tab_actions_persist_active_and_delete_records() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let client = persistence.client();
    let mut workflow = FastrockWorkflow::default();
    workflow.open_editor_tab(EditorBuffer::from_text("first.rs", "first\n"));
    workflow.open_editor_tab(EditorBuffer::from_text("second.rs", "second\n"));

    let selected = handle_gui_action(&mut workflow, "editor-select:first.rs", 10);
    assert_eq!(selected.status, "Editor tab selected: first.rs");
    persist_gui_action_writes(&client, selected.writes)
        .await
        .unwrap();
    let tabs = persistence.list_editor_tabs().await.unwrap();
    assert_eq!(tabs.len(), 2);
    assert!(tabs.iter().any(|tab| tab.path == "first.rs" && tab.active));
    assert!(
        tabs.iter()
            .any(|tab| tab.path == "second.rs" && !tab.active)
    );

    let closed = handle_gui_action(&mut workflow, "editor-close:first.rs", 11);
    assert_eq!(closed.status, "Editor tab closed: first.rs");
    persist_gui_action_writes(&client, closed.writes)
        .await
        .unwrap();
    let tabs = persistence.list_editor_tabs().await.unwrap();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].path, "second.rs");
    assert!(tabs[0].active);

    persistence.shutdown().await.unwrap();
}

#[tokio::test]
async fn t22_gui_editor_edit_save_writes_file_off_action_path() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let client = persistence.client();
    let file_path = temp_dir.path().join("note.txt");
    fs::write(&file_path, "alpha\n").unwrap();
    let mut workflow = FastrockWorkflow::default();
    workflow.open_editor_tab(EditorBuffer::open(&file_path).unwrap());

    let appended = handle_gui_action(&mut workflow, "editor-append:beta", 10);
    assert_eq!(
        appended.status,
        format!("Editor appended: {} dirty=true", file_path.display())
    );
    persist_gui_action_writes(&client, appended.writes)
        .await
        .unwrap();
    let tabs = persistence.list_editor_tabs().await.unwrap();
    assert_eq!(tabs.len(), 1);
    assert!(tabs[0].document_json.contains("\"dirty\":true"));
    assert_eq!(workflow.shell_snapshot().diff_rows.len(), 1);

    let reverted = handle_gui_action(&mut workflow, "editor-revert", 11);
    assert_eq!(
        reverted.status,
        format!("Editor reverted: {} dirty=false", file_path.display())
    );
    persist_gui_action_writes(&client, reverted.writes)
        .await
        .unwrap();
    assert_eq!(fs::read_to_string(&file_path).unwrap(), "alpha\n");
    let tabs = persistence.list_editor_tabs().await.unwrap();
    assert_eq!(tabs.len(), 1);
    assert!(tabs[0].document_json.contains("\"dirty\":false"));
    assert!(workflow.shell_snapshot().diff_rows.is_empty());

    let appended = handle_gui_action(&mut workflow, "editor-append:beta", 12);
    assert_eq!(
        appended.status,
        format!("Editor appended: {} dirty=true", file_path.display())
    );
    persist_gui_action_writes(&client, appended.writes)
        .await
        .unwrap();

    let saved = handle_gui_action(&mut workflow, "editor-save", 13);
    assert_eq!(
        saved.status,
        format!("Editor save queued: {}", file_path.display())
    );
    assert!(matches!(
        saved.writes.first(),
        Some(fastrock_app::GuiPersistenceWrite::SaveEditorFile { path, contents })
            if path == file_path.to_string_lossy().as_ref() && contents.contains("beta")
    ));
    persist_gui_action_writes(&client, saved.writes)
        .await
        .unwrap();

    assert_eq!(fs::read_to_string(&file_path).unwrap(), "alpha\nbeta");
    let tabs = persistence.list_editor_tabs().await.unwrap();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].path, file_path.to_string_lossy());
    assert!(tabs[0].document_json.contains("\"dirty\":false"));

    persistence.shutdown().await.unwrap();
}

#[tokio::test]
async fn t18_t19_gui_mcp_and_skill_actions_persist_through_write_queue() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let client = persistence.client();
    let mut workflow = FastrockWorkflow::default();

    let added = handle_gui_action(&mut workflow, "mcp-add", 10);
    persist_gui_action_writes(&client, added.writes)
        .await
        .unwrap();
    let server_id = workflow.mcp_config().servers[0].id.clone();
    let document = persistence
        .load_settings_document(ConfigScope::Global, "mcp.config")
        .await
        .unwrap()
        .unwrap();
    assert!(document.document_json.contains(&server_id));

    let restart = handle_gui_action(&mut workflow, &format!("mcp-restart:{server_id}"), 11);
    persist_gui_action_writes(&client, restart.writes)
        .await
        .unwrap();
    let statuses = persistence.list_mcp_statuses().await.unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].server_id, server_id);
    assert_eq!(statuses[0].status, "starting");

    let disabled = handle_gui_action(&mut workflow, &format!("mcp-toggle:{server_id}"), 12);
    persist_gui_action_writes(&client, disabled.writes)
        .await
        .unwrap();
    let hydrated = load_persisted_workflow(&persistence).await.unwrap();
    assert!(!hydrated.mcp_config().servers[0].enabled);

    let deleted = handle_gui_action(&mut workflow, &format!("mcp-delete:{server_id}"), 13);
    persist_gui_action_writes(&client, deleted.writes)
        .await
        .unwrap();
    let document = persistence
        .load_settings_document(ConfigScope::Global, "mcp.config")
        .await
        .unwrap()
        .unwrap();
    assert!(document.document_json.contains(r#""servers":[]"#));
    assert!(persistence.list_mcp_statuses().await.unwrap().is_empty());

    let created = handle_gui_action(&mut workflow, "skill-create", 20);
    persist_gui_action_writes(&client, created.writes)
        .await
        .unwrap();
    let tabs = persistence.list_editor_tabs().await.unwrap();
    assert_eq!(tabs.len(), 1);
    assert!(tabs[0].path.ends_with("SKILL.md"));

    let skill_name = workflow.skills()[0].manifest.name.clone();
    let removed = handle_gui_action(&mut workflow, &format!("skill-delete:{skill_name}"), 21);
    persist_gui_action_writes(&client, removed.writes)
        .await
        .unwrap();
    assert!(persistence.list_editor_tabs().await.unwrap().is_empty());

    persistence.shutdown().await.unwrap();
}

#[tokio::test]
async fn t17_workflow_loads_local_project_metadata_and_persists_recent_conversations() {
    let temp_dir = tempdir().unwrap();
    let project_root = temp_dir.path().join("repo");
    fs::create_dir_all(project_root.join(".fastrock").join("skills")).unwrap();
    fs::create_dir_all(project_root.join(".roo")).unwrap();
    fs::create_dir_all(project_root.join(".git")).unwrap();
    fs::write(
        project_root.join("AGENTS.md"),
        "Use rtk for every command.\n",
    )
    .unwrap();
    fs::write(
        project_root.join("CLAUDE.md"),
        "No approval prompts by default.\n",
    )
    .unwrap();
    fs::write(
        project_root.join(".roo").join("rules.md"),
        "Prefer Bedrock Mantle.\n",
    )
    .unwrap();

    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let mut workflow = FastrockWorkflow::default();
    workflow
        .upsert_persisted_project_folder(
            &persistence,
            ProjectFolderRecord {
                id: "folder-local".to_owned(),
                label: "Fastrock Local".to_owned(),
                target_kind: "local".to_owned(),
                path: project_root.to_string_lossy().into_owned(),
                remote_target_id: None,
                document_json: serde_json::json!({
                    "default_profile_id": "profile-mantle",
                    "watch": true
                })
                .to_string(),
                updated_at_ms: 10,
            },
        )
        .await
        .unwrap();

    workflow
        .create_persisted_conversation(&persistence, "conversation-a", "A", "folder-local", 11)
        .await
        .unwrap();
    workflow
        .create_persisted_conversation(&persistence, "conversation-b", "B", "folder-local", 12)
        .await
        .unwrap();

    let persisted_folder = persistence
        .list_project_folders()
        .await
        .unwrap()
        .into_iter()
        .find(|folder| folder.id == "folder-local")
        .unwrap();
    let persisted_document: serde_json::Value =
        serde_json::from_str(&persisted_folder.document_json).unwrap();
    assert_eq!(persisted_folder.updated_at_ms, 12);
    assert_eq!(persisted_document["watch"], true);
    assert_eq!(
        persisted_document["recent_conversation_ids"],
        serde_json::json!(["conversation-b", "conversation-a"])
    );

    let loaded = load_persisted_workflow(&persistence).await.unwrap();
    let snapshot = loaded
        .load_local_project_metadata_snapshot("folder-local")
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.folder_id, "folder-local");
    assert_eq!(snapshot.label, "Fastrock Local");
    assert_eq!(
        snapshot.default_profile_id.as_deref(),
        Some("profile-mantle")
    );
    assert_eq!(
        snapshot.recent_conversation_ids,
        vec!["conversation-b".to_owned(), "conversation-a".to_owned()]
    );
    assert!(snapshot.settings_path.ends_with(".fastrock/settings.toml"));
    assert!(snapshot.skills_dir.ends_with(".fastrock/skills"));
    assert!(snapshot.mcp_config_path.ends_with(".fastrock/mcp.json"));
    assert!(snapshot.compatible_mcp_config_path.ends_with(".mcp.json"));
    assert_eq!(snapshot.rtk_status, "available");
    assert!(snapshot.rtk_resolved_path.is_some());
    assert!(snapshot.git.unwrap().git_dir.ends_with(".git"));
    assert_eq!(
        snapshot
            .instructions
            .iter()
            .map(|instruction| instruction.kind)
            .collect::<Vec<_>>(),
        vec![
            ProjectInstructionSnapshotKind::Agents,
            ProjectInstructionSnapshotKind::Claude,
            ProjectInstructionSnapshotKind::RooRules,
        ]
    );
    assert!(
        snapshot.instructions[0]
            .contents
            .contains("Use rtk for every command")
    );

    persistence.shutdown().await.unwrap();
}

#[tokio::test]
async fn v14_project_file_events_hot_reload_mcp_and_skills() {
    let temp_dir = tempdir().unwrap();
    let project_root = temp_dir.path().join("repo");
    fs::create_dir_all(project_root.join(".fastrock").join("skills").join("hot")).unwrap();
    fs::create_dir_all(project_root.join(".fastrock")).unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let client = persistence.client();
    let mut workflow = FastrockWorkflow::default();
    workflow
        .upsert_persisted_project_folder(
            &persistence,
            ProjectFolderRecord {
                id: "folder-local".to_owned(),
                label: "Fastrock Local".to_owned(),
                target_kind: "local".to_owned(),
                path: project_root.to_string_lossy().into_owned(),
                remote_target_id: None,
                document_json: "{}".to_owned(),
                updated_at_ms: 10,
            },
        )
        .await
        .unwrap();

    let project_mcp = McpConfigSet {
        servers: vec![mcp_stdio_server("project-files", true)],
    };
    fs::write(
        project_root.join(".fastrock").join("mcp.json"),
        serde_json::to_string(&project_mcp).unwrap(),
    )
    .unwrap();
    let mcp_reload = workflow
        .handle_local_project_file_event(
            "folder-local",
            &project_root,
            ProjectFileEvent::Created {
                path: PathBuf::from(".fastrock/mcp.json"),
            },
            20,
        )
        .unwrap();
    assert_eq!(
        mcp_reload.status,
        "Project MCP hot reload: added:project-files"
    );
    persist_gui_action_writes(&client, mcp_reload.writes)
        .await
        .unwrap();
    assert!(matches!(
        workflow.mcp_config().servers[0].scope,
        McpConfigScope::Project { ref project_folder_id } if project_folder_id == "folder-local"
    ));
    let project_document = persistence
        .load_settings_document(
            ConfigScope::Project("folder-local".to_owned()),
            "mcp.config",
        )
        .await
        .unwrap()
        .unwrap();
    assert!(project_document.document_json.contains("project-files"));

    let hydrated = load_persisted_workflow(&persistence).await.unwrap();
    assert!(matches!(
        hydrated.mcp_config().servers[0].scope,
        McpConfigScope::Project { ref project_folder_id } if project_folder_id == "folder-local"
    ));

    fs::write(
        project_root
            .join(".fastrock")
            .join("skills")
            .join("hot")
            .join("SKILL.md"),
        "---\nname: hot\ndescription: Hot skill\n---\nUse rtk after hot reload.\n",
    )
    .unwrap();
    let skill_reload = workflow
        .handle_local_project_file_event(
            "folder-local",
            &project_root,
            ProjectFileEvent::Created {
                path: PathBuf::from(".fastrock/skills/hot/SKILL.md"),
            },
            21,
        )
        .unwrap();
    assert_eq!(skill_reload.status, "Project skills hot reload: 1 skills");
    assert!(workflow.skills().iter().any(|skill| {
        skill.manifest.name == "hot"
            && matches!(
                &skill.manifest.scope,
                SkillScope::Project(project_id) if project_id == "folder-local"
            )
            && skill.body.contains("hot reload")
    }));

    fs::remove_dir_all(project_root.join(".fastrock").join("skills").join("hot")).unwrap();
    let skill_delete = workflow
        .handle_local_project_file_event(
            "folder-local",
            &project_root,
            ProjectFileEvent::Deleted {
                path: PathBuf::from(".fastrock/skills/hot/SKILL.md"),
            },
            22,
        )
        .unwrap();
    assert_eq!(skill_delete.status, "Project skills hot reload: 0 skills");
    assert!(workflow.skills().iter().all(
        |skill| !matches!(&skill.manifest.scope, SkillScope::Project(project_id)
            if project_id == "folder-local")
    ));

    persistence.shutdown().await.unwrap();
}

#[test]
fn v14_watchable_local_project_folders_respect_watch_flag_and_target_kind() {
    let temp_dir = tempdir().unwrap();
    let mut workflow = FastrockWorkflow::default();
    workflow.upsert_project_folder(ProjectFolderRecord {
        id: "default-watch".to_owned(),
        label: "Default Watch".to_owned(),
        target_kind: "local".to_owned(),
        path: temp_dir
            .path()
            .join("default")
            .to_string_lossy()
            .into_owned(),
        remote_target_id: None,
        document_json: "{}".to_owned(),
        updated_at_ms: 1,
    });
    workflow.upsert_project_folder(ProjectFolderRecord {
        id: "disabled-watch".to_owned(),
        label: "Disabled Watch".to_owned(),
        target_kind: "local".to_owned(),
        path: temp_dir
            .path()
            .join("disabled")
            .to_string_lossy()
            .into_owned(),
        remote_target_id: None,
        document_json: serde_json::json!({ "watch": false }).to_string(),
        updated_at_ms: 2,
    });
    workflow.upsert_project_folder(ProjectFolderRecord {
        id: "ssh-watch".to_owned(),
        label: "SSH Watch".to_owned(),
        target_kind: "ssh".to_owned(),
        path: "/srv/app".to_owned(),
        remote_target_id: Some("ssh".to_owned()),
        document_json: serde_json::json!({ "watch": true }).to_string(),
        updated_at_ms: 3,
    });

    let folders = workflow.watchable_local_project_folders().unwrap();

    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].folder_id, "default-watch");
    assert_eq!(folders[0].label, "Default Watch");
    assert!(folders[0].root.ends_with("default"));
}

#[tokio::test]
async fn t18_workflow_persists_mcp_gui_management_edits() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let mut workflow = FastrockWorkflow::default();

    assert_eq!(
        workflow
            .add_persisted_mcp_server(&persistence, mcp_stdio_server("filesystem", true), 10)
            .await
            .unwrap()
            .unwrap(),
        vec![McpConfigChange::Added("filesystem".to_owned())]
    );
    assert_eq!(
        workflow
            .set_persisted_mcp_tool_disabled(&persistence, "filesystem", "delete_file", true, 11)
            .await
            .unwrap()
            .unwrap(),
        vec![McpConfigChange::Updated("filesystem".to_owned())]
    );
    assert_eq!(
        workflow
            .set_persisted_mcp_tool_always_allowed(
                &persistence,
                "filesystem",
                "delete_file",
                true,
                12,
            )
            .await
            .unwrap()
            .unwrap(),
        vec![McpConfigChange::Updated("filesystem".to_owned())]
    );
    assert!(
        workflow.mcp_config().servers[0]
            .always_allow_tools
            .contains("delete_file")
    );
    assert!(
        !workflow.mcp_config().servers[0]
            .disabled_tools
            .contains("delete_file")
    );
    assert_eq!(
        workflow
            .set_persisted_mcp_server_enabled(&persistence, "filesystem", false, 13)
            .await
            .unwrap()
            .unwrap(),
        vec![McpConfigChange::Disabled("filesystem".to_owned())]
    );
    let runtime_snapshot = McpRuntimeSnapshot::stopped(&workflow.mcp_config().servers[0], 13)
        .with_discovery(
            ["read_file", "write_file"],
            ["file://README.md"],
            ["file://{path}"],
        )
        .mark_running(14);
    workflow
        .record_persisted_mcp_status(&persistence, runtime_snapshot)
        .await
        .unwrap();
    assert_eq!(
        workflow.mcp_status("filesystem").unwrap().status,
        McpServerRuntimeStatus::Running
    );

    let mut updated = mcp_stdio_server("filesystem", false);
    updated.name = "Filesystem Edited".to_owned();
    updated.always_allow_tools.insert("delete_file".to_owned());
    assert_eq!(
        workflow
            .update_persisted_mcp_server(&persistence, updated, 14)
            .await
            .unwrap()
            .unwrap(),
        vec![McpConfigChange::Updated("filesystem".to_owned())]
    );
    let hydrated = load_persisted_workflow(&persistence).await.unwrap();
    assert_eq!(hydrated.mcp_config().servers[0].name, "Filesystem Edited");
    assert!(
        hydrated.mcp_config().servers[0]
            .always_allow_tools
            .contains("delete_file")
    );
    assert_eq!(
        hydrated.mcp_status("filesystem").unwrap().discovered_tools,
        vec!["read_file", "write_file"]
    );

    assert_eq!(
        workflow
            .delete_persisted_mcp_server(&persistence, "filesystem", 15)
            .await
            .unwrap()
            .unwrap(),
        vec![McpConfigChange::Removed("filesystem".to_owned())]
    );
    let document = persistence
        .load_settings_document(ConfigScope::Global, "mcp.config")
        .await
        .unwrap()
        .unwrap();
    assert!(document.document_json.contains(r#""servers":[]"#));
    assert!(workflow.mcp_status("filesystem").is_none());
    assert!(persistence.list_mcp_statuses().await.unwrap().is_empty());

    persistence.shutdown().await.unwrap();
}

#[tokio::test]
async fn t18_mcp_oauth_tokens_are_keyring_backed_secret_refs() {
    let temp_dir = tempdir().unwrap();
    let database_path = temp_dir.path().join(DATABASE_FILE_NAME);
    let persistence = PersistenceActorHandle::spawn_with_secret_store(
        PersistenceConfig::new(&database_path),
        MemorySecretStore::default(),
    )
    .unwrap();
    let mut workflow = FastrockWorkflow::default();
    workflow
        .configure_mcp(McpConfigSet {
            servers: vec![mcp_oauth_server("oauth-server")],
        })
        .unwrap();
    let tokens = McpOAuthTokenSet {
        access_token: "not-a-real-access-token".to_owned(),
        refresh_token: Some("not-a-real-refresh-token".to_owned()),
        token_type: "Bearer".to_owned(),
        expires_at_ms: Some(123_456),
        scopes: vec!["tools".to_owned()],
    };

    let snapshot = workflow
        .store_persisted_mcp_oauth_tokens(&persistence, "oauth-server", tokens.clone(), 50)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        snapshot.oauth_state,
        McpOAuthState::Authorized {
            token_ref: mcp_oauth_token_secret_ref("oauth-server"),
        }
    );
    assert_eq!(
        workflow
            .load_persisted_mcp_oauth_tokens(&persistence, "oauth-server")
            .await
            .unwrap(),
        Some(tokens)
    );
    assert!(
        !format!("{:?}", workflow.mcp_status("oauth-server").unwrap())
            .contains("not-a-real-access-token")
    );
    let refs = persistence.list_secret_refs().await.unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].service, MCP_OAUTH_TOKEN_SECRET_SERVICE);
    assert_eq!(refs[0].account, "oauth-server");
    let export = persistence.export_redacted_snapshot().await.unwrap();
    let export_json = export.to_json_pretty().unwrap();
    assert!(export_json.contains("secret:mcp-oauth:oauth-server"));
    assert!(!export_json.contains("not-a-real-access-token"));

    persistence.shutdown().await.unwrap();
    assert!(
        !sqlite_file_contains(&database_path, "not-a-real-access-token").unwrap(),
        "database must not contain OAuth token material"
    );
}

#[test]
fn t19_workflow_manages_skill_files_for_gui_actions() {
    let temp_dir = tempdir().unwrap();
    let global_root = temp_dir.path().join("global");
    let project_root = temp_dir.path().join("project");
    let mut workflow = FastrockWorkflow::default();
    let mut request = SkillWriteRequest::new("deploy", "Deploy skill", "Use rtk deploy.");
    request.mode_slugs = vec!["code".to_owned()];

    let created = workflow
        .create_skill(&global_root, SkillScope::Global, request.clone())
        .unwrap();

    assert_eq!(created.manifest.scope, SkillScope::Global);
    assert_eq!(workflow.skills().len(), 1);

    request.description = "Deploy debug skill".to_owned();
    request.mode_slugs = vec!["debug".to_owned()];
    let edited = workflow
        .edit_skill(&global_root, SkillScope::Global, request)
        .unwrap();

    assert_eq!(edited.manifest.description, "Deploy debug skill");
    assert_eq!(workflow.skills()[0].manifest.mode_slugs, vec!["debug"]);

    let moved = workflow
        .move_skill(
            &global_root,
            &SkillScope::Global,
            &project_root,
            SkillScope::Project("project-1".to_owned()),
            "deploy",
        )
        .unwrap();

    assert_eq!(
        moved.manifest.scope,
        SkillScope::Project("project-1".to_owned())
    );
    assert!(!global_root.join("deploy").exists());
    assert!(project_root.join("deploy").join("SKILL.md").exists());
    assert_eq!(workflow.skills().len(), 1);
    assert_eq!(
        workflow.skills()[0].manifest.scope,
        SkillScope::Project("project-1".to_owned())
    );

    assert!(
        workflow
            .delete_skill(
                &project_root,
                &SkillScope::Project("project-1".to_owned()),
                "deploy",
            )
            .unwrap()
    );
    assert!(workflow.skills().is_empty());
}

#[tokio::test]
async fn t11_conversation_actor_persists_transcript_events_to_sqlite() {
    let temp_dir = tempdir().unwrap();
    let persistence = PersistenceActorHandle::spawn(PersistenceConfig::new(
        temp_dir.path().join(DATABASE_FILE_NAME),
    ))
    .unwrap();
    let mut workflow = FastrockWorkflow::default();
    let conversation_id = workflow
        .create_persisted_conversation(&persistence, "actor", "Actor", "/repo", 200)
        .await
        .unwrap();
    let store = Arc::new(PersistenceTranscriptStore::new(persistence.client()));
    let (handle, mut actor_events) = ConversationActor::spawn_on(
        &tokio::runtime::Handle::current(),
        conversation_id.clone(),
        store.clone(),
        ConversationActorConfig::default(),
    )
    .unwrap();

    assert!(matches!(
        actor_events.recv().await,
        Some(ConversationActorEvent::Started { .. })
    ));
    handle
        .send(ConversationCommand::AppendUserMessage {
            text: "hello".to_owned(),
        })
        .await
        .unwrap();
    handle
        .send(ConversationCommand::SetStatus {
            status: ConversationStatus::WaitingForModel,
        })
        .await
        .unwrap();

    let mut appended = 0;
    while appended < 2 {
        if matches!(
            actor_events.recv().await,
            Some(ConversationActorEvent::TranscriptAppended { .. })
        ) {
            appended += 1;
        }
    }
    handle.send(ConversationCommand::Stop).await.unwrap();
    loop {
        if matches!(
            actor_events.recv().await,
            Some(ConversationActorEvent::Stopped { .. })
        ) {
            break;
        }
    }

    let loaded = store.load(conversation_id.clone()).await.unwrap();
    assert_eq!(loaded.len(), 2);
    assert!(matches!(
        &loaded[0].kind,
        TranscriptEventKind::UserMessage { text } if text == "hello"
    ));
    assert!(matches!(
        &loaded[1].kind,
        TranscriptEventKind::StatusChanged {
            status: ConversationStatus::WaitingForModel
        }
    ));
    assert_eq!(
        persistence
            .load_conversation_events(conversation_id.0)
            .await
            .unwrap()
            .len(),
        3
    );

    persistence.shutdown().await.unwrap();
}

fn profile_request(
    id: &str,
    name: &str,
    provider: UiLlmProvider,
    default_for_new_conversations: bool,
) -> BedrockProfileRequest {
    BedrockProfileRequest {
        id: id.to_owned(),
        name: name.to_owned(),
        provider,
        model_id: "anthropic.claude-3-5-sonnet-20241022-v2:0".to_owned(),
        region: "us-east-1".to_owned(),
        aws_profile: "default".to_owned(),
        default_for_new_conversations,
    }
}

fn rtk_request(cwd: &str) -> RtkCommandRequest {
    RtkCommandRequest {
        cwd: cwd.to_owned(),
        command: vec!["true".to_owned()],
        shell: None,
        pty: false,
        timeout_ms: None,
        env: BTreeMap::new(),
        stdin: None,
    }
}

fn test_ssh_target() -> SshRemoteTarget {
    SshRemoteTarget {
        id: "ssh-policy".to_owned(),
        host_label: "ssh-policy".to_owned(),
        user: None,
        host: "example.test".to_owned(),
        root: "/repo".to_owned(),
        rtk_binary: "rtk".to_owned(),
    }
}

fn test_ssm_target() -> SsmRemoteTarget {
    SsmRemoteTarget {
        id: "ssm-policy".to_owned(),
        profile_name: Some("default".to_owned()),
        region: "us-east-1".to_owned(),
        target: "i-test".to_owned(),
        root: "/repo".to_owned(),
        rtk_binary: "rtk".to_owned(),
    }
}

fn argv(parts: Vec<&str>) -> Vec<String> {
    parts.into_iter().map(str::to_owned).collect()
}

fn diagnostic_status(
    diagnostics: &fastrock_projects::RemoteTargetDiagnostics,
    code: RemoteDiagnosticCheckCode,
) -> Option<RemoteDiagnosticStatus> {
    diagnostics
        .checks
        .iter()
        .find(|check| check.code == code)
        .map(|check| check.status)
}

fn only_run_model_work(runtime_work: &[GuiRuntimeWork]) -> BedrockModelRunRequest {
    match runtime_work {
        [GuiRuntimeWork::RunModel(request)] => request.clone(),
        other => panic!("expected exactly one model work item, got {other:?}"),
    }
}

#[derive(Debug, Clone)]
struct SlowMantleTransport {
    delay_ms: u64,
}

impl SlowMantleTransport {
    fn new(delay_ms: u64) -> Self {
        Self { delay_ms }
    }
}

impl MantleHttpTransport for SlowMantleTransport {
    fn send(
        &self,
        _request: MantleHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<MantleHttpResponse, MantleClientError>> + Send + '_>>
    {
        let delay_ms = self.delay_ms;
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            Ok(MantleHttpResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: b"event: response.created\ndata: {\"id\":\"resp-slow\"}\n\nevent: response.completed\ndata: {}\n\n".to_vec(),
            })
        })
    }
}

fn create_fake_rtk(root: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let path = root.join("fake-rtk.cmd");
        std::fs::write(&path, "@echo off\necho ok\n").unwrap();
        path
    }

    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let path = root.join("fake-rtk");
        std::fs::write(&path, "#!/bin/sh\nprintf 'ok\\n'\n").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }
}

fn create_fake_credential_process(root: &Path) -> PathBuf {
    let payload = r#"{"Version":1,"AccessKeyId":"process-key","SecretAccessKey":"process-secret","SessionToken":"process-token"}"#;
    #[cfg(windows)]
    {
        let path = root.join("fake-credential-process.cmd");
        std::fs::write(&path, format!("@echo off\necho {payload}\n")).unwrap();
        path
    }

    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let path = root.join("fake-credential-process");
        std::fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{payload}'\n")).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }
}

fn create_fake_aws_cli_export_credentials(root: &Path) -> PathBuf {
    let sso_payload = r#"{"Version":1,"AccessKeyId":"sso-export-key","SecretAccessKey":"sso-export-secret","SessionToken":"sso-export-token"}"#;
    let role_payload = r#"{"Version":1,"AccessKeyId":"role-export-key","SecretAccessKey":"role-export-secret","SessionToken":"role-export-token"}"#;
    let default_payload = r#"{"Version":1,"AccessKeyId":"default-export-key","SecretAccessKey":"default-export-secret","SessionToken":"default-export-token"}"#;
    #[cfg(windows)]
    {
        let path = root.join("fake-aws.cmd");
        std::fs::write(
            &path,
            format!(
                "@echo off\r\necho %* | find \"--profile sso\" >NUL\r\nif %ERRORLEVEL%==0 echo {sso_payload}& exit /B 0\r\necho %* | find \"--profile role\" >NUL\r\nif %ERRORLEVEL%==0 echo {role_payload}& exit /B 0\r\necho %* | find \"--profile\" >NUL\r\nif %ERRORLEVEL%==1 echo {default_payload}& exit /B 0\r\nexit /B 7\r\n"
            ),
        )
        .unwrap();
        path
    }

    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let path = root.join("fake-aws");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\ncase \" $* \" in\n  *\" --profile sso \"*) printf '%s\\n' '{sso_payload}'; exit 0 ;;\n  *\" --profile role \"*) printf '%s\\n' '{role_payload}'; exit 0 ;;\n  *\" --profile \"*) exit 7 ;;\n  *) printf '%s\\n' '{default_payload}'; exit 0 ;;\nesac\n"
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }
}

fn mcp_stdio_server(id: &str, enabled: bool) -> McpServerConfig {
    McpServerConfig {
        id: id.to_owned(),
        name: id.to_owned(),
        scope: McpConfigScope::Global,
        enabled,
        transport: McpTransportConfig::Stdio {
            command: format!("mcp-{id}"),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
        },
        timeout_ms: 30_000,
        always_allow_tools: BTreeSet::new(),
        disabled_tools: BTreeSet::new(),
        oauth: None,
    }
}

fn mcp_oauth_server(id: &str) -> McpServerConfig {
    McpServerConfig {
        id: id.to_owned(),
        name: id.to_owned(),
        scope: McpConfigScope::Global,
        enabled: true,
        transport: McpTransportConfig::StreamableHttp {
            url: format!("https://mcp.example.test/{id}"),
            headers: BTreeMap::new(),
        },
        timeout_ms: 30_000,
        always_allow_tools: BTreeSet::new(),
        disabled_tools: BTreeSet::new(),
        oauth: Some(McpOAuthConfig {
            client_id: "client-1".to_owned(),
            client_secret_ref: "secret:mcp-client:client-1".to_owned(),
            authorization_url: "https://mcp.example.test/oauth/authorize".to_owned(),
            token_url: "https://mcp.example.test/oauth/token".to_owned(),
            scopes: vec!["tools".to_owned()],
        }),
    }
}

#[derive(Default)]
struct MemorySecretStore {
    values: Mutex<BTreeMap<(String, String), String>>,
}

impl SecretStore for MemorySecretStore {
    fn put_secret(&self, record: &SecretRefRecord, secret: &str) -> Result<(), SecretStoreError> {
        self.values.lock().unwrap().insert(
            (record.service.clone(), record.account.clone()),
            secret.to_owned(),
        );
        Ok(())
    }

    fn get_secret(&self, record: &SecretRefRecord) -> Result<String, SecretStoreError> {
        self.values
            .lock()
            .unwrap()
            .get(&(record.service.clone(), record.account.clone()))
            .cloned()
            .ok_or(SecretStoreError::NotFound)
    }
}
