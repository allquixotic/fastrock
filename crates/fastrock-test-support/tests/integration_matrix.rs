use std::collections::BTreeMap;

use crc32fast::Hasher as Crc32Hasher;
use fastrock_bedrock::{
    AwsCredentialSource, AwsStaticCredentials, BedrockRuntimeClient, MantleClient,
    MantleClientAuth, MantleResponseRequest, MissingMantleSigV4Signer, ResolvedBedrockAuth,
    RuntimeContentBlock, RuntimeConverseRequest, RuntimeHttpResponse, RuntimeMessage,
    StaticCredentialsRuntimeSigV4Signer, normalize_mantle_stream_events,
    normalize_runtime_stream_events,
};
use fastrock_core::{ModelProviderKind, NormalizedModelStreamEventKind};
use fastrock_editor::EditorBuffer;
use fastrock_mcp::{McpConfigChange, McpConfigSet};
use fastrock_projects::{
    SshRemoteTarget, SsmRemoteTarget, read_ssh_remote_file, read_ssm_remote_file,
    run_ssh_remote_rtk_command, run_ssm_remote_rtk_command, write_ssh_remote_file,
    write_ssm_remote_file,
};
use fastrock_rtk::RtkCommandRequest;
use fastrock_test_support::{
    FakeMantleTransport, FakeRemoteTransport, FakeRuntimeTransport, fake_mcp_config_set,
};

#[tokio::test]
async fn t26_mocked_bedrock_streams_normalize_across_mantle_and_runtime() {
    let mantle = MantleClient::new(
        "https://bedrock-mantle.us-east-1.api.aws/v1",
        MantleClientAuth::BearerApiKey {
            api_key: "key".to_owned(),
            project_id: None,
        },
        FakeMantleTransport::new(vec![fastrock_bedrock::MantleHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: b"event: response.output_text.delta\ndata: {\"delta\":\"mantle\"}\n\nevent: response.completed\ndata: {}\n\n".to_vec(),
        }]),
        MissingMantleSigV4Signer,
    );
    let mantle_events = mantle
        .stream_response(MantleResponseRequest {
            model: "model".to_owned(),
            input: "hello".to_owned(),
            previous_response_id: None,
            store: true,
            stream: true,
            max_output_tokens: None,
            temperature: None,
            top_p: None,
        })
        .await
        .unwrap();
    let normalized_mantle = normalize_mantle_stream_events(mantle_events);

    let resolved = ResolvedBedrockAuth {
        credential_source: AwsCredentialSource::DefaultChain,
        credential_profile: None,
        region: "us-east-1".to_owned(),
        mantle_sigv4_base_url: "https://bedrock-mantle.us-east-1.api.aws/v1".to_owned(),
        runtime_endpoint: "https://bedrock-runtime.us-east-1.amazonaws.com".to_owned(),
    };
    let runtime = BedrockRuntimeClient::new(
        resolved.runtime_endpoint.clone(),
        resolved,
        FakeRuntimeTransport::new(vec![RuntimeHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: encode_event_stream_messages(&[
                br#"{"contentBlockDelta":{"delta":{"text":"runtime"}}}"#,
                br#"{"messageStop":{}}"#,
            ]),
        }]),
        StaticCredentialsRuntimeSigV4Signer::new(AwsStaticCredentials::new("key", "secret", None)),
    );
    let normalized_runtime = normalize_runtime_stream_events(
        runtime
            .converse_stream(RuntimeConverseRequest {
                model_id: "model".to_owned(),
                additional_model_request_fields: None,
                inference_config: None,
                system: Vec::new(),
                messages: vec![RuntimeMessage {
                    role: "user".to_owned(),
                    content: vec![RuntimeContentBlock::Text {
                        text: "hello".to_owned(),
                    }],
                }],
                service_tier: None,
                tool_config: None,
            })
            .await
            .unwrap(),
    );

    assert_eq!(
        normalized_mantle[0].provider,
        ModelProviderKind::BedrockMantle
    );
    assert!(matches!(
        normalized_mantle[0].kind,
        NormalizedModelStreamEventKind::TextDelta { .. }
    ));
    assert_eq!(
        normalized_runtime[0].provider,
        ModelProviderKind::BedrockRuntime
    );
    assert!(matches!(
        normalized_runtime[0].kind,
        NormalizedModelStreamEventKind::TextDelta { .. }
    ));
}

