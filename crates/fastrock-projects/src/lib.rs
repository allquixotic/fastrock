#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use fastrock_rtk::{
    DEFAULT_RTK_BINARY, RtkBinaryDiagnostic, RtkCommandRequest, RtkSavingsMetadata,
    diagnose_rtk_binary_for, local_rtk_request_argv_with_binary,
};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_PROJECT_WATCH_CAPACITY: usize = 256;
pub const PROJECT_SETTINGS_RELATIVE_PATH: &str = ".fastrock/settings.toml";
pub const PROJECT_SKILLS_RELATIVE_PATH: &str = ".fastrock/skills";
pub const PROJECT_MCP_RELATIVE_PATH: &str = ".fastrock/mcp.json";
pub const COMPAT_MCP_RELATIVE_PATH: &str = ".mcp.json";
pub const PROJECT_AGENTS_FILE_NAME: &str = "AGENTS.md";
pub const PROJECT_CLAUDE_FILE_NAME: &str = "CLAUDE.md";
pub const PROJECT_ROO_RULES_RELATIVE_PATH: &str = ".roo/rules.md";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFolder {
    pub label: String,
    pub path: String,
    pub target: ProjectTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectTarget {
    Local,
    Remote(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteKind {
    Ssh {
        host_label: String,
    },
    AwsSessionManager {
        profile_name: Option<String>,
        region: String,
        target: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalProjectFolder {
    pub label: String,
    pub root: PathBuf,
}

impl LocalProjectFolder {
    pub fn open(folder: ProjectFolder) -> Result<Self, ProjectError> {
        if folder.target != ProjectTarget::Local {
            return Err(ProjectError::TargetMismatch);
        }
        let root = std::fs::canonicalize(&folder.path).map_err(ProjectError::Io)?;
        if !root.is_dir() {
            return Err(ProjectError::NotDirectory(root));
        }
        Ok(Self {
            label: folder.label,
            root,
        })
    }

    pub fn resolve_child(&self, child: impl AsRef<Path>) -> Result<PathBuf, ProjectError> {
        let candidate = self.root.join(child.as_ref());
        let normalized = normalize_existing_or_parent(&candidate)?;
        if normalized.starts_with(&self.root) {
            Ok(candidate)
        } else {
            Err(ProjectError::PathEscapesRoot {
                root: self.root.clone(),
                candidate,
            })
        }
    }

    pub fn metadata(
        &self,
        default_profile_id: Option<String>,
        recent_conversation_ids: Vec<String>,
    ) -> Result<LocalProjectMetadata, ProjectError> {
        self.metadata_with_rtk_path_env(
            default_profile_id,
            recent_conversation_ids,
            std::env::var_os("PATH").as_deref(),
        )
    }

    pub fn metadata_with_rtk_path_env(
        &self,
        default_profile_id: Option<String>,
        recent_conversation_ids: Vec<String>,
        path_env: Option<&OsStr>,
    ) -> Result<LocalProjectMetadata, ProjectError> {
        Ok(LocalProjectMetadata {
            label: self.label.clone(),
            root: self.root.clone(),
            settings_path: self.resolve_child(PROJECT_SETTINGS_RELATIVE_PATH)?,
            skills_dir: self.resolve_child(PROJECT_SKILLS_RELATIVE_PATH)?,
            mcp_config_path: self.resolve_child(PROJECT_MCP_RELATIVE_PATH)?,
            compatible_mcp_config_path: self.resolve_child(COMPAT_MCP_RELATIVE_PATH)?,
            rtk_diagnostic: diagnose_rtk_binary_for(DEFAULT_RTK_BINARY, path_env, &self.root),
            instructions: load_project_instructions(&self.root)?,
            git: detect_git_metadata(&self.root)?,
            default_profile_id,
            recent_conversation_ids,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalProjectMetadata {
    pub label: String,
    pub root: PathBuf,
    pub settings_path: PathBuf,
    pub skills_dir: PathBuf,
    pub mcp_config_path: PathBuf,
    pub compatible_mcp_config_path: PathBuf,
    pub rtk_diagnostic: RtkBinaryDiagnostic,
    pub instructions: Vec<ProjectInstruction>,
    pub git: Option<GitMetadata>,
    pub default_profile_id: Option<String>,
    pub recent_conversation_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInstruction {
    pub kind: ProjectInstructionKind,
    pub path: PathBuf,
    pub contents: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectInstructionKind {
    Agents,
    Claude,
    RooRules,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitMetadata {
    pub worktree_root: PathBuf,
    pub git_dir: PathBuf,
}

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("project target is not local")]
    TargetMismatch,
    #[error("project path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("project path escapes root {root}: {candidate}")]
    PathEscapesRoot { root: PathBuf, candidate: PathBuf },
    #[error("project I/O error: {0}")]
    Io(std::io::Error),
    #[error("project watcher capacity must be greater than zero")]
    ZeroWatchCapacity,
}

pub fn record_recent_conversation(
    recent_conversation_ids: &mut Vec<String>,
    conversation_id: impl Into<String>,
    max_recent: usize,
) {
    if max_recent == 0 {
        recent_conversation_ids.clear();
        return;
    }
    let conversation_id = conversation_id.into();
    recent_conversation_ids.retain(|id| id != &conversation_id);
    recent_conversation_ids.insert(0, conversation_id);
    recent_conversation_ids.truncate(max_recent);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectFileEvent {
    Created { path: PathBuf },
    Modified { path: PathBuf },
    Deleted { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshRemoteTarget {
    pub id: String,
    pub host_label: String,
    pub user: Option<String>,
    pub host: String,
    pub root: String,
    pub rtk_binary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsmRemoteTarget {
    pub id: String,
    pub profile_name: Option<String>,
    pub region: String,
    pub target: String,
    pub root: String,
    pub rtk_binary: String,
}

impl SshRemoteTarget {
    pub fn resolve_child(&self, child: &str) -> Result<String, ProjectError> {
        resolve_remote_child(&self.root, child)
    }
}

impl SsmRemoteTarget {
    pub fn resolve_child(&self, child: &str) -> Result<String, ProjectError> {
        resolve_remote_child(&self.root, child)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteDiagnosticKind {
    Ssh,
    AwsSessionManager,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteDiagnosticStatus {
    Pass,
    Warning,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteDiagnosticCheckCode {
    TargetIdentity,
    SshHost,
    SshHostLabel,
    SshUser,
    SsmProfile,
    SsmRegion,
    SsmTarget,
    SsmIamPermissions,
    SsmAgent,
    RemoteRoot,
    RemoteShell,
    RemoteRtk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDiagnosticCheck {
    pub code: RemoteDiagnosticCheckCode,
    pub status: RemoteDiagnosticStatus,
    pub summary: String,
    pub detail: String,
    pub suggested_action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTargetDiagnostics {
    pub target_id: String,
    pub kind: RemoteDiagnosticKind,
    pub host_label: String,
    pub checks: Vec<RemoteDiagnosticCheck>,
}

impl RemoteTargetDiagnostics {
    pub fn overall_status(&self) -> RemoteDiagnosticStatus {
        if self
            .checks
            .iter()
            .any(|check| check.status == RemoteDiagnosticStatus::Fail)
        {
            RemoteDiagnosticStatus::Fail
        } else if self
            .checks
            .iter()
            .any(|check| check.status == RemoteDiagnosticStatus::Warning)
        {
            RemoteDiagnosticStatus::Warning
        } else {
            RemoteDiagnosticStatus::Pass
        }
    }

    pub fn has_failures(&self) -> bool {
        self.overall_status() == RemoteDiagnosticStatus::Fail
    }
}

pub fn diagnose_ssh_target_config(target: &SshRemoteTarget) -> RemoteTargetDiagnostics {
    let checks = vec![
        required_check(
            RemoteDiagnosticCheckCode::TargetIdentity,
            "target id",
            &target.id,
            "Set a stable remote target id before saving the SSH target.",
        ),
        required_check(
            RemoteDiagnosticCheckCode::SshHost,
            "SSH host",
            &target.host,
            "Set the host from OpenSSH config or an explicit hostname.",
        ),
        warning_check(
            RemoteDiagnosticCheckCode::SshHostLabel,
            "SSH host label",
            &target.host_label,
            "Set a label so remote project rows are clearly identified.",
        ),
        optional_check(
            RemoteDiagnosticCheckCode::SshUser,
            "SSH user",
            target.user.as_deref(),
            "No explicit user configured; OpenSSH config or current user must supply it.",
        ),
        remote_root_check(&target.root),
        static_pass_check(
            RemoteDiagnosticCheckCode::RemoteShell,
            "remote shell",
            "SSH command execution will start the remote login shell through the transport.",
        ),
        required_check(
            RemoteDiagnosticCheckCode::RemoteRtk,
            "remote rtk",
            &target.rtk_binary,
            "Install rtk on the remote host or configure the remote rtk binary path.",
        ),
    ];
    RemoteTargetDiagnostics {
        target_id: target.id.clone(),
        kind: RemoteDiagnosticKind::Ssh,
        host_label: target.host_label.clone(),
        checks,
    }
}

pub fn diagnose_ssm_target_config(target: &SsmRemoteTarget) -> RemoteTargetDiagnostics {
    let checks = vec![
        required_check(
            RemoteDiagnosticCheckCode::TargetIdentity,
            "target id",
            &target.id,
            "Set a stable remote target id before saving the Session Manager target.",
        ),
        optional_check(
            RemoteDiagnosticCheckCode::SsmProfile,
            "AWS CLI profile",
            target.profile_name.as_deref(),
            "No named profile configured; AWS default chain must supply credentials.",
        ),
        required_check(
            RemoteDiagnosticCheckCode::SsmRegion,
            "AWS region",
            &target.region,
            "Set the AWS region for the Session Manager endpoint.",
        ),
        required_check(
            RemoteDiagnosticCheckCode::SsmTarget,
            "SSM target instance",
            &target.target,
            "Set the managed instance id or target selector.",
        ),
        advisory_check(
            RemoteDiagnosticCheckCode::SsmIamPermissions,
            "IAM permissions",
            "Requires ssm:StartSession plus channel permissions for the target instance.",
        ),
        advisory_check(
            RemoteDiagnosticCheckCode::SsmAgent,
            "SSM agent state",
            "Requires the target instance to be online with a healthy SSM agent.",
        ),
        remote_root_check(&target.root),
        static_pass_check(
            RemoteDiagnosticCheckCode::RemoteShell,
            "remote shell",
            "Session Manager command execution will start a remote shell through the transport.",
        ),
        required_check(
            RemoteDiagnosticCheckCode::RemoteRtk,
            "remote rtk",
            &target.rtk_binary,
            "Install rtk on the managed instance or configure the remote rtk binary path.",
        ),
    ];
    RemoteTargetDiagnostics {
        target_id: target.id.clone(),
        kind: RemoteDiagnosticKind::AwsSessionManager,
        host_label: target.target.clone(),
        checks,
    }
}

fn required_check(
    code: RemoteDiagnosticCheckCode,
    summary: &str,
    value: &str,
    suggested_action: &str,
) -> RemoteDiagnosticCheck {
    if value.trim().is_empty() {
        RemoteDiagnosticCheck {
            code,
            status: RemoteDiagnosticStatus::Fail,
            summary: format!("{summary} missing"),
            detail: format!("Required {summary} is empty."),
            suggested_action: Some(suggested_action.to_owned()),
        }
    } else {
        RemoteDiagnosticCheck {
            code,
            status: RemoteDiagnosticStatus::Pass,
            summary: format!("{summary} configured"),
            detail: value.to_owned(),
            suggested_action: None,
        }
    }
}

fn warning_check(
    code: RemoteDiagnosticCheckCode,
    summary: &str,
    value: &str,
    suggested_action: &str,
) -> RemoteDiagnosticCheck {
    if value.trim().is_empty() {
        RemoteDiagnosticCheck {
            code,
            status: RemoteDiagnosticStatus::Warning,
            summary: format!("{summary} missing"),
            detail: format!("Optional {summary} is empty."),
            suggested_action: Some(suggested_action.to_owned()),
        }
    } else {
        RemoteDiagnosticCheck {
            code,
            status: RemoteDiagnosticStatus::Pass,
            summary: format!("{summary} configured"),
            detail: value.to_owned(),
            suggested_action: None,
        }
    }
}

fn optional_check(
    code: RemoteDiagnosticCheckCode,
    summary: &str,
    value: Option<&str>,
    warning: &str,
) -> RemoteDiagnosticCheck {
    match value.filter(|value| !value.trim().is_empty()) {
        Some(value) => RemoteDiagnosticCheck {
            code,
            status: RemoteDiagnosticStatus::Pass,
            summary: format!("{summary} configured"),
            detail: value.to_owned(),
            suggested_action: None,
        },
        None => RemoteDiagnosticCheck {
            code,
            status: RemoteDiagnosticStatus::Warning,
            summary: format!("{summary} uses default"),
            detail: warning.to_owned(),
            suggested_action: None,
        },
    }
}

fn advisory_check(
    code: RemoteDiagnosticCheckCode,
    summary: &str,
    detail: &str,
) -> RemoteDiagnosticCheck {
    RemoteDiagnosticCheck {
        code,
        status: RemoteDiagnosticStatus::Warning,
        summary: format!("{summary} requires live check"),
        detail: detail.to_owned(),
        suggested_action: Some(
            "Run connection diagnostics before first command execution.".to_owned(),
        ),
    }
}

fn static_pass_check(
    code: RemoteDiagnosticCheckCode,
    summary: &str,
    detail: &str,
) -> RemoteDiagnosticCheck {
    RemoteDiagnosticCheck {
        code,
        status: RemoteDiagnosticStatus::Pass,
        summary: format!("{summary} configured"),
        detail: detail.to_owned(),
        suggested_action: None,
    }
}

fn remote_root_check(root: &str) -> RemoteDiagnosticCheck {
    if root.trim().is_empty() {
        RemoteDiagnosticCheck {
            code: RemoteDiagnosticCheckCode::RemoteRoot,
            status: RemoteDiagnosticStatus::Fail,
            summary: "remote root missing".to_owned(),
            detail: "Remote project root is empty.".to_owned(),
            suggested_action: Some("Set an absolute remote project root path.".to_owned()),
        }
    } else if !root.starts_with('/') {
        RemoteDiagnosticCheck {
            code: RemoteDiagnosticCheckCode::RemoteRoot,
            status: RemoteDiagnosticStatus::Fail,
            summary: "remote root must be absolute".to_owned(),
            detail: root.to_owned(),
            suggested_action: Some(
                "Use an absolute POSIX path such as /home/user/repo.".to_owned(),
            ),
        }
    } else {
        RemoteDiagnosticCheck {
            code: RemoteDiagnosticCheckCode::RemoteRoot,
            status: RemoteDiagnosticStatus::Pass,
            summary: "remote root configured".to_owned(),
            detail: root.to_owned(),
            suggested_action: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCommandRequest {
    pub cwd: String,
    pub argv: Vec<String>,
    pub timeout_ms: Option<u64>,
    pub env: BTreeMap<String, String>,
    pub stdin: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCommandTranscript {
    pub target_id: String,
    pub cwd: String,
    pub argv: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub rtk_savings: Option<RtkSavingsMetadata>,
    pub policy_labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFileReadRequest {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFileWriteRequest {
    pub path: String,
    pub contents: Vec<u8>,
    pub create_parent_dirs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFileReadResult {
    pub target_id: String,
    pub path: String,
    pub contents: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFileWriteResult {
    pub target_id: String,
    pub path: String,
    pub bytes_written: usize,
}

#[derive(Debug, Error)]
pub enum RemoteCommandError {
    #[error(
        "remote rtk is required for SSH target {target_id}: install rtk or configure the remote target"
    )]
    RemoteRtkMissing { target_id: String },
    #[error("remote command denied by policy: {0}")]
    PolicyDenied(String),
    #[error("remote command failed: {0}")]
    Transport(String),
    #[error("project error: {0}")]
    Project(#[from] ProjectError),
}

pub trait SshRemoteTransport {
    fn execute(
        &self,
        target: SshRemoteTarget,
        request: RemoteCommandRequest,
    ) -> Pin<
        Box<dyn Future<Output = Result<RemoteCommandTranscript, RemoteCommandError>> + Send + '_>,
    >;
}

pub trait SsmRemoteTransport {
    fn execute(
        &self,
        target: SsmRemoteTarget,
        request: RemoteCommandRequest,
    ) -> Pin<
        Box<dyn Future<Output = Result<RemoteCommandTranscript, RemoteCommandError>> + Send + '_>,
    >;
}

pub trait SshRemoteFileTransport {
    fn read_file(
        &self,
        target: SshRemoteTarget,
        request: RemoteFileReadRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteFileReadResult, RemoteCommandError>> + Send + '_>>;

    fn write_file(
        &self,
        target: SshRemoteTarget,
        request: RemoteFileWriteRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteFileWriteResult, RemoteCommandError>> + Send + '_>>;
}

pub trait SsmRemoteFileTransport {
    fn read_file(
        &self,
        target: SsmRemoteTarget,
        request: RemoteFileReadRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteFileReadResult, RemoteCommandError>> + Send + '_>>;

    fn write_file(
        &self,
        target: SsmRemoteTarget,
        request: RemoteFileWriteRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteFileWriteResult, RemoteCommandError>> + Send + '_>>;
}

pub async fn run_ssh_remote_rtk_command<T>(
    transport: &T,
    target: SshRemoteTarget,
    request: RtkCommandRequest,
) -> Result<RemoteCommandTranscript, RemoteCommandError>
where
    T: SshRemoteTransport,
{
    if target.rtk_binary.trim().is_empty() {
        return Err(RemoteCommandError::RemoteRtkMissing {
            target_id: target.id,
        });
    }

    let cwd = resolve_remote_child(&target.root, &request.cwd)?;
    let argv = local_rtk_request_argv_with_binary(&target.rtk_binary, &request);
    transport
        .execute(
            target,
            RemoteCommandRequest {
                cwd,
                argv,
                timeout_ms: request.timeout_ms,
                env: request.env,
                stdin: request.stdin,
            },
        )
        .await
}

pub async fn run_ssm_remote_rtk_command<T>(
    transport: &T,
    target: SsmRemoteTarget,
    request: RtkCommandRequest,
) -> Result<RemoteCommandTranscript, RemoteCommandError>
where
    T: SsmRemoteTransport,
{
    if target.rtk_binary.trim().is_empty() {
        return Err(RemoteCommandError::RemoteRtkMissing {
            target_id: target.id,
        });
    }

    let cwd = resolve_remote_child(&target.root, &request.cwd)?;
    let argv = local_rtk_request_argv_with_binary(&target.rtk_binary, &request);
    transport
        .execute(
            target,
            RemoteCommandRequest {
                cwd,
                argv,
                timeout_ms: request.timeout_ms,
                env: request.env,
                stdin: request.stdin,
            },
        )
        .await
}

pub async fn read_ssh_remote_file<T>(
    transport: &T,
    target: SshRemoteTarget,
    child: &str,
) -> Result<RemoteFileReadResult, RemoteCommandError>
where
    T: SshRemoteFileTransport,
{
    let path = target.resolve_child(child)?;
    transport
        .read_file(target, RemoteFileReadRequest { path })
        .await
}

pub async fn write_ssh_remote_file<T>(
    transport: &T,
    target: SshRemoteTarget,
    child: &str,
    contents: Vec<u8>,
) -> Result<RemoteFileWriteResult, RemoteCommandError>
where
    T: SshRemoteFileTransport,
{
    let path = target.resolve_child(child)?;
    transport
        .write_file(
            target,
            RemoteFileWriteRequest {
                path,
                contents,
                create_parent_dirs: true,
            },
        )
        .await
}

pub async fn read_ssm_remote_file<T>(
    transport: &T,
    target: SsmRemoteTarget,
    child: &str,
) -> Result<RemoteFileReadResult, RemoteCommandError>
where
    T: SsmRemoteFileTransport,
{
    let path = target.resolve_child(child)?;
    transport
        .read_file(target, RemoteFileReadRequest { path })
        .await
}

pub async fn write_ssm_remote_file<T>(
    transport: &T,
    target: SsmRemoteTarget,
    child: &str,
    contents: Vec<u8>,
) -> Result<RemoteFileWriteResult, RemoteCommandError>
where
    T: SsmRemoteFileTransport,
{
    let path = target.resolve_child(child)?;
    transport
        .write_file(
            target,
            RemoteFileWriteRequest {
                path,
                contents,
                create_parent_dirs: true,
            },
        )
        .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWatcherConfig {
    pub poll_interval: Duration,
    pub event_capacity: usize,
}

impl Default for ProjectWatcherConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(250),
            event_capacity: DEFAULT_PROJECT_WATCH_CAPACITY,
        }
    }
}

#[derive(Debug)]
pub struct LocalProjectWatcher {
    shutdown: CancellationToken,
}

impl LocalProjectWatcher {
    pub fn spawn(
        folder: LocalProjectFolder,
        config: ProjectWatcherConfig,
    ) -> Result<(Self, mpsc::Receiver<ProjectFileEvent>), ProjectError> {
        if config.event_capacity == 0 {
            return Err(ProjectError::ZeroWatchCapacity);
        }
        let (event_tx, event_rx) = mpsc::channel(config.event_capacity);
        let shutdown = CancellationToken::new();
        let worker_shutdown = shutdown.clone();
        let mut snapshot = scan_project_files(&folder.root)?;

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = worker_shutdown.cancelled() => break,
                    _ = tokio::time::sleep(config.poll_interval) => {
                        let Ok(next_snapshot) = scan_project_files(&folder.root) else {
                            continue;
                        };
                        let events = diff_snapshots(&snapshot, &next_snapshot);
                        snapshot = next_snapshot;
                        for event in events {
                            if event_tx.send(event).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });

        Ok((Self { shutdown }, event_rx))
    }

    pub fn stop(&self) {
        self.shutdown.cancel();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSnapshot {
    modified: Option<SystemTime>,
    len: u64,
}

fn scan_project_files(root: &Path) -> Result<BTreeMap<PathBuf, FileSnapshot>, ProjectError> {
    let mut files = BTreeMap::new();
    scan_project_files_inner(root, root, &mut files)?;
    Ok(files)
}

fn scan_project_files_inner(
    root: &Path,
    current: &Path,
    files: &mut BTreeMap<PathBuf, FileSnapshot>,
) -> Result<(), ProjectError> {
    for entry in std::fs::read_dir(current).map_err(ProjectError::Io)? {
        let entry = entry.map_err(ProjectError::Io)?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(ProjectError::Io)?;
        if metadata.is_dir() {
            scan_project_files_inner(root, &path, files)?;
        } else if metadata.is_file() {
            let relative = path.strip_prefix(root).unwrap_or(&path).to_owned();
            files.insert(
                relative,
                FileSnapshot {
                    modified: metadata.modified().ok(),
                    len: metadata.len(),
                },
            );
        }
    }
    Ok(())
}

fn diff_snapshots(
    previous: &BTreeMap<PathBuf, FileSnapshot>,
    next: &BTreeMap<PathBuf, FileSnapshot>,
) -> Vec<ProjectFileEvent> {
    let paths = previous
        .keys()
        .chain(next.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    paths
        .into_iter()
        .filter_map(|path| match (previous.get(&path), next.get(&path)) {
            (None, Some(_)) => Some(ProjectFileEvent::Created { path }),
            (Some(_), None) => Some(ProjectFileEvent::Deleted { path }),
            (Some(previous), Some(next)) if previous != next => {
                Some(ProjectFileEvent::Modified { path })
            }
            _ => None,
        })
        .collect()
}

fn normalize_existing_or_parent(path: &Path) -> Result<PathBuf, ProjectError> {
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut current = path.parent();
            while let Some(parent) = current {
                match std::fs::canonicalize(parent) {
                    Ok(path) => return Ok(path),
                    Err(parent_error) if parent_error.kind() == std::io::ErrorKind::NotFound => {
                        current = parent.parent();
                    }
                    Err(parent_error) => return Err(ProjectError::Io(parent_error)),
                }
            }
            Err(ProjectError::Io(error))
        }
        Err(error) => Err(ProjectError::Io(error)),
    }
}

fn load_project_instructions(root: &Path) -> Result<Vec<ProjectInstruction>, ProjectError> {
    let candidates = [
        (
            ProjectInstructionKind::Agents,
            PathBuf::from(PROJECT_AGENTS_FILE_NAME),
        ),
        (
            ProjectInstructionKind::Claude,
            PathBuf::from(PROJECT_CLAUDE_FILE_NAME),
        ),
        (
            ProjectInstructionKind::RooRules,
            PathBuf::from(PROJECT_ROO_RULES_RELATIVE_PATH),
        ),
    ];

    let mut instructions = Vec::new();
    for (kind, relative_path) in candidates {
        let path = root.join(relative_path);
        match std::fs::read_to_string(&path) {
            Ok(contents) => instructions.push(ProjectInstruction {
                kind,
                path,
                contents,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ProjectError::Io(error)),
        }
    }
    Ok(instructions)
}

fn detect_git_metadata(root: &Path) -> Result<Option<GitMetadata>, ProjectError> {
    let git_path = root.join(".git");
    if git_path.is_dir() {
        return Ok(Some(GitMetadata {
            worktree_root: root.to_path_buf(),
            git_dir: git_path,
        }));
    }
    if git_path.is_file() {
        let contents = std::fs::read_to_string(&git_path).map_err(ProjectError::Io)?;
        if let Some(git_dir) = contents.trim().strip_prefix("gitdir:") {
            let git_dir = git_dir.trim();
            let git_dir = if Path::new(git_dir).is_absolute() {
                PathBuf::from(git_dir)
            } else {
                root.join(git_dir)
            };
            return Ok(Some(GitMetadata {
                worktree_root: root.to_path_buf(),
                git_dir,
            }));
        }
    }
    Ok(None)
}

fn resolve_remote_child(root: &str, child: &str) -> Result<String, ProjectError> {
    let root_parts = normalize_remote_parts(root)?;
    let child_parts = normalize_remote_parts(child)?;
    if child.starts_with('/') {
        return Err(ProjectError::PathEscapesRoot {
            root: PathBuf::from(root),
            candidate: PathBuf::from(child),
        });
    }

    let mut combined = root_parts.clone();
    for part in child_parts {
        if part == ".." {
            if combined.len() <= root_parts.len() {
                return Err(ProjectError::PathEscapesRoot {
                    root: PathBuf::from(root),
                    candidate: PathBuf::from(child),
                });
            }
            combined.pop();
        } else if part != "." {
            combined.push(part);
        }
    }

    Ok(format!("/{}", combined.join("/")))
}

fn normalize_remote_parts(path: &str) -> Result<Vec<String>, ProjectError> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(ProjectError::PathEscapesRoot {
                        root: PathBuf::from(path),
                        candidate: PathBuf::from(path),
                    });
                }
            }
            part => parts.push(part.to_owned()),
        }
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tempfile::tempdir;
    use tokio::time::timeout;

    use super::*;

    #[test]
    fn v8_local_project_resolves_paths_under_local_root() {
        let temp_dir = tempdir().unwrap();
        let folder = LocalProjectFolder::open(ProjectFolder {
            label: "repo".to_owned(),
            path: temp_dir.path().to_string_lossy().into_owned(),
            target: ProjectTarget::Local,
        })
        .unwrap();

        let child = folder.resolve_child("src/main.rs").unwrap();

        assert!(child.starts_with(&folder.root));
    }

    #[test]
    fn v8_local_project_rejects_paths_that_escape_root() {
        let temp_dir = tempdir().unwrap();
        let folder = LocalProjectFolder::open(ProjectFolder {
            label: "repo".to_owned(),
            path: temp_dir.path().to_string_lossy().into_owned(),
            target: ProjectTarget::Local,
        })
        .unwrap();

        let error = folder.resolve_child("../outside").unwrap_err();

        assert!(matches!(error, ProjectError::PathEscapesRoot { .. }));
    }

    #[test]
    fn t15_snapshot_diff_detects_create_modify_delete() {
        let first = BTreeMap::from([(
            PathBuf::from("a.txt"),
            FileSnapshot {
                modified: None,
                len: 1,
            },
        )]);
        let second = BTreeMap::from([
            (
                PathBuf::from("a.txt"),
                FileSnapshot {
                    modified: None,
                    len: 2,
                },
            ),
            (
                PathBuf::from("b.txt"),
                FileSnapshot {
                    modified: None,
                    len: 1,
                },
            ),
        ]);

        let events = diff_snapshots(&first, &second);

        assert_eq!(
            events,
            vec![
                ProjectFileEvent::Modified {
                    path: PathBuf::from("a.txt")
                },
                ProjectFileEvent::Created {
                    path: PathBuf::from("b.txt")
                },
            ]
        );
        assert_eq!(
            diff_snapshots(&second, &BTreeMap::new()),
            vec![
                ProjectFileEvent::Deleted {
                    path: PathBuf::from("a.txt")
                },
                ProjectFileEvent::Deleted {
                    path: PathBuf::from("b.txt")
                },
            ]
        );
    }

    #[tokio::test]
    async fn t15_local_project_watcher_emits_file_events() {
        let temp_dir = tempdir().unwrap();
        let folder = LocalProjectFolder::open(ProjectFolder {
            label: "repo".to_owned(),
            path: temp_dir.path().to_string_lossy().into_owned(),
            target: ProjectTarget::Local,
        })
        .unwrap();
        let (watcher, mut events) = LocalProjectWatcher::spawn(
            folder,
            ProjectWatcherConfig {
                poll_interval: Duration::from_millis(10),
                event_capacity: 8,
            },
        )
        .unwrap();

        std::fs::write(temp_dir.path().join("new.txt"), "hello").unwrap();
        let event = timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        watcher.stop();

        assert_eq!(
            event,
            ProjectFileEvent::Created {
                path: PathBuf::from("new.txt")
            }
        );
    }

    #[test]
    fn t17_local_project_metadata_loads_settings_instructions_skills_mcp_and_git() {
        let temp_dir = tempdir().unwrap();
        std::fs::create_dir_all(temp_dir.path().join(".fastrock/skills")).unwrap();
        std::fs::create_dir_all(temp_dir.path().join(".roo")).unwrap();
        std::fs::create_dir(temp_dir.path().join("bin")).unwrap();
        std::fs::create_dir(temp_dir.path().join(".git")).unwrap();
        std::fs::write(temp_dir.path().join("bin/rtk"), "").unwrap();
        std::fs::write(temp_dir.path().join("AGENTS.md"), "Use rtk.\n").unwrap();
        std::fs::write(temp_dir.path().join("CLAUDE.md"), "Legacy instructions.\n").unwrap();
        std::fs::write(temp_dir.path().join(".roo/rules.md"), "Roo rules.\n").unwrap();
        std::fs::write(temp_dir.path().join(".mcp.json"), "{}\n").unwrap();
        let folder = LocalProjectFolder::open(ProjectFolder {
            label: "repo".to_owned(),
            path: temp_dir.path().to_string_lossy().into_owned(),
            target: ProjectTarget::Local,
        })
        .unwrap();
        let path_env = temp_dir.path().join("bin");

        let metadata = folder
            .metadata_with_rtk_path_env(
                Some("profile-mantle".to_owned()),
                vec!["conversation-2".to_owned(), "conversation-1".to_owned()],
                Some(path_env.as_os_str()),
            )
            .unwrap();
        let root = temp_dir.path().canonicalize().unwrap();

        assert_eq!(metadata.label, "repo");
        assert_eq!(metadata.settings_path, root.join(".fastrock/settings.toml"));
        assert_eq!(metadata.skills_dir, root.join(".fastrock/skills"));
        assert_eq!(metadata.mcp_config_path, root.join(".fastrock/mcp.json"));
        assert_eq!(metadata.compatible_mcp_config_path, root.join(".mcp.json"));
        assert!(metadata.rtk_diagnostic.is_available());
        assert_eq!(metadata.rtk_diagnostic.binary, "rtk");
        assert_eq!(
            metadata.default_profile_id.as_deref(),
            Some("profile-mantle")
        );
        assert_eq!(
            metadata.recent_conversation_ids,
            vec!["conversation-2", "conversation-1"]
        );
        assert_eq!(
            metadata
                .instructions
                .iter()
                .map(|instruction| instruction.kind)
                .collect::<Vec<_>>(),
            vec![
                ProjectInstructionKind::Agents,
                ProjectInstructionKind::Claude,
                ProjectInstructionKind::RooRules,
            ]
        );
        assert!(metadata.instructions[0].contents.contains("rtk"));
        assert_eq!(
            metadata.git,
            Some(GitMetadata {
                worktree_root: root.clone(),
                git_dir: root.join(".git"),
            })
        );
    }

    #[test]
    fn t17_git_metadata_supports_worktree_gitdir_files() {
        let temp_dir = tempdir().unwrap();
        let external_git = temp_dir.path().join("actual.git");
        std::fs::create_dir(&external_git).unwrap();
        let worktree = temp_dir.path().join("worktree");
        std::fs::create_dir(&worktree).unwrap();
        std::fs::write(worktree.join(".git"), "gitdir: ../actual.git\n").unwrap();
        let folder = LocalProjectFolder::open(ProjectFolder {
            label: "worktree".to_owned(),
            path: worktree.to_string_lossy().into_owned(),
            target: ProjectTarget::Local,
        })
        .unwrap();

        let metadata = folder.metadata(None, Vec::new()).unwrap();

        assert_eq!(
            metadata.git,
            Some(GitMetadata {
                worktree_root: worktree.canonicalize().unwrap(),
                git_dir: worktree.canonicalize().unwrap().join("../actual.git"),
            })
        );
    }

    #[test]
    fn t17_recent_conversations_are_deduped_and_bounded() {
        let mut recent = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];

        record_recent_conversation(&mut recent, "b", 3);
        record_recent_conversation(&mut recent, "d", 3);

        assert_eq!(recent, vec!["d", "b", "a"]);
        record_recent_conversation(&mut recent, "ignored", 0);
        assert!(recent.is_empty());
    }

    #[test]
    fn t16_ssh_diagnostics_report_remote_rtk_and_root_failures() {
        let mut target = ssh_target();
        target.host_label.clear();
        target.user = None;
        target.root = "relative/repo".to_owned();
        target.rtk_binary.clear();

        let diagnostics = diagnose_ssh_target_config(&target);

        assert_eq!(diagnostics.kind, RemoteDiagnosticKind::Ssh);
        assert_eq!(diagnostics.overall_status(), RemoteDiagnosticStatus::Fail);
        assert!(diagnostics.has_failures());
        assert_eq!(
            diagnostic_status(&diagnostics, RemoteDiagnosticCheckCode::SshHostLabel),
            Some(RemoteDiagnosticStatus::Warning)
        );
        assert_eq!(
            diagnostic_status(&diagnostics, RemoteDiagnosticCheckCode::SshUser),
            Some(RemoteDiagnosticStatus::Warning)
        );
        assert_eq!(
            diagnostic_status(&diagnostics, RemoteDiagnosticCheckCode::RemoteRoot),
            Some(RemoteDiagnosticStatus::Fail)
        );
        assert_eq!(
            diagnostic_status(&diagnostics, RemoteDiagnosticCheckCode::RemoteRtk),
            Some(RemoteDiagnosticStatus::Fail)
        );
    }

    #[test]
    fn t17_ssm_diagnostics_surface_profile_iam_agent_region_and_target_state() {
        let mut target = ssm_target();
        target.profile_name = None;
        target.region.clear();
        target.target.clear();

        let diagnostics = diagnose_ssm_target_config(&target);

        assert_eq!(diagnostics.kind, RemoteDiagnosticKind::AwsSessionManager);
        assert_eq!(diagnostics.overall_status(), RemoteDiagnosticStatus::Fail);
        assert_eq!(
            diagnostic_status(&diagnostics, RemoteDiagnosticCheckCode::SsmProfile),
            Some(RemoteDiagnosticStatus::Warning)
        );
        assert_eq!(
            diagnostic_status(&diagnostics, RemoteDiagnosticCheckCode::SsmRegion),
            Some(RemoteDiagnosticStatus::Fail)
        );
        assert_eq!(
            diagnostic_status(&diagnostics, RemoteDiagnosticCheckCode::SsmTarget),
            Some(RemoteDiagnosticStatus::Fail)
        );
        assert_eq!(
            diagnostic_status(&diagnostics, RemoteDiagnosticCheckCode::SsmIamPermissions),
            Some(RemoteDiagnosticStatus::Warning)
        );
        assert_eq!(
            diagnostic_status(&diagnostics, RemoteDiagnosticCheckCode::SsmAgent),
            Some(RemoteDiagnosticStatus::Warning)
        );
    }

    #[test]
    fn v8_ssh_remote_paths_resolve_relative_to_remote_root() {
        let target = ssh_target();

        assert_eq!(
            target.resolve_child("src/main.rs").unwrap(),
            "/srv/app/src/main.rs"
        );
        assert!(matches!(
            target.resolve_child("../outside"),
            Err(ProjectError::PathEscapesRoot { .. })
        ));
        assert!(matches!(
            target.resolve_child("/etc/passwd"),
            Err(ProjectError::PathEscapesRoot { .. })
        ));
    }

    #[tokio::test]
    async fn t16_ssh_remote_command_invokes_remote_rtk() {
        let transport = FakeSshTransport::default();
        let target = ssh_target();
        let transcript = run_ssh_remote_rtk_command(
            &transport,
            target.clone(),
            RtkCommandRequest {
                cwd: "subdir".to_owned(),
                command: vec!["cargo".to_owned(), "test".to_owned()],
                shell: None,
                pty: false,
                timeout_ms: None,
                env: BTreeMap::new(),
                stdin: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(transcript.target_id, target.id);
        assert_eq!(transcript.cwd, "/srv/app/subdir");
        assert_eq!(
            transcript.argv,
            vec!["rtk".to_owned(), "cargo".to_owned(), "test".to_owned()]
        );
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn v2_ssh_remote_command_preserves_shell_through_remote_rtk() {
        let transport = FakeSshTransport::default();
        let target = ssh_target();
        let transcript = run_ssh_remote_rtk_command(
            &transport,
            target,
            RtkCommandRequest {
                cwd: ".".to_owned(),
                command: vec![
                    "cargo".to_owned(),
                    "test".to_owned(),
                    "unit test".to_owned(),
                ],
                shell: Some("bash".to_owned()),
                pty: false,
                timeout_ms: None,
                env: BTreeMap::new(),
                stdin: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            transcript.argv,
            vec![
                "rtk".to_owned(),
                "bash".to_owned(),
                "-lc".to_owned(),
                "cargo test 'unit test'".to_owned(),
            ]
        );
    }

    #[tokio::test]
    async fn t16_ssh_remote_file_ops_resolve_under_remote_root() {
        let transport = FakeSshTransport::default();
        let target = ssh_target();

        let read = read_ssh_remote_file(&transport, target.clone(), "src/main.rs")
            .await
            .unwrap();
        let write = write_ssh_remote_file(
            &transport,
            target.clone(),
            "src/lib.rs",
            b"pub fn x() {}\n".to_vec(),
        )
        .await
        .unwrap();
        let escape = read_ssh_remote_file(&transport, target, "../secret").await;

        assert_eq!(read.path, "/srv/app/src/main.rs");
        assert_eq!(read.contents, b"remote bytes".to_vec());
        assert_eq!(write.path, "/srv/app/src/lib.rs");
        assert_eq!(write.bytes_written, b"pub fn x() {}\n".len());
        assert!(matches!(
            escape,
            Err(RemoteCommandError::Project(
                ProjectError::PathEscapesRoot { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn v2_ssh_remote_command_fails_when_remote_rtk_missing() {
        let transport = FakeSshTransport::default();
        let mut target = ssh_target();
        target.rtk_binary.clear();

        let error = run_ssh_remote_rtk_command(
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
        .unwrap_err();

        assert!(matches!(error, RemoteCommandError::RemoteRtkMissing { .. }));
        assert!(transport.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn v8_ssm_remote_paths_resolve_relative_to_remote_root() {
        let target = ssm_target();

        assert_eq!(
            target.resolve_child("deploy/app.toml").unwrap(),
            "/opt/service/deploy/app.toml"
        );
        assert!(matches!(
            target.resolve_child("../../etc/passwd"),
            Err(ProjectError::PathEscapesRoot { .. })
        ));
    }

    #[tokio::test]
    async fn t17_ssm_remote_command_invokes_remote_rtk() {
        let transport = FakeSsmTransport::default();
        let target = ssm_target();
        let transcript = run_ssm_remote_rtk_command(
            &transport,
            target.clone(),
            RtkCommandRequest {
                cwd: ".".to_owned(),
                command: vec!["cargo".to_owned(), "test".to_owned()],
                shell: None,
                pty: false,
                timeout_ms: None,
                env: BTreeMap::new(),
                stdin: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(transcript.target_id, target.id);
        assert_eq!(transcript.cwd, "/opt/service");
        assert_eq!(
            transcript.argv,
            vec!["rtk".to_owned(), "cargo".to_owned(), "test".to_owned()]
        );
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn v2_ssm_remote_command_preserves_shell_through_remote_rtk() {
        let transport = FakeSsmTransport::default();
        let target = ssm_target();
        let transcript = run_ssm_remote_rtk_command(
            &transport,
            target,
            RtkCommandRequest {
                cwd: ".".to_owned(),
                command: vec!["rg".to_owned(), "hello world".to_owned()],
                shell: Some("zsh".to_owned()),
                pty: false,
                timeout_ms: None,
                env: BTreeMap::new(),
                stdin: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(
            transcript.argv,
            vec![
                "rtk".to_owned(),
                "zsh".to_owned(),
                "-lc".to_owned(),
                "rg 'hello world'".to_owned(),
            ]
        );
    }

    #[tokio::test]
    async fn t17_ssm_remote_file_ops_resolve_under_remote_root() {
        let transport = FakeSsmTransport::default();
        let target = ssm_target();

        let read = read_ssm_remote_file(&transport, target.clone(), "config/app.toml")
            .await
            .unwrap();
        let write = write_ssm_remote_file(
            &transport,
            target.clone(),
            "config/app.toml",
            b"name = \"fastrock\"\n".to_vec(),
        )
        .await
        .unwrap();
        let escape = write_ssm_remote_file(&transport, target, "/etc/passwd", Vec::new()).await;

        assert_eq!(read.path, "/opt/service/config/app.toml");
        assert_eq!(write.path, "/opt/service/config/app.toml");
        assert_eq!(write.bytes_written, b"name = \"fastrock\"\n".len());
        assert!(matches!(
            escape,
            Err(RemoteCommandError::Project(
                ProjectError::PathEscapesRoot { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn v2_ssm_remote_command_fails_when_remote_rtk_missing() {
        let transport = FakeSsmTransport::default();
        let mut target = ssm_target();
        target.rtk_binary.clear();

        let error = run_ssm_remote_rtk_command(
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
        .unwrap_err();

        assert!(matches!(error, RemoteCommandError::RemoteRtkMissing { .. }));
        assert!(transport.requests.lock().unwrap().is_empty());
    }

    fn ssm_target() -> SsmRemoteTarget {
        SsmRemoteTarget {
            id: "ssm-1".to_owned(),
            profile_name: Some("prod".to_owned()),
            region: "us-east-1".to_owned(),
            target: "i-0123456789abcdef0".to_owned(),
            root: "/opt/service".to_owned(),
            rtk_binary: "rtk".to_owned(),
        }
    }

    fn ssh_target() -> SshRemoteTarget {
        SshRemoteTarget {
            id: "ssh-1".to_owned(),
            host_label: "prod".to_owned(),
            user: Some("ubuntu".to_owned()),
            host: "example.test".to_owned(),
            root: "/srv/app".to_owned(),
            rtk_binary: "rtk".to_owned(),
        }
    }

    fn diagnostic_status(
        diagnostics: &RemoteTargetDiagnostics,
        code: RemoteDiagnosticCheckCode,
    ) -> Option<RemoteDiagnosticStatus> {
        diagnostics
            .checks
            .iter()
            .find(|check| check.code == code)
            .map(|check| check.status)
    }

    #[derive(Default)]
    struct FakeSshTransport {
        requests: Arc<Mutex<Vec<(SshRemoteTarget, RemoteCommandRequest)>>>,
        file_reads: Arc<Mutex<Vec<(SshRemoteTarget, RemoteFileReadRequest)>>>,
        file_writes: Arc<Mutex<Vec<(SshRemoteTarget, RemoteFileWriteRequest)>>>,
    }

    impl SshRemoteTransport for FakeSshTransport {
        fn execute(
            &self,
            target: SshRemoteTarget,
            request: RemoteCommandRequest,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<RemoteCommandTranscript, RemoteCommandError>>
                    + Send
                    + '_,
            >,
        > {
            self.requests
                .lock()
                .unwrap()
                .push((target.clone(), request.clone()));
            Box::pin(async move {
                Ok(RemoteCommandTranscript {
                    target_id: target.id,
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
                })
            })
        }
    }

    impl SshRemoteFileTransport for FakeSshTransport {
        fn read_file(
            &self,
            target: SshRemoteTarget,
            request: RemoteFileReadRequest,
        ) -> Pin<
            Box<dyn Future<Output = Result<RemoteFileReadResult, RemoteCommandError>> + Send + '_>,
        > {
            self.file_reads
                .lock()
                .unwrap()
                .push((target.clone(), request.clone()));
            Box::pin(async move {
                Ok(RemoteFileReadResult {
                    target_id: target.id,
                    path: request.path,
                    contents: b"remote bytes".to_vec(),
                })
            })
        }

        fn write_file(
            &self,
            target: SshRemoteTarget,
            request: RemoteFileWriteRequest,
        ) -> Pin<
            Box<dyn Future<Output = Result<RemoteFileWriteResult, RemoteCommandError>> + Send + '_>,
        > {
            self.file_writes
                .lock()
                .unwrap()
                .push((target.clone(), request.clone()));
            Box::pin(async move {
                Ok(RemoteFileWriteResult {
                    target_id: target.id,
                    path: request.path,
                    bytes_written: request.contents.len(),
                })
            })
        }
    }

    #[derive(Default)]
    struct FakeSsmTransport {
        requests: Arc<Mutex<Vec<(SsmRemoteTarget, RemoteCommandRequest)>>>,
        file_reads: Arc<Mutex<Vec<(SsmRemoteTarget, RemoteFileReadRequest)>>>,
        file_writes: Arc<Mutex<Vec<(SsmRemoteTarget, RemoteFileWriteRequest)>>>,
    }

    impl SsmRemoteTransport for FakeSsmTransport {
        fn execute(
            &self,
            target: SsmRemoteTarget,
            request: RemoteCommandRequest,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<RemoteCommandTranscript, RemoteCommandError>>
                    + Send
                    + '_,
            >,
        > {
            self.requests
                .lock()
                .unwrap()
                .push((target.clone(), request.clone()));
            Box::pin(async move {
                Ok(RemoteCommandTranscript {
                    target_id: target.id,
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
                })
            })
        }
    }

    impl SsmRemoteFileTransport for FakeSsmTransport {
        fn read_file(
            &self,
            target: SsmRemoteTarget,
            request: RemoteFileReadRequest,
        ) -> Pin<
            Box<dyn Future<Output = Result<RemoteFileReadResult, RemoteCommandError>> + Send + '_>,
        > {
            self.file_reads
                .lock()
                .unwrap()
                .push((target.clone(), request.clone()));
            Box::pin(async move {
                Ok(RemoteFileReadResult {
                    target_id: target.id,
                    path: request.path,
                    contents: b"remote bytes".to_vec(),
                })
            })
        }

        fn write_file(
            &self,
            target: SsmRemoteTarget,
            request: RemoteFileWriteRequest,
        ) -> Pin<
            Box<dyn Future<Output = Result<RemoteFileWriteResult, RemoteCommandError>> + Send + '_>,
        > {
            self.file_writes
                .lock()
                .unwrap()
                .push((target.clone(), request.clone()));
            Box::pin(async move {
                Ok(RemoteFileWriteResult {
                    target_id: target.id,
                    path: request.path,
                    bytes_written: request.contents.len(),
                })
            })
        }
    }
}
