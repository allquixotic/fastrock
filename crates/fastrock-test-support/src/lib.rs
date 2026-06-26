#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use fastrock_bedrock::{
    MantleClientError, MantleHttpRequest, MantleHttpResponse, MantleHttpTransport,
    RuntimeClientError, RuntimeHttpRequest, RuntimeHttpResponse, RuntimeHttpTransport,
};
use fastrock_mcp::{McpConfigScope, McpConfigSet, McpServerConfig, McpTransportConfig};
use fastrock_projects::{
    RemoteCommandError, RemoteCommandRequest, RemoteCommandTranscript, RemoteFileReadRequest,
    RemoteFileReadResult, RemoteFileWriteRequest, RemoteFileWriteResult, SshRemoteFileTransport,
    SshRemoteTarget, SshRemoteTransport, SsmRemoteFileTransport, SsmRemoteTarget,
    SsmRemoteTransport,
};
use fastrock_rtk::{RtkCommandEvent, RtkCommandTranscript};

pub const TEST_SUPPORT_CRATE_NAME: &str = "fastrock-test-support";

#[derive(Debug, Clone)]
pub struct FakeMantleTransport {
    responses: Arc<Mutex<Vec<MantleHttpResponse>>>,
    requests: Arc<Mutex<Vec<MantleHttpRequest>>>,
}