#[tokio::test]
async fn t26_fake_remote_transport_covers_ssh_and_ssm_rtk_execution() {
    let transport = FakeRemoteTransport::default();
    let request = RtkCommandRequest {
        cwd: ".".to_owned(),
        command: vec!["true".to_owned()],
        shell: None,
        pty: false,
        timeout_ms: None,
        env: BTreeMap::new(),
        stdin: None,
    };

    run_ssh_remote_rtk_command(
        &transport,
        SshRemoteTarget {
            id: "ssh".to_owned(),
            host_label: "ssh".to_owned(),
            user: None,
            host: "example.test".to_owned(),
            root: "/repo".to_owned(),
            rtk_binary: "rtk".to_owned(),
        },
        request.clone(),
    )
    .await
    .unwrap();
    run_ssm_remote_rtk_command(
        &transport,
        SsmRemoteTarget {
            id: "ssm".to_owned(),
            profile_name: Some("default".to_owned()),
            region: "us-east-1".to_owned(),
            target: "i-test".to_owned(),
            root: "/repo".to_owned(),
            rtk_binary: "rtk".to_owned(),
        },
        request,
    )
    .await
    .unwrap();

    assert_eq!(transport.requests().len(), 2);
}

#[tokio::test]
async fn t26_fake_remote_transport_covers_ssh_and_ssm_file_ops() {
    let transport = FakeRemoteTransport::default();
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

    let ssh_read = read_ssh_remote_file(&transport, ssh_target.clone(), "src/main.rs")
        .await
        .unwrap();
    let ssh_write = write_ssh_remote_file(
        &transport,
        ssh_target,
        "src/lib.rs",
        b"pub fn ok() {}\n".to_vec(),
    )
    .await
    .unwrap();
    let ssm_read = read_ssm_remote_file(&transport, ssm_target.clone(), "config/app.toml")
        .await
        .unwrap();
    let ssm_write = write_ssm_remote_file(
        &transport,
        ssm_target,
        "config/app.toml",
        b"enabled = true\n".to_vec(),
    )
    .await
    .unwrap();

    assert_eq!(ssh_read.path, "/repo/src/main.rs");
    assert_eq!(ssh_write.path, "/repo/src/lib.rs");
    assert_eq!(ssh_write.bytes_written, b"pub fn ok() {}\n".len());
    assert_eq!(ssm_read.path, "/repo/config/app.toml");
    assert_eq!(ssm_write.path, "/repo/config/app.toml");
    assert_eq!(ssm_write.bytes_written, b"enabled = true\n".len());
    assert_eq!(transport.file_reads().len(), 2);
    assert_eq!(transport.file_writes().len(), 2);
}

#[test]
fn t26_fake_mcp_config_participates_in_hot_reload_diff() {
    let old = McpConfigSet::default();
    let next = fake_mcp_config_set();

    assert_eq!(
        old.diff_for_hot_reload(&next).unwrap(),
        vec![McpConfigChange::Added("fake".to_owned())]
    );
}

#[test]
fn t26_large_editor_visible_range_stays_bounded() {
    let text = (0..10_000)
        .map(|index| format!("line {index}\n"))
        .collect::<String>();
    let buffer = EditorBuffer::from_text("large.txt", &text);

    let visible = buffer.visible_lines(9_000, 20);

    assert_eq!(visible.len(), 20);
    assert_eq!(visible[0].line_index, 9_000);
}

fn encode_event_stream_messages(payloads: &[&[u8]]) -> Vec<u8> {
    payloads
        .iter()
        .flat_map(|payload| encode_event_stream_message(payload))
        .collect()
}

fn encode_event_stream_message(payload: &[u8]) -> Vec<u8> {
    let headers = encode_event_stream_headers(&[
        (":message-type", "event"),
        (":event-type", "chunk"),
        (":content-type", "application/json"),
    ]);
    let total_len = 16 + headers.len() + payload.len();
    let mut message = Vec::with_capacity(total_len);
    message.extend_from_slice(&(total_len as u32).to_be_bytes());
    message.extend_from_slice(&(headers.len() as u32).to_be_bytes());
    message.extend_from_slice(&crc32(&message).to_be_bytes());
    message.extend_from_slice(&headers);
    message.extend_from_slice(payload);
    message.extend_from_slice(&crc32(&message).to_be_bytes());
    message
}

fn encode_event_stream_headers(headers: &[(&str, &str)]) -> Vec<u8> {
    let mut encoded = Vec::new();
    for (name, value) in headers {
        encoded.push(name.len() as u8);
        encoded.extend_from_slice(name.as_bytes());
        encoded.push(7);
        encoded.extend_from_slice(&(value.len() as u16).to_be_bytes());
        encoded.extend_from_slice(value.as_bytes());
    }
    encoded
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut hasher = Crc32Hasher::new();
    hasher.update(bytes);
    hasher.finalize()
}