impl FakeMantleTransport {
    pub fn new(responses: Vec<MantleHttpResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().rev().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn requests(&self) -> Vec<MantleHttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl MantleHttpTransport for FakeMantleTransport {
    fn send(
        &self,
        request: MantleHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<MantleHttpResponse, MantleClientError>> + Send + '_>>
    {
        self.requests.lock().unwrap().push(request);
        let response = self.responses.lock().unwrap().pop().unwrap();
        Box::pin(async move { Ok(response) })
    }
}

#[derive(Debug, Clone)]
pub struct FakeRuntimeTransport {
    responses: Arc<Mutex<Vec<RuntimeHttpResponse>>>,
    requests: Arc<Mutex<Vec<RuntimeHttpRequest>>>,
}

impl FakeRuntimeTransport {
    pub fn new(responses: Vec<RuntimeHttpResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().rev().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn requests(&self) -> Vec<RuntimeHttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl RuntimeHttpTransport for FakeRuntimeTransport {
    fn send(
        &self,
        request: RuntimeHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RuntimeHttpResponse, RuntimeClientError>> + Send + '_>>
    {
        self.requests.lock().unwrap().push(request);
        let response = self.responses.lock().unwrap().pop().unwrap();
        Box::pin(async move { Ok(response) })
    }
}

#[derive(Debug, Clone, Default)]
pub struct FakeRemoteTransport {
    requests: Arc<Mutex<Vec<(String, RemoteCommandRequest)>>>,
    file_reads: Arc<Mutex<Vec<(String, RemoteFileReadRequest)>>>,
    file_writes: Arc<Mutex<Vec<(String, RemoteFileWriteRequest)>>>,
}

impl FakeRemoteTransport {
    pub fn requests(&self) -> Vec<(String, RemoteCommandRequest)> {
        self.requests.lock().unwrap().clone()
    }

    pub fn file_reads(&self) -> Vec<(String, RemoteFileReadRequest)> {
        self.file_reads.lock().unwrap().clone()
    }

    pub fn file_writes(&self) -> Vec<(String, RemoteFileWriteRequest)> {
        self.file_writes.lock().unwrap().clone()
    }
}

impl SshRemoteTransport for FakeRemoteTransport {
    fn execute(
        &self,
        target: SshRemoteTarget,
        request: RemoteCommandRequest,
    ) -> Pin<
        Box<dyn Future<Output = Result<RemoteCommandTranscript, RemoteCommandError>> + Send + '_>,
    > {
        self.requests
            .lock()
            .unwrap()
            .push((target.id.clone(), request.clone()));
        Box::pin(async move { Ok(fake_remote_transcript(target.id, request)) })
    }
}

impl SsmRemoteTransport for FakeRemoteTransport {
    fn execute(
        &self,
        target: SsmRemoteTarget,
        request: RemoteCommandRequest,
    ) -> Pin<
        Box<dyn Future<Output = Result<RemoteCommandTranscript, RemoteCommandError>> + Send + '_>,
    > {
        self.requests
            .lock()
            .unwrap()
            .push((target.id.clone(), request.clone()));
        Box::pin(async move { Ok(fake_remote_transcript(target.id, request)) })
    }
}

impl SshRemoteFileTransport for FakeRemoteTransport {
    fn read_file(
        &self,
        target: SshRemoteTarget,
        request: RemoteFileReadRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteFileReadResult, RemoteCommandError>> + Send + '_>>
    {
        self.file_reads
            .lock()
            .unwrap()
            .push((target.id.clone(), request.clone()));
        Box::pin(async move { Ok(fake_remote_file_read(target.id, request)) })
    }

    fn write_file(
        &self,
        target: SshRemoteTarget,
        request: RemoteFileWriteRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteFileWriteResult, RemoteCommandError>> + Send + '_>>
    {
        self.file_writes
            .lock()
            .unwrap()
            .push((target.id.clone(), request.clone()));
        Box::pin(async move { Ok(fake_remote_file_write(target.id, request)) })
    }
}

impl SsmRemoteFileTransport for FakeRemoteTransport {
    fn read_file(
        &self,
        target: SsmRemoteTarget,
        request: RemoteFileReadRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteFileReadResult, RemoteCommandError>> + Send + '_>>
    {
        self.file_reads
            .lock()
            .unwrap()
            .push((target.id.clone(), request.clone()));
        Box::pin(async move { Ok(fake_remote_file_read(target.id, request)) })
    }

    fn write_file(
        &self,
        target: SsmRemoteTarget,
        request: RemoteFileWriteRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteFileWriteResult, RemoteCommandError>> + Send + '_>>
    {
        self.file_writes
            .lock()
            .unwrap()
            .push((target.id.clone(), request.clone()));
        Box::pin(async move { Ok(fake_remote_file_write(target.id, request)) })
    }
}

pub fn fake_remote_transcript(
    target_id: String,
    request: RemoteCommandRequest,
) -> RemoteCommandTranscript {
    RemoteCommandTranscript {
        target_id,
        cwd: request.cwd,
        argv: request.argv,
        exit_code: Some(0),
        stdout_bytes: b"ok".len(),
        stderr_bytes: 0,
        stdout: b"ok".to_vec(),
        stderr: Vec::new(),
        stdout_truncated: false,
        stderr_truncated: false,
        rtk_savings: None,
        policy_labels: Vec::new(),
    }
}

pub fn fake_remote_file_read(
    target_id: String,
    request: RemoteFileReadRequest,
) -> RemoteFileReadResult {
    RemoteFileReadResult {
        target_id,
        path: request.path.clone(),
        contents: format!("fake remote file: {}\n", request.path).into_bytes(),
    }
}

pub fn fake_remote_file_write(
    target_id: String,
    request: RemoteFileWriteRequest,
) -> RemoteFileWriteResult {
    RemoteFileWriteResult {
        target_id,
        path: request.path,
        bytes_written: request.contents.len(),
    }
}

pub fn fake_rtk_transcript(argv: Vec<String>) -> RtkCommandTranscript {
    RtkCommandTranscript {
        cwd: ".".to_owned(),
        argv: argv.clone(),
        pty_requested: false,
        exit_code: Some(0),
        timed_out: false,
        cancelled: false,
        stdout_bytes: b"ok\n".len(),
        stderr_bytes: 0,
        stdout: b"ok\n".to_vec(),
        stderr: Vec::new(),
        stdout_truncated: false,
        stderr_truncated: false,
        rtk_savings: None,
        policy_labels: Vec::new(),
        events: vec![
            RtkCommandEvent::Started {
                cwd: ".".to_owned(),
                argv,
            },
            RtkCommandEvent::Stdout {
                bytes: b"ok\n".to_vec(),
            },
            RtkCommandEvent::Exited { exit_code: Some(0) },
        ],
    }
}

pub fn fake_mcp_config_set() -> McpConfigSet {
    McpConfigSet {
        servers: vec![McpServerConfig {
            id: "fake".to_owned(),
            name: "Fake MCP".to_owned(),
            scope: McpConfigScope::Project {
                project_folder_id: "project".to_owned(),
            },
            enabled: true,
            transport: McpTransportConfig::Stdio {
                command: "fake-mcp".to_owned(),
                args: Vec::new(),
                cwd: None,
                env: BTreeMap::new(),
            },
            timeout_ms: 30_000,
            always_allow_tools: BTreeSet::from(["read_file".to_owned()]),
            disabled_tools: BTreeSet::from(["delete_project".to_owned()]),
            oauth: None,
        }],
    }
}

#[cfg(test)]
mod tests {
    use fastrock_bedrock::{MantleClient, MantleClientAuth, MissingMantleSigV4Signer};
    use fastrock_projects::{
        ProjectTarget, RemoteKind, SshRemoteTarget, run_ssh_remote_rtk_command,
    };
    use fastrock_rtk::RtkCommandRequest;

    use super::*;

    #[tokio::test]
    async fn t25_fake_mantle_transport_records_requests() {
        let transport = FakeMantleTransport::new(vec![MantleHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"data":[]}"#.to_vec(),
        }]);
        let client = MantleClient::new(
            "https://bedrock-mantle.us-east-1.api.aws/v1",
            MantleClientAuth::BearerApiKey {
                api_key: "key".to_owned(),
                project_id: None,
            },
            transport.clone(),
            MissingMantleSigV4Signer,
        );

        client.list_models().await.unwrap();

        assert_eq!(transport.requests().len(), 1);
    }

    #[tokio::test]
    async fn t25_fake_remote_transport_supports_ssh_runner() {
        let transport = FakeRemoteTransport::default();
        let target = SshRemoteTarget {
            id: "ssh".to_owned(),
            host_label: "host".to_owned(),
            user: None,
            host: "example.test".to_owned(),
            root: "/repo".to_owned(),
            rtk_binary: "rtk".to_owned(),
        };
        let transcript = run_ssh_remote_rtk_command(
            &transport,
            target,
            RtkCommandRequest {
                cwd: ".".to_owned(),
                command: vec!["true".to_owned()],
                shell: None,
                pty: false,
                timeout_ms: None,
                env: BTreeMap::new(),
                stdin: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(transcript.exit_code, Some(0));
        assert_eq!(transport.requests().len(), 1);
    }

    #[test]
    fn t25_fake_mcp_config_is_valid() {
        assert_eq!(fake_mcp_config_set().validate(), Ok(()));
    }

    #[test]
    fn t25_exports_project_remote_types() {
        assert_eq!(
            ProjectTarget::Remote("ssh".to_owned()),
            ProjectTarget::Remote("ssh".to_owned())
        );
        assert_eq!(
            RemoteKind::Ssh {
                host_label: "host".to_owned()
            },
            RemoteKind::Ssh {
                host_label: "host".to_owned()
            }
        );
    }
}
