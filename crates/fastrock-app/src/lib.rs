#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fastrock_bedrock::{
    AwsCredentialSource, AwsSharedConfig, AwsSharedConfigFiles, AwsSharedProfile,
    AwsStaticCredentials, BedrockFloatMilli, BedrockProviderErrorDiagnostic, BedrockRuntimeClient,
    MantleAnthropicMessagesRequest, MantleApiShape, MantleChatCompletionRequest, MantleChatMessage,
    MantleClient, MantleClientAuth, MantleClientError, MantleHttpRequest, MantleHttpTransport,
    MantleResponseRequest, MantleSigV4Signer, MantleStoredResponsePolicy,
    MantleStoredResponseState, ReqwestBedrockHttpTransport, ResolvedBedrockAuth,
    RuntimeCachePointBlock, RuntimeClientError, RuntimeContentBlock, RuntimeConverseRequest,
    RuntimeHttpRequest, RuntimeHttpTransport, RuntimeInferenceConfiguration, RuntimeMessage,
    RuntimeModelResponse, RuntimeServiceTier, RuntimeSigV4Signer, RuntimeSystemContentBlock,
    RuntimeTokenUsage, StaticCredentialsMantleSigV4Signer, StaticCredentialsRuntimeSigV4Signer,
    mantle_base_url, normalize_mantle_stream_events, runtime_endpoint,
};
use fastrock_cache::{
    CacheProviderPlane, CacheUsage, CacheUsageLedger, CacheUsageRecord, CacheUsageSnapshot,
    ContextFragmentCacheResult, ContextFragmentInput, ContextFragmentKind, ContextFragmentRecord,
    LocalContextFragmentCache, MemoryInjection, MemoryKind, MemoryRecord, MemoryScope,
    MemorySearchResult, MemoryStore, ToolOutputDedupCache, ToolOutputDedupResult, ToolOutputKind,
    ToolOutputRecord,
};
use fastrock_core::{
    AgentMode, CommandPolicy, CommandPolicyDecision, CommandPolicyRequest, ConversationId,
    ConversationScheduler, ConversationStatus, FastrockError, FastrockErrorContext,
    FastrockErrorKind, GoalLoopState, ModelProviderKind, ModelRequestMetadata,
    NormalizedModelStreamEvent, NormalizedModelStreamEventKind, PlanArtifact, PlanArtifactStatus,
    ThreadGoal, ThreadGoalSnapshot, ThreadGoalStatus, ToolMutationClass, ToolRouteDecision,
    ToolRouteRequest, TranscriptEvent, TranscriptEventKind, TranscriptEventStore,
    TranscriptStoreError, evaluate_command_policy, route_plan_mode_rtk_command, route_tool_request,
};
use fastrock_editor::{EditorBuffer, EditorError, EditorTabSet};
use fastrock_mcp::{
    MCP_OAUTH_TOKEN_SECRET_SERVICE, McpConfigChange, McpConfigError, McpConfigScope, McpConfigSet,
    McpOAuthState, McpOAuthTokenSet, McpRuntimeSnapshot, McpRuntimeSnapshotSet, McpServerConfig,
    McpServerRuntimeStatus, McpTransportConfig, mcp_oauth_token_secret_ref,
};
use fastrock_persistence::{
    CacheMetadataRecord, ConfigScope, ConversationEvent, ConversationMetadata, EditorTabRecord,
    GoalRecord, LlmProfileRecord, McpStatusSnapshot, PersistenceActorClient,
    PersistenceActorHandle, PersistenceError, PersistenceResult, ProjectFolderRecord, SecretRef,
    SecretRefRecord, SettingsDocument, StoredConversationEvent,
};
use fastrock_projects::{
    COMPAT_MCP_RELATIVE_PATH, LocalProjectFolder, LocalProjectMetadata, PROJECT_MCP_RELATIVE_PATH,
    PROJECT_SKILLS_RELATIVE_PATH, ProjectFileEvent, ProjectFolder as ProjectFolderSpec,
    ProjectInstructionKind, ProjectTarget, RemoteCommandError, RemoteCommandTranscript,
    RemoteFileReadResult, RemoteFileWriteResult, RemoteTargetDiagnostics, SshRemoteFileTransport,
    SshRemoteTarget, SshRemoteTransport, SsmRemoteFileTransport, SsmRemoteTarget,
    SsmRemoteTransport, diagnose_ssh_target_config, diagnose_ssm_target_config,
    read_ssh_remote_file as project_read_ssh_remote_file,
    read_ssm_remote_file as project_read_ssm_remote_file,
    record_recent_conversation as record_project_recent_conversation, run_ssh_remote_rtk_command,
    run_ssm_remote_rtk_command, write_ssh_remote_file as project_write_ssh_remote_file,
    write_ssm_remote_file as project_write_ssm_remote_file,
};
use fastrock_rtk::{
    RtkBinaryStatus, RtkCommandRequest, RtkCommandTranscript, RtkRunError, RtkRunnerConfig,
};
use fastrock_skills::{
    SkillDocument, SkillError, SkillScope, SkillWriteRequest, create_skill as create_skill_file,
    delete_skill as delete_skill_file, discover_skills, edit_skill as edit_skill_file,
    move_skill as move_skill_file,
};
use fastrock_ui::{
    ConversationListRow, ConversationSidebarModel, LlmProfileForm, LlmProfileSettingsState,
    UiAwsCredentialPreview, UiCredentialSource, UiLlmProvider, UiMantleApiShape, UiMantleAuthMode,
    UiProjectTargetKind, UiRuntimeTargetType,
};

const MCP_CONFIG_SETTINGS_KEY: &str = "mcp.config";
const COMMAND_POLICY_SETTINGS_KEY: &str = "command.policy";
const APP_PREFERENCES_SETTINGS_KEY: &str = "app.preferences";
const CACHE_USAGE_KEY_PREFIX: &str = "usage:";
const CONTEXT_FRAGMENT_CACHE_KEY_PREFIX: &str = "context:";
const TOOL_OUTPUT_CACHE_KEY_PREFIX: &str = "tool-output:";
const LOCAL_CONTEXT_PROVIDER_PLANE: &str = "local_context";
const TOOL_OUTPUT_PROVIDER_PLANE: &str = "tool_output";
const PROJECT_DOCUMENT_DEFAULT_PROFILE_ID_KEY: &str = "default_profile_id";
const PROJECT_DOCUMENT_RECENT_CONVERSATION_IDS_KEY: &str = "recent_conversation_ids";
const MAX_PROJECT_RECENT_CONVERSATIONS: usize = 20;
const SHELL_EDITOR_VISIBLE_LINE_COUNT: usize = 24;
const SHELL_DIFF_VISIBLE_LINE_COUNT: usize = 64;
const CROCKFORD_BASE32: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
static GENERATED_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockProfileRequest {
    pub id: String,
    pub name: String,
    pub provider: UiLlmProvider,
    pub model_id: String,
    pub region: String,
    pub aws_profile: String,
    pub default_for_new_conversations: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsCliProfileSummary {
    pub name: String,
    pub region: Option<String>,
    pub source_type: String,
    pub can_resolve_credentials: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsCredentialPreviewUpdate {
    pub profile_id: String,
    pub preview: UiAwsCredentialPreview,
    pub profile_found: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectHotReloadError {
    UnknownProjectFolder(String),
    InvalidEventPath(String),
    Io(String),
    Json(String),
    Mcp(McpConfigError),
    Skill(SkillError),
}

impl fmt::Display for ProjectHotReloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownProjectFolder(folder_id) => {
                write!(formatter, "unknown project folder: {folder_id}")
            }
            Self::InvalidEventPath(path) => write!(formatter, "invalid project event path: {path}"),
            Self::Io(message) => write!(formatter, "project hot reload I/O error: {message}"),
            Self::Json(message) => write!(formatter, "project hot reload JSON error: {message}"),
            Self::Mcp(error) => write!(formatter, "project MCP hot reload failed: {error}"),
            Self::Skill(error) => write!(formatter, "project skill hot reload failed: {error}"),
        }
    }
}

impl std::error::Error for ProjectHotReloadError {}

impl From<McpConfigError> for ProjectHotReloadError {
    fn from(error: McpConfigError) -> Self {
        Self::Mcp(error)
    }
}

impl From<SkillError> for ProjectHotReloadError {
    fn from(error: SkillError) -> Self {
        Self::Skill(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockModelDiscoveryResult {
    pub profile_id: String,
    pub endpoint: String,
    pub models: Vec<BedrockDiscoveredModel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockDiscoveredModel {
    pub id: String,
    pub owned_by: Option<String>,
    pub api_shapes: Vec<UiMantleApiShape>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockRuntimeConnectionTestResult {
    pub profile_id: String,
    pub endpoint: String,
    pub model_id: String,
    pub output_text: String,
    pub invoked_model_id: Option<String>,
    pub usage: Option<RuntimeTokenUsage>,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockModelRunRequest {
    pub conversation_id: ConversationId,
    pub profile_id: String,
    pub message: String,
    pub prompt_compression_enabled: bool,
    pub prompt_compression_level: String,
    pub mantle_previous_response_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockModelRunResult {
    pub conversation_id: ConversationId,
    pub metadata: ModelRequestMetadata,
    pub events: Vec<NormalizedModelStreamEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BedrockModelRunError {
    Profile(BedrockProfileProbeError),
    TimedOut {
        timeout_ms: u64,
    },
    MantleClient {
        message: String,
        diagnostic: Option<BedrockProviderErrorDiagnostic>,
    },
    RuntimeClient {
        message: String,
        diagnostic: Option<BedrockProviderErrorDiagnostic>,
    },
}

impl fmt::Display for BedrockModelRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(error) => write!(formatter, "{error}"),
            Self::TimedOut { timeout_ms } => {
                write!(
                    formatter,
                    "Bedrock model run timed out after {timeout_ms}ms"
                )
            }
            Self::MantleClient { message, .. } => {
                write!(formatter, "Mantle model run failed: {message}")
            }
            Self::RuntimeClient { message, .. } => {
                write!(formatter, "Runtime model run failed: {message}")
            }
        }
    }
}

impl std::error::Error for BedrockModelRunError {}

impl BedrockModelRunError {
    fn retryable(&self) -> bool {
        match self {
            Self::TimedOut { .. } => true,
            Self::MantleClient { diagnostic, .. } | Self::RuntimeClient { diagnostic, .. } => {
                diagnostic
                    .as_ref()
                    .is_some_and(|diagnostic| diagnostic.retryable)
            }
            Self::Profile(_) => false,
        }
    }
}

pub struct BedrockModelRunIo<'a, TM, SM, TR, SR> {
    pub shared_config: &'a AwsSharedConfig,
    pub bearer_api_key: Option<String>,
    pub mantle_transport: TM,
    pub mantle_signer: SM,
    pub runtime_transport: TR,
    pub runtime_signer: SR,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BedrockProfileProbeError {
    ProfileNotFound(String),
    WrongProvider {
        profile_id: String,
        expected: UiLlmProvider,
        actual: UiLlmProvider,
    },
    MissingBearerApiKey {
        profile_id: String,
        secret_ref: String,
    },
    AwsConfig(String),
    MantleClient {
        message: String,
        diagnostic: Option<BedrockProviderErrorDiagnostic>,
    },
    RuntimeClient {
        message: String,
        diagnostic: Option<BedrockProviderErrorDiagnostic>,
    },
}

impl fmt::Display for BedrockProfileProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProfileNotFound(profile_id) => {
                write!(formatter, "Bedrock profile not found: {profile_id}")
            }
            Self::WrongProvider {
                profile_id,
                expected,
                actual,
            } => write!(
                formatter,
                "Bedrock profile {profile_id} uses provider {actual:?}, expected {expected:?}"
            ),
            Self::MissingBearerApiKey {
                profile_id,
                secret_ref,
            } => write!(
                formatter,
                "Bedrock profile {profile_id} needs Mantle API key secret {secret_ref}"
            ),
            Self::AwsConfig(message) => write!(formatter, "AWS config error: {message}"),
            Self::MantleClient { message, .. } => {
                write!(formatter, "Mantle probe failed: {message}")
            }
            Self::RuntimeClient { message, .. } => {
                write!(formatter, "Runtime probe failed: {message}")
            }
        }
    }
}

impl std::error::Error for BedrockProfileProbeError {}

impl BedrockProfileRequest {
    fn into_form(self) -> LlmProfileForm {
        let mut form = LlmProfileForm::new_bedrock_mantle(self.id);
        form.name = self.name;
        form.provider = self.provider;
        form.model_id = self.model_id.clone();
        form.region = self.region;
        form.credential_source = UiCredentialSource::AwsCliProfile {
            profile_name: self.aws_profile,
        };
        form.runtime_settings.target_type = UiRuntimeTargetType::FoundationModel;
        form.runtime_settings.target = self.model_id;
        form.enabled = true;
        form.default_for_new_conversations = self.default_for_new_conversations;
        form
    }
}

pub async fn run_bedrock_model_message_with_transports<TM, SM, TR, SR>(
    request: BedrockModelRunRequest,
    profile: LlmProfileForm,
    io: BedrockModelRunIo<'_, TM, SM, TR, SR>,
) -> Result<BedrockModelRunResult, BedrockModelRunError>
where
    TM: MantleHttpTransport + Clone,
    SM: MantleSigV4Signer + Clone,
    TR: RuntimeHttpTransport + Clone,
    SR: RuntimeSigV4Signer + Clone,
{
    let attempts = profile
        .request_tuning
        .retry_max_attempts
        .unwrap_or(1)
        .max(1);
    let timeout_ms = profile.request_tuning.timeout_ms;
    let shared_config = io.shared_config;
    let bearer_api_key = io.bearer_api_key;
    let mantle_transport = io.mantle_transport;
    let mantle_signer = io.mantle_signer;
    let runtime_transport = io.runtime_transport;
    let runtime_signer = io.runtime_signer;
    let mut last_error = None;

    for attempt in 0..attempts {
        let future = run_bedrock_model_message_once(
            request.clone(),
            profile.clone(),
            BedrockModelRunIo {
                shared_config,
                bearer_api_key: bearer_api_key.clone(),
                mantle_transport: mantle_transport.clone(),
                mantle_signer: mantle_signer.clone(),
                runtime_transport: runtime_transport.clone(),
                runtime_signer: runtime_signer.clone(),
            },
        );
        let result = if let Some(timeout_ms) = timeout_ms {
            match tokio::time::timeout(Duration::from_millis(timeout_ms), future).await {
                Ok(result) => result,
                Err(_) => Err(BedrockModelRunError::TimedOut { timeout_ms }),
            }
        } else {
            future.await
        };
        match result {
            Ok(result) => return Ok(result),
            Err(error) if attempt + 1 < attempts && error.retryable() => {
                last_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.expect("retry loop records failed attempt"))
}

async fn run_bedrock_model_message_once<TM, SM, TR, SR>(
    request: BedrockModelRunRequest,
    profile: LlmProfileForm,
    io: BedrockModelRunIo<'_, TM, SM, TR, SR>,
) -> Result<BedrockModelRunResult, BedrockModelRunError>
where
    TM: MantleHttpTransport,
    SM: MantleSigV4Signer,
    TR: RuntimeHttpTransport,
    SR: RuntimeSigV4Signer,
{
    match profile.provider {
        UiLlmProvider::BedrockMantle => {
            run_mantle_model_message(
                request,
                profile,
                io.shared_config,
                io.bearer_api_key,
                io.mantle_transport,
                io.mantle_signer,
            )
            .await
        }
        UiLlmProvider::BedrockRuntime => {
            run_runtime_model_message(
                request,
                profile,
                io.shared_config,
                io.runtime_transport,
                io.runtime_signer,
            )
            .await
        }
    }
}

pub async fn run_bedrock_model_message_with_default_http(
    request: BedrockModelRunRequest,
    profile: LlmProfileForm,
    shared_config: &AwsSharedConfig,
    bearer_api_key: Option<String>,
) -> Result<BedrockModelRunResult, BedrockModelRunError> {
    run_bedrock_model_message_with_default_http_and_static_secrets(
        request,
        profile,
        shared_config,
        bearer_api_key,
        BTreeMap::new(),
    )
    .await
}

pub async fn run_bedrock_model_message_with_default_http_and_static_secrets(
    request: BedrockModelRunRequest,
    profile: LlmProfileForm,
    shared_config: &AwsSharedConfig,
    bearer_api_key: Option<String>,
    static_secret_credentials: BTreeMap<String, AwsStaticCredentials>,
) -> Result<BedrockModelRunResult, BedrockModelRunError> {
    let mut signer = AwsSharedConfigSigV4Signer::new(shared_config.clone());
    for (secret_ref, credentials) in static_secret_credentials {
        signer = signer.with_static_secret_ref(secret_ref, credentials);
    }
    run_bedrock_model_message_with_transports(
        request,
        profile,
        BedrockModelRunIo {
            shared_config,
            bearer_api_key,
            mantle_transport: ReqwestBedrockHttpTransport::default(),
            mantle_signer: signer.clone(),
            runtime_transport: ReqwestBedrockHttpTransport::default(),
            runtime_signer: signer,
        },
    )
    .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsSharedConfigSigV4Signer {
    shared_config: AwsSharedConfig,
    aws_cli_binary: String,
    static_secret_credentials: BTreeMap<String, AwsStaticCredentials>,
    default_chain_reads_environment: bool,
}

impl AwsSharedConfigSigV4Signer {
    pub fn new(shared_config: AwsSharedConfig) -> Self {
        Self {
            shared_config,
            aws_cli_binary: "aws".to_owned(),
            static_secret_credentials: BTreeMap::new(),
            default_chain_reads_environment: true,
        }
    }

    pub fn with_aws_cli_binary(mut self, aws_cli_binary: impl Into<String>) -> Self {
        self.aws_cli_binary = aws_cli_binary.into();
        self
    }

    pub fn with_default_chain_environment_enabled(mut self, enabled: bool) -> Self {
        self.default_chain_reads_environment = enabled;
        self
    }

    pub fn with_static_secret_ref(
        mut self,
        secret_ref: impl Into<String>,
        credentials: AwsStaticCredentials,
    ) -> Self {
        self.static_secret_credentials
            .insert(secret_ref.into(), credentials);
        self
    }

    fn credentials(
        &self,
        resolved_auth: &ResolvedBedrockAuth,
    ) -> Result<AwsStaticCredentials, String> {
        match &resolved_auth.credential_source {
            AwsCredentialSource::CliProfile { profile_name } => {
                let profile = self
                    .shared_config
                    .profile(profile_name)
                    .ok_or_else(|| format!("AWS CLI profile not found: {profile_name}"))?;
                if let Some(credentials) = self
                    .shared_config
                    .static_credentials_for_profile(profile_name)
                {
                    return Ok(credentials);
                }
                if let Some(command_line) = &profile.credential_process {
                    return credential_process_credentials(command_line);
                }
                aws_cli_export_credentials(&self.aws_cli_binary, Some(profile_name))
            }
            AwsCredentialSource::Environment => static_credentials_from_environment()
                .ok_or_else(|| "environment AWS credentials are missing".to_owned()),
            AwsCredentialSource::DefaultChain => {
                if self.default_chain_reads_environment
                    && let Some(credentials) = static_credentials_from_environment()
                {
                    return Ok(credentials);
                }
                if let Ok(profile_name) = std::env::var("AWS_PROFILE") {
                    let profile_name = profile_name.trim();
                    if !profile_name.is_empty() {
                        if let Some(credentials) = self
                            .shared_config
                            .static_credentials_for_profile(profile_name)
                        {
                            return Ok(credentials);
                        }
                        if let Some(profile) = self.shared_config.profile(profile_name)
                            && let Some(command_line) = &profile.credential_process
                        {
                            return credential_process_credentials(command_line);
                        }
                        return aws_cli_export_credentials(
                            &self.aws_cli_binary,
                            Some(profile_name),
                        );
                    }
                }
                aws_cli_export_credentials(&self.aws_cli_binary, None)
            }
            AwsCredentialSource::ExplicitStaticSecret { secret_ref } => self
                .static_secret_credentials
                .get(secret_ref)
                .cloned()
                .ok_or_else(|| format!("explicit static secret ref not loaded: {secret_ref}")),
        }
    }
}

impl MantleSigV4Signer for AwsSharedConfigSigV4Signer {
    fn sign(
        &self,
        request: &mut MantleHttpRequest,
        resolved_auth: &ResolvedBedrockAuth,
    ) -> Result<(), MantleClientError> {
        let credentials = self
            .credentials(resolved_auth)
            .map_err(MantleClientError::SigV4)?;
        StaticCredentialsMantleSigV4Signer::new(credentials).sign(request, resolved_auth)
    }
}

impl RuntimeSigV4Signer for AwsSharedConfigSigV4Signer {
    fn sign(
        &self,
        request: &mut RuntimeHttpRequest,
        resolved_auth: &ResolvedBedrockAuth,
    ) -> Result<(), RuntimeClientError> {
        let credentials = self
            .credentials(resolved_auth)
            .map_err(RuntimeClientError::SigV4)?;
        StaticCredentialsRuntimeSigV4Signer::new(credentials).sign(request, resolved_auth)
    }
}

fn credential_process_credentials(command_line: &str) -> Result<AwsStaticCredentials, String> {
    let argv = split_credential_process_command(command_line)?;
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| "credential_process command is empty".to_owned())?;
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("credential_process failed to start: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "credential_process exited with status {}",
            process_exit_status(&output.status)
        ));
    }
    process_json_credentials(&output.stdout, "credential_process")
}

fn aws_cli_export_credentials(
    aws_cli_binary: &str,
    profile_name: Option<&str>,
) -> Result<AwsStaticCredentials, String> {
    let mut command = std::process::Command::new(aws_cli_binary);
    command.args(["configure", "export-credentials"]);
    if let Some(profile_name) = profile_name {
        command.args(["--profile", profile_name]);
        command.env("AWS_PROFILE", profile_name);
    }
    let output = command
        .args(["--format", "process"])
        .output()
        .map_err(|error| {
            let label = aws_cli_export_credentials_label(profile_name);
            format!("AWS CLI export-credentials for {label} failed to start: {error}")
        })?;
    if !output.status.success() {
        let label = aws_cli_export_credentials_label(profile_name);
        return Err(format!(
            "AWS CLI export-credentials for {label} exited with status {}",
            process_exit_status(&output.status)
        ));
    }
    process_json_credentials(&output.stdout, "AWS CLI export-credentials")
}

fn aws_cli_export_credentials_label(profile_name: Option<&str>) -> String {
    profile_name
        .map(|profile_name| format!("profile {profile_name}"))
        .unwrap_or_else(|| "default chain".to_owned())
}

fn process_exit_status(status: &std::process::ExitStatus) -> String {
    status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_owned())
}

fn process_json_credentials(
    stdout: &[u8],
    source_label: &'static str,
) -> Result<AwsStaticCredentials, String> {
    let payload: serde_json::Value = serde_json::from_slice(stdout)
        .map_err(|error| format!("{source_label} returned invalid JSON: {error}"))?;
    let version = payload
        .get("Version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("{source_label} returned missing Version"))?;
    if version != 1 {
        return Err(format!(
            "{source_label} returned unsupported Version {}",
            version
        ));
    }
    let access_key_id = process_json_string_field(&payload, "AccessKeyId", source_label)?;
    let secret_access_key = process_json_string_field(&payload, "SecretAccessKey", source_label)?;
    let session_token = payload
        .get("SessionToken")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    if access_key_id.trim().is_empty() || secret_access_key.trim().is_empty() {
        return Err(format!("{source_label} returned empty access key material"));
    }
    Ok(AwsStaticCredentials::new(
        access_key_id,
        secret_access_key,
        session_token,
    ))
}

fn process_json_string_field(
    payload: &serde_json::Value,
    field: &'static str,
    source_label: &'static str,
) -> Result<String, String> {
    payload
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{source_label} returned missing {field}"))
}

pub fn aws_static_credentials_from_secret_material(
    secret_material: &str,
) -> Result<AwsStaticCredentials, String> {
    let trimmed = secret_material.trim();
    if trimmed.starts_with('{') {
        return process_json_credentials(trimmed.as_bytes(), "AWS static secret ref");
    }

    let mut access_key_id = None;
    let mut secret_access_key = None;
    let mut session_token = None;
    for raw_line in trimmed.lines() {
        let line = raw_line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with(';')
            || line.starts_with('[')
        {
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        let value = raw_value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_owned();
        match key {
            "aws_access_key_id" | "AccessKeyId" | "AWS_ACCESS_KEY_ID" => {
                access_key_id = Some(value)
            }
            "aws_secret_access_key" | "SecretAccessKey" | "AWS_SECRET_ACCESS_KEY" => {
                secret_access_key = Some(value)
            }
            "aws_session_token" | "SessionToken" | "AWS_SESSION_TOKEN" => {
                session_token = Some(value)
            }
            _ => {}
        }
    }

    let access_key_id =
        access_key_id.ok_or_else(|| "AWS static secret ref missing access key id".to_owned())?;
    let secret_access_key = secret_access_key
        .ok_or_else(|| "AWS static secret ref missing secret access key".to_owned())?;
    if access_key_id.trim().is_empty() || secret_access_key.trim().is_empty() {
        return Err("AWS static secret ref contains empty access key material".to_owned());
    }
    Ok(AwsStaticCredentials::new(
        access_key_id,
        secret_access_key,
        session_token,
    ))
}

fn split_credential_process_command(command_line: &str) -> Result<Vec<String>, String> {
    let mut argv = Vec::new();
    let mut current = String::new();
    let mut chars = command_line.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' | '"' if quote == Some(ch) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(ch),
            '\\' if quote == Some('"')
                && chars
                    .peek()
                    .is_some_and(|next| *next == '"' || *next == '\\') =>
            {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ch if ch.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    argv.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if let Some(quote) = quote {
        return Err(format!("credential_process has unterminated {quote} quote"));
    }
    if !current.is_empty() {
        argv.push(current);
    }
    Ok(argv)
}

fn static_credentials_from_environment() -> Option<AwsStaticCredentials> {
    let access_key_id = std::env::var("AWS_ACCESS_KEY_ID").ok()?;
    let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY").ok()?;
    let session_token = std::env::var("AWS_SESSION_TOKEN").ok();
    Some(AwsStaticCredentials::new(
        access_key_id,
        secret_access_key,
        session_token,
    ))
}

pub fn default_aws_shared_config_files() -> AwsSharedConfigFiles {
    let config_path = std::env::var_os("AWS_CONFIG_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| aws_home_path("config"));
    let credentials_path = std::env::var_os("AWS_SHARED_CREDENTIALS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| aws_home_path("credentials"));
    AwsSharedConfigFiles::new(config_path, credentials_path)
}

fn aws_home_path(file_name: &str) -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".aws")
        .join(file_name)
}

fn user_home_dir_display() -> String {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .display()
        .to_string()
}

#[derive(Clone)]
pub struct PersistenceTranscriptStore {
    persistence: PersistenceActorClient,
}

impl PersistenceTranscriptStore {
    pub fn new(persistence: PersistenceActorClient) -> Self {
        Self { persistence }
    }
}

impl TranscriptEventStore for PersistenceTranscriptStore {
    fn append(
        &self,
        event: TranscriptEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), TranscriptStoreError>> + Send + '_>> {
        Box::pin(async move {
            self.persistence
                .append_conversation_event(ConversationEvent {
                    conversation_id: event.conversation_id.0.clone(),
                    event_type: transcript_event_type(&event.kind).to_owned(),
                    payload_json: serde_json::to_string(&event).map_err(transcript_json_error)?,
                    created_at_ms: now_ms(),
                })
                .await
                .map(|_| ())
                .map_err(transcript_persistence_error)
        })
    }

    fn load(
        &self,
        conversation_id: ConversationId,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<TranscriptEvent>, TranscriptStoreError>> + Send + '_>>
    {
        Box::pin(async move {
            self.persistence
                .load_conversation_events(conversation_id.0)
                .await
                .map_err(transcript_persistence_error)?
                .into_iter()
                .filter(|record| record.event_type.starts_with("transcript."))
                .map(transcript_event_from_record)
                .collect()
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteFileOperationKind {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFileOperationTranscript {
    pub target_id: String,
    pub path: String,
    pub kind: RemoteFileOperationKind,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalProjectMetadataSnapshot {
    pub folder_id: String,
    pub label: String,
    pub root: String,
    pub settings_path: String,
    pub skills_dir: String,
    pub mcp_config_path: String,
    pub compatible_mcp_config_path: String,
    pub rtk_status: String,
    pub rtk_detail: String,
    pub rtk_resolved_path: Option<String>,
    pub instructions: Vec<ProjectInstructionSnapshot>,
    pub git: Option<GitMetadataSnapshot>,
    pub default_profile_id: Option<String>,
    pub recent_conversation_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchableProjectFolder {
    pub folder_id: String,
    pub label: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInstructionSnapshot {
    pub kind: ProjectInstructionSnapshotKind,
    pub path: String,
    pub contents: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectInstructionSnapshotKind {
    Agents,
    Claude,
    RooRules,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitMetadataSnapshot {
    pub worktree_root: String,
    pub git_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowShellSnapshot {
    pub active_conversation_title: String,
    pub active_folder: String,
    pub active_profile: String,
    pub active_mode: String,
    pub plan_mode_enabled: bool,
    pub rtk_enabled: bool,
    pub prompt_compression_enabled: bool,
    pub prompt_compression_level: String,
    pub send_shortcut: String,
    pub send_shortcut_label: String,
    pub sidebar_tree_rows: Vec<WorkflowSidebarTreeRow>,
    pub conversation_rows: Vec<WorkflowConversationRow>,
    pub project_folders: Vec<WorkflowProjectFolderRow>,
    pub transcript_blocks: Vec<WorkflowTranscriptBlock>,
    pub profile_rows: Vec<WorkflowProfileRow>,
    pub file_tabs: Vec<WorkflowFileTabRow>,
    pub editor_lines: Vec<WorkflowEditorLineRow>,
    pub diff_rows: Vec<WorkflowDiffRow>,
    pub mcp_servers: Vec<WorkflowMcpServerRow>,
    pub mcp_tools: Vec<WorkflowMcpToolRow>,
    pub mcp_discovery: Vec<WorkflowMcpDiscoveryRow>,
    pub skills: Vec<WorkflowSkillRow>,
    pub memory_rows: Vec<WorkflowMemoryRow>,
    pub settings_rows: Vec<WorkflowSettingsRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowSidebarTreeRow {
    pub row_id: String,
    pub kind: String,
    pub project_folder_id: String,
    pub conversation_id: String,
    pub title: String,
    pub subtitle: String,
    pub status: String,
    pub depth: i32,
    pub expanded: bool,
    pub selected: bool,
    pub tooltip: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowConversationRow {
    pub conversation_id: String,
    pub title: String,
    pub folder: String,
    pub profile: String,
    pub mode: String,
    pub status: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowProjectFolderRow {
    pub label: String,
    pub path: String,
    pub target: String,
    pub default_profile: String,
    pub recent_conversations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowTranscriptBlock {
    pub speaker: String,
    pub body: String,
    pub accent_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowFileTabRow {
    pub path: String,
    pub dirty: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowEditorLineRow {
    pub line_number: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDiffRow {
    pub line_number: usize,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowProfileRow {
    pub profile_id: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    pub region: String,
    pub enabled: bool,
    pub default_for_new_conversations: bool,
    pub selected: bool,
    pub validation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowMcpServerRow {
    pub server_id: String,
    pub name: String,
    pub scope: String,
    pub transport: String,
    pub status: String,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowMcpToolRow {
    pub server_id: String,
    pub tool_name: String,
    pub policy: String,
    pub discovered: bool,
    pub disabled: bool,
    pub always_allowed: bool,
    pub disable_action: String,
    pub allow_action: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowMcpDiscoveryRow {
    pub server_id: String,
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowSkillRow {
    pub name: String,
    pub scope: String,
    pub modes: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowMemoryRow {
    pub record_id: String,
    pub kind: String,
    pub scope: String,
    pub source: String,
    pub status: String,
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowSettingsRow {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiPersistenceWrite {
    UpsertLlmProfile(LlmProfileRecord),
    DeleteLlmProfile(String),
    StoreSettingsDocument(SettingsDocument),
    UpsertProjectFolder(ProjectFolderRecord),
    DeleteProjectFolder(String),
    UpsertConversation(ConversationMetadata),
    DeleteConversation(String),
    AppendConversationEvent(ConversationEvent),
    UpsertGoal(GoalRecord),
    DeleteGoal(String),
    UpsertEditorTab(EditorTabRecord),
    DeleteEditorTab(String),
    SaveEditorFile { path: String, contents: String },
    UpsertMcpStatus(McpStatusSnapshot),
    DeleteMcpStatus(String),
    UpsertMemoryRecord(MemoryRecord),
    DeleteMemoryRecord(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiRuntimeWork {
    RunModel(BedrockModelRunRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiActionResult {
    pub status: String,
    pub writes: Vec<GuiPersistenceWrite>,
    pub runtime_work: Vec<GuiRuntimeWork>,
}

impl GuiActionResult {
    fn status(status: impl Into<String>) -> Self {
        Self {
            status: status.into(),
            writes: Vec::new(),
            runtime_work: Vec::new(),
        }
    }

    fn with_writes(status: impl Into<String>, writes: Vec<GuiPersistenceWrite>) -> Self {
        Self {
            status: status.into(),
            writes,
            runtime_work: Vec::new(),
        }
    }

    fn with_writes_and_runtime_work(
        status: impl Into<String>,
        writes: Vec<GuiPersistenceWrite>,
        runtime_work: Vec<GuiRuntimeWork>,
    ) -> Self {
        Self {
            status: status.into(),
            writes,
            runtime_work,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPreferences {
    pub rtk_enabled: bool,
    pub prompt_compression_enabled: bool,
    pub prompt_compression_level: PromptCompressionLevel,
    pub send_shortcut: SendShortcut,
    pub collapsed_project_folder_ids: BTreeSet<String>,
    pub unbound_collapsed: bool,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            rtk_enabled: true,
            prompt_compression_enabled: true,
            prompt_compression_level: PromptCompressionLevel::Full,
            send_shortcut: SendShortcut::ControlEnter,
            collapsed_project_folder_ids: BTreeSet::new(),
            unbound_collapsed: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptCompressionLevel {
    Lite,
    Full,
    Ultra,
}

impl PromptCompressionLevel {
    fn stored_label(self) -> &'static str {
        match self {
            Self::Lite => "lite",
            Self::Full => "full",
            Self::Ultra => "ultra",
        }
    }

    fn display_label(self) -> &'static str {
        match self {
            Self::Lite => "Lite",
            Self::Full => "Full",
            Self::Ultra => "Ultra",
        }
    }

    fn from_stored_label(value: &str) -> Option<Self> {
        match value {
            "lite" | "Lite" => Some(Self::Lite),
            "full" | "Full" => Some(Self::Full),
            "ultra" | "Ultra" => Some(Self::Ultra),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendShortcut {
    ControlEnter,
    Enter,
    MetaEnter,
}

impl SendShortcut {
    fn stored_label(self) -> &'static str {
        match self {
            Self::ControlEnter => "control-enter",
            Self::Enter => "enter",
            Self::MetaEnter => "meta-enter",
        }
    }

    fn display_label(self) -> &'static str {
        match self {
            Self::ControlEnter => "Ctrl+Enter",
            Self::Enter => "Enter",
            Self::MetaEnter => "Meta+Enter",
        }
    }

    fn from_stored_label(value: &str) -> Option<Self> {
        match value {
            "control-enter" | "ctrl-enter" | "Ctrl+Enter" => Some(Self::ControlEnter),
            "enter" | "Enter" => Some(Self::Enter),
            "meta-enter" | "cmd-enter" | "Meta+Enter" => Some(Self::MetaEnter),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct FastrockWorkflow {
    profiles: LlmProfileSettingsState,
    project_folders: BTreeMap<String, ProjectFolderRecord>,
    selected_project_folder_id: Option<String>,
    conversation_metadata: BTreeMap<String, ConversationMetadata>,
    conversation_events: BTreeMap<ConversationId, Vec<TranscriptEvent>>,
    conversation_modes: BTreeMap<ConversationId, AgentMode>,
    scheduler: ConversationScheduler,
    goals: BTreeMap<ConversationId, GoalLoopState>,
    editor_tabs: EditorTabSet,
    editor_tab_records: BTreeMap<String, EditorTabRecord>,
    mcp_config: McpConfigSet,
    mcp_statuses: McpRuntimeSnapshotSet,
    skills: Vec<SkillDocument>,
    cache_ledger: CacheUsageLedger,
    context_fragment_cache: LocalContextFragmentCache,
    tool_output_cache: ToolOutputDedupCache,
    memory_store: MemoryStore,
    command_policy: CommandPolicy,
    active_commands: usize,
    local_command_transcripts: Vec<RtkCommandTranscript>,
    remote_command_transcripts: Vec<RemoteCommandTranscript>,
    remote_file_transcripts: Vec<RemoteFileOperationTranscript>,
    saved_editor_paths: Vec<String>,
    app_preferences: AppPreferences,
}

impl FastrockWorkflow {
    pub fn profiles(&self) -> &LlmProfileSettingsState {
        &self.profiles
    }

    pub fn project_folders(&self) -> &BTreeMap<String, ProjectFolderRecord> {
        &self.project_folders
    }

    pub fn watchable_local_project_folders(
        &self,
    ) -> PersistenceResult<Vec<WatchableProjectFolder>> {
        self.project_folders
            .values()
            .filter(|record| record.target_kind == "local")
            .filter_map(|record| match project_folder_watch_enabled(record) {
                Ok(true) => Some(Ok(WatchableProjectFolder {
                    folder_id: record.id.clone(),
                    label: record.label.clone(),
                    root: PathBuf::from(record.path.clone()),
                })),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    pub fn conversation_mode(&self, conversation_id: &ConversationId) -> AgentMode {
        self.conversation_modes
            .get(conversation_id)
            .cloned()
            .unwrap_or(AgentMode::Code)
    }

    pub fn set_conversation_mode(
        &mut self,
        conversation_id: &ConversationId,
        mode: AgentMode,
    ) -> bool {
        if self.scheduler.conversation(conversation_id).is_none() {
            return false;
        }
        if mode == AgentMode::Code {
            self.conversation_modes.remove(conversation_id);
        } else {
            self.conversation_modes
                .insert(conversation_id.clone(), mode);
        }
        true
    }

    pub fn toggle_active_plan_mode(&mut self) -> Option<(ConversationId, AgentMode)> {
        let conversation_id = self.scheduler.visible_conversation_id()?.clone();
        let next_mode = if self.conversation_mode(&conversation_id) == AgentMode::Plan {
            AgentMode::Code
        } else {
            AgentMode::Plan
        };
        self.set_conversation_mode(&conversation_id, next_mode.clone())
            .then_some((conversation_id, next_mode))
    }

    pub fn append_user_message_to_visible(
        &mut self,
        text: impl Into<String>,
    ) -> Option<TranscriptEvent> {
        let text = text.into();
        if text.trim().is_empty() {
            return None;
        }
        self.append_transcript_event_to_visible(TranscriptEventKind::UserMessage { text })
    }

    pub fn append_transcript_event_to_visible(
        &mut self,
        kind: TranscriptEventKind,
    ) -> Option<TranscriptEvent> {
        let conversation_id = self.scheduler.visible_conversation_id()?.clone();
        self.append_transcript_event_to_conversation(&conversation_id, kind)
    }

    pub fn append_transcript_event_to_conversation(
        &mut self,
        conversation_id: &ConversationId,
        kind: TranscriptEventKind,
    ) -> Option<TranscriptEvent> {
        self.scheduler.conversation(conversation_id)?;
        let sequence = self.next_conversation_event_sequence(conversation_id);
        let event = TranscriptEvent {
            conversation_id: conversation_id.clone(),
            sequence,
            kind,
        };
        self.conversation_events
            .entry(conversation_id.clone())
            .or_default()
            .push(event.clone());
        Some(event)
    }

    pub fn append_model_run_result(
        &mut self,
        result: BedrockModelRunResult,
        now_ms: i64,
    ) -> Vec<GuiPersistenceWrite> {
        let BedrockModelRunResult {
            conversation_id,
            metadata,
            events,
        } = result;
        let mut writes = Vec::new();
        if let Some(event) = self.append_transcript_event_to_conversation(
            &conversation_id,
            TranscriptEventKind::ModelRequest {
                metadata: Box::new(metadata),
            },
        ) && let Some(write) = gui_transcript_event_write(&event, now_ms)
        {
            writes.push(write);
        }
        for model_event in events {
            if let Some(event) = self.append_transcript_event_to_conversation(
                &conversation_id,
                TranscriptEventKind::ModelStreamEvent { event: model_event },
            ) && let Some(write) = gui_transcript_event_write(&event, now_ms)
            {
                writes.push(write);
            }
        }
        writes
    }

    pub fn mantle_stored_response_state(
        &self,
        conversation_id: &ConversationId,
        project_id: Option<&str>,
    ) -> MantleStoredResponseState {
        let target_project_id = project_id.map(str::to_owned);
        let mut active_stored_request_project: Option<Option<String>> = None;
        let mut last_response_id = None;

        for event in self
            .conversation_events
            .get(conversation_id)
            .into_iter()
            .flatten()
        {
            match &event.kind {
                TranscriptEventKind::ModelRequest { metadata }
                    if metadata.provider == ModelProviderKind::BedrockMantle =>
                {
                    active_stored_request_project =
                        (metadata.store == Some(true)).then(|| metadata.project_id.clone());
                }
                TranscriptEventKind::ModelStreamEvent { event }
                    if event.provider == ModelProviderKind::BedrockMantle =>
                {
                    if let NormalizedModelStreamEventKind::Started {
                        response_id: Some(response_id),
                        ..
                    } = &event.kind
                        && active_stored_request_project.as_ref() == Some(&target_project_id)
                    {
                        last_response_id = Some(response_id.clone());
                    }
                }
                _ => {}
            }
        }

        MantleStoredResponseState {
            project_id: target_project_id,
            last_response_id,
        }
    }

    pub fn append_model_run_error(
        &mut self,
        request: &BedrockModelRunRequest,
        error: impl fmt::Display,
        now_ms: i64,
    ) -> Vec<GuiPersistenceWrite> {
        let fastrock_error = FastrockError::new(
            FastrockErrorKind::ProviderStreamDisconnect,
            error.to_string(),
        )
        .with_conversation_id(request.conversation_id.0.clone())
        .with_profile_id(request.profile_id.clone());
        let Some(event) = self.append_transcript_event_to_conversation(
            &request.conversation_id,
            TranscriptEventKind::Error {
                error: Box::new(fastrock_error),
            },
        ) else {
            return Vec::new();
        };
        gui_transcript_event_write(&event, now_ms)
            .into_iter()
            .collect()
    }

    pub fn start_plan_for_visible(&mut self, prompt: impl Into<String>) -> Option<TranscriptEvent> {
        let prompt = prompt.into();
        if prompt.trim().is_empty() {
            return None;
        }
        let conversation_id = self.scheduler.visible_conversation_id()?.clone();
        let _ = self.set_conversation_mode(&conversation_id, AgentMode::Plan);
        let sequence = self.next_conversation_event_sequence(&conversation_id);
        let artifact = PlanArtifact {
            id: format!("plan-{}", generated_persisted_id()),
            prompt,
            status: PlanArtifactStatus::Draft,
            created_sequence: sequence,
            executed_sequence: None,
        };
        let event = TranscriptEvent {
            conversation_id: conversation_id.clone(),
            sequence,
            kind: TranscriptEventKind::PlanCreated {
                artifact: Box::new(artifact),
            },
        };
        self.conversation_events
            .entry(conversation_id)
            .or_default()
            .push(event.clone());
        Some(event)
    }

    pub fn execute_latest_plan_for_visible(
        &mut self,
    ) -> Option<(TranscriptEvent, AgentMode, String)> {
        let conversation_id = self.scheduler.visible_conversation_id()?.clone();
        let plan_id = self.latest_unexecuted_plan_id(&conversation_id)?;
        let sequence = self.next_conversation_event_sequence(&conversation_id);
        let event = TranscriptEvent {
            conversation_id: conversation_id.clone(),
            sequence,
            kind: TranscriptEventKind::PlanExecuted {
                plan_id: plan_id.clone(),
            },
        };
        self.conversation_events
            .entry(conversation_id.clone())
            .or_default()
            .push(event.clone());
        let mode = AgentMode::Code;
        let _ = self.set_conversation_mode(&conversation_id, mode.clone());
        Some((event, mode, plan_id))
    }

    fn next_conversation_event_sequence(&self, conversation_id: &ConversationId) -> u64 {
        self.conversation_events
            .get(conversation_id)
            .and_then(|events| events.iter().map(|event| event.sequence).max())
            .unwrap_or_default()
            + 1
    }

    fn latest_unexecuted_plan_id(&self, conversation_id: &ConversationId) -> Option<String> {
        let events = self.conversation_events.get(conversation_id)?;
        let executed = events
            .iter()
            .filter_map(|event| match &event.kind {
                TranscriptEventKind::PlanExecuted { plan_id } => Some(plan_id.as_str()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        events.iter().rev().find_map(|event| match &event.kind {
            TranscriptEventKind::PlanCreated { artifact }
                if artifact.status == PlanArtifactStatus::Draft
                    && !executed.contains(artifact.id.as_str()) =>
            {
                Some(artifact.id.clone())
            }
            _ => None,
        })
    }

    pub fn conversation_sidebar_model(&self) -> ConversationSidebarModel {
        let visible_id = self.scheduler.visible_conversation_id();
        let rows = self
            .scheduler
            .conversations()
            .into_iter()
            .map(|conversation| {
                let metadata = self
                    .conversation_metadata
                    .get(&conversation.conversation_id.0);
                let project_folder_id =
                    metadata.and_then(|metadata| metadata.project_folder_id.clone());
                let project = project_folder_id
                    .as_ref()
                    .and_then(|project_folder_id| self.project_folders.get(project_folder_id));
                let default_profile_id =
                    project.and_then(|project| project_default_profile_id(project).ok().flatten());
                let profile_name = default_profile_id
                    .as_deref()
                    .and_then(|profile_id| self.profiles.profile(profile_id))
                    .or_else(|| {
                        self.profiles
                            .profiles()
                            .iter()
                            .find(|profile| profile.default_for_new_conversations)
                    })
                    .or_else(|| self.profiles.profiles().first())
                    .map(|profile| profile.name.clone())
                    .unwrap_or_else(|| "No profile".to_owned());
                ConversationListRow {
                    conversation_id: conversation.conversation_id.0.clone(),
                    title: conversation.title.clone(),
                    project_folder_id,
                    folder_label: project
                        .map(|project| project.label.clone())
                        .or_else(|| {
                            metadata.and_then(|metadata| metadata.project_folder_id.clone())
                        })
                        .unwrap_or_else(|| "Home".to_owned()),
                    folder_path: project
                        .map(|project| project.path.clone())
                        .unwrap_or_else(user_home_dir_display),
                    target_kind: project
                        .map(project_target_kind)
                        .unwrap_or(UiProjectTargetKind::Local),
                    target_label: project_target_label(project),
                    profile_name,
                    status: conversation_status_label(&conversation.status).to_owned(),
                    goal_status: self
                        .goals
                        .get(&conversation.conversation_id)
                        .and_then(GoalLoopState::snapshot)
                        .map(|goal| goal_status_label(&goal.status).to_owned()),
                    last_updated_ms: metadata
                        .map(|metadata| metadata.updated_at_ms)
                        .unwrap_or_default(),
                    selected: visible_id == Some(&conversation.conversation_id),
                    hidden_unread_events: conversation
                        .hidden_progress
                        .as_ref()
                        .map(|progress| progress.unread_events)
                        .unwrap_or_default(),
                }
            })
            .collect();
        ConversationSidebarModel::new(rows)
    }

    pub fn shell_snapshot(&self) -> WorkflowShellSnapshot {
        let sidebar = self.conversation_sidebar_model();
        let active_row = sidebar
            .rows()
            .iter()
            .find(|row| row.selected)
            .or_else(|| sidebar.rows().first());
        let active_conversation_title = active_row
            .map(|row| row.title.clone())
            .unwrap_or_else(|| "No conversation".to_owned());
        let active_folder = active_row
            .map(shell_folder_label)
            .unwrap_or_else(|| "No folder".to_owned());
        let active_profile = self
            .shell_profile(active_row)
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| "No profile".to_owned());
        let active_mode = active_row
            .map(|row| self.conversation_mode(&ConversationId::new(row.conversation_id.clone())))
            .unwrap_or(AgentMode::Code);
        let active_mode_label = agent_mode_display(&active_mode).to_owned();
        let plan_mode_enabled = active_mode == AgentMode::Plan;
        let sidebar_tree_rows = self.shell_sidebar_tree_rows(&sidebar);
        let conversation_rows = sidebar
            .rows()
            .iter()
            .map(|row| WorkflowConversationRow {
                conversation_id: row.conversation_id.clone(),
                title: row.title.clone(),
                folder: shell_folder_label(row),
                profile: row.profile_name.clone(),
                mode: agent_mode_display(
                    &self.conversation_mode(&ConversationId::new(row.conversation_id.clone())),
                )
                .to_owned(),
                status: row
                    .goal_status
                    .as_ref()
                    .map(|goal_status| format!("{} / {goal_status}", row.status))
                    .unwrap_or_else(|| row.status.clone()),
                selected: row.selected,
            })
            .collect::<Vec<_>>();
        let project_folders = self.shell_project_folder_rows();
        let file_tabs = self
            .editor_tabs
            .tabs()
            .iter()
            .map(|tab| WorkflowFileTabRow {
                path: tab.path.clone(),
                dirty: tab.dirty,
                selected: self.editor_tabs.active_path() == Some(tab.path.as_str()),
            })
            .collect::<Vec<_>>();
        let editor_lines = self.shell_editor_lines();
        let diff_rows = self.shell_diff_rows();
        let transcript_blocks = self.shell_transcript_blocks(active_row);
        let profile_rows = self.shell_profile_rows();
        let mcp_servers = self.shell_mcp_server_rows();
        let mcp_tools = self.shell_mcp_tool_rows();
        let mcp_discovery = self.shell_mcp_discovery_rows();
        let skills = self.shell_skill_rows();
        let memory_rows = self.shell_memory_rows();
        let settings_rows = self.shell_settings_rows(active_row);

        WorkflowShellSnapshot {
            active_conversation_title,
            active_folder,
            active_profile,
            active_mode: active_mode_label,
            plan_mode_enabled,
            rtk_enabled: self.app_preferences.rtk_enabled,
            prompt_compression_enabled: self.app_preferences.prompt_compression_enabled,
            prompt_compression_level: self
                .app_preferences
                .prompt_compression_level
                .display_label()
                .to_owned(),
            send_shortcut: self.app_preferences.send_shortcut.stored_label().to_owned(),
            send_shortcut_label: self
                .app_preferences
                .send_shortcut
                .display_label()
                .to_owned(),
            sidebar_tree_rows,
            conversation_rows,
            project_folders,
            transcript_blocks,
            profile_rows,
            file_tabs,
            editor_lines,
            diff_rows,
            mcp_servers,
            mcp_tools,
            mcp_discovery,
            skills,
            memory_rows,
            settings_rows,
        }
    }

    pub fn load_local_project_metadata_snapshot(
        &self,
        folder_id: &str,
    ) -> PersistenceResult<Option<LocalProjectMetadataSnapshot>> {
        let Some(record) = self.project_folders.get(folder_id) else {
            return Ok(None);
        };
        if record.target_kind != "local" {
            return Ok(None);
        }

        let local = LocalProjectFolder::open(ProjectFolderSpec {
            label: record.label.clone(),
            path: record.path.clone(),
            target: ProjectTarget::Local,
        })
        .map_err(project_metadata_persistence_error)?;
        let metadata = local
            .metadata(
                project_default_profile_id(record)?,
                project_recent_conversation_ids(record)?,
            )
            .map_err(project_metadata_persistence_error)?;
        Ok(Some(local_project_metadata_snapshot(folder_id, metadata)))
    }

    pub fn scheduler(&self) -> &ConversationScheduler {
        &self.scheduler
    }

    pub fn editor_tabs(&self) -> &EditorTabSet {
        &self.editor_tabs
    }

    pub fn editor_tab_records(&self) -> &BTreeMap<String, EditorTabRecord> {
        &self.editor_tab_records
    }

    pub fn local_command_transcripts(&self) -> &[RtkCommandTranscript] {
        &self.local_command_transcripts
    }

    pub fn command_policy(&self) -> &CommandPolicy {
        &self.command_policy
    }

    pub fn set_command_policy(&mut self, policy: CommandPolicy) {
        self.command_policy = policy;
    }

    pub fn app_preferences(&self) -> &AppPreferences {
        &self.app_preferences
    }

    pub fn set_app_preferences(&mut self, preferences: AppPreferences) {
        self.app_preferences = preferences;
    }

    pub async fn set_persisted_command_policy(
        &mut self,
        persistence: &PersistenceActorHandle,
        policy: CommandPolicy,
        now_ms: i64,
    ) -> PersistenceResult<()> {
        self.command_policy = policy;
        self.persist_command_policy(persistence, now_ms).await
    }

    pub async fn set_persisted_app_preferences(
        &mut self,
        persistence: &PersistenceActorHandle,
        preferences: AppPreferences,
        now_ms: i64,
    ) -> PersistenceResult<()> {
        self.app_preferences = preferences;
        self.persist_app_preferences(persistence, now_ms).await
    }

    pub fn remote_command_transcripts(&self) -> &[RemoteCommandTranscript] {
        &self.remote_command_transcripts
    }

    pub fn remote_file_transcripts(&self) -> &[RemoteFileOperationTranscript] {
        &self.remote_file_transcripts
    }

    pub fn skills(&self) -> &[SkillDocument] {
        &self.skills
    }

    pub fn mcp_config(&self) -> &McpConfigSet {
        &self.mcp_config
    }

    pub fn mcp_statuses(&self) -> Vec<McpRuntimeSnapshot> {
        self.mcp_statuses.list()
    }

    pub fn mcp_status(&self, server_id: &str) -> Option<&McpRuntimeSnapshot> {
        self.mcp_statuses.get(server_id)
    }

    pub fn memory_records(&self) -> Vec<MemoryRecord> {
        self.memory_store.records()
    }

    pub fn search_memory(
        &self,
        query: &str,
        project_folder_id: Option<&str>,
        limit: usize,
    ) -> Vec<MemorySearchResult> {
        self.memory_store.search(query, project_folder_id, limit)
    }

    pub fn memory_injections(
        &self,
        query: &str,
        project_folder_id: Option<&str>,
        max_items: usize,
        max_bytes: usize,
    ) -> Vec<MemoryInjection> {
        self.memory_store
            .bounded_injections(query, project_folder_id, max_items, max_bytes)
    }

    pub fn remember_memory(&mut self, record: MemoryRecord) {
        self.memory_store.upsert(record);
    }

    pub async fn remember_persisted_memory(
        &mut self,
        persistence: &PersistenceActorHandle,
        record: MemoryRecord,
    ) -> PersistenceResult<()> {
        persistence.upsert_memory_record(record.clone()).await?;
        self.remember_memory(record);
        Ok(())
    }

    pub fn set_memory_enabled(&mut self, record_id: &str, enabled: bool, now_ms: i64) -> bool {
        self.memory_store.set_enabled(record_id, enabled, now_ms)
    }

    pub async fn set_persisted_memory_enabled(
        &mut self,
        persistence: &PersistenceActorHandle,
        record_id: &str,
        enabled: bool,
        now_ms: i64,
    ) -> PersistenceResult<bool> {
        let changed_on_disk = persistence
            .set_memory_enabled(record_id, enabled, now_ms)
            .await?;
        let changed_in_memory = self.set_memory_enabled(record_id, enabled, now_ms);
        Ok(changed_in_memory || changed_on_disk)
    }

    pub fn delete_memory(&mut self, record_id: &str) -> bool {
        self.memory_store.delete(record_id)
    }

    pub async fn delete_persisted_memory(
        &mut self,
        persistence: &PersistenceActorHandle,
        record_id: &str,
    ) -> PersistenceResult<bool> {
        let removed_from_disk = persistence.delete_memory_record(record_id).await?;
        let removed_from_memory = self.delete_memory(record_id);
        Ok(removed_from_memory || removed_from_disk)
    }

    pub fn create_bedrock_profile(
        &mut self,
        request: BedrockProfileRequest,
    ) -> Result<(), Vec<fastrock_ui::ProfileValidationError>> {
        self.save_profile_form(request.into_form())
    }

    pub fn save_profile_form(
        &mut self,
        profile: LlmProfileForm,
    ) -> Result<(), Vec<fastrock_ui::ProfileValidationError>> {
        self.profiles.update_draft(profile);
        self.profiles.save_draft()
    }

    pub fn create_default_bedrock_profile(
        &mut self,
        provider: UiLlmProvider,
    ) -> Result<LlmProfileForm, Vec<fastrock_ui::ProfileValidationError>> {
        let draft = self.start_default_bedrock_profile_draft(provider);
        self.profiles.save_draft()?;
        Ok(self
            .profiles
            .profile(&draft.id)
            .expect("saved default profile exists")
            .clone())
    }

    pub fn start_default_bedrock_profile_draft(
        &mut self,
        provider: UiLlmProvider,
    ) -> LlmProfileForm {
        let mut draft = LlmProfileForm::new_bedrock_mantle(self.generated_profile_id());
        draft.name = unique_profile_name(self.profiles.profiles(), default_profile_name(provider));
        draft.provider = provider;
        draft.model_id = "anthropic.claude-3-5-sonnet-20241022-v2:0".to_owned();
        draft.region = "us-east-1".to_owned();
        draft.credential_source = UiCredentialSource::AwsCliProfile {
            profile_name: "default".to_owned(),
        };
        draft.runtime_settings.target_type = UiRuntimeTargetType::FoundationModel;
        draft.runtime_settings.target = draft.model_id.clone();
        draft.default_for_new_conversations = self.profiles.profiles().is_empty();
        self.profiles.update_draft(draft.clone());
        draft
    }

    pub fn save_profile_draft(
        &mut self,
    ) -> Result<LlmProfileForm, Vec<fastrock_ui::ProfileValidationError>> {
        let Some(draft) = self.profiles.draft().cloned() else {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "draft",
                message: "no profile is being edited",
            }]);
        };
        self.profiles.save_draft()?;
        Ok(self
            .profiles
            .profile(&draft.id)
            .expect("saved draft profile exists")
            .clone())
    }

    pub async fn create_persisted_bedrock_profile(
        &mut self,
        persistence: &PersistenceActorHandle,
        request: BedrockProfileRequest,
        now_ms: i64,
    ) -> PersistenceResult<Result<(), Vec<fastrock_ui::ProfileValidationError>>> {
        let profile_id = request.id.clone();
        match self.create_bedrock_profile(request) {
            Ok(()) => {
                self.persist_profile(persistence, &profile_id, now_ms)
                    .await?;
                Ok(Ok(()))
            }
            Err(errors) => Ok(Err(errors)),
        }
    }

    pub fn duplicate_profile(
        &mut self,
        profile_id: &str,
    ) -> Result<LlmProfileForm, Vec<fastrock_ui::ProfileValidationError>> {
        let Some(mut duplicate) = self.profiles.profile(profile_id).cloned() else {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "id",
                message: "profile not found",
            }]);
        };
        let duplicate_id = self.generated_profile_id();
        duplicate.id = duplicate_id.clone();
        duplicate.name = unique_profile_name(
            self.profiles.profiles(),
            &format!("{} Copy", duplicate.name),
        );
        duplicate.default_for_new_conversations = false;
        self.profiles.update_draft(duplicate);
        self.profiles.save_draft()?;
        Ok(self
            .profiles
            .profile(&duplicate_id)
            .expect("saved duplicated profile exists")
            .clone())
    }

    pub fn duplicate_selected_or_default_profile(
        &mut self,
    ) -> Result<LlmProfileForm, Vec<fastrock_ui::ProfileValidationError>> {
        let Some(profile_id) = self.selected_or_default_profile_id() else {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "profile",
                message: "no profile to duplicate",
            }]);
        };
        self.duplicate_profile(&profile_id)
    }

    pub fn duplicate_selected_or_default_profile_draft(
        &mut self,
    ) -> Result<LlmProfileForm, Vec<fastrock_ui::ProfileValidationError>> {
        let Some(profile_id) = self.selected_or_default_profile_id() else {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "profile",
                message: "no profile to duplicate",
            }]);
        };
        let Some(draft) = self.profiles.duplicate_profile(&profile_id).cloned() else {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "id",
                message: "profile not found",
            }]);
        };
        Ok(draft)
    }

    pub async fn duplicate_persisted_profile(
        &mut self,
        persistence: &PersistenceActorHandle,
        profile_id: &str,
        now_ms: i64,
    ) -> PersistenceResult<Result<LlmProfileForm, Vec<fastrock_ui::ProfileValidationError>>> {
        match self.duplicate_profile(profile_id) {
            Ok(profile) => {
                self.persist_profile(persistence, &profile.id, now_ms)
                    .await?;
                Ok(Ok(profile))
            }
            Err(errors) => Ok(Err(errors)),
        }
    }

    pub fn edit_profile_name(
        &mut self,
        profile_id: &str,
        name: &str,
    ) -> Result<(), Vec<fastrock_ui::ProfileValidationError>> {
        if !self.profiles.edit_profile(profile_id) {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "id",
                message: "profile not found",
            }]);
        }
        let mut draft = self.profiles.draft().expect("draft exists").clone();
        draft.name = name.to_owned();
        self.profiles.update_draft(draft);
        self.profiles.save_draft()
    }

    pub fn edit_selected_or_default_profile_name(
        &mut self,
    ) -> Result<LlmProfileForm, Vec<fastrock_ui::ProfileValidationError>> {
        let Some(profile_id) = self.selected_or_default_profile_id() else {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "profile",
                message: "no profile to edit",
            }]);
        };
        let current_name = self
            .profiles
            .profile(&profile_id)
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        let edited_name =
            unique_profile_name_excluding(self.profiles.profiles(), &current_name, " Edited");
        self.edit_profile_name(&profile_id, &edited_name)?;
        Ok(self
            .profiles
            .profile(&profile_id)
            .expect("edited profile exists")
            .clone())
    }

    pub fn edit_selected_or_default_profile_name_draft(
        &mut self,
    ) -> Result<LlmProfileForm, Vec<fastrock_ui::ProfileValidationError>> {
        let Some(profile_id) = self.selected_or_default_profile_id() else {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "profile",
                message: "no profile to edit",
            }]);
        };
        let current_name = self
            .profiles
            .profile(&profile_id)
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        if !self.profiles.edit_profile(&profile_id) {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "id",
                message: "profile not found",
            }]);
        }
        let mut draft = self.profiles.draft().expect("draft exists").clone();
        draft.name =
            unique_profile_name_excluding(self.profiles.profiles(), &current_name, " Edited");
        self.profiles.update_draft(draft.clone());
        Ok(draft)
    }

    pub async fn edit_persisted_profile_name(
        &mut self,
        persistence: &PersistenceActorHandle,
        profile_id: &str,
        name: &str,
        now_ms: i64,
    ) -> PersistenceResult<Result<(), Vec<fastrock_ui::ProfileValidationError>>> {
        match self.edit_profile_name(profile_id, name) {
            Ok(()) => {
                self.persist_profile(persistence, profile_id, now_ms)
                    .await?;
                Ok(Ok(()))
            }
            Err(errors) => Ok(Err(errors)),
        }
    }

    pub fn delete_profile(&mut self, profile_id: &str) -> bool {
        self.profiles.remove_profile(profile_id)
    }

    pub fn delete_selected_or_default_profile(&mut self) -> Option<String> {
        let profile_id = self.selected_or_default_profile_id()?;
        self.delete_profile(&profile_id).then_some(profile_id)
    }

    pub fn select_profile(&mut self, profile_id: &str) -> bool {
        self.profiles.edit_profile(profile_id)
    }

    pub fn set_profile_enabled(
        &mut self,
        profile_id: &str,
        enabled: bool,
    ) -> Result<LlmProfileForm, Vec<fastrock_ui::ProfileValidationError>> {
        let Some(mut profile) = self.profiles.profile(profile_id).cloned() else {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "profile_id",
                message: "profile not found",
            }]);
        };
        profile.enabled = enabled;
        self.profiles.update_draft(profile);
        self.profiles.save_draft()?;
        Ok(self
            .profiles
            .profile(profile_id)
            .expect("profile exists after enabled update")
            .clone())
    }

    pub fn set_profile_default(
        &mut self,
        profile_id: &str,
    ) -> Result<LlmProfileForm, Vec<fastrock_ui::ProfileValidationError>> {
        let Some(mut profile) = self.profiles.profile(profile_id).cloned() else {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "profile_id",
                message: "profile not found",
            }]);
        };
        profile.enabled = true;
        profile.default_for_new_conversations = true;
        self.profiles.update_draft(profile);
        self.profiles.save_draft()?;
        Ok(self
            .profiles
            .profile(profile_id)
            .expect("profile exists after default update")
            .clone())
    }

    pub fn update_selected_or_default_profile_tuning(
        &mut self,
        edit: impl FnOnce(&mut fastrock_ui::UiRequestTuning),
    ) -> Result<LlmProfileForm, Vec<fastrock_ui::ProfileValidationError>> {
        let Some(profile_id) = self.selected_or_default_profile_id() else {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "profile",
                message: "no profile to edit",
            }]);
        };
        let Some(mut profile) = self.profiles.profile(&profile_id).cloned() else {
            return Err(vec![fastrock_ui::ProfileValidationError {
                field: "profile_id",
                message: "profile not found",
            }]);
        };
        edit(&mut profile.request_tuning);
        self.profiles.update_draft(profile);
        self.profiles.save_draft()?;
        Ok(self
            .profiles
            .profile(&profile_id)
            .expect("profile exists after tuning update")
            .clone())
    }

    pub async fn delete_persisted_profile(
        &mut self,
        persistence: &PersistenceActorHandle,
        profile_id: &str,
    ) -> PersistenceResult<bool> {
        let removed_from_memory = self.delete_profile(profile_id);
        let removed_from_disk = persistence.delete_llm_profile(profile_id).await?;
        Ok(removed_from_memory || removed_from_disk)
    }

    pub fn discover_aws_cli_profiles_from_files(
        files: &AwsSharedConfigFiles,
    ) -> Result<Vec<AwsCliProfileSummary>, fastrock_bedrock::AwsSharedConfigError> {
        let shared_config = files.load()?;
        Ok(Self::discover_aws_cli_profiles(&shared_config))
    }

    pub fn discover_aws_cli_profiles(shared_config: &AwsSharedConfig) -> Vec<AwsCliProfileSummary> {
        aws_cli_profile_summaries(shared_config)
    }

    pub fn refresh_aws_credential_previews_from_files(
        &mut self,
        files: &AwsSharedConfigFiles,
    ) -> Result<Vec<AwsCredentialPreviewUpdate>, fastrock_bedrock::AwsSharedConfigError> {
        let shared_config = files.load()?;
        Ok(self.refresh_aws_credential_previews(&shared_config))
    }

    pub fn refresh_aws_credential_previews(
        &mut self,
        shared_config: &AwsSharedConfig,
    ) -> Vec<AwsCredentialPreviewUpdate> {
        let profile_inputs = self
            .profiles
            .profiles()
            .iter()
            .map(|profile| {
                (
                    profile.id.clone(),
                    profile.region.clone(),
                    profile.credential_source.clone(),
                )
            })
            .collect::<Vec<_>>();

        profile_inputs
            .into_iter()
            .map(|(profile_id, region, source)| {
                let (preview, profile_found) =
                    credential_preview_for_source(shared_config, &source, &region);
                self.profiles
                    .set_credential_preview(&profile_id, Some(preview.clone()));
                AwsCredentialPreviewUpdate {
                    profile_id,
                    preview,
                    profile_found,
                }
            })
            .collect()
    }

    pub async fn discover_mantle_models<T, S>(
        &self,
        profile_id: &str,
        shared_config: &AwsSharedConfig,
        bearer_api_key: Option<String>,
        transport: T,
        signer: S,
    ) -> Result<BedrockModelDiscoveryResult, BedrockProfileProbeError>
    where
        T: MantleHttpTransport,
        S: MantleSigV4Signer,
    {
        let profile = self.bedrock_profile(profile_id, UiLlmProvider::BedrockMantle)?;
        let endpoint =
            profile_endpoint_or(&profile, mantle_base_url(profile.region.trim()).as_str());
        let auth = mantle_probe_auth(&profile, shared_config, bearer_api_key)?;
        let client = MantleClient::new(endpoint.clone(), auth, transport, signer);
        let models = client
            .list_models()
            .await
            .map_err(mantle_probe_client_error)?;

        Ok(BedrockModelDiscoveryResult {
            profile_id: profile.id,
            endpoint,
            models: models
                .into_iter()
                .map(discovered_model_from_mantle)
                .collect(),
        })
    }

    pub async fn test_runtime_connection<T, S>(
        &self,
        profile_id: &str,
        shared_config: &AwsSharedConfig,
        transport: T,
        signer: S,
    ) -> Result<BedrockRuntimeConnectionTestResult, BedrockProfileProbeError>
    where
        T: RuntimeHttpTransport,
        S: RuntimeSigV4Signer,
    {
        let profile = self.bedrock_profile(profile_id, UiLlmProvider::BedrockRuntime)?;
        let endpoint =
            profile_endpoint_or(&profile, runtime_endpoint(profile.region.trim()).as_str());
        let resolved_auth = resolve_profile_auth(&profile, shared_config)?;
        let model_id = runtime_probe_model_id(&profile);
        let request = runtime_connection_probe_request(&model_id);
        let client = BedrockRuntimeClient::new(endpoint.clone(), resolved_auth, transport, signer);
        let response = client
            .converse(request)
            .await
            .map_err(runtime_probe_client_error)?;

        Ok(BedrockRuntimeConnectionTestResult {
            profile_id: profile.id,
            endpoint,
            model_id,
            output_text: response.output_text,
            invoked_model_id: response.invoked_model_id,
            usage: response.usage,
            stop_reason: response.stop_reason,
        })
    }

    pub async fn upsert_persisted_project_folder(
        &mut self,
        persistence: &PersistenceActorHandle,
        record: ProjectFolderRecord,
    ) -> PersistenceResult<()> {
        persistence.upsert_project_folder(record.clone()).await?;
        self.project_folders.insert(record.id.clone(), record);
        Ok(())
    }

    pub fn upsert_project_folder(&mut self, record: ProjectFolderRecord) {
        self.project_folders.insert(record.id.clone(), record);
    }

    pub fn record_project_conversation(
        &mut self,
        folder_id: &str,
        conversation_id: &str,
        now_ms: i64,
    ) -> PersistenceResult<bool> {
        let Some(mut record) = self.project_folders.get(folder_id).cloned() else {
            return Ok(false);
        };
        let changed =
            update_project_recent_conversation_document(&mut record, conversation_id, now_ms)?;
        if changed {
            self.project_folders.insert(record.id.clone(), record);
        }
        Ok(changed)
    }

    pub async fn record_persisted_project_conversation(
        &mut self,
        persistence: &PersistenceActorHandle,
        folder_id: &str,
        conversation_id: &str,
        now_ms: i64,
    ) -> PersistenceResult<bool> {
        let Some(mut record) = self.project_folders.get(folder_id).cloned() else {
            return Ok(false);
        };
        let changed =
            update_project_recent_conversation_document(&mut record, conversation_id, now_ms)?;
        if changed {
            persistence.upsert_project_folder(record.clone()).await?;
            self.project_folders.insert(record.id.clone(), record);
        }
        Ok(changed)
    }

    pub async fn delete_persisted_project_folder(
        &mut self,
        persistence: &PersistenceActorHandle,
        folder_id: &str,
    ) -> PersistenceResult<bool> {
        let removed_from_memory = self.project_folders.remove(folder_id).is_some();
        let removed_from_disk = persistence.delete_project_folder(folder_id).await?;
        Ok(removed_from_memory || removed_from_disk)
    }

    pub fn create_unique_conversation(&mut self, title: &str, folder: &str) -> ConversationId {
        let conversation_id = self.unique_conversation_id(title);
        self.create_conversation(&conversation_id, title, folder)
    }

    pub fn create_unique_idle_conversation(&mut self, title: &str, folder: &str) -> ConversationId {
        let conversation_id = self.unique_conversation_id(title);
        self.create_conversation_with_status(
            &conversation_id,
            title,
            folder,
            ConversationStatus::Idle,
            0,
        )
    }

    pub fn create_conversation(
        &mut self,
        conversation_id: &str,
        title: &str,
        folder: &str,
    ) -> ConversationId {
        self.create_conversation_with_status(
            conversation_id,
            title,
            folder,
            ConversationStatus::Running,
            0,
        )
    }

    fn create_conversation_with_status(
        &mut self,
        conversation_id: &str,
        title: &str,
        folder: &str,
        status: ConversationStatus,
        now_ms: i64,
    ) -> ConversationId {
        let id = ConversationId::new(conversation_id);
        self.scheduler.add_conversation(id.clone(), title);
        self.scheduler.set_status(&id, status.clone());
        self.scheduler.record_progress(&id, folder);
        self.conversation_modes.remove(&id);
        self.conversation_metadata.insert(
            id.0.clone(),
            ConversationMetadata {
                id: id.0.clone(),
                title: title.to_owned(),
                project_folder_id: Some(folder.to_owned()),
                status: conversation_status_label(&status).to_owned(),
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            },
        );
        id
    }

    fn create_unbound_conversation_with_status(
        &mut self,
        conversation_id: &str,
        title: &str,
        status: ConversationStatus,
        now_ms: i64,
    ) -> ConversationId {
        let id = ConversationId::new(conversation_id);
        self.scheduler.add_conversation(id.clone(), title);
        self.scheduler.set_status(&id, status.clone());
        self.scheduler.record_progress(&id, user_home_dir_display());
        self.conversation_modes.remove(&id);
        self.conversation_metadata.insert(
            id.0.clone(),
            ConversationMetadata {
                id: id.0.clone(),
                title: title.to_owned(),
                project_folder_id: None,
                status: conversation_status_label(&status).to_owned(),
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            },
        );
        id
    }

    pub async fn create_persisted_conversation(
        &mut self,
        persistence: &PersistenceActorHandle,
        conversation_id: &str,
        title: &str,
        folder: &str,
        now_ms: i64,
    ) -> PersistenceResult<ConversationId> {
        let id = ConversationId::new(conversation_id);
        let status = ConversationStatus::Running;
        persistence
            .upsert_conversation(ConversationMetadata {
                id: id.0.clone(),
                title: title.to_owned(),
                project_folder_id: Some(folder.to_owned()),
                status: conversation_status_label(&status).to_owned(),
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            })
            .await?;
        persistence
            .append_conversation_event(ConversationEvent {
                conversation_id: id.0.clone(),
                event_type: "conversation.created".to_owned(),
                payload_json: serde_json::json!({
                    "title": title,
                    "project_folder_id": folder,
                    "status": conversation_status_label(&status),
                })
                .to_string(),
                created_at_ms: now_ms,
            })
            .await?;

        self.scheduler.add_conversation(id.clone(), title);
        self.scheduler.set_status(&id, status.clone());
        self.scheduler.record_progress(&id, folder);
        self.conversation_modes.remove(&id);
        self.conversation_metadata.insert(
            id.0.clone(),
            ConversationMetadata {
                id: id.0.clone(),
                title: title.to_owned(),
                project_folder_id: Some(folder.to_owned()),
                status: conversation_status_label(&status).to_owned(),
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            },
        );
        self.record_persisted_project_conversation(persistence, folder, &id.0, now_ms)
            .await?;
        Ok(id)
    }

    pub fn switch_visible_conversation(&mut self, conversation_id: &ConversationId) -> bool {
        let switched = self.scheduler.switch_visible(conversation_id);
        if switched {
            self.selected_project_folder_id = self
                .conversation_metadata
                .get(&conversation_id.0)
                .and_then(|metadata| metadata.project_folder_id.clone());
        }
        switched
    }

    pub async fn run_local_command(
        &mut self,
        mut request: RtkCommandRequest,
        mut config: RtkRunnerConfig,
    ) -> Result<&RtkCommandTranscript, RtkRunError> {
        let labels = match evaluate_workflow_command_policy(
            &self.command_policy,
            &request,
            &request.cwd,
            self.active_commands,
        ) {
            Ok(labels) => labels,
            Err(reason) => return Err(RtkRunError::PolicyDenied(reason)),
        };
        clamp_command_timeout(&mut request, self.command_policy.max_runtime_ms);
        if let Some(max_output_bytes) = self.command_policy.max_output_bytes {
            config.max_capture_bytes = config.max_capture_bytes.min(max_output_bytes);
        }
        if !self.app_preferences.rtk_enabled {
            let argv = fastrock_rtk::rtk_request_args(&request);
            let stderr = b"RTK disabled in Fastrock settings; raw local execution fallback is not enabled for this command path.\n".to_vec();
            let mut policy_labels = labels;
            policy_labels.push("rtk disabled".to_owned());
            self.local_command_transcripts.push(RtkCommandTranscript {
                cwd: request.cwd,
                argv,
                pty_requested: request.pty,
                exit_code: Some(126),
                timed_out: false,
                cancelled: false,
                stdout_bytes: 0,
                stderr_bytes: stderr.len(),
                stdout: Vec::new(),
                stderr,
                stdout_truncated: false,
                stderr_truncated: false,
                rtk_savings: None,
                policy_labels,
                events: Vec::new(),
            });
            return Ok(self
                .local_command_transcripts
                .last()
                .expect("transcript stored"));
        }
        self.active_commands += 1;
        let result = fastrock_rtk::run_local_rtk_command(request, config).await;
        self.active_commands = self.active_commands.saturating_sub(1);
        let mut transcript = result?;
        transcript.policy_labels = labels;
        self.local_command_transcripts.push(transcript);
        Ok(self
            .local_command_transcripts
            .last()
            .expect("transcript stored"))
    }

    pub async fn run_ssh_remote_command<T>(
        &mut self,
        transport: &T,
        target: SshRemoteTarget,
        mut request: RtkCommandRequest,
    ) -> Result<&RemoteCommandTranscript, RemoteCommandError>
    where
        T: SshRemoteTransport,
    {
        let labels = evaluate_workflow_command_policy(
            &self.command_policy,
            &request,
            &target.root,
            self.active_commands,
        )
        .map_err(RemoteCommandError::PolicyDenied)?;
        clamp_command_timeout(&mut request, self.command_policy.max_runtime_ms);
        if !self.app_preferences.rtk_enabled {
            return Err(RemoteCommandError::PolicyDenied(
                "RTK disabled in Fastrock settings; raw remote execution is blocked".to_owned(),
            ));
        }
        self.active_commands += 1;
        let result = run_ssh_remote_rtk_command(transport, target, request).await;
        self.active_commands = self.active_commands.saturating_sub(1);
        let mut transcript = result?;
        apply_remote_command_policy_metadata(
            &mut transcript,
            labels,
            self.command_policy.max_output_bytes,
        );
        self.remote_command_transcripts.push(transcript);
        Ok(self
            .remote_command_transcripts
            .last()
            .expect("transcript stored"))
    }

    pub fn diagnose_ssh_remote_target(&self, target: &SshRemoteTarget) -> RemoteTargetDiagnostics {
        diagnose_ssh_target_config(target)
    }

    pub async fn run_ssm_remote_command<T>(
        &mut self,
        transport: &T,
        target: SsmRemoteTarget,
        mut request: RtkCommandRequest,
    ) -> Result<&RemoteCommandTranscript, RemoteCommandError>
    where
        T: SsmRemoteTransport,
    {
        let labels = evaluate_workflow_command_policy(
            &self.command_policy,
            &request,
            &target.root,
            self.active_commands,
        )
        .map_err(RemoteCommandError::PolicyDenied)?;
        clamp_command_timeout(&mut request, self.command_policy.max_runtime_ms);
        if !self.app_preferences.rtk_enabled {
            return Err(RemoteCommandError::PolicyDenied(
                "RTK disabled in Fastrock settings; raw remote execution is blocked".to_owned(),
            ));
        }
        self.active_commands += 1;
        let result = run_ssm_remote_rtk_command(transport, target, request).await;
        self.active_commands = self.active_commands.saturating_sub(1);
        let mut transcript = result?;
        apply_remote_command_policy_metadata(
            &mut transcript,
            labels,
            self.command_policy.max_output_bytes,
        );
        self.remote_command_transcripts.push(transcript);
        Ok(self
            .remote_command_transcripts
            .last()
            .expect("transcript stored"))
    }

    pub fn diagnose_ssm_remote_target(&self, target: &SsmRemoteTarget) -> RemoteTargetDiagnostics {
        diagnose_ssm_target_config(target)
    }

    pub async fn read_ssh_remote_file<T>(
        &mut self,
        transport: &T,
        target: SshRemoteTarget,
        child: &str,
    ) -> Result<&RemoteFileOperationTranscript, RemoteCommandError>
    where
        T: SshRemoteFileTransport,
    {
        let result = project_read_ssh_remote_file(transport, target, child).await?;
        self.record_remote_file_read(result);
        Ok(self
            .remote_file_transcripts
            .last()
            .expect("transcript stored"))
    }

    pub async fn write_ssh_remote_file<T>(
        &mut self,
        transport: &T,
        target: SshRemoteTarget,
        child: &str,
        contents: Vec<u8>,
    ) -> Result<&RemoteFileOperationTranscript, RemoteCommandError>
    where
        T: SshRemoteFileTransport,
    {
        let result = project_write_ssh_remote_file(transport, target, child, contents).await?;
        self.record_remote_file_write(result);
        Ok(self
            .remote_file_transcripts
            .last()
            .expect("transcript stored"))
    }

    pub async fn read_ssm_remote_file<T>(
        &mut self,
        transport: &T,
        target: SsmRemoteTarget,
        child: &str,
    ) -> Result<&RemoteFileOperationTranscript, RemoteCommandError>
    where
        T: SsmRemoteFileTransport,
    {
        let result = project_read_ssm_remote_file(transport, target, child).await?;
        self.record_remote_file_read(result);
        Ok(self
            .remote_file_transcripts
            .last()
            .expect("transcript stored"))
    }

    pub async fn write_ssm_remote_file<T>(
        &mut self,
        transport: &T,
        target: SsmRemoteTarget,
        child: &str,
        contents: Vec<u8>,
    ) -> Result<&RemoteFileOperationTranscript, RemoteCommandError>
    where
        T: SsmRemoteFileTransport,
    {
        let result = project_write_ssm_remote_file(transport, target, child, contents).await?;
        self.record_remote_file_write(result);
        Ok(self
            .remote_file_transcripts
            .last()
            .expect("transcript stored"))
    }

    fn record_remote_file_read(&mut self, result: RemoteFileReadResult) {
        self.remote_file_transcripts
            .push(RemoteFileOperationTranscript {
                target_id: result.target_id,
                path: result.path,
                kind: RemoteFileOperationKind::Read,
                bytes: result.contents.len(),
            });
    }

    fn record_remote_file_write(&mut self, result: RemoteFileWriteResult) {
        self.remote_file_transcripts
            .push(RemoteFileOperationTranscript {
                target_id: result.target_id,
                path: result.path,
                kind: RemoteFileOperationKind::Write,
                bytes: result.bytes_written,
            });
    }

    pub fn route_plan_mode_tool(
        &self,
        conversation_id: ConversationId,
        mutation: ToolMutationClass,
    ) -> ToolRouteDecision {
        route_tool_request(&ToolRouteRequest {
            conversation_id,
            mode: AgentMode::Plan,
            tool_name: "workflow".to_owned(),
            mutation,
        })
    }

    pub fn route_plan_mode_rtk_command(
        &self,
        conversation_id: ConversationId,
        command: &[String],
    ) -> ToolRouteDecision {
        route_plan_mode_rtk_command(conversation_id, command)
    }

    pub fn create_goal(
        &mut self,
        conversation_id: ConversationId,
        objective: &str,
        token_budget: Option<u64>,
        now_ms: i64,
    ) {
        self.goals.insert(
            conversation_id,
            GoalLoopState::new_goal(objective, token_budget, now_ms),
        );
    }

    pub async fn create_persisted_goal(
        &mut self,
        persistence: &PersistenceActorHandle,
        conversation_id: ConversationId,
        objective: &str,
        token_budget: Option<u64>,
        now_ms: i64,
    ) -> PersistenceResult<()> {
        self.create_goal(conversation_id.clone(), objective, token_budget, now_ms);
        self.persist_goal(persistence, &conversation_id).await
    }

    pub fn update_goal_status(
        &mut self,
        conversation_id: &ConversationId,
        status: ThreadGoalStatus,
        now_ms: i64,
    ) -> bool {
        let Some(goal) = self.goals.get_mut(conversation_id) else {
            return false;
        };
        goal.update_status(status, now_ms);
        true
    }

    pub async fn update_persisted_goal_status(
        &mut self,
        persistence: &PersistenceActorHandle,
        conversation_id: &ConversationId,
        status: ThreadGoalStatus,
        now_ms: i64,
    ) -> PersistenceResult<bool> {
        if !self.update_goal_status(conversation_id, status, now_ms) {
            return Ok(false);
        }
        self.persist_goal(persistence, conversation_id).await?;
        Ok(true)
    }

    pub fn clear_goal(&mut self, conversation_id: &ConversationId) -> bool {
        self.goals
            .remove(conversation_id)
            .and_then(|mut goal| goal.clear())
            .is_some()
    }

    pub async fn clear_persisted_goal(
        &mut self,
        persistence: &PersistenceActorHandle,
        conversation_id: &ConversationId,
    ) -> PersistenceResult<bool> {
        let removed_from_memory = self.clear_goal(conversation_id);
        let removed_from_disk = persistence.delete_goal(conversation_id.0.clone()).await?;
        Ok(removed_from_memory || removed_from_disk)
    }

    pub fn resume_goal(&mut self, conversation_id: &ConversationId, now_ms: i64) -> bool {
        self.goals
            .get_mut(conversation_id)
            .is_some_and(|goal| goal.resume(now_ms))
    }

    pub fn mark_goal_usage_limited(
        &mut self,
        conversation_id: &ConversationId,
        now_ms: i64,
    ) -> bool {
        self.goals
            .get_mut(conversation_id)
            .is_some_and(|goal| goal.mark_usage_limited(now_ms))
    }

    pub fn record_goal_blocking_error(
        &mut self,
        conversation_id: &ConversationId,
        error_key: &str,
        now_ms: i64,
        threshold: u32,
    ) -> bool {
        self.goals
            .get_mut(conversation_id)
            .is_some_and(|goal| goal.record_blocking_error(error_key.to_owned(), now_ms, threshold))
    }

    pub fn charge_goal_turn_usage(
        &mut self,
        conversation_id: &ConversationId,
        turn_id: impl Into<String>,
        tokens: u64,
        elapsed_ms: u64,
        now_ms: i64,
    ) -> bool {
        self.goals
            .get_mut(conversation_id)
            .is_some_and(|goal| goal.charge_turn_usage(turn_id, tokens, elapsed_ms, now_ms))
    }

    pub async fn charge_persisted_goal_turn_usage(
        &mut self,
        persistence: &PersistenceActorHandle,
        conversation_id: &ConversationId,
        turn_id: impl Into<String>,
        tokens: u64,
        elapsed_ms: u64,
        now_ms: i64,
    ) -> PersistenceResult<bool> {
        if !self.charge_goal_turn_usage(conversation_id, turn_id, tokens, elapsed_ms, now_ms) {
            return Ok(false);
        }
        self.persist_goal(persistence, conversation_id).await?;
        Ok(true)
    }

    pub fn goal_status(&self, conversation_id: &ConversationId) -> Option<ThreadGoalStatus> {
        self.goals
            .get(conversation_id)
            .and_then(GoalLoopState::snapshot)
            .map(|snapshot| snapshot.status)
    }

    pub fn goal_snapshot(&self, conversation_id: &ConversationId) -> Option<ThreadGoalSnapshot> {
        self.goals
            .get(conversation_id)
            .and_then(GoalLoopState::snapshot)
    }

    pub async fn hydrate_from_persistence(
        &mut self,
        persistence: &PersistenceActorHandle,
    ) -> PersistenceResult<()> {
        let client = persistence.client();
        self.hydrate_from_persistence_client(&client).await
    }

    pub async fn hydrate_from_persistence_client(
        &mut self,
        persistence: &PersistenceActorClient,
    ) -> PersistenceResult<()> {
        let profiles = persistence
            .list_llm_profiles()
            .await?
            .into_iter()
            .map(profile_from_record)
            .collect::<PersistenceResult<Vec<_>>>()?;
        self.profiles
            .replace_profiles(profiles)
            .map_err(validation_persistence_error)?;
        self.project_folders = persistence
            .list_project_folders()
            .await?
            .into_iter()
            .map(|folder| (folder.id.clone(), folder))
            .collect();
        self.editor_tabs = EditorTabSet::default();
        self.editor_tab_records = BTreeMap::new();
        let editor_tab_records = persistence.list_editor_tabs().await?;
        let active_editor_path = editor_tab_records
            .iter()
            .find(|record| record.active)
            .map(|record| record.path.clone());
        for record in editor_tab_records {
            self.editor_tabs
                .open_tab(EditorBuffer::from_text(record.path.clone(), ""));
            self.editor_tab_records.insert(record.path.clone(), record);
        }
        if let Some(active_editor_path) = active_editor_path {
            self.editor_tabs.activate_tab(&active_editor_path);
        }

        self.conversation_metadata = BTreeMap::new();
        self.conversation_events = BTreeMap::new();
        self.conversation_modes = BTreeMap::new();
        let persisted_conversations = persistence.list_conversations().await?;
        let mut project_folder_reference_counts = BTreeMap::<String, usize>::new();
        for metadata in &persisted_conversations {
            if let Some(folder_id) = &metadata.project_folder_id {
                *project_folder_reference_counts
                    .entry(folder_id.clone())
                    .or_default() += 1;
            }
        }
        let goal_records = persistence.list_goals().await?;
        let goal_conversation_ids = goal_records
            .iter()
            .map(|record| record.conversation_id.clone())
            .collect::<BTreeSet<_>>();
        for metadata in persisted_conversations {
            let id = ConversationId::new(metadata.id.clone());
            let stored_events = persistence
                .load_conversation_events(metadata.id.clone())
                .await?;
            let project_folder = metadata
                .project_folder_id
                .as_ref()
                .and_then(|folder_id| self.project_folders.get(folder_id))
                .cloned();
            let has_editor_tab = self
                .editor_tab_records
                .values()
                .any(|record| record.conversation_id.as_deref() == Some(metadata.id.as_str()));
            if is_empty_legacy_demo_conversation(&metadata, project_folder.as_ref(), &stored_events)
                && !goal_conversation_ids.contains(&metadata.id)
                && !has_editor_tab
            {
                persistence.delete_conversation(metadata.id.clone()).await?;
                if let (Some(folder_id), Some(folder)) =
                    (metadata.project_folder_id.as_ref(), project_folder.as_ref())
                    && project_folder_reference_counts
                        .get(folder_id)
                        .copied()
                        .unwrap_or_default()
                        <= 1
                    && legacy_demo_project_folder_matches(&metadata.title, folder)
                {
                    persistence.delete_project_folder(folder_id.clone()).await?;
                    self.project_folders.remove(folder_id);
                }
                continue;
            }
            self.scheduler
                .add_conversation(id.clone(), metadata.title.clone());
            self.scheduler
                .set_status(&id, conversation_status_from_label(&metadata.status));
            if let Some(mode) = latest_conversation_mode(&stored_events)? {
                self.set_conversation_mode(&id, mode);
            }
            let transcript_events = stored_events
                .iter()
                .filter(|record| record.event_type.starts_with("transcript."))
                .map(transcript_event_from_stored_record)
                .collect::<PersistenceResult<Vec<_>>>()?;
            if !transcript_events.is_empty() {
                self.conversation_events
                    .insert(id.clone(), transcript_events);
            }
            self.conversation_metadata
                .insert(metadata.id.clone(), metadata);
        }

        for record in goal_records {
            let conversation_id = ConversationId::new(record.conversation_id.clone());
            self.goals
                .insert(conversation_id, goal_state_from_record(&record)?);
        }

        if let Some(document) = persistence
            .load_settings_document(ConfigScope::Global, MCP_CONFIG_SETTINGS_KEY)
            .await?
        {
            let config: McpConfigSet =
                serde_json::from_str(&document.document_json).map_err(json_persistence_error)?;
            self.configure_mcp(config)
                .map_err(mcp_config_persistence_error)?;
        }
        for folder_id in self.project_folders.keys().cloned().collect::<Vec<_>>() {
            if let Some(document) = persistence
                .load_settings_document(
                    ConfigScope::Project(folder_id.clone()),
                    MCP_CONFIG_SETTINGS_KEY,
                )
                .await?
            {
                let config: McpConfigSet = serde_json::from_str(&document.document_json)
                    .map_err(json_persistence_error)?;
                let config = project_scoped_mcp_config(&folder_id, config)
                    .map_err(mcp_config_persistence_error)?;
                self.replace_project_mcp_config(&folder_id, config)
                    .map_err(mcp_config_persistence_error)?;
            }
        }
        if let Some(document) = persistence
            .load_settings_document(ConfigScope::Global, COMMAND_POLICY_SETTINGS_KEY)
            .await?
        {
            self.command_policy =
                serde_json::from_str(&document.document_json).map_err(json_persistence_error)?;
        } else {
            self.command_policy = CommandPolicy::default();
        }
        if let Some(document) = persistence
            .load_settings_document(ConfigScope::Global, APP_PREFERENCES_SETTINGS_KEY)
            .await?
        {
            self.app_preferences = app_preferences_from_document(&document)?;
        } else {
            self.app_preferences = AppPreferences::default();
        }
        self.mcp_statuses = McpRuntimeSnapshotSet::default();
        for record in persistence.list_mcp_statuses().await? {
            let snapshot = mcp_runtime_snapshot_from_record(record)?;
            self.mcp_statuses.upsert(snapshot);
        }

        self.cache_ledger = CacheUsageLedger::default();
        self.context_fragment_cache = LocalContextFragmentCache::default();
        self.tool_output_cache = ToolOutputDedupCache::default();
        for record in persistence.list_cache_metadata().await? {
            if record.cache_key.starts_with(CACHE_USAGE_KEY_PREFIX) {
                let snapshot: CacheUsageSnapshot =
                    serde_json::from_str(&record.document_json).map_err(json_persistence_error)?;
                if record.provider_plane != snapshot.provider_plane.label() {
                    return Err(cache_metadata_persistence_error(format!(
                        "cache provider mismatch for {}: record={} snapshot={}",
                        record.cache_key,
                        record.provider_plane,
                        snapshot.provider_plane.label()
                    )));
                }
                self.cache_ledger.replace_snapshot(snapshot);
            } else if record
                .cache_key
                .starts_with(CONTEXT_FRAGMENT_CACHE_KEY_PREFIX)
            {
                if record.provider_plane != LOCAL_CONTEXT_PROVIDER_PLANE {
                    return Err(cache_metadata_persistence_error(format!(
                        "context cache provider mismatch for {}: {}",
                        record.cache_key, record.provider_plane
                    )));
                }
                let fragment: ContextFragmentRecord =
                    serde_json::from_str(&record.document_json).map_err(json_persistence_error)?;
                if record.cache_key != context_fragment_cache_key(&fragment) {
                    return Err(cache_metadata_persistence_error(format!(
                        "context cache key mismatch for {}",
                        record.cache_key
                    )));
                }
                self.context_fragment_cache.restore_record(fragment);
            } else if record.cache_key.starts_with(TOOL_OUTPUT_CACHE_KEY_PREFIX) {
                if record.provider_plane != TOOL_OUTPUT_PROVIDER_PLANE {
                    return Err(cache_metadata_persistence_error(format!(
                        "tool output cache provider mismatch for {}: {}",
                        record.cache_key, record.provider_plane
                    )));
                }
                let output: ToolOutputRecord =
                    serde_json::from_str(&record.document_json).map_err(json_persistence_error)?;
                if record.cache_key != tool_output_cache_key(&output) {
                    return Err(cache_metadata_persistence_error(format!(
                        "tool output cache key mismatch for {}",
                        record.cache_key
                    )));
                }
                self.tool_output_cache.restore_record(output);
            }
        }

        self.memory_store = MemoryStore::default();
        for record in persistence.list_memory_records().await? {
            self.memory_store.upsert(record);
        }

        Ok(())
    }

    async fn persist_profile(
        &self,
        persistence: &PersistenceActorHandle,
        profile_id: &str,
        now_ms: i64,
    ) -> PersistenceResult<()> {
        let Some(profile) = self.profiles.profile(profile_id) else {
            return Ok(());
        };
        persistence
            .upsert_llm_profile(LlmProfileRecord {
                id: profile.id.clone(),
                provider: profile_provider_label(profile.provider).to_owned(),
                enabled: profile.enabled,
                document_json: serde_json::to_string(profile).map_err(json_persistence_error)?,
                updated_at_ms: now_ms,
            })
            .await
    }

    async fn persist_goal(
        &self,
        persistence: &PersistenceActorHandle,
        conversation_id: &ConversationId,
    ) -> PersistenceResult<()> {
        let Some(goal) = self
            .goals
            .get(conversation_id)
            .and_then(GoalLoopState::goal)
        else {
            return Ok(());
        };
        persistence
            .upsert_goal(GoalRecord {
                conversation_id: conversation_id.0.clone(),
                status: goal_status_label(&goal.status).to_owned(),
                document_json: serde_json::json!({
                    "objective": &goal.objective,
                    "status": goal_status_label(&goal.status),
                    "token_budget": goal.token_budget,
                    "tokens_used": goal.tokens_used,
                    "elapsed_ms": goal.elapsed_ms,
                    "created_at_ms": goal.created_at_ms,
                    "updated_at_ms": goal.updated_at_ms,
                    "charged_turn_ids": self
                        .goals
                        .get(conversation_id)
                        .map(|goal| goal.charged_turn_ids().collect::<Vec<_>>())
                        .unwrap_or_default(),
                })
                .to_string(),
                updated_at_ms: goal.updated_at_ms,
            })
            .await
    }

    pub fn configure_mcp(&mut self, config: McpConfigSet) -> Result<(), McpConfigError> {
        config.validate()?;
        self.mcp_config = config;
        Ok(())
    }

    pub async fn configure_persisted_mcp(
        &mut self,
        persistence: &PersistenceActorHandle,
        config: McpConfigSet,
        now_ms: i64,
    ) -> PersistenceResult<Result<(), McpConfigError>> {
        match self.configure_mcp(config) {
            Ok(()) => {
                self.persist_mcp_config(persistence, now_ms).await?;
                Ok(Ok(()))
            }
            Err(error) => Ok(Err(error)),
        }
    }

    pub fn add_mcp_server(
        &mut self,
        server: McpServerConfig,
    ) -> Result<Vec<McpConfigChange>, McpConfigError> {
        self.apply_mcp_edit(|config| config.add_server(server))
    }

    pub async fn add_persisted_mcp_server(
        &mut self,
        persistence: &PersistenceActorHandle,
        server: McpServerConfig,
        now_ms: i64,
    ) -> PersistenceResult<Result<Vec<McpConfigChange>, McpConfigError>> {
        let result = self.add_mcp_server(server);
        self.persist_mcp_edit_result(persistence, now_ms, result)
            .await
    }

    pub fn update_mcp_server(
        &mut self,
        server: McpServerConfig,
    ) -> Result<Vec<McpConfigChange>, McpConfigError> {
        self.apply_mcp_edit(|config| config.update_server(server))
    }

    pub async fn update_persisted_mcp_server(
        &mut self,
        persistence: &PersistenceActorHandle,
        server: McpServerConfig,
        now_ms: i64,
    ) -> PersistenceResult<Result<Vec<McpConfigChange>, McpConfigError>> {
        let result = self.update_mcp_server(server);
        self.persist_mcp_edit_result(persistence, now_ms, result)
            .await
    }

    pub fn delete_mcp_server(&mut self, id: &str) -> Result<Vec<McpConfigChange>, McpConfigError> {
        self.apply_mcp_edit(|config| config.remove_server(id).map(|_| ()))
    }

    pub async fn delete_persisted_mcp_server(
        &mut self,
        persistence: &PersistenceActorHandle,
        id: &str,
        now_ms: i64,
    ) -> PersistenceResult<Result<Vec<McpConfigChange>, McpConfigError>> {
        let result = self.delete_mcp_server(id);
        if result.is_ok() {
            self.mcp_statuses.remove(id);
            persistence.delete_mcp_status(id).await?;
        }
        self.persist_mcp_edit_result(persistence, now_ms, result)
            .await
    }

    pub fn set_mcp_server_enabled(
        &mut self,
        id: &str,
        enabled: bool,
    ) -> Result<Vec<McpConfigChange>, McpConfigError> {
        self.apply_mcp_edit(|config| config.set_server_enabled(id, enabled))
    }

    pub async fn set_persisted_mcp_server_enabled(
        &mut self,
        persistence: &PersistenceActorHandle,
        id: &str,
        enabled: bool,
        now_ms: i64,
    ) -> PersistenceResult<Result<Vec<McpConfigChange>, McpConfigError>> {
        let result = self.set_mcp_server_enabled(id, enabled);
        self.persist_mcp_edit_result(persistence, now_ms, result)
            .await
    }

    pub fn set_mcp_tool_always_allowed(
        &mut self,
        id: &str,
        tool_name: &str,
        always_allowed: bool,
    ) -> Result<Vec<McpConfigChange>, McpConfigError> {
        self.apply_mcp_edit(|config| config.set_tool_always_allowed(id, tool_name, always_allowed))
    }

    pub async fn set_persisted_mcp_tool_always_allowed(
        &mut self,
        persistence: &PersistenceActorHandle,
        id: &str,
        tool_name: &str,
        always_allowed: bool,
        now_ms: i64,
    ) -> PersistenceResult<Result<Vec<McpConfigChange>, McpConfigError>> {
        let result = self.set_mcp_tool_always_allowed(id, tool_name, always_allowed);
        self.persist_mcp_edit_result(persistence, now_ms, result)
            .await
    }

    pub fn set_mcp_tool_disabled(
        &mut self,
        id: &str,
        tool_name: &str,
        disabled: bool,
    ) -> Result<Vec<McpConfigChange>, McpConfigError> {
        self.apply_mcp_edit(|config| config.set_tool_disabled(id, tool_name, disabled))
    }

    pub async fn set_persisted_mcp_tool_disabled(
        &mut self,
        persistence: &PersistenceActorHandle,
        id: &str,
        tool_name: &str,
        disabled: bool,
        now_ms: i64,
    ) -> PersistenceResult<Result<Vec<McpConfigChange>, McpConfigError>> {
        let result = self.set_mcp_tool_disabled(id, tool_name, disabled);
        self.persist_mcp_edit_result(persistence, now_ms, result)
            .await
    }

    pub fn record_mcp_status(&mut self, snapshot: McpRuntimeSnapshot) {
        self.mcp_statuses.upsert(snapshot);
    }

    pub async fn record_persisted_mcp_status(
        &mut self,
        persistence: &PersistenceActorHandle,
        snapshot: McpRuntimeSnapshot,
    ) -> PersistenceResult<()> {
        persistence
            .upsert_mcp_status(mcp_status_snapshot_record(&snapshot)?)
            .await?;
        self.record_mcp_status(snapshot);
        Ok(())
    }

    pub async fn store_persisted_mcp_oauth_tokens(
        &mut self,
        persistence: &PersistenceActorHandle,
        server_id: &str,
        tokens: McpOAuthTokenSet,
        now_ms: i64,
    ) -> PersistenceResult<Option<McpRuntimeSnapshot>> {
        let Some(server) = self.mcp_config.server(server_id).cloned() else {
            return Ok(None);
        };
        let token_ref = mcp_oauth_token_secret_ref(server_id);
        persistence
            .store_secret(
                SecretRefRecord {
                    id: SecretRef(token_ref.clone()),
                    service: MCP_OAUTH_TOKEN_SECRET_SERVICE.to_owned(),
                    account: server_id.to_owned(),
                    created_at_ms: now_ms,
                },
                serde_json::to_string(&tokens).map_err(json_persistence_error)?,
            )
            .await?;

        let mut snapshot = self
            .mcp_status(server_id)
            .cloned()
            .unwrap_or_else(|| McpRuntimeSnapshot::stopped(&server, now_ms));
        snapshot.oauth_state = McpOAuthState::Authorized {
            token_ref: token_ref.clone(),
        };
        snapshot.updated_at_ms = now_ms;
        self.record_persisted_mcp_status(persistence, snapshot.clone())
            .await?;
        Ok(Some(snapshot))
    }

    pub async fn load_persisted_mcp_oauth_tokens(
        &self,
        persistence: &PersistenceActorHandle,
        server_id: &str,
    ) -> PersistenceResult<Option<McpOAuthTokenSet>> {
        let Some(secret) = persistence
            .get_secret(SecretRef(mcp_oauth_token_secret_ref(server_id)))
            .await?
        else {
            return Ok(None);
        };
        serde_json::from_str(&secret)
            .map(Some)
            .map_err(json_persistence_error)
    }

    pub fn discover_skills(
        &mut self,
        root: impl AsRef<Path>,
        scope: SkillScope,
    ) -> Result<(), SkillError> {
        self.skills = discover_skills(root, scope)?;
        Ok(())
    }

    pub fn handle_local_project_file_event(
        &mut self,
        folder_id: &str,
        project_root: impl AsRef<Path>,
        event: ProjectFileEvent,
        now_ms: i64,
    ) -> Result<GuiActionResult, ProjectHotReloadError> {
        if !self.project_folders.contains_key(folder_id) {
            return Err(ProjectHotReloadError::UnknownProjectFolder(
                folder_id.to_owned(),
            ));
        }
        let event_path = project_event_path(&event)?.to_owned();
        let project_root = project_root.as_ref();
        if is_project_mcp_event_path(&event_path) {
            return self.reload_project_mcp_from_event(folder_id, project_root, event, now_ms);
        }
        if is_project_skill_event_path(&event_path) {
            return self.reload_project_skills_from_event(folder_id, project_root);
        }
        Ok(GuiActionResult::status(format!(
            "Project event ignored: {}",
            event_path.display()
        )))
    }

    fn reload_project_mcp_from_event(
        &mut self,
        folder_id: &str,
        project_root: &Path,
        event: ProjectFileEvent,
        now_ms: i64,
    ) -> Result<GuiActionResult, ProjectHotReloadError> {
        let event_path = project_event_path(&event)?;
        let project_config = if matches!(event, ProjectFileEvent::Deleted { .. }) {
            McpConfigSet::default()
        } else {
            let path = project_root.join(event_path);
            let contents = std::fs::read_to_string(&path)
                .map_err(|error| ProjectHotReloadError::Io(error.to_string()))?;
            let parsed: McpConfigSet = serde_json::from_str(&contents)
                .map_err(|error| ProjectHotReloadError::Json(error.to_string()))?;
            project_scoped_mcp_config(folder_id, parsed)?
        };
        let changes = self.replace_project_mcp_config(folder_id, project_config.clone())?;
        let document_json = serde_json::to_string(&project_config)
            .map_err(|error| ProjectHotReloadError::Json(error.to_string()))?;
        Ok(GuiActionResult::with_writes(
            format!("Project MCP hot reload: {}", mcp_changes_display(&changes)),
            vec![GuiPersistenceWrite::StoreSettingsDocument(
                SettingsDocument {
                    scope: ConfigScope::Project(folder_id.to_owned()),
                    key: MCP_CONFIG_SETTINGS_KEY.to_owned(),
                    document_json,
                    updated_at_ms: now_ms,
                },
            )],
        ))
    }

    fn reload_project_skills_from_event(
        &mut self,
        folder_id: &str,
        project_root: &Path,
    ) -> Result<GuiActionResult, ProjectHotReloadError> {
        let scope = SkillScope::Project(folder_id.to_owned());
        let skills_root = project_root.join(PROJECT_SKILLS_RELATIVE_PATH);
        let project_skills = if skills_root.exists() {
            discover_skills(&skills_root, scope.clone())?
        } else {
            Vec::new()
        };
        let count = project_skills.len();
        self.replace_project_skill_documents(folder_id, project_skills);
        Ok(GuiActionResult::status(format!(
            "Project skills hot reload: {count} skills"
        )))
    }

    fn replace_project_mcp_config(
        &mut self,
        folder_id: &str,
        project_config: McpConfigSet,
    ) -> Result<Vec<McpConfigChange>, McpConfigError> {
        let before = self.mcp_config.clone();
        let mut next = before.clone();
        next.servers.retain(|server| {
            !matches!(
                &server.scope,
                McpConfigScope::Project { project_folder_id } if project_folder_id == folder_id
            )
        });
        next.servers.extend(project_config.servers);
        next.servers.sort_by(|left, right| left.id.cmp(&right.id));
        let changes = before.diff_for_hot_reload(&next)?;
        self.mcp_config = next;
        Ok(changes)
    }

    fn replace_project_skill_documents(
        &mut self,
        folder_id: &str,
        project_skills: Vec<SkillDocument>,
    ) {
        self.skills.retain(|skill| {
            !matches!(
                &skill.manifest.scope,
                SkillScope::Project(project_id) if project_id == folder_id
            )
        });
        for document in project_skills {
            self.upsert_skill_document(document);
        }
    }

    pub fn create_skill(
        &mut self,
        skills_root: impl AsRef<Path>,
        scope: SkillScope,
        request: SkillWriteRequest,
    ) -> Result<SkillDocument, SkillError> {
        let document = create_skill_file(skills_root, scope, request)?;
        self.upsert_skill_document(document.clone());
        Ok(document)
    }

    pub fn edit_skill(
        &mut self,
        skills_root: impl AsRef<Path>,
        scope: SkillScope,
        request: SkillWriteRequest,
    ) -> Result<SkillDocument, SkillError> {
        let document = edit_skill_file(skills_root, scope, request)?;
        self.upsert_skill_document(document.clone());
        Ok(document)
    }

    pub fn delete_skill(
        &mut self,
        skills_root: impl AsRef<Path>,
        scope: &SkillScope,
        name: &str,
    ) -> Result<bool, SkillError> {
        let removed_from_disk = delete_skill_file(skills_root, name)?;
        let before = self.skills.len();
        self.skills
            .retain(|skill| !(skill.manifest.name == name && &skill.manifest.scope == scope));
        Ok(removed_from_disk || self.skills.len() != before)
    }

    pub fn move_skill(
        &mut self,
        source_root: impl AsRef<Path>,
        source_scope: &SkillScope,
        destination_root: impl AsRef<Path>,
        destination_scope: SkillScope,
        name: &str,
    ) -> Result<SkillDocument, SkillError> {
        let document = move_skill_file(source_root, destination_root, destination_scope, name)?;
        self.skills.retain(|skill| {
            !(skill.manifest.name == name && &skill.manifest.scope == source_scope)
        });
        self.upsert_skill_document(document.clone());
        Ok(document)
    }

    pub fn create_gui_skill(&mut self, now_ms: i64) -> SkillDocument {
        let name = self.next_gui_skill_name();
        let document = gui_skill_document(
            &name,
            "Fastrock GUI skill",
            "Use rtk for shell commands and keep workflow terse.\n",
            SkillScope::Global,
            true,
        );
        self.upsert_skill_document(document.clone());
        self.open_editor_tab(EditorBuffer::from_text(
            document.manifest.path.clone(),
            &skill_document_source(&document),
        ));
        if let Some(record) = self.editor_tab_records.get_mut(&document.manifest.path) {
            record.updated_at_ms = now_ms;
        }
        document
    }

    pub fn edit_gui_skill(&mut self, name: &str, now_ms: i64) -> Option<SkillDocument> {
        let index = self
            .skills
            .iter()
            .position(|skill| skill.manifest.name == name)?;
        let mut document = self.skills[index].clone();
        document.manifest.description = format!("{} Edited", document.manifest.description);
        document.body = format!("{}Updated from Fastrock GUI at {now_ms}.\n", document.body);
        self.upsert_skill_document(document.clone());
        self.open_editor_tab(EditorBuffer::from_text(
            document.manifest.path.clone(),
            &skill_document_source(&document),
        ));
        Some(document)
    }

    pub fn toggle_gui_skill_mode_restriction(
        &mut self,
        name: &str,
        now_ms: i64,
    ) -> Option<SkillDocument> {
        let index = self
            .skills
            .iter()
            .position(|skill| skill.manifest.name == name)?;
        let mut document = self.skills[index].clone();
        if document.manifest.mode_slugs.is_empty() {
            document.manifest.mode_slugs = vec!["code".to_owned()];
        } else {
            document.manifest.mode_slugs.clear();
        }
        self.upsert_skill_document(document.clone());
        self.open_editor_tab(EditorBuffer::from_text(
            document.manifest.path.clone(),
            &skill_document_source(&document),
        ));
        if let Some(record) = self.editor_tab_records.get_mut(&document.manifest.path) {
            record.updated_at_ms = now_ms;
        }
        Some(document)
    }

    pub fn toggle_gui_skill_scope(&mut self, name: &str, now_ms: i64) -> Option<SkillDocument> {
        let index = self
            .skills
            .iter()
            .position(|skill| skill.manifest.name == name)?;
        let mut document = self.skills[index].clone();
        let source_scope = document.manifest.scope.clone();
        document.manifest.scope = match &source_scope {
            SkillScope::Global => {
                SkillScope::Project(self.active_project_folder_id_for_gui_scope())
            }
            SkillScope::Project(_) => SkillScope::Global,
        };
        self.skills
            .retain(|skill| !(skill.manifest.name == name && skill.manifest.scope == source_scope));
        self.upsert_skill_document(document.clone());
        self.open_editor_tab(EditorBuffer::from_text(
            document.manifest.path.clone(),
            &skill_document_source(&document),
        ));
        if let Some(record) = self.editor_tab_records.get_mut(&document.manifest.path) {
            record.updated_at_ms = now_ms;
        }
        Some(document)
    }

    pub fn delete_gui_skill(&mut self, name: &str) -> Option<SkillDocument> {
        let index = self
            .skills
            .iter()
            .position(|skill| skill.manifest.name == name)?;
        Some(self.skills.remove(index))
    }

    fn next_gui_skill_name(&self) -> String {
        (1..)
            .map(|index| format!("fastrock-skill-{index}"))
            .find(|candidate| {
                self.skills
                    .iter()
                    .all(|skill| skill.manifest.name != candidate.as_str())
            })
            .expect("unbounded iterator returns a skill name")
    }

    fn active_project_folder_id_for_gui_scope(&self) -> String {
        self.scheduler()
            .visible_conversation_id()
            .and_then(|id| self.conversation_metadata.get(&id.0))
            .and_then(|metadata| metadata.project_folder_id.clone())
            .or_else(|| self.project_folders.keys().next().cloned())
            .unwrap_or_else(|| "folder-local".to_owned())
    }

    pub fn open_editor_tab(&mut self, buffer: EditorBuffer) {
        self.editor_tabs.open_tab(buffer);
    }

    pub fn select_editor_tab(&mut self, path: &str) -> bool {
        self.editor_tabs.activate_tab(path)
    }

    pub fn close_editor_tab(&mut self, path: &str) -> bool {
        let removed_from_tabs = self.editor_tabs.close_tab(path);
        let removed_from_records = self.editor_tab_records.remove(path).is_some();
        removed_from_tabs || removed_from_records
    }

    pub fn append_to_active_editor(
        &mut self,
        text: &str,
    ) -> Option<Result<(String, bool), EditorError>> {
        let buffer = self.editor_tabs.active_mut()?;
        let path = buffer.path.clone();
        let insert = if buffer.text().ends_with('\n') {
            text.to_owned()
        } else {
            format!("\n{text}")
        };
        Some(
            buffer
                .replace_range(buffer.char_count(), buffer.char_count(), &insert)
                .map(|()| (path, buffer.dirty)),
        )
    }

    pub fn save_active_editor_request(&mut self) -> Option<(String, String)> {
        let buffer = self.editor_tabs.active_mut()?;
        let path = buffer.path.clone();
        let contents = buffer.text();
        buffer.mark_clean_after_external_save();
        self.saved_editor_paths.push(path.clone());
        Some((path, contents))
    }

    pub fn undo_active_editor(&mut self) -> Option<Result<(String, bool), EditorError>> {
        let buffer = self.editor_tabs.active_mut()?;
        let path = buffer.path.clone();
        Some(buffer.undo().map(|_| (path, buffer.dirty)))
    }

    pub fn redo_active_editor(&mut self) -> Option<Result<(String, bool), EditorError>> {
        let buffer = self.editor_tabs.active_mut()?;
        let path = buffer.path.clone();
        Some(buffer.redo().map(|_| (path, buffer.dirty)))
    }

    pub fn revert_active_editor(&mut self) -> Option<Result<(String, bool), EditorError>> {
        let buffer = self.editor_tabs.active_mut()?;
        let path = buffer.path.clone();
        Some(buffer.revert_to_last_clean().map(|()| (path, buffer.dirty)))
    }

    pub fn find_in_active_editor(&self, needle: &str) -> Option<(String, usize)> {
        let buffer = self.editor_tabs.active()?;
        Some((buffer.path.clone(), buffer.find_all(needle).len()))
    }

    pub fn diff_active_editor(&self) -> Option<(String, usize)> {
        let buffer = self.editor_tabs.active()?;
        Some((buffer.path.clone(), buffer.diff_against_clean().len()))
    }

    pub fn active_editor_goto_line(&self, line_number: usize) -> Option<(String, usize)> {
        let buffer = self.editor_tabs.active()?;
        let cursor = buffer.go_to_line(line_number)?;
        Some((buffer.path.clone(), cursor.line_number))
    }

    pub async fn open_persisted_editor_tab(
        &mut self,
        persistence: &PersistenceActorHandle,
        buffer: EditorBuffer,
        conversation_id: Option<ConversationId>,
        now_ms: i64,
    ) -> PersistenceResult<()> {
        let path = buffer.path.clone();
        self.editor_tabs.open_tab(buffer);

        let prior_paths = self.editor_tab_records.keys().cloned().collect::<Vec<_>>();
        for prior_path in prior_paths {
            if let Some(record) = self.editor_tab_records.get_mut(&prior_path) {
                record.active = prior_path == path;
                record.updated_at_ms = now_ms;
                persistence.upsert_editor_tab(record.clone()).await?;
            }
        }

        let record = EditorTabRecord {
            path: path.clone(),
            conversation_id: conversation_id.map(|id| id.0),
            active: true,
            document_json: serde_json::json!({
                "path": path,
                "dirty": false,
                "loaded": false,
            })
            .to_string(),
            updated_at_ms: now_ms,
        };
        persistence.upsert_editor_tab(record.clone()).await?;
        self.editor_tab_records.insert(record.path.clone(), record);
        Ok(())
    }

    pub async fn close_persisted_editor_tab(
        &mut self,
        persistence: &PersistenceActorHandle,
        path: &str,
    ) -> PersistenceResult<bool> {
        let removed_from_memory = self.editor_tabs.close_tab(path);
        let removed_record = self.editor_tab_records.remove(path).is_some();
        let removed_from_disk = persistence.delete_editor_tab(path).await?;
        Ok(removed_from_memory || removed_record || removed_from_disk)
    }

    pub fn open_edit_save_file(
        &mut self,
        path: impl AsRef<Path>,
        replacement_text: &str,
    ) -> Result<(), EditorError> {
        let path = path.as_ref();
        let mut buffer = EditorBuffer::open(path)?;
        let end = buffer.char_count();
        buffer.replace_range(0, end, replacement_text)?;
        buffer.save()?;
        self.saved_editor_paths
            .push(path.to_string_lossy().into_owned());
        self.editor_tabs.open_tab(buffer);
        Ok(())
    }

    pub fn record_cache_usage(&mut self, record: CacheUsageRecord) {
        self.cache_ledger.record(record);
    }

    pub async fn record_persisted_cache_usage(
        &mut self,
        persistence: &PersistenceActorHandle,
        record: CacheUsageRecord,
        now_ms: i64,
    ) -> PersistenceResult<()> {
        let provider_plane = record.provider_plane;
        self.record_cache_usage(record);
        self.persist_cache_usage(persistence, provider_plane, now_ms)
            .await
    }

    pub fn cache_usage(&self, provider_plane: CacheProviderPlane) -> CacheUsage {
        self.cache_ledger.usage(provider_plane)
    }

    pub fn cache_prompt_tokens(&self, provider_plane: CacheProviderPlane) -> u64 {
        self.cache_ledger.usage(provider_plane).prompt_tokens
    }

    pub fn remember_context_fragment(
        &mut self,
        input: ContextFragmentInput,
        now_ms: i64,
    ) -> ContextFragmentCacheResult {
        self.context_fragment_cache.remember_fragment(input, now_ms)
    }

    pub async fn remember_persisted_context_fragment(
        &mut self,
        persistence: &PersistenceActorHandle,
        input: ContextFragmentInput,
        now_ms: i64,
    ) -> PersistenceResult<ContextFragmentCacheResult> {
        let result = self.remember_context_fragment(input, now_ms);
        persistence
            .upsert_cache_metadata(context_fragment_cache_metadata_record(&result.record)?)
            .await?;
        Ok(result)
    }

    pub fn remember_tool_output(
        &mut self,
        conversation_id: impl Into<String>,
        kind: ToolOutputKind,
        bytes: &[u8],
        now_ms: i64,
    ) -> ToolOutputDedupResult {
        self.tool_output_cache
            .remember_output(conversation_id, kind, bytes, now_ms)
    }

    pub async fn remember_persisted_tool_output(
        &mut self,
        persistence: &PersistenceActorHandle,
        conversation_id: impl Into<String>,
        kind: ToolOutputKind,
        bytes: &[u8],
        now_ms: i64,
    ) -> PersistenceResult<ToolOutputDedupResult> {
        let result = self.remember_tool_output(conversation_id, kind, bytes, now_ms);
        if let Some(record) = &result.record {
            persistence
                .upsert_cache_metadata(tool_output_cache_metadata_record(record)?)
                .await?;
        }
        Ok(result)
    }

    pub fn context_fragment_records(&self) -> Vec<ContextFragmentRecord> {
        self.context_fragment_cache.records()
    }

    pub fn tool_output_records(&self) -> Vec<ToolOutputRecord> {
        self.tool_output_cache.records()
    }

    pub fn release_evidence(&self) -> ReleaseEvidence {
        ReleaseEvidence {
            mantle_profile: self
                .profiles
                .profiles()
                .iter()
                .any(|profile| profile.provider == UiLlmProvider::BedrockMantle && profile.enabled),
            runtime_profile: self.profiles.profiles().iter().any(|profile| {
                profile.provider == UiLlmProvider::BedrockRuntime && profile.enabled
            }),
            parallel_conversations: self.scheduler.conversation_count() >= 2,
            local_rtk_command: self.local_command_transcripts.iter().any(|transcript| {
                transcript
                    .argv
                    .first()
                    .is_some_and(|arg| arg.contains("rtk"))
            }),
            remote_rtk_commands: self.remote_command_transcripts.len() >= 2,
            remote_file_operations: self.remote_file_transcripts.len() >= 4,
            configured_mcp: !self.mcp_config.servers.is_empty(),
            configured_skills: !self.skills.is_empty(),
            memory_present: !self.memory_store.records().is_empty(),
            open_editor_tab: !self.editor_tabs.is_empty(),
            saved_editor_file: !self.saved_editor_paths.is_empty(),
            goal_present: !self.goals.is_empty(),
        }
    }
}

impl FastrockWorkflow {
    fn bedrock_profile(
        &self,
        profile_id: &str,
        expected: UiLlmProvider,
    ) -> Result<LlmProfileForm, BedrockProfileProbeError> {
        let profile = self
            .profiles
            .profile(profile_id)
            .cloned()
            .ok_or_else(|| BedrockProfileProbeError::ProfileNotFound(profile_id.to_owned()))?;
        if profile.provider != expected {
            return Err(BedrockProfileProbeError::WrongProvider {
                profile_id: profile.id,
                expected,
                actual: profile.provider,
            });
        }
        Ok(profile)
    }

    fn selected_or_default_profile_id(&self) -> Option<String> {
        self.profiles
            .selected_profile_id()
            .map(str::to_owned)
            .or_else(|| {
                self.profiles
                    .profiles()
                    .iter()
                    .find(|profile| profile.default_for_new_conversations)
                    .map(|profile| profile.id.clone())
            })
            .or_else(|| {
                self.profiles
                    .profiles()
                    .first()
                    .map(|profile| profile.id.clone())
            })
    }

    fn generated_profile_id(&self) -> String {
        (0..)
            .map(|_| generated_persisted_id())
            .find(|candidate| {
                self.profiles.profile(candidate).is_none()
                    && self
                        .profiles
                        .draft()
                        .is_none_or(|draft| draft.id != candidate.as_str())
            })
            .expect("generated IDs do not exhaust")
    }

    fn unique_conversation_id(&self, _title: &str) -> String {
        (0..)
            .map(|_| generated_persisted_id())
            .find(|candidate| self.conversation_id_is_available(candidate))
            .expect("unbounded iterator returns a conversation id")
    }

    fn conversation_id_is_available(&self, conversation_id: &str) -> bool {
        let id = ConversationId::new(conversation_id);
        self.scheduler.conversation(&id).is_none()
            && !self.conversation_metadata.contains_key(conversation_id)
    }

    fn shell_profile(&self, active_row: Option<&ConversationListRow>) -> Option<&LlmProfileForm> {
        self.profiles
            .selected_profile_id()
            .and_then(|profile_id| self.profiles.profile(profile_id))
            .or_else(|| {
                active_row.and_then(|row| {
                    self.profiles
                        .profiles()
                        .iter()
                        .find(|profile| profile.name == row.profile_name)
                })
            })
            .or_else(|| {
                self.profiles
                    .profiles()
                    .iter()
                    .find(|profile| profile.default_for_new_conversations)
            })
            .or_else(|| self.profiles.profiles().first())
    }

    fn shell_editor_lines(&self) -> Vec<WorkflowEditorLineRow> {
        self.editor_tabs
            .active()
            .map(|buffer| {
                buffer
                    .visible_viewport(0, SHELL_EDITOR_VISIBLE_LINE_COUNT)
                    .lines
                    .into_iter()
                    .map(|line| WorkflowEditorLineRow {
                        line_number: line.line_index + 1,
                        text: line.text.trim_end_matches(['\r', '\n']).to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn shell_diff_rows(&self) -> Vec<WorkflowDiffRow> {
        self.editor_tabs
            .active()
            .map(|buffer| {
                buffer
                    .diff_against_clean()
                    .into_iter()
                    .take(SHELL_DIFF_VISIBLE_LINE_COUNT)
                    .map(|diff| WorkflowDiffRow {
                        line_number: diff.line_index + 1,
                        before: trim_editor_line_for_shell(diff.left),
                        after: trim_editor_line_for_shell(diff.right),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn shell_project_folder_rows(&self) -> Vec<WorkflowProjectFolderRow> {
        self.project_folders
            .values()
            .map(|folder| {
                let default_profile = project_default_profile_id(folder)
                    .ok()
                    .flatten()
                    .and_then(|profile_id| {
                        self.profiles
                            .profile(&profile_id)
                            .map(|profile| profile.name.clone())
                            .or(Some(profile_id))
                    })
                    .unwrap_or_else(|| "No default profile".to_owned());
                let recent_conversations = project_recent_conversation_ids(folder)
                    .map(|recent| recent.len())
                    .unwrap_or_default();

                WorkflowProjectFolderRow {
                    label: folder.label.clone(),
                    path: folder.path.clone(),
                    target: project_folder_target_display(folder),
                    default_profile,
                    recent_conversations,
                }
            })
            .collect()
    }

    fn shell_sidebar_tree_rows(
        &self,
        sidebar: &ConversationSidebarModel,
    ) -> Vec<WorkflowSidebarTreeRow> {
        let mut rows = Vec::new();
        let mut conversations_by_folder =
            BTreeMap::<Option<String>, Vec<&ConversationListRow>>::new();
        for row in sidebar.rows() {
            conversations_by_folder
                .entry(row.project_folder_id.clone())
                .or_default()
                .push(row);
        }
        for conversations in conversations_by_folder.values_mut() {
            conversations.sort_by(|left, right| {
                right
                    .last_updated_ms
                    .cmp(&left.last_updated_ms)
                    .then_with(|| left.title.cmp(&right.title))
            });
        }

        if let Some(unbound_rows) = conversations_by_folder.remove(&None)
            && !unbound_rows.is_empty()
        {
            let expanded = !self.app_preferences.unbound_collapsed;
            rows.push(WorkflowSidebarTreeRow {
                row_id: "unbound".to_owned(),
                kind: "unbound".to_owned(),
                project_folder_id: String::new(),
                conversation_id: String::new(),
                title: "Unbound conversations".to_owned(),
                subtitle: format!("Home · {}", user_home_dir_display()),
                status: format!("{} chats", unbound_rows.len()),
                depth: 0,
                expanded,
                selected: self.selected_project_folder_id.is_none()
                    && sidebar.rows().iter().all(|row| !row.selected),
                tooltip: "Conversations not bound to a project folder. Commands default to your home directory.".to_owned(),
                action: "sidebar-toggle-unbound".to_owned(),
            });
            if expanded {
                rows.extend(
                    unbound_rows
                        .into_iter()
                        .map(|row| self.shell_sidebar_conversation_row(row, 1)),
                );
            }
        }

        let mut project_folders = self.project_folders.values().collect::<Vec<_>>();
        project_folders.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.path.cmp(&right.path))
                .then_with(|| left.id.cmp(&right.id))
        });
        for folder in project_folders {
            let folder_key = Some(folder.id.clone());
            let folder_conversations = conversations_by_folder
                .remove(&folder_key)
                .unwrap_or_default();
            let expanded = !self
                .app_preferences
                .collapsed_project_folder_ids
                .contains(&folder.id);
            let selected = self.selected_project_folder_id.as_deref() == Some(folder.id.as_str())
                && !folder_conversations.iter().any(|row| row.selected);
            rows.push(WorkflowSidebarTreeRow {
                row_id: format!("folder:{}", folder.id),
                kind: "folder".to_owned(),
                project_folder_id: folder.id.clone(),
                conversation_id: String::new(),
                title: folder.label.clone(),
                subtitle: format!(
                    "{} · {}",
                    folder.path,
                    project_folder_target_display(folder)
                ),
                status: format!("{} chats", folder_conversations.len()),
                depth: 0,
                expanded,
                selected,
                tooltip: format!(
                    "Project folder {} at {}. Click to expand or collapse.",
                    folder.label, folder.path
                ),
                action: format!(
                    "sidebar-toggle-folder:{}",
                    encode_gui_action_part(&folder.id)
                ),
            });
            if expanded {
                rows.extend(
                    folder_conversations
                        .into_iter()
                        .map(|row| self.shell_sidebar_conversation_row(row, 1)),
                );
            }
        }

        for (folder_id, folder_conversations) in conversations_by_folder {
            let Some(folder_id) = folder_id else {
                continue;
            };
            let expanded = !self
                .app_preferences
                .collapsed_project_folder_ids
                .contains(&folder_id);
            rows.push(WorkflowSidebarTreeRow {
                row_id: format!("missing-folder:{folder_id}"),
                kind: "folder".to_owned(),
                project_folder_id: folder_id.clone(),
                conversation_id: String::new(),
                title: "Missing project folder".to_owned(),
                subtitle: folder_id.clone(),
                status: format!("{} chats", folder_conversations.len()),
                depth: 0,
                expanded,
                selected: self.selected_project_folder_id.as_deref() == Some(folder_id.as_str())
                    && !folder_conversations.iter().any(|row| row.selected),
                tooltip: format!(
                    "Conversations still reference project folder {folder_id}, but the folder record is missing."
                ),
                action: format!("sidebar-toggle-folder:{}", encode_gui_action_part(&folder_id)),
            });
            if expanded {
                rows.extend(
                    folder_conversations
                        .into_iter()
                        .map(|row| self.shell_sidebar_conversation_row(row, 1)),
                );
            }
        }

        rows
    }

    fn shell_sidebar_conversation_row(
        &self,
        row: &ConversationListRow,
        depth: i32,
    ) -> WorkflowSidebarTreeRow {
        let conversation_mode =
            self.conversation_mode(&ConversationId::new(row.conversation_id.clone()));
        let mode = agent_mode_display(&conversation_mode);
        WorkflowSidebarTreeRow {
            row_id: format!("conversation:{}", row.conversation_id),
            kind: "conversation".to_owned(),
            project_folder_id: row.project_folder_id.clone().unwrap_or_default(),
            conversation_id: row.conversation_id.clone(),
            title: row.title.clone(),
            subtitle: format!("{} / {}", row.profile_name, mode),
            status: row
                .goal_status
                .as_ref()
                .map(|goal_status| format!("{} / {goal_status}", row.status))
                .unwrap_or_else(|| row.status.clone()),
            depth,
            expanded: false,
            selected: row.selected,
            tooltip: format!(
                "Open conversation {} in {}",
                row.title,
                shell_folder_label(row)
            ),
            action: format!("conversation-select:{}", row.conversation_id),
        }
    }

    fn shell_profile_rows(&self) -> Vec<WorkflowProfileRow> {
        let selected_profile_id = self.profiles.selected_profile_id();
        let draft = self.profiles.draft();
        let mut rows = self
            .profiles
            .profiles()
            .iter()
            .map(|saved| {
                let (profile, validation) = if draft.is_some_and(|draft| draft.id == saved.id) {
                    (draft.expect("draft checked"), "unsaved")
                } else {
                    (saved, "valid")
                };
                workflow_profile_row(
                    profile,
                    selected_profile_id == Some(profile.id.as_str()) || validation == "unsaved",
                    validation,
                )
            })
            .collect::<Vec<_>>();

        if let Some(draft) = draft
            && !self
                .profiles
                .profiles()
                .iter()
                .any(|profile| profile.id == draft.id)
        {
            rows.push(workflow_profile_row(draft, true, "unsaved"));
        }
        rows
    }

    fn shell_mcp_server_rows(&self) -> Vec<WorkflowMcpServerRow> {
        self.mcp_config
            .servers
            .iter()
            .map(|server| {
                let snapshot = self.mcp_statuses.get(&server.id);
                let status = if !server.enabled {
                    "disabled".to_owned()
                } else {
                    snapshot
                        .map(|snapshot| snapshot.status.label().to_owned())
                        .unwrap_or_else(|| "configured".to_owned())
                };
                let details = snapshot
                    .map(|snapshot| {
                        format!(
                            "{} tools / {} resources / {} templates / {}",
                            snapshot.discovered_tools.len(),
                            snapshot.discovered_resources.len(),
                            snapshot.discovered_resource_templates.len(),
                            mcp_oauth_state_display(&snapshot.oauth_state)
                        )
                    })
                    .unwrap_or_else(|| {
                        if server.oauth.is_some() {
                            "0 tools / 0 resources / 0 templates / oauth required".to_owned()
                        } else {
                            "0 tools / 0 resources / 0 templates / oauth none".to_owned()
                        }
                    });

                WorkflowMcpServerRow {
                    server_id: server.id.clone(),
                    name: server.name.clone(),
                    scope: mcp_scope_display(&server.scope),
                    transport: mcp_transport_display(&server.transport),
                    status,
                    details,
                }
            })
            .collect()
    }

    fn shell_mcp_tool_rows(&self) -> Vec<WorkflowMcpToolRow> {
        let mut rows = Vec::new();
        for server in &self.mcp_config.servers {
            let discovered_tools = self
                .mcp_statuses
                .get(&server.id)
                .map(|snapshot| {
                    snapshot
                        .discovered_tools
                        .iter()
                        .cloned()
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or_default();
            let mut tool_names = discovered_tools.clone();
            tool_names.extend(server.always_allow_tools.iter().cloned());
            tool_names.extend(server.disabled_tools.iter().cloned());

            for tool_name in tool_names {
                let disabled = server.disabled_tools.contains(&tool_name);
                let always_allowed = server.always_allow_tools.contains(&tool_name);
                let policy = if disabled {
                    "disabled"
                } else if always_allowed {
                    "always allowed"
                } else {
                    "enabled"
                }
                .to_owned();

                rows.push(WorkflowMcpToolRow {
                    server_id: server.id.clone(),
                    tool_name: tool_name.clone(),
                    policy,
                    discovered: discovered_tools.contains(&tool_name),
                    disabled,
                    always_allowed,
                    disable_action: gui_mcp_tool_policy_action(
                        "mcp-tool-disable",
                        &server.id,
                        &tool_name,
                    ),
                    allow_action: gui_mcp_tool_policy_action(
                        "mcp-tool-allow",
                        &server.id,
                        &tool_name,
                    ),
                });
            }
        }
        rows
    }

    fn shell_mcp_discovery_rows(&self) -> Vec<WorkflowMcpDiscoveryRow> {
        let mut rows = Vec::new();
        for server in &self.mcp_config.servers {
            let Some(snapshot) = self.mcp_statuses.get(&server.id) else {
                continue;
            };
            rows.extend(snapshot.discovered_resources.iter().map(|resource| {
                WorkflowMcpDiscoveryRow {
                    server_id: server.id.clone(),
                    kind: "resource".to_owned(),
                    name: resource.clone(),
                }
            }));
            rows.extend(
                snapshot
                    .discovered_resource_templates
                    .iter()
                    .map(|template| WorkflowMcpDiscoveryRow {
                        server_id: server.id.clone(),
                        kind: "template".to_owned(),
                        name: template.clone(),
                    }),
            );
        }
        rows
    }

    fn shell_skill_rows(&self) -> Vec<WorkflowSkillRow> {
        self.skills
            .iter()
            .map(|skill| {
                let manifest = &skill.manifest;
                WorkflowSkillRow {
                    name: manifest.name.clone(),
                    scope: skill_scope_display(&manifest.scope),
                    modes: if manifest.mode_slugs.is_empty() {
                        "all modes".to_owned()
                    } else {
                        manifest.mode_slugs.join(", ")
                    },
                    status: if manifest.enabled {
                        "enabled".to_owned()
                    } else {
                        "disabled".to_owned()
                    },
                }
            })
            .collect()
    }

    fn shell_memory_rows(&self) -> Vec<WorkflowMemoryRow> {
        self.memory_store
            .records()
            .into_iter()
            .map(|record| WorkflowMemoryRow {
                record_id: record.id.clone(),
                kind: memory_kind_display(record.kind).to_owned(),
                scope: memory_scope_display(&record.scope),
                source: record.source.clone(),
                status: if record.enabled {
                    "enabled".to_owned()
                } else {
                    "disabled".to_owned()
                },
                preview: memory_preview(&record.text),
            })
            .collect()
    }

    fn shell_transcript_blocks(
        &self,
        active_row: Option<&ConversationListRow>,
    ) -> Vec<WorkflowTranscriptBlock> {
        let mut blocks = Vec::new();
        if let Some(row) = active_row {
            let active_conversation_id = ConversationId::new(row.conversation_id.clone());
            blocks.push(WorkflowTranscriptBlock {
                speaker: "conversation".to_owned(),
                body: format!(
                    "{} in {} using {} ({})",
                    row.title,
                    shell_folder_label(row),
                    row.profile_name,
                    row.status
                ),
                accent_kind: "conversation".to_owned(),
            });
            if let Some(goal_status) = &row.goal_status {
                blocks.push(WorkflowTranscriptBlock {
                    speaker: "goal".to_owned(),
                    body: format!("Goal state: {goal_status}"),
                    accent_kind: "goal".to_owned(),
                });
            }
            if row.hidden_unread_events > 0 {
                blocks.push(WorkflowTranscriptBlock {
                    speaker: "background".to_owned(),
                    body: format!("Hidden conversation events: {}", row.hidden_unread_events),
                    accent_kind: "status".to_owned(),
                });
            }
            if let Some(events) = self.conversation_events.get(&active_conversation_id) {
                blocks.extend(
                    events
                        .iter()
                        .rev()
                        .take(12)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .filter_map(transcript_event_shell_block),
                );
            }
        } else {
            blocks.push(WorkflowTranscriptBlock {
                speaker: "system".to_owned(),
                body: "Create or select a conversation to start agent work.".to_owned(),
                accent_kind: "status".to_owned(),
            });
        }

        if let Some(command) = self.local_command_transcripts.last() {
            blocks.push(WorkflowTranscriptBlock {
                speaker: "rtk".to_owned(),
                body: format!(
                    "{} -> {} ({})",
                    command.argv.join(" "),
                    command_exit_label(command.exit_code, command.timed_out, command.cancelled),
                    local_command_io_label(command)
                ),
                accent_kind: "tool".to_owned(),
            });
        }
        if let Some(command) = self.remote_command_transcripts.last() {
            blocks.push(WorkflowTranscriptBlock {
                speaker: "remote rtk".to_owned(),
                body: format!(
                    "{} on {} -> {} ({})",
                    command.argv.join(" "),
                    command.target_id,
                    command_exit_label(command.exit_code, false, false),
                    remote_command_io_label(command)
                ),
                accent_kind: "tool".to_owned(),
            });
        }
        if let Some(file_op) = self.remote_file_transcripts.last() {
            blocks.push(WorkflowTranscriptBlock {
                speaker: "remote file".to_owned(),
                body: format!(
                    "{:?} {} on {} ({} bytes)",
                    file_op.kind, file_op.path, file_op.target_id, file_op.bytes
                ),
                accent_kind: "file".to_owned(),
            });
        }
        if !self.memory_store.records().is_empty() {
            blocks.push(WorkflowTranscriptBlock {
                speaker: "memory".to_owned(),
                body: format!(
                    "{} memory records available",
                    self.memory_store.records().len()
                ),
                accent_kind: "memory".to_owned(),
            });
        }
        blocks
    }

    fn shell_settings_rows(
        &self,
        active_row: Option<&ConversationListRow>,
    ) -> Vec<WorkflowSettingsRow> {
        let profile = self.shell_profile(active_row);
        let mut rows = Vec::new();
        if let Some(profile) = profile {
            rows.push(shell_settings_row(
                "Provider",
                profile_provider_display(profile.provider),
            ));
            rows.push(shell_settings_row("Model", profile.model_id.as_str()));
            rows.push(shell_settings_row("Region", profile.region.as_str()));
            rows.push(shell_settings_row(
                "AWS profile",
                credential_source_display(&profile.credential_source),
            ));
            if let Some(preview) = &profile.credential_preview {
                rows.push(shell_settings_row(
                    "AWS credential",
                    preview.source_type.as_str(),
                ));
                rows.push(shell_settings_row(
                    "AWS region source",
                    preview.region_source.as_deref().unwrap_or("unknown"),
                ));
            }
            rows.push(shell_settings_row(
                "Mantle API",
                mantle_api_shape_display(profile.mantle_settings.api_shape),
            ));
            rows.push(shell_settings_row(
                "Mantle auth",
                mantle_auth_display(&profile.mantle_settings.auth_mode),
            ));
            rows.push(shell_settings_row("Mantle store", "off (not retained)"));
            rows.push(shell_settings_row(
                "Mantle project",
                profile
                    .mantle_settings
                    .project_id
                    .as_deref()
                    .unwrap_or("none"),
            ));
            rows.push(shell_settings_row(
                "Runtime target",
                runtime_target_display(profile.runtime_settings.target_type),
            ));
            rows.push(shell_settings_row(
                "Runtime cache",
                profile
                    .runtime_settings
                    .prompt_cache_ttl_seconds
                    .map(|ttl| format!("{ttl}s"))
                    .unwrap_or_else(|| "off".to_owned()),
            ));
            rows.push(shell_settings_row(
                "Request tuning",
                request_tuning_display(&profile.request_tuning),
            ));
            rows.push(shell_settings_row(
                "Bedrock probes",
                profile_probe_capability_label(profile),
            ));
        } else {
            rows.push(shell_settings_row("Provider", "No profile"));
            rows.push(shell_settings_row("Model", "No model"));
            rows.push(shell_settings_row("AWS profile", "No credential source"));
            rows.push(shell_settings_row("Mantle API", "unconfigured"));
            rows.push(shell_settings_row("Mantle store", "off (not retained)"));
            rows.push(shell_settings_row("Mantle project", "unconfigured"));
            rows.push(shell_settings_row("Runtime target", "unconfigured"));
            rows.push(shell_settings_row("Runtime cache", "unconfigured"));
            rows.push(shell_settings_row("Request tuning", "unconfigured"));
            rows.push(shell_settings_row("Bedrock probes", "unconfigured"));
        }

        rows.push(shell_settings_row(
            "Profile draft",
            self.profiles
                .draft()
                .map(|draft| format!("unsaved: {}", draft.name))
                .unwrap_or_else(|| "none".to_owned()),
        ));
        rows.push(shell_settings_row("Execution", "approval never"));
        rows.push(shell_settings_row(
            "RTK",
            if self.app_preferences.rtk_enabled {
                "enabled"
            } else {
                "disabled"
            },
        ));
        rows.push(shell_settings_row(
            "Prompt compression",
            if self.app_preferences.prompt_compression_enabled {
                "enabled"
            } else {
                "disabled"
            },
        ));
        rows.push(shell_settings_row(
            "Compression level",
            self.app_preferences
                .prompt_compression_level
                .display_label(),
        ));
        rows.push(shell_settings_row(
            "Send shortcut",
            self.app_preferences.send_shortcut.display_label(),
        ));
        rows.push(shell_settings_row(
            "Command policy",
            command_policy_display(&self.command_policy),
        ));
        rows.push(shell_settings_row(
            "Goal",
            active_row
                .and_then(|row| row.goal_status.as_deref())
                .unwrap_or("none"),
        ));
        rows.push(shell_settings_row(
            "Mode",
            active_row
                .map(|row| {
                    self.conversation_mode(&ConversationId::new(row.conversation_id.clone()))
                })
                .as_ref()
                .map(agent_mode_display)
                .unwrap_or("Code"),
        ));
        rows.push(shell_settings_row(
            "Remote",
            active_row
                .map(|row| row.target_label.as_str())
                .filter(|label| !label.is_empty())
                .unwrap_or("local"),
        ));
        rows.push(shell_settings_row(
            "MCP Servers",
            format!(
                "{} configured / {} status",
                self.mcp_config.servers.len(),
                self.mcp_statuses.list().len()
            ),
        ));
        rows.push(shell_settings_row(
            "Skills",
            format!("{} enabled", self.skills.len()),
        ));
        rows.push(shell_settings_row(
            "Open Tabs",
            format!("{}", self.editor_tabs.len()),
        ));
        rows.push(shell_settings_row(
            "Mantle cache",
            cache_usage_display(self.cache_usage(CacheProviderPlane::BedrockMantle)),
        ));
        rows.push(shell_settings_row(
            "Runtime cache",
            cache_usage_display(self.cache_usage(CacheProviderPlane::BedrockRuntime)),
        ));
        rows
    }

    fn apply_mcp_edit(
        &mut self,
        edit: impl FnOnce(&mut McpConfigSet) -> Result<(), McpConfigError>,
    ) -> Result<Vec<McpConfigChange>, McpConfigError> {
        let before = self.mcp_config.clone();
        let mut next = before.clone();
        edit(&mut next)?;
        let changes = before.diff_for_hot_reload(&next)?;
        self.mcp_config = next;
        Ok(changes)
    }

    async fn persist_mcp_edit_result(
        &self,
        persistence: &PersistenceActorHandle,
        now_ms: i64,
        result: Result<Vec<McpConfigChange>, McpConfigError>,
    ) -> PersistenceResult<Result<Vec<McpConfigChange>, McpConfigError>> {
        if result.is_ok() {
            self.persist_mcp_config(persistence, now_ms).await?;
        }
        Ok(result)
    }

    fn upsert_skill_document(&mut self, document: SkillDocument) {
        self.skills.retain(|skill| {
            !(skill.manifest.name == document.manifest.name
                && skill.manifest.scope == document.manifest.scope)
        });
        self.skills.push(document);
        self.skills.sort_by(|left, right| {
            left.manifest
                .name
                .cmp(&right.manifest.name)
                .then_with(|| left.manifest.path.cmp(&right.manifest.path))
        });
    }

    async fn persist_mcp_config(
        &self,
        persistence: &PersistenceActorHandle,
        now_ms: i64,
    ) -> PersistenceResult<()> {
        persistence
            .store_settings_document(SettingsDocument {
                scope: ConfigScope::Global,
                key: MCP_CONFIG_SETTINGS_KEY.to_owned(),
                document_json: serde_json::to_string(&self.mcp_config)
                    .map_err(json_persistence_error)?,
                updated_at_ms: now_ms,
            })
            .await
    }

    async fn persist_command_policy(
        &self,
        persistence: &PersistenceActorHandle,
        now_ms: i64,
    ) -> PersistenceResult<()> {
        persistence
            .store_settings_document(SettingsDocument {
                scope: ConfigScope::Global,
                key: COMMAND_POLICY_SETTINGS_KEY.to_owned(),
                document_json: serde_json::to_string(&self.command_policy)
                    .map_err(json_persistence_error)?,
                updated_at_ms: now_ms,
            })
            .await
    }

    async fn persist_app_preferences(
        &self,
        persistence: &PersistenceActorHandle,
        now_ms: i64,
    ) -> PersistenceResult<()> {
        persistence
            .store_settings_document(
                app_preferences_document(&self.app_preferences, now_ms)
                    .map_err(json_persistence_error)?,
            )
            .await
    }

    async fn persist_cache_usage(
        &self,
        persistence: &PersistenceActorHandle,
        provider_plane: CacheProviderPlane,
        now_ms: i64,
    ) -> PersistenceResult<()> {
        let snapshot = CacheUsageSnapshot {
            provider_plane,
            usage: self.cache_ledger.usage(provider_plane),
        };
        persistence
            .upsert_cache_metadata(CacheMetadataRecord {
                cache_key: cache_usage_key(provider_plane),
                provider_plane: provider_plane.label().to_owned(),
                document_json: serde_json::to_string(&snapshot).map_err(json_persistence_error)?,
                updated_at_ms: now_ms,
            })
            .await
    }
}

fn gui_mcp_tool_policy_action(prefix: &str, server_id: &str, tool_name: &str) -> String {
    format!(
        "{prefix}:{}:{}",
        encode_gui_action_part(server_id),
        encode_gui_action_part(tool_name)
    )
}

fn decode_gui_mcp_tool_policy_action(
    action: &str,
    prefix: &str,
) -> Result<(String, String), &'static str> {
    let payload = action
        .strip_prefix(prefix)
        .and_then(|payload| payload.strip_prefix(':'))
        .ok_or("invalid action prefix")?;
    let Some((encoded_server, encoded_tool)) = payload.split_once(':') else {
        return Err("missing server/tool payload");
    };
    let server_id = decode_gui_action_part(encoded_server)?;
    let tool_name = decode_gui_action_part(encoded_tool)?;
    Ok((server_id, tool_name))
}

fn encode_gui_action_part(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_gui_action_part(encoded: &str) -> Result<String, &'static str> {
    if !encoded.len().is_multiple_of(2) {
        return Err("invalid encoded payload");
    }
    let mut bytes = Vec::with_capacity(encoded.len() / 2);
    for pair in encoded.as_bytes().chunks_exact(2) {
        let high = decode_hex_nibble(pair[0])?;
        let low = decode_hex_nibble(pair[1])?;
        bytes.push((high << 4) | low);
    }
    String::from_utf8(bytes).map_err(|_| "payload is not utf-8")
}

fn decode_hex_nibble(byte: u8) -> Result<u8, &'static str> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("invalid encoded payload"),
    }
}

pub fn handle_gui_action(
    workflow: &mut FastrockWorkflow,
    action: &str,
    now_ms: i64,
) -> GuiActionResult {
    match action {
        "profile-add" | "profile-add-mantle" => {
            match workflow.create_default_bedrock_profile(UiLlmProvider::BedrockMantle) {
                Ok(profile) => {
                    let writes = gui_profile_write(&profile, now_ms).into_iter().collect();
                    GuiActionResult::with_writes(
                        format!("Mantle profile ready: {}", profile.name),
                        writes,
                    )
                }
                Err(errors) => GuiActionResult::status(format!(
                    "Profile add failed: {}",
                    format_profile_errors(&errors)
                )),
            }
        }
        "profile-add-runtime" => {
            match workflow.create_default_bedrock_profile(UiLlmProvider::BedrockRuntime) {
                Ok(profile) => {
                    let writes = gui_profile_write(&profile, now_ms).into_iter().collect();
                    GuiActionResult::with_writes(
                        format!("Runtime profile saved: {}", profile.name),
                        writes,
                    )
                }
                Err(errors) => GuiActionResult::status(format!(
                    "Profile save failed: {}",
                    format_profile_errors(&errors)
                )),
            }
        }
        "profile-draft-mantle" => {
            let draft = workflow.start_default_bedrock_profile_draft(UiLlmProvider::BedrockMantle);
            GuiActionResult::status(format!("Mantle profile draft: {}", draft.name))
        }
        "profile-draft-runtime" => {
            let draft = workflow.start_default_bedrock_profile_draft(UiLlmProvider::BedrockRuntime);
            GuiActionResult::status(format!("Runtime profile draft: {}", draft.name))
        }
        "profile-save" => match workflow.save_profile_draft() {
            Ok(profile) => GuiActionResult::with_writes(
                format!("Profile saved: {}", profile.name),
                gui_all_profile_writes(workflow, now_ms),
            ),
            Err(errors) => GuiActionResult::status(format!(
                "Profile save failed: {}",
                format_profile_errors(&errors)
            )),
        },
        action if action.starts_with("profile-select:") => {
            let profile_id = action
                .strip_prefix("profile-select:")
                .unwrap_or_default()
                .trim();
            if profile_id.is_empty() {
                return GuiActionResult::status("Profile select failed: empty profile id");
            }
            if workflow.select_profile(profile_id) {
                GuiActionResult::status(format!("Profile selected: {profile_id}"))
            } else {
                GuiActionResult::status(format!("Profile select failed: {profile_id}"))
            }
        }
        action if action.starts_with("profile-toggle:") => {
            let profile_id = action
                .strip_prefix("profile-toggle:")
                .unwrap_or_default()
                .trim();
            let Some(profile) = workflow.profiles().profile(profile_id) else {
                return GuiActionResult::status(format!("Profile toggle failed: {profile_id}"));
            };
            let enabled = !profile.enabled;
            match workflow.set_profile_enabled(profile_id, enabled) {
                Ok(profile) => {
                    let writes = gui_profile_write(&profile, now_ms).into_iter().collect();
                    GuiActionResult::with_writes(
                        format!(
                            "Profile {}: {}",
                            if enabled { "enabled" } else { "disabled" },
                            profile.name
                        ),
                        writes,
                    )
                }
                Err(errors) => GuiActionResult::status(format!(
                    "Profile toggle failed: {}",
                    format_profile_errors(&errors)
                )),
            }
        }
        action if action.starts_with("profile-default:") => {
            let profile_id = action
                .strip_prefix("profile-default:")
                .unwrap_or_default()
                .trim();
            match workflow.set_profile_default(profile_id) {
                Ok(profile) => GuiActionResult::with_writes(
                    format!("Default profile set: {}", profile.name),
                    gui_all_profile_writes(workflow, now_ms),
                ),
                Err(errors) => GuiActionResult::status(format!(
                    "Default profile failed: {}",
                    format_profile_errors(&errors)
                )),
            }
        }
        "profile-edit" => match workflow.edit_selected_or_default_profile_name_draft() {
            Ok(profile) => GuiActionResult::status(format!("Profile edit draft: {}", profile.name)),
            Err(errors) => GuiActionResult::status(format!(
                "Profile edit failed: {}",
                format_profile_errors(&errors)
            )),
        },
        "profile-delete" => {
            let deleted = workflow.delete_selected_or_default_profile();
            let writes = deleted
                .iter()
                .cloned()
                .map(GuiPersistenceWrite::DeleteLlmProfile)
                .collect();
            GuiActionResult::with_writes(
                format!(
                    "Profile delete requested: {}",
                    deleted.unwrap_or_else(|| "none".to_owned())
                ),
                writes,
            )
        }
        "profile-copy" => match workflow.duplicate_selected_or_default_profile_draft() {
            Ok(profile) => GuiActionResult::status(format!("Profile copy draft: {}", profile.name)),
            Err(errors) => GuiActionResult::status(format!(
                "Profile duplicate failed: {}",
                format_profile_errors(&errors)
            )),
        },
        "profile-discover" => GuiActionResult::status(profile_probe_action_status(
            workflow,
            BedrockGuiProbeAction::DiscoverModels,
        )),
        "profile-test" => GuiActionResult::status(profile_probe_action_status(
            workflow,
            BedrockGuiProbeAction::TestConnection,
        )),
        "profile-tuning-max-4096" => {
            gui_set_profile_tuning(workflow, now_ms, "Request max output set: 4096", |tuning| {
                tuning.max_output_tokens = Some(4096);
            })
        }
        "profile-tuning-max-off" => {
            gui_set_profile_tuning(workflow, now_ms, "Request max output cleared", |tuning| {
                tuning.max_output_tokens = None;
            })
        }
        "profile-tuning-temp-02" => {
            gui_set_profile_tuning(workflow, now_ms, "Request temperature set: 0.2", |tuning| {
                tuning.temperature_milli = Some(200);
            })
        }
        "profile-tuning-temp-off" => {
            gui_set_profile_tuning(workflow, now_ms, "Request temperature cleared", |tuning| {
                tuning.temperature_milli = None;
            })
        }
        "profile-tuning-topp-09" => {
            gui_set_profile_tuning(workflow, now_ms, "Request top-p set: 0.9", |tuning| {
                tuning.top_p_milli = Some(900);
            })
        }
        "profile-tuning-topp-off" => {
            gui_set_profile_tuning(workflow, now_ms, "Request top-p cleared", |tuning| {
                tuning.top_p_milli = None;
            })
        }
        "profile-tuning-timeout-120s" => {
            gui_set_profile_tuning(workflow, now_ms, "Request timeout set: 120s", |tuning| {
                tuning.timeout_ms = Some(120_000);
            })
        }
        "profile-tuning-timeout-off" => {
            gui_set_profile_tuning(workflow, now_ms, "Request timeout cleared", |tuning| {
                tuning.timeout_ms = None;
            })
        }
        "profile-tuning-retry-2" => {
            gui_set_profile_tuning(workflow, now_ms, "Request retry set: 2", |tuning| {
                tuning.retry_max_attempts = Some(2);
            })
        }
        "profile-tuning-retry-off" => {
            gui_set_profile_tuning(workflow, now_ms, "Request retry cleared", |tuning| {
                tuning.retry_max_attempts = None;
            })
        }
        "command-policy-default" => match gui_set_command_policy(
            workflow,
            CommandPolicy::default(),
            "Command policy set: default",
            now_ms,
        ) {
            Ok(result) => result,
            Err(error) => {
                GuiActionResult::status(format!("Command policy update failed: {}", error))
            }
        },
        "command-policy-disabled" => match gui_set_command_policy(
            workflow,
            CommandPolicy {
                commands_enabled: false,
                ..CommandPolicy::default()
            },
            "Command policy set: disabled",
            now_ms,
        ) {
            Ok(result) => result,
            Err(error) => {
                GuiActionResult::status(format!("Command policy update failed: {}", error))
            }
        },
        "command-policy-locked" => match gui_set_command_policy(
            workflow,
            gui_locked_command_policy(),
            "Command policy set: locked down",
            now_ms,
        ) {
            Ok(result) => result,
            Err(error) => {
                GuiActionResult::status(format!("Command policy update failed: {}", error))
            }
        },
        "settings-toggle-rtk" => {
            workflow.app_preferences.rtk_enabled = !workflow.app_preferences.rtk_enabled;
            match gui_app_preferences_write(workflow, now_ms) {
                Ok(write) => GuiActionResult::with_writes(
                    if workflow.app_preferences.rtk_enabled {
                        "RTK enabled"
                    } else {
                        "RTK disabled"
                    },
                    vec![write],
                ),
                Err(error) => {
                    GuiActionResult::status(format!("App preference update failed: {}", error))
                }
            }
        }
        "settings-toggle-prompt-compression" => {
            workflow.app_preferences.prompt_compression_enabled =
                !workflow.app_preferences.prompt_compression_enabled;
            match gui_app_preferences_write(workflow, now_ms) {
                Ok(write) => GuiActionResult::with_writes(
                    if workflow.app_preferences.prompt_compression_enabled {
                        "Prompt compression enabled"
                    } else {
                        "Prompt compression disabled"
                    },
                    vec![write],
                ),
                Err(error) => {
                    GuiActionResult::status(format!("App preference update failed: {}", error))
                }
            }
        }
        action if action.starts_with("settings-compression-level:") => {
            let level = action
                .strip_prefix("settings-compression-level:")
                .unwrap_or_default()
                .trim();
            let Some(level) = PromptCompressionLevel::from_stored_label(level) else {
                return GuiActionResult::status(format!("Unknown compression level: {level}"));
            };
            workflow.app_preferences.prompt_compression_level = level;
            match gui_app_preferences_write(workflow, now_ms) {
                Ok(write) => GuiActionResult::with_writes(
                    format!("Prompt compression level set: {}", level.display_label()),
                    vec![write],
                ),
                Err(error) => {
                    GuiActionResult::status(format!("App preference update failed: {}", error))
                }
            }
        }
        action if action.starts_with("settings-send-shortcut:") => {
            let shortcut = action
                .strip_prefix("settings-send-shortcut:")
                .unwrap_or_default()
                .trim();
            let Some(shortcut) = SendShortcut::from_stored_label(shortcut) else {
                return GuiActionResult::status(format!("Unknown send shortcut: {shortcut}"));
            };
            workflow.app_preferences.send_shortcut = shortcut;
            match gui_app_preferences_write(workflow, now_ms) {
                Ok(write) => GuiActionResult::with_writes(
                    format!("Send shortcut set: {}", shortcut.display_label()),
                    vec![write],
                ),
                Err(error) => {
                    GuiActionResult::status(format!("App preference update failed: {}", error))
                }
            }
        }
        "sidebar-toggle-unbound" => {
            workflow.app_preferences.unbound_collapsed =
                !workflow.app_preferences.unbound_collapsed;
            workflow.selected_project_folder_id = None;
            match gui_app_preferences_write(workflow, now_ms) {
                Ok(write) => GuiActionResult::with_writes("Unbound group toggled", vec![write]),
                Err(error) => {
                    GuiActionResult::status(format!("App preference update failed: {}", error))
                }
            }
        }
        action if action.starts_with("sidebar-toggle-folder:") => {
            let encoded_folder_id = action
                .strip_prefix("sidebar-toggle-folder:")
                .unwrap_or_default()
                .trim();
            let folder_id = match decode_gui_action_part(encoded_folder_id) {
                Ok(folder_id) => folder_id,
                Err(error) => {
                    return GuiActionResult::status(format!(
                        "Project folder toggle failed: {error}"
                    ));
                }
            };
            if workflow
                .app_preferences
                .collapsed_project_folder_ids
                .contains(&folder_id)
            {
                workflow
                    .app_preferences
                    .collapsed_project_folder_ids
                    .remove(&folder_id);
            } else {
                workflow
                    .app_preferences
                    .collapsed_project_folder_ids
                    .insert(folder_id.clone());
            }
            workflow.selected_project_folder_id = Some(folder_id.clone());
            match gui_app_preferences_write(workflow, now_ms) {
                Ok(write) => GuiActionResult::with_writes(
                    format!("Project folder toggled: {folder_id}"),
                    vec![write],
                ),
                Err(error) => {
                    GuiActionResult::status(format!("App preference update failed: {}", error))
                }
            }
        }
        action if action == "plan-start" || action.starts_with("plan-start:") => {
            let prompt = action
                .strip_prefix("plan-start:")
                .unwrap_or("Inspect current conversation and create a concrete execution plan.")
                .trim()
                .to_owned();
            if prompt.is_empty() {
                return GuiActionResult::status("Plan create failed: empty prompt");
            }
            let Some(event) = workflow.start_plan_for_visible(prompt) else {
                return GuiActionResult::status("Plan create unavailable: no active conversation");
            };
            let mut writes = Vec::new();
            writes.push(gui_mode_changed_write(
                &event.conversation_id,
                &AgentMode::Plan,
                now_ms,
            ));
            if let Some(write) = gui_transcript_event_write(&event, now_ms) {
                writes.push(write);
            }
            GuiActionResult::with_writes("Plan artifact created", writes)
        }
        "plan-execute" => {
            let Some((event, mode, plan_id)) = workflow.execute_latest_plan_for_visible() else {
                return GuiActionResult::status("Plan execute unavailable: no draft plan");
            };
            let mut writes = Vec::new();
            if let Some(write) = gui_transcript_event_write(&event, now_ms) {
                writes.push(write);
            }
            writes.push(gui_mode_changed_write(
                &event.conversation_id,
                &mode,
                now_ms,
            ));
            GuiActionResult::with_writes(format!("Plan executed: {plan_id}"), writes)
        }
        "plan-toggle" => {
            if let Some((conversation_id, mode)) = workflow.toggle_active_plan_mode() {
                let write = gui_mode_changed_write(&conversation_id, &mode, now_ms);
                GuiActionResult::with_writes(
                    format!("Conversation mode set: {}", agent_mode_display(&mode)),
                    vec![write],
                )
            } else {
                GuiActionResult::status("Plan mode unavailable: no active conversation")
            }
        }
        "goal-pause" => {
            gui_update_active_goal_status(workflow, ThreadGoalStatus::Paused, "Goal paused", now_ms)
        }
        "goal-resume" => {
            let Some(conversation_id) = workflow.scheduler().visible_conversation_id().cloned()
            else {
                return GuiActionResult::status("Goal unavailable: no active conversation");
            };
            if !workflow.resume_goal(&conversation_id, now_ms) {
                return GuiActionResult::status("Goal resume unavailable");
            }
            let writes = gui_goal_write(workflow, &conversation_id, now_ms)
                .into_iter()
                .collect();
            GuiActionResult::with_writes("Goal resumed", writes)
        }
        "goal-complete" => gui_update_active_goal_status(
            workflow,
            ThreadGoalStatus::Complete,
            "Goal complete",
            now_ms,
        ),
        "goal-clear" => {
            let Some(conversation_id) = workflow.scheduler().visible_conversation_id().cloned()
            else {
                return GuiActionResult::status("Goal unavailable: no active conversation");
            };
            if !workflow.clear_goal(&conversation_id) {
                return GuiActionResult::status("Goal clear unavailable");
            }
            GuiActionResult::with_writes(
                "Goal cleared",
                vec![GuiPersistenceWrite::DeleteGoal(conversation_id.0)],
            )
        }
        "conversation-new" => {
            let selected_folder_id = workflow
                .selected_project_folder_id
                .clone()
                .filter(|folder_id| workflow.project_folders.contains_key(folder_id));
            let conversation_id = ConversationId::new(generated_persisted_id());
            let conversation = if let Some(folder_id) = selected_folder_id.as_deref() {
                workflow.create_conversation_with_status(
                    &conversation_id.0,
                    "New chat",
                    folder_id,
                    ConversationStatus::Idle,
                    now_ms,
                )
            } else {
                workflow.create_unbound_conversation_with_status(
                    &conversation_id.0,
                    "New chat",
                    ConversationStatus::Idle,
                    now_ms,
                )
            };
            let _ = workflow.switch_visible_conversation(&conversation);
            let writes = gui_conversation_writes(
                &conversation,
                "New chat",
                selected_folder_id.as_deref(),
                ConversationStatus::Idle,
                now_ms,
            );
            GuiActionResult::with_writes("New conversation created", writes)
        }
        "mcp-add" => {
            let server = gui_default_mcp_server(format!(
                "mcp-{}-{}",
                now_ms,
                workflow.mcp_config().servers.len() + 1
            ));
            match workflow.add_mcp_server(server.clone()) {
                Ok(changes) => match gui_mcp_config_write(workflow, now_ms) {
                    Ok(write) => GuiActionResult::with_writes(
                        format!(
                            "MCP server added: {} ({})",
                            server.name,
                            mcp_changes_display(&changes)
                        ),
                        vec![write],
                    ),
                    Err(error) => GuiActionResult::status(format!("MCP add failed: {}", error)),
                },
                Err(error) => GuiActionResult::status(format!("MCP add failed: {error}")),
            }
        }
        action if action.starts_with("mcp-edit:") => {
            let server_id = action.strip_prefix("mcp-edit:").unwrap_or_default().trim();
            if server_id.is_empty() {
                return GuiActionResult::status("MCP edit failed: empty server id");
            }
            let Some(mut server) = workflow.mcp_config().server(server_id).cloned() else {
                return GuiActionResult::status(format!("MCP edit failed: {server_id}"));
            };
            server.name = format!("{} Edited {now_ms}", server.name);
            match workflow.update_mcp_server(server.clone()) {
                Ok(changes) => match gui_mcp_config_write(workflow, now_ms) {
                    Ok(write) => GuiActionResult::with_writes(
                        format!(
                            "MCP server edited: {} ({})",
                            server.name,
                            mcp_changes_display(&changes)
                        ),
                        vec![write],
                    ),
                    Err(error) => GuiActionResult::status(format!("MCP edit failed: {}", error)),
                },
                Err(error) => GuiActionResult::status(format!("MCP edit failed: {error}")),
            }
        }
        action if action.starts_with("mcp-toggle:") => {
            let server_id = action
                .strip_prefix("mcp-toggle:")
                .unwrap_or_default()
                .trim();
            if server_id.is_empty() {
                return GuiActionResult::status("MCP toggle failed: empty server id");
            }
            let Some(server) = workflow.mcp_config().server(server_id).cloned() else {
                return GuiActionResult::status(format!("MCP toggle failed: {server_id}"));
            };
            let enabled = !server.enabled;
            match workflow.set_mcp_server_enabled(server_id, enabled) {
                Ok(changes) => match gui_mcp_config_write(workflow, now_ms) {
                    Ok(write) => GuiActionResult::with_writes(
                        format!(
                            "MCP server {}: {} ({})",
                            if enabled { "enabled" } else { "disabled" },
                            server_id,
                            mcp_changes_display(&changes)
                        ),
                        vec![write],
                    ),
                    Err(error) => GuiActionResult::status(format!("MCP toggle failed: {}", error)),
                },
                Err(error) => GuiActionResult::status(format!("MCP toggle failed: {error}")),
            }
        }
        action if action.starts_with("mcp-restart:") => {
            let server_id = action
                .strip_prefix("mcp-restart:")
                .unwrap_or_default()
                .trim();
            if server_id.is_empty() {
                return GuiActionResult::status("MCP restart failed: empty server id");
            }
            let Some(server) = workflow.mcp_config().server(server_id).cloned() else {
                return GuiActionResult::status(format!("MCP restart failed: {server_id}"));
            };
            let mut snapshot = workflow
                .mcp_status(server_id)
                .cloned()
                .unwrap_or_else(|| McpRuntimeSnapshot::stopped(&server, now_ms));
            snapshot.status = McpServerRuntimeStatus::Starting;
            snapshot.restart_count = snapshot.restart_count.saturating_add(1);
            snapshot.last_error = None;
            snapshot.log_excerpt = Some("restart requested from GUI".to_owned());
            snapshot.updated_at_ms = now_ms;
            workflow.record_mcp_status(snapshot.clone());
            match mcp_status_snapshot_record(&snapshot) {
                Ok(record) => GuiActionResult::with_writes(
                    format!("MCP server restart requested: {server_id}"),
                    vec![GuiPersistenceWrite::UpsertMcpStatus(record)],
                ),
                Err(error) => GuiActionResult::status(format!("MCP restart failed: {}", error)),
            }
        }
        action if action.starts_with("mcp-delete:") => {
            let server_id = action
                .strip_prefix("mcp-delete:")
                .unwrap_or_default()
                .trim();
            if server_id.is_empty() {
                return GuiActionResult::status("MCP delete failed: empty server id");
            }
            match workflow.delete_mcp_server(server_id) {
                Ok(changes) => match gui_mcp_config_write(workflow, now_ms) {
                    Ok(write) => {
                        workflow.mcp_statuses.remove(server_id);
                        GuiActionResult::with_writes(
                            format!(
                                "MCP server deleted: {} ({})",
                                server_id,
                                mcp_changes_display(&changes)
                            ),
                            vec![
                                write,
                                GuiPersistenceWrite::DeleteMcpStatus(server_id.to_owned()),
                            ],
                        )
                    }
                    Err(error) => GuiActionResult::status(format!("MCP delete failed: {}", error)),
                },
                Err(error) => GuiActionResult::status(format!("MCP delete failed: {error}")),
            }
        }
        action if action.starts_with("mcp-tool-disable:") => {
            let (server_id, tool_name) =
                match decode_gui_mcp_tool_policy_action(action, "mcp-tool-disable") {
                    Ok(parts) => parts,
                    Err(error) => {
                        return GuiActionResult::status(format!("MCP tool toggle failed: {error}"));
                    }
                };
            let disabled = workflow
                .mcp_config()
                .server(&server_id)
                .map(|server| !server.disabled_tools.contains(&tool_name))
                .unwrap_or(false);
            match workflow.set_mcp_tool_disabled(&server_id, &tool_name, disabled) {
                Ok(changes) => match gui_mcp_config_write(workflow, now_ms) {
                    Ok(write) => GuiActionResult::with_writes(
                        format!(
                            "MCP tool {}: {} / {} ({})",
                            if disabled { "disabled" } else { "enabled" },
                            server_id,
                            tool_name,
                            mcp_changes_display(&changes)
                        ),
                        vec![write],
                    ),
                    Err(error) => {
                        GuiActionResult::status(format!("MCP tool toggle failed: {}", error))
                    }
                },
                Err(error) => GuiActionResult::status(format!("MCP tool toggle failed: {error}")),
            }
        }
        action if action.starts_with("mcp-tool-allow:") => {
            let (server_id, tool_name) =
                match decode_gui_mcp_tool_policy_action(action, "mcp-tool-allow") {
                    Ok(parts) => parts,
                    Err(error) => {
                        return GuiActionResult::status(format!("MCP tool allow failed: {error}"));
                    }
                };
            let always_allowed = workflow
                .mcp_config()
                .server(&server_id)
                .map(|server| !server.always_allow_tools.contains(&tool_name))
                .unwrap_or(false);
            match workflow.set_mcp_tool_always_allowed(&server_id, &tool_name, always_allowed) {
                Ok(changes) => match gui_mcp_config_write(workflow, now_ms) {
                    Ok(write) => GuiActionResult::with_writes(
                        format!(
                            "MCP tool {}: {} / {} ({})",
                            if always_allowed {
                                "always allowed"
                            } else {
                                "requires approval"
                            },
                            server_id,
                            tool_name,
                            mcp_changes_display(&changes)
                        ),
                        vec![write],
                    ),
                    Err(error) => {
                        GuiActionResult::status(format!("MCP tool allow failed: {}", error))
                    }
                },
                Err(error) => GuiActionResult::status(format!("MCP tool allow failed: {error}")),
            }
        }
        "skill-create" => {
            let document = workflow.create_gui_skill(now_ms);
            let active_conversation_id = workflow.scheduler().visible_conversation_id().cloned();
            let writes = gui_editor_tab_writes(workflow, active_conversation_id.as_ref(), now_ms);
            GuiActionResult::with_writes(
                format!("Skill created: {}", document.manifest.name),
                writes,
            )
        }
        action if action.starts_with("skill-edit:") => {
            let name = action
                .strip_prefix("skill-edit:")
                .unwrap_or_default()
                .trim();
            if name.is_empty() {
                return GuiActionResult::status("Skill edit failed: empty skill name");
            }
            let Some(document) = workflow.edit_gui_skill(name, now_ms) else {
                return GuiActionResult::status(format!("Skill edit failed: {name}"));
            };
            let active_conversation_id = workflow.scheduler().visible_conversation_id().cloned();
            let writes = gui_editor_tab_writes(workflow, active_conversation_id.as_ref(), now_ms);
            GuiActionResult::with_writes(
                format!("Skill opened for edit: {}", document.manifest.name),
                writes,
            )
        }
        action if action.starts_with("skill-mode:") => {
            let name = action
                .strip_prefix("skill-mode:")
                .unwrap_or_default()
                .trim();
            if name.is_empty() {
                return GuiActionResult::status("Skill mode failed: empty skill name");
            }
            let Some(document) = workflow.toggle_gui_skill_mode_restriction(name, now_ms) else {
                return GuiActionResult::status(format!("Skill mode failed: {name}"));
            };
            let active_conversation_id = workflow.scheduler().visible_conversation_id().cloned();
            let writes = gui_editor_tab_writes(workflow, active_conversation_id.as_ref(), now_ms);
            let modes = if document.manifest.mode_slugs.is_empty() {
                "all".to_owned()
            } else {
                document.manifest.mode_slugs.join(",")
            };
            GuiActionResult::with_writes(
                format!("Skill modes set: {} -> {modes}", document.manifest.name),
                writes,
            )
        }
        action if action.starts_with("skill-scope:") => {
            let name = action
                .strip_prefix("skill-scope:")
                .unwrap_or_default()
                .trim();
            if name.is_empty() {
                return GuiActionResult::status("Skill scope failed: empty skill name");
            }
            let Some(document) = workflow.toggle_gui_skill_scope(name, now_ms) else {
                return GuiActionResult::status(format!("Skill scope failed: {name}"));
            };
            let active_conversation_id = workflow.scheduler().visible_conversation_id().cloned();
            let writes = gui_editor_tab_writes(workflow, active_conversation_id.as_ref(), now_ms);
            GuiActionResult::with_writes(
                format!(
                    "Skill scope set: {} -> {}",
                    document.manifest.name,
                    skill_scope_display(&document.manifest.scope)
                ),
                writes,
            )
        }
        action if action.starts_with("skill-delete:") => {
            let name = action
                .strip_prefix("skill-delete:")
                .unwrap_or_default()
                .trim();
            if name.is_empty() {
                return GuiActionResult::status("Skill delete failed: empty skill name");
            }
            let Some(document) = workflow.delete_gui_skill(name) else {
                return GuiActionResult::status(format!("Skill delete failed: {name}"));
            };
            let mut writes = Vec::new();
            if workflow.close_editor_tab(&document.manifest.path) {
                writes.push(GuiPersistenceWrite::DeleteEditorTab(
                    document.manifest.path.clone(),
                ));
                let active_conversation_id =
                    workflow.scheduler().visible_conversation_id().cloned();
                writes.extend(gui_editor_tab_writes(
                    workflow,
                    active_conversation_id.as_ref(),
                    now_ms,
                ));
            }
            GuiActionResult::with_writes(format!("Skill deleted: {name}"), writes)
        }
        action if action.starts_with("memory-inspect:") => {
            let record_id = action
                .strip_prefix("memory-inspect:")
                .unwrap_or_default()
                .trim();
            if record_id.is_empty() {
                return GuiActionResult::status("Memory inspect failed: empty record id");
            }
            let Some(record) = workflow
                .memory_records()
                .into_iter()
                .find(|record| record.id == record_id)
            else {
                return GuiActionResult::status(format!("Memory inspect failed: {record_id}"));
            };
            let path = format!("memory://{}", record.id);
            workflow.open_editor_tab(EditorBuffer::read_only_from_text(
                path,
                &memory_inspection_source(&record),
            ));
            let active_conversation_id = workflow.scheduler().visible_conversation_id().cloned();
            let writes = gui_editor_tab_writes(workflow, active_conversation_id.as_ref(), now_ms);
            GuiActionResult::with_writes(format!("Memory opened: {record_id}"), writes)
        }
        action if action.starts_with("memory-toggle:") => {
            let record_id = action
                .strip_prefix("memory-toggle:")
                .unwrap_or_default()
                .trim();
            if record_id.is_empty() {
                return GuiActionResult::status("Memory toggle failed: empty record id");
            }
            let Some(mut record) = workflow
                .memory_records()
                .into_iter()
                .find(|record| record.id == record_id)
            else {
                return GuiActionResult::status(format!("Memory toggle failed: {record_id}"));
            };
            let enabled = !record.enabled;
            if !workflow.set_memory_enabled(record_id, enabled, now_ms) {
                return GuiActionResult::status(format!("Memory toggle failed: {record_id}"));
            }
            record.enabled = enabled;
            record.updated_at_ms = now_ms;
            GuiActionResult::with_writes(
                format!(
                    "Memory {}: {record_id}",
                    if enabled { "enabled" } else { "disabled" }
                ),
                vec![GuiPersistenceWrite::UpsertMemoryRecord(record)],
            )
        }
        action if action.starts_with("memory-delete:") => {
            let record_id = action
                .strip_prefix("memory-delete:")
                .unwrap_or_default()
                .trim();
            if record_id.is_empty() {
                return GuiActionResult::status("Memory delete failed: empty record id");
            }
            if !workflow.delete_memory(record_id) {
                return GuiActionResult::status(format!("Memory delete failed: {record_id}"));
            }
            GuiActionResult::with_writes(
                format!("Memory deleted: {record_id}"),
                vec![GuiPersistenceWrite::DeleteMemoryRecord(
                    record_id.to_owned(),
                )],
            )
        }
        action if action.starts_with("conversation-select:") => {
            let conversation_id = ConversationId::new(
                action
                    .strip_prefix("conversation-select:")
                    .unwrap_or_default()
                    .trim(),
            );
            if workflow.switch_visible_conversation(&conversation_id) {
                let title = workflow
                    .scheduler()
                    .conversation(&conversation_id)
                    .map(|conversation| conversation.title.as_str())
                    .unwrap_or(conversation_id.0.as_str());
                GuiActionResult::status(format!("Conversation selected: {title}"))
            } else {
                GuiActionResult::status(format!(
                    "Conversation select failed: {}",
                    conversation_id.0
                ))
            }
        }
        action if action == "conversation-send" || action.starts_with("conversation-send:") => {
            let message = action
                .strip_prefix("conversation-send:")
                .unwrap_or("finish current task")
                .trim()
                .to_owned();
            if message.is_empty() {
                return GuiActionResult::status("Message empty");
            }
            if let Some(conversation_id) = workflow.scheduler().visible_conversation_id().cloned() {
                let runtime_work = workflow
                    .selected_or_default_profile_id()
                    .and_then(|profile_id| {
                        let profile = workflow.profiles().profile(&profile_id)?;
                        let mantle_previous_response_id = (profile.provider
                            == UiLlmProvider::BedrockMantle)
                            .then(|| {
                                workflow
                                    .mantle_stored_response_state(
                                        &conversation_id,
                                        profile.mantle_settings.project_id.as_deref(),
                                    )
                                    .last_response_id
                            })
                            .flatten();
                        Some(GuiRuntimeWork::RunModel(BedrockModelRunRequest {
                            conversation_id: conversation_id.clone(),
                            profile_id,
                            message: message.clone(),
                            prompt_compression_enabled: workflow
                                .app_preferences
                                .prompt_compression_enabled,
                            prompt_compression_level: workflow
                                .app_preferences
                                .prompt_compression_level
                                .stored_label()
                                .to_owned(),
                            mantle_previous_response_id,
                        }))
                    })
                    .into_iter()
                    .collect();
                let user_event = workflow.append_user_message_to_visible(message.clone());
                let mut writes = Vec::new();
                if let Some(write) =
                    gui_touch_conversation_write(workflow, &conversation_id, now_ms)
                {
                    writes.push(write);
                }
                if let Some(write) = user_event
                    .as_ref()
                    .and_then(|event| gui_transcript_event_write(event, now_ms))
                {
                    writes.push(write);
                }
                return GuiActionResult::with_writes_and_runtime_work(
                    "Message queued",
                    writes,
                    runtime_work,
                );
            }
            let selected_folder_id = workflow
                .selected_project_folder_id
                .clone()
                .filter(|folder_id| workflow.project_folders.contains_key(folder_id));
            let conversation_id = ConversationId::new(generated_persisted_id());
            let conversation_id = if let Some(folder_id) = selected_folder_id.as_deref() {
                workflow.create_conversation_with_status(
                    &conversation_id.0,
                    "New chat",
                    folder_id,
                    ConversationStatus::Running,
                    now_ms,
                )
            } else {
                workflow.create_unbound_conversation_with_status(
                    &conversation_id.0,
                    "New chat",
                    ConversationStatus::Running,
                    now_ms,
                )
            };
            let _ = workflow.switch_visible_conversation(&conversation_id);
            let user_event = workflow.append_user_message_to_visible(message.clone());
            let runtime_work = workflow
                .selected_or_default_profile_id()
                .and_then(|profile_id| {
                    let profile = workflow.profiles().profile(&profile_id)?;
                    let mantle_previous_response_id = (profile.provider
                        == UiLlmProvider::BedrockMantle)
                        .then(|| {
                            workflow
                                .mantle_stored_response_state(
                                    &conversation_id,
                                    profile.mantle_settings.project_id.as_deref(),
                                )
                                .last_response_id
                        })
                        .flatten();
                    Some(GuiRuntimeWork::RunModel(BedrockModelRunRequest {
                        conversation_id: conversation_id.clone(),
                        profile_id,
                        message: message.clone(),
                        prompt_compression_enabled: workflow
                            .app_preferences
                            .prompt_compression_enabled,
                        prompt_compression_level: workflow
                            .app_preferences
                            .prompt_compression_level
                            .stored_label()
                            .to_owned(),
                        mantle_previous_response_id,
                    }))
                })
                .into_iter()
                .collect();
            let mut writes = Vec::new();
            writes.extend(gui_conversation_writes(
                &conversation_id,
                "New chat",
                selected_folder_id.as_deref(),
                ConversationStatus::Running,
                now_ms,
            ));
            if let Some(write) = user_event
                .as_ref()
                .and_then(|event| gui_transcript_event_write(event, now_ms))
            {
                writes.push(write);
            }
            GuiActionResult::with_writes_and_runtime_work("Message queued", writes, runtime_work)
        }
        action if action.starts_with("editor-select:") => {
            let path = action
                .strip_prefix("editor-select:")
                .unwrap_or_default()
                .trim();
            if path.is_empty() {
                return GuiActionResult::status("Editor select failed: empty path");
            }
            if !workflow.select_editor_tab(path) {
                return GuiActionResult::status(format!("Editor select failed: {path}"));
            }
            let active_conversation_id = workflow.scheduler().visible_conversation_id().cloned();
            let writes = gui_editor_tab_writes(workflow, active_conversation_id.as_ref(), now_ms);
            GuiActionResult::with_writes(format!("Editor tab selected: {path}"), writes)
        }
        action if action.starts_with("editor-close:") => {
            let path = action
                .strip_prefix("editor-close:")
                .unwrap_or_default()
                .trim();
            if path.is_empty() {
                return GuiActionResult::status("Editor close failed: empty path");
            }
            if !workflow.close_editor_tab(path) {
                return GuiActionResult::status(format!("Editor close failed: {path}"));
            }
            let active_conversation_id = workflow.scheduler().visible_conversation_id().cloned();
            let mut writes = vec![GuiPersistenceWrite::DeleteEditorTab(path.to_owned())];
            writes.extend(gui_editor_tab_writes(
                workflow,
                active_conversation_id.as_ref(),
                now_ms,
            ));
            GuiActionResult::with_writes(format!("Editor tab closed: {path}"), writes)
        }
        action if action == "editor-append" || action.starts_with("editor-append:") => {
            let text = action
                .strip_prefix("editor-append:")
                .unwrap_or("// Fastrock editor edit\n")
                .trim_end();
            match workflow.append_to_active_editor(text) {
                Some(Ok((path, dirty))) => {
                    let active_conversation_id =
                        workflow.scheduler().visible_conversation_id().cloned();
                    let writes =
                        gui_editor_tab_writes(workflow, active_conversation_id.as_ref(), now_ms);
                    GuiActionResult::with_writes(
                        format!("Editor appended: {path} dirty={dirty}"),
                        writes,
                    )
                }
                Some(Err(error)) => {
                    GuiActionResult::status(format!("Editor append failed: {error}"))
                }
                None => GuiActionResult::status("Editor append failed: no active tab"),
            }
        }
        "editor-save" => {
            let Some((path, contents)) = workflow.save_active_editor_request() else {
                return GuiActionResult::status("Editor save failed: no active tab");
            };
            let active_conversation_id = workflow.scheduler().visible_conversation_id().cloned();
            let mut writes = vec![GuiPersistenceWrite::SaveEditorFile {
                path: path.clone(),
                contents,
            }];
            writes.extend(gui_editor_tab_writes(
                workflow,
                active_conversation_id.as_ref(),
                now_ms,
            ));
            GuiActionResult::with_writes(format!("Editor save queued: {path}"), writes)
        }
        "editor-undo" => match workflow.undo_active_editor() {
            Some(Ok((path, dirty))) => {
                let active_conversation_id =
                    workflow.scheduler().visible_conversation_id().cloned();
                let writes =
                    gui_editor_tab_writes(workflow, active_conversation_id.as_ref(), now_ms);
                GuiActionResult::with_writes(format!("Editor undo: {path} dirty={dirty}"), writes)
            }
            Some(Err(error)) => GuiActionResult::status(format!("Editor undo failed: {error}")),
            None => GuiActionResult::status("Editor undo failed: no active tab"),
        },
        "editor-redo" => match workflow.redo_active_editor() {
            Some(Ok((path, dirty))) => {
                let active_conversation_id =
                    workflow.scheduler().visible_conversation_id().cloned();
                let writes =
                    gui_editor_tab_writes(workflow, active_conversation_id.as_ref(), now_ms);
                GuiActionResult::with_writes(format!("Editor redo: {path} dirty={dirty}"), writes)
            }
            Some(Err(error)) => GuiActionResult::status(format!("Editor redo failed: {error}")),
            None => GuiActionResult::status("Editor redo failed: no active tab"),
        },
        "editor-revert" => match workflow.revert_active_editor() {
            Some(Ok((path, dirty))) => {
                let active_conversation_id =
                    workflow.scheduler().visible_conversation_id().cloned();
                let writes =
                    gui_editor_tab_writes(workflow, active_conversation_id.as_ref(), now_ms);
                GuiActionResult::with_writes(
                    format!("Editor reverted: {path} dirty={dirty}"),
                    writes,
                )
            }
            Some(Err(error)) => GuiActionResult::status(format!("Editor revert failed: {error}")),
            None => GuiActionResult::status("Editor revert failed: no active tab"),
        },
        "editor-diff" => match workflow.diff_active_editor() {
            Some((path, changes)) => {
                GuiActionResult::status(format!("Editor diff: {changes} changed lines in {path}"))
            }
            None => GuiActionResult::status("Editor diff failed: no active tab"),
        },
        action if action.starts_with("editor-find:") => {
            let needle = action.strip_prefix("editor-find:").unwrap_or_default();
            if needle.is_empty() {
                return GuiActionResult::status("Editor find failed: empty query");
            }
            match workflow.find_in_active_editor(needle) {
                Some((path, matches)) => {
                    GuiActionResult::status(format!("Editor find: {matches} matches in {path}"))
                }
                None => GuiActionResult::status("Editor find failed: no active tab"),
            }
        }
        action if action.starts_with("editor-goto:") => {
            let line_number = action
                .strip_prefix("editor-goto:")
                .unwrap_or_default()
                .trim()
                .parse::<usize>()
                .unwrap_or(1);
            match workflow.active_editor_goto_line(line_number) {
                Some((path, line)) => {
                    GuiActionResult::status(format!("Editor goto: {path}:{line}"))
                }
                None => GuiActionResult::status("Editor goto failed: no active tab"),
            }
        }
        _ => GuiActionResult::status(format!("Unhandled action: {action}")),
    }
}

pub async fn persist_gui_action_writes(
    persistence: &PersistenceActorClient,
    writes: Vec<GuiPersistenceWrite>,
) -> PersistenceResult<()> {
    for write in writes {
        match write {
            GuiPersistenceWrite::UpsertLlmProfile(record) => {
                persistence.upsert_llm_profile(record).await?;
            }
            GuiPersistenceWrite::DeleteLlmProfile(id) => {
                persistence.delete_llm_profile(id).await?;
            }
            GuiPersistenceWrite::StoreSettingsDocument(document) => {
                persistence.store_settings_document(document).await?;
            }
            GuiPersistenceWrite::UpsertProjectFolder(record) => {
                persistence.upsert_project_folder(record).await?;
            }
            GuiPersistenceWrite::DeleteProjectFolder(id) => {
                persistence.delete_project_folder(id).await?;
            }
            GuiPersistenceWrite::UpsertConversation(metadata) => {
                persistence.upsert_conversation(metadata).await?;
            }
            GuiPersistenceWrite::DeleteConversation(id) => {
                persistence.delete_conversation(id).await?;
            }
            GuiPersistenceWrite::AppendConversationEvent(event) => {
                persistence.append_conversation_event(event).await?;
            }
            GuiPersistenceWrite::UpsertGoal(record) => {
                persistence.upsert_goal(record).await?;
            }
            GuiPersistenceWrite::DeleteGoal(conversation_id) => {
                persistence.delete_goal(conversation_id).await?;
            }
            GuiPersistenceWrite::UpsertEditorTab(record) => {
                persistence.upsert_editor_tab(record).await?;
            }
            GuiPersistenceWrite::DeleteEditorTab(path) => {
                persistence.delete_editor_tab(path).await?;
            }
            GuiPersistenceWrite::SaveEditorFile { path, contents } => {
                save_editor_file_async(path, contents).await?;
            }
            GuiPersistenceWrite::UpsertMcpStatus(record) => {
                persistence.upsert_mcp_status(record).await?;
            }
            GuiPersistenceWrite::DeleteMcpStatus(server_id) => {
                persistence.delete_mcp_status(server_id).await?;
            }
            GuiPersistenceWrite::UpsertMemoryRecord(record) => {
                persistence.upsert_memory_record(record).await?;
            }
            GuiPersistenceWrite::DeleteMemoryRecord(record_id) => {
                persistence.delete_memory_record(record_id).await?;
            }
        }
    }
    Ok(())
}

async fn save_editor_file_async(path: String, contents: String) -> PersistenceResult<()> {
    let path = PathBuf::from(path);
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        tokio::fs::create_dir_all(parent).await?;
    }

    let file_name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("buffer.txt");
    let tmp_path = path.with_file_name(format!(
        ".{file_name}.fastrock-gui-tmp-{}-{}",
        std::process::id(),
        now_ms()
    ));

    let mut file = tokio::fs::File::create(&tmp_path).await?;
    tokio::io::AsyncWriteExt::write_all(&mut file, contents.as_bytes()).await?;
    file.sync_all().await?;
    drop(file);

    match tokio::fs::rename(&tmp_path, &path).await {
        Ok(()) => Ok(()),
        Err(_) if tokio::fs::try_exists(&path).await.unwrap_or(false) => {
            tokio::fs::remove_file(&path).await?;
            tokio::fs::rename(&tmp_path, &path).await?;
            Ok(())
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            Err(PersistenceError::Io(error))
        }
    }
}

pub fn format_profile_errors(errors: &[fastrock_ui::ProfileValidationError]) -> String {
    errors
        .iter()
        .map(|error| format!("{} {}", error.field, error.message))
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BedrockGuiProbeAction {
    DiscoverModels,
    TestConnection,
}

fn gui_update_active_goal_status(
    workflow: &mut FastrockWorkflow,
    status: ThreadGoalStatus,
    message: &'static str,
    now_ms: i64,
) -> GuiActionResult {
    let Some(conversation_id) = workflow.scheduler().visible_conversation_id().cloned() else {
        return GuiActionResult::status("Goal unavailable: no active conversation");
    };
    if !workflow.update_goal_status(&conversation_id, status, now_ms) {
        return GuiActionResult::status("Goal unavailable: no active goal");
    }
    let writes = gui_goal_write(workflow, &conversation_id, now_ms)
        .into_iter()
        .collect();
    GuiActionResult::with_writes(message, writes)
}

fn gui_set_profile_tuning(
    workflow: &mut FastrockWorkflow,
    now_ms: i64,
    status: &'static str,
    edit: impl FnOnce(&mut fastrock_ui::UiRequestTuning),
) -> GuiActionResult {
    match workflow.update_selected_or_default_profile_tuning(edit) {
        Ok(profile) => GuiActionResult::with_writes(
            status,
            gui_profile_write(&profile, now_ms).into_iter().collect(),
        ),
        Err(errors) => GuiActionResult::status(format!(
            "Request tuning failed: {}",
            format_profile_errors(&errors)
        )),
    }
}

fn gui_mcp_config_write(
    workflow: &FastrockWorkflow,
    now_ms: i64,
) -> Result<GuiPersistenceWrite, serde_json::Error> {
    Ok(GuiPersistenceWrite::StoreSettingsDocument(
        SettingsDocument {
            scope: ConfigScope::Global,
            key: MCP_CONFIG_SETTINGS_KEY.to_owned(),
            document_json: serde_json::to_string(workflow.mcp_config())?,
            updated_at_ms: now_ms,
        },
    ))
}

fn gui_set_command_policy(
    workflow: &mut FastrockWorkflow,
    policy: CommandPolicy,
    status: &'static str,
    now_ms: i64,
) -> Result<GuiActionResult, serde_json::Error> {
    workflow.set_command_policy(policy);
    Ok(GuiActionResult::with_writes(
        status,
        vec![gui_command_policy_write(workflow, now_ms)?],
    ))
}

fn gui_command_policy_write(
    workflow: &FastrockWorkflow,
    now_ms: i64,
) -> Result<GuiPersistenceWrite, serde_json::Error> {
    Ok(GuiPersistenceWrite::StoreSettingsDocument(
        SettingsDocument {
            scope: ConfigScope::Global,
            key: COMMAND_POLICY_SETTINGS_KEY.to_owned(),
            document_json: serde_json::to_string(workflow.command_policy())?,
            updated_at_ms: now_ms,
        },
    ))
}

fn gui_app_preferences_write(
    workflow: &FastrockWorkflow,
    now_ms: i64,
) -> Result<GuiPersistenceWrite, serde_json::Error> {
    Ok(GuiPersistenceWrite::StoreSettingsDocument(
        app_preferences_document(workflow.app_preferences(), now_ms)?,
    ))
}

fn app_preferences_document(
    preferences: &AppPreferences,
    now_ms: i64,
) -> Result<SettingsDocument, serde_json::Error> {
    Ok(SettingsDocument {
        scope: ConfigScope::Global,
        key: APP_PREFERENCES_SETTINGS_KEY.to_owned(),
        document_json: serde_json::to_string(&app_preferences_json(preferences))?,
        updated_at_ms: now_ms,
    })
}

fn app_preferences_from_document(document: &SettingsDocument) -> PersistenceResult<AppPreferences> {
    let value: serde_json::Value =
        serde_json::from_str(&document.document_json).map_err(json_persistence_error)?;
    Ok(app_preferences_from_json(&value))
}

fn app_preferences_from_json(value: &serde_json::Value) -> AppPreferences {
    let mut preferences = AppPreferences::default();
    preferences.rtk_enabled = value
        .get("rtk_enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(preferences.rtk_enabled);
    preferences.prompt_compression_enabled = value
        .get("prompt_compression_enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(preferences.prompt_compression_enabled);
    preferences.prompt_compression_level = value
        .get("prompt_compression_level")
        .and_then(serde_json::Value::as_str)
        .and_then(PromptCompressionLevel::from_stored_label)
        .unwrap_or(preferences.prompt_compression_level);
    preferences.send_shortcut = value
        .get("send_shortcut")
        .and_then(serde_json::Value::as_str)
        .and_then(SendShortcut::from_stored_label)
        .unwrap_or(preferences.send_shortcut);
    preferences.collapsed_project_folder_ids = value
        .get("collapsed_project_folder_ids")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    preferences.unbound_collapsed = value
        .get("unbound_collapsed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(preferences.unbound_collapsed);
    preferences
}

fn app_preferences_json(preferences: &AppPreferences) -> serde_json::Value {
    serde_json::json!({
        "rtk_enabled": preferences.rtk_enabled,
        "prompt_compression_enabled": preferences.prompt_compression_enabled,
        "prompt_compression_level": preferences.prompt_compression_level.stored_label(),
        "send_shortcut": preferences.send_shortcut.stored_label(),
        "collapsed_project_folder_ids": preferences
            .collapsed_project_folder_ids
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        "unbound_collapsed": preferences.unbound_collapsed,
    })
}

fn gui_locked_command_policy() -> CommandPolicy {
    CommandPolicy {
        allow_prefixes: vec![
            gui_command_prefix(&["rtk", "cargo"]),
            gui_command_prefix(&["rtk", "rg"]),
            gui_command_prefix(&["rtk", "git", "status"]),
        ],
        max_runtime_ms: Some(120_000),
        max_output_bytes: Some(64 * 1024),
        max_concurrent_commands: Some(1),
        ..CommandPolicy::default()
    }
}

fn gui_command_prefix(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

fn gui_default_mcp_server(id: String) -> McpServerConfig {
    McpServerConfig {
        id: id.clone(),
        name: format!("Local RTK MCP {id}"),
        scope: McpConfigScope::Global,
        enabled: true,
        transport: McpTransportConfig::Stdio {
            command: "rtk".to_owned(),
            args: vec!["mcp".to_owned()],
            cwd: None,
            env: BTreeMap::new(),
        },
        timeout_ms: 30_000,
        always_allow_tools: BTreeSet::new(),
        disabled_tools: BTreeSet::new(),
        oauth: None,
    }
}

fn mcp_changes_display(changes: &[McpConfigChange]) -> String {
    if changes.is_empty() {
        return "no hot reload changes".to_owned();
    }
    changes
        .iter()
        .map(|change| match change {
            McpConfigChange::Added(id) => format!("added:{id}"),
            McpConfigChange::Removed(id) => format!("removed:{id}"),
            McpConfigChange::Updated(id) => format!("updated:{id}"),
            McpConfigChange::Enabled(id) => format!("enabled:{id}"),
            McpConfigChange::Disabled(id) => format!("disabled:{id}"),
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn gui_skill_document(
    name: &str,
    description: &str,
    body: &str,
    scope: SkillScope,
    enabled: bool,
) -> SkillDocument {
    SkillDocument {
        manifest: fastrock_skills::SkillManifest {
            name: name.to_owned(),
            description: description.to_owned(),
            path: format!(".fastrock/skills/{name}/SKILL.md"),
            mode_slugs: Vec::new(),
            enabled,
            scope,
        },
        body: body.to_owned(),
    }
}

fn skill_document_source(document: &SkillDocument) -> String {
    let mut source = String::new();
    source.push_str("---\n");
    source.push_str(&format!("name: {}\n", document.manifest.name));
    source.push_str(&format!("description: {}\n", document.manifest.description));
    if !document.manifest.mode_slugs.is_empty() {
        source.push_str(&format!(
            "modes: [{}]\n",
            document.manifest.mode_slugs.join(", ")
        ));
    }
    if !document.manifest.enabled {
        source.push_str("enabled: false\n");
    }
    source.push_str("---\n");
    source.push_str(document.body.trim());
    source.push('\n');
    source
}

fn memory_inspection_source(record: &MemoryRecord) -> String {
    format!(
        "id: {}\nkind: {}\nscope: {}\nsource: {}\nstatus: {}\n\n{}\n",
        record.id,
        memory_kind_display(record.kind),
        memory_scope_display(&record.scope),
        record.source,
        if record.enabled {
            "enabled"
        } else {
            "disabled"
        },
        record.text
    )
}

fn profile_probe_action_status(
    workflow: &FastrockWorkflow,
    action: BedrockGuiProbeAction,
) -> String {
    let Some(profile_id) = workflow.selected_or_default_profile_id() else {
        return "Bedrock probe unavailable: no profile selected".to_owned();
    };
    let Some(profile) = workflow.profiles.profile(&profile_id) else {
        return format!("Bedrock probe unavailable: profile {profile_id} not found");
    };

    match (action, profile.provider) {
        (BedrockGuiProbeAction::DiscoverModels, UiLlmProvider::BedrockMantle) => format!(
            "Model discovery ready: {} via {} /models",
            profile.name,
            profile_endpoint_or(profile, mantle_base_url(profile.region.trim()).as_str())
        ),
        (BedrockGuiProbeAction::DiscoverModels, UiLlmProvider::BedrockRuntime) => format!(
            "Runtime capability discovery ready: {} target {} in {}",
            profile.name,
            runtime_probe_model_id(profile),
            profile.region
        ),
        (BedrockGuiProbeAction::TestConnection, UiLlmProvider::BedrockMantle) => format!(
            "Mantle test connection ready: {} via {}",
            profile.name,
            profile_endpoint_or(profile, mantle_base_url(profile.region.trim()).as_str())
        ),
        (BedrockGuiProbeAction::TestConnection, UiLlmProvider::BedrockRuntime) => format!(
            "Runtime test connection ready: {} via {} /converse",
            profile.name,
            profile_endpoint_or(profile, runtime_endpoint(profile.region.trim()).as_str())
        ),
    }
}

fn gui_profile_write(profile: &LlmProfileForm, now_ms: i64) -> Option<GuiPersistenceWrite> {
    Some(GuiPersistenceWrite::UpsertLlmProfile(LlmProfileRecord {
        id: profile.id.clone(),
        provider: profile_provider_label(profile.provider).to_owned(),
        enabled: profile.enabled,
        document_json: serde_json::to_string(profile).ok()?,
        updated_at_ms: now_ms,
    }))
}

fn gui_all_profile_writes(workflow: &FastrockWorkflow, now_ms: i64) -> Vec<GuiPersistenceWrite> {
    workflow
        .profiles()
        .profiles()
        .iter()
        .filter_map(|profile| gui_profile_write(profile, now_ms))
        .collect()
}

fn gui_mode_changed_write(
    conversation_id: &ConversationId,
    mode: &AgentMode,
    now_ms: i64,
) -> GuiPersistenceWrite {
    GuiPersistenceWrite::AppendConversationEvent(ConversationEvent {
        conversation_id: conversation_id.0.clone(),
        event_type: "conversation.mode_changed".to_owned(),
        payload_json: serde_json::json!({
            "mode": agent_mode_display(mode),
        })
        .to_string(),
        created_at_ms: now_ms,
    })
}

fn gui_transcript_event_write(event: &TranscriptEvent, now_ms: i64) -> Option<GuiPersistenceWrite> {
    Some(GuiPersistenceWrite::AppendConversationEvent(
        ConversationEvent {
            conversation_id: event.conversation_id.0.clone(),
            event_type: transcript_event_type(&event.kind).to_owned(),
            payload_json: serde_json::to_string(event).ok()?,
            created_at_ms: now_ms,
        },
    ))
}

fn gui_touch_conversation_write(
    workflow: &FastrockWorkflow,
    conversation_id: &ConversationId,
    now_ms: i64,
) -> Option<GuiPersistenceWrite> {
    let mut metadata = workflow
        .conversation_metadata
        .get(&conversation_id.0)?
        .clone();
    metadata.updated_at_ms = now_ms;
    if let Some(conversation) = workflow.scheduler().conversation(conversation_id) {
        metadata.status = conversation_status_label(&conversation.status).to_owned();
    }
    Some(GuiPersistenceWrite::UpsertConversation(metadata))
}

fn transcript_event_shell_block(event: &TranscriptEvent) -> Option<WorkflowTranscriptBlock> {
    match &event.kind {
        TranscriptEventKind::UserMessage { text } => Some(WorkflowTranscriptBlock {
            speaker: "user".to_owned(),
            body: text.clone(),
            accent_kind: "user".to_owned(),
        }),
        TranscriptEventKind::AssistantTextDelta { delta } => Some(WorkflowTranscriptBlock {
            speaker: "assistant".to_owned(),
            body: delta.clone(),
            accent_kind: "assistant".to_owned(),
        }),
        TranscriptEventKind::PlanCreated { artifact } => Some(WorkflowTranscriptBlock {
            speaker: "plan".to_owned(),
            body: format!("{} ({:?})", artifact.prompt, artifact.status),
            accent_kind: "goal".to_owned(),
        }),
        TranscriptEventKind::PlanExecuted { plan_id } => Some(WorkflowTranscriptBlock {
            speaker: "plan".to_owned(),
            body: format!("Executed plan {plan_id}"),
            accent_kind: "goal".to_owned(),
        }),
        TranscriptEventKind::ConversationSettingChanged { key, value } => {
            Some(WorkflowTranscriptBlock {
                speaker: "setting".to_owned(),
                body: format!("{key}={value}"),
                accent_kind: "status".to_owned(),
            })
        }
        TranscriptEventKind::Error { error } => Some(WorkflowTranscriptBlock {
            speaker: "error".to_owned(),
            body: fastrock_error_shell_body(error),
            accent_kind: "error".to_owned(),
        }),
        TranscriptEventKind::StatusChanged { status } => Some(WorkflowTranscriptBlock {
            speaker: "status".to_owned(),
            body: format!("Status changed: {status:?}"),
            accent_kind: "status".to_owned(),
        }),
        TranscriptEventKind::ModelRequest { metadata } => Some(WorkflowTranscriptBlock {
            speaker: "model".to_owned(),
            body: format!(
                "{} {} via {}",
                metadata.request_kind, metadata.model_id, metadata.endpoint
            ),
            accent_kind: "model".to_owned(),
        }),
        TranscriptEventKind::ModelStreamEvent { event } => model_stream_event_shell_block(event),
    }
}

fn model_stream_event_shell_block(
    event: &NormalizedModelStreamEvent,
) -> Option<WorkflowTranscriptBlock> {
    match &event.kind {
        NormalizedModelStreamEventKind::TextDelta { delta } => Some(WorkflowTranscriptBlock {
            speaker: "assistant".to_owned(),
            body: delta.clone(),
            accent_kind: "assistant".to_owned(),
        }),
        NormalizedModelStreamEventKind::Started { response_id, role } => {
            Some(WorkflowTranscriptBlock {
                speaker: "model".to_owned(),
                body: format!(
                    "started{}{}",
                    response_id
                        .as_ref()
                        .map(|id| format!(" response={id}"))
                        .unwrap_or_default(),
                    role.as_ref()
                        .map(|role| format!(" role={role}"))
                        .unwrap_or_default()
                ),
                accent_kind: "model".to_owned(),
            })
        }
        NormalizedModelStreamEventKind::Completed => Some(WorkflowTranscriptBlock {
            speaker: "model".to_owned(),
            body: "completed".to_owned(),
            accent_kind: "model".to_owned(),
        }),
        NormalizedModelStreamEventKind::Usage {
            input_tokens,
            output_tokens,
            total_tokens,
        } => Some(WorkflowTranscriptBlock {
            speaker: "model".to_owned(),
            body: format!(
                "usage input={} output={} total={}",
                optional_token_count(*input_tokens),
                optional_token_count(*output_tokens),
                optional_token_count(*total_tokens)
            ),
            accent_kind: "model".to_owned(),
        }),
        NormalizedModelStreamEventKind::Error { message } => Some(WorkflowTranscriptBlock {
            speaker: "error".to_owned(),
            body: message.clone(),
            accent_kind: "error".to_owned(),
        }),
        NormalizedModelStreamEventKind::Raw { event, data } => Some(WorkflowTranscriptBlock {
            speaker: "model".to_owned(),
            body: format!("raw {event}: {data}"),
            accent_kind: "model".to_owned(),
        }),
    }
}

fn optional_token_count(tokens: Option<u64>) -> String {
    tokens
        .map(|tokens| tokens.to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn fastrock_error_shell_body(error: &FastrockError) -> String {
    let retry = if error.retryable {
        "retryable"
    } else {
        "not retryable"
    };
    let context = fastrock_error_context_shell_suffix(&error.context);
    format!(
        "{}: {} ({retry}; source={:?}{}). {} Action: {}",
        error.code, error.summary, error.source, context, error.detail, error.suggested_action
    )
}

fn fastrock_error_context_shell_suffix(context: &FastrockErrorContext) -> String {
    let mut parts = Vec::new();
    if let Some(conversation_id) = &context.conversation_id {
        parts.push(format!("conversation={conversation_id}"));
    }
    if let Some(project_folder_id) = &context.project_folder_id {
        parts.push(format!("project={project_folder_id}"));
    }
    if let Some(profile_id) = &context.profile_id {
        parts.push(format!("profile={profile_id}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("; {}", parts.join("; "))
    }
}

fn gui_conversation_writes(
    conversation_id: &ConversationId,
    title: &str,
    project_folder_id: Option<&str>,
    status: ConversationStatus,
    now_ms: i64,
) -> Vec<GuiPersistenceWrite> {
    let status = conversation_status_label(&status).to_owned();
    let project_folder_json = project_folder_id
        .map(|folder| serde_json::Value::String(folder.to_owned()))
        .unwrap_or(serde_json::Value::Null);
    vec![
        GuiPersistenceWrite::UpsertConversation(ConversationMetadata {
            id: conversation_id.0.clone(),
            title: title.to_owned(),
            project_folder_id: project_folder_id.map(ToOwned::to_owned),
            status: status.clone(),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        }),
        GuiPersistenceWrite::AppendConversationEvent(ConversationEvent {
            conversation_id: conversation_id.0.clone(),
            event_type: "conversation.created".to_owned(),
            payload_json: serde_json::json!({
                "title": title,
                "project_folder_id": project_folder_json,
                "status": status,
            })
            .to_string(),
            created_at_ms: now_ms,
        }),
    ]
}

fn gui_goal_write(
    workflow: &FastrockWorkflow,
    conversation_id: &ConversationId,
    now_ms: i64,
) -> Option<GuiPersistenceWrite> {
    let snapshot = workflow.goal_snapshot(conversation_id)?;
    Some(GuiPersistenceWrite::UpsertGoal(GoalRecord {
        conversation_id: conversation_id.0.clone(),
        status: goal_status_label(&snapshot.status).to_owned(),
        document_json: serde_json::json!({
            "objective": snapshot.objective,
            "status": goal_status_label(&snapshot.status),
            "token_budget": snapshot.token_budget,
            "tokens_used": snapshot.tokens_used,
            "elapsed_ms": snapshot.elapsed_ms,
            "created_at_ms": now_ms,
            "updated_at_ms": now_ms,
            "charged_turn_ids": [],
        })
        .to_string(),
        updated_at_ms: now_ms,
    }))
}

fn gui_editor_tab_writes(
    workflow: &FastrockWorkflow,
    conversation_id: Option<&ConversationId>,
    now_ms: i64,
) -> Vec<GuiPersistenceWrite> {
    let active_path = workflow.editor_tabs().active_path().map(str::to_owned);
    workflow
        .editor_tabs()
        .tabs()
        .iter()
        .map(|tab| {
            GuiPersistenceWrite::UpsertEditorTab(EditorTabRecord {
                path: tab.path.clone(),
                conversation_id: conversation_id.map(|id| id.0.clone()),
                active: active_path.as_deref() == Some(tab.path.as_str()),
                document_json: serde_json::json!({
                    "path": &tab.path,
                    "dirty": tab.dirty,
                    "loaded": true,
                })
                .to_string(),
                updated_at_ms: now_ms,
            })
        })
        .collect()
}

pub async fn load_persisted_workflow(
    persistence: &PersistenceActorHandle,
) -> PersistenceResult<FastrockWorkflow> {
    let client = persistence.client();
    load_persisted_workflow_from_client(&client).await
}

pub async fn load_persisted_workflow_from_client(
    persistence: &PersistenceActorClient,
) -> PersistenceResult<FastrockWorkflow> {
    let mut workflow = FastrockWorkflow::default();
    workflow
        .hydrate_from_persistence_client(persistence)
        .await?;
    Ok(workflow)
}

fn conversation_status_label(status: &ConversationStatus) -> &'static str {
    match status {
        ConversationStatus::Idle => "Idle",
        ConversationStatus::Running => "Running",
        ConversationStatus::WaitingForModel => "WaitingForModel",
        ConversationStatus::WaitingForTool => "WaitingForTool",
        ConversationStatus::Cancelling => "Cancelling",
        ConversationStatus::Paused => "Paused",
        ConversationStatus::Failed => "Failed",
    }
}

fn conversation_status_from_label(value: &str) -> ConversationStatus {
    match value {
        "Running" => ConversationStatus::Running,
        "WaitingForModel" => ConversationStatus::WaitingForModel,
        "WaitingForTool" => ConversationStatus::WaitingForTool,
        "Cancelling" => ConversationStatus::Cancelling,
        "Paused" => ConversationStatus::Paused,
        "Failed" => ConversationStatus::Failed,
        _ => ConversationStatus::Idle,
    }
}

fn is_empty_legacy_demo_conversation(
    metadata: &ConversationMetadata,
    project_folder: Option<&ProjectFolderRecord>,
    stored_events: &[StoredConversationEvent],
) -> bool {
    let Some(project_folder) = project_folder else {
        return false;
    };
    legacy_demo_project_folder_matches(&metadata.title, project_folder)
        && stored_events
            .iter()
            .all(|event| !event.event_type.starts_with("transcript."))
}

fn legacy_demo_project_folder_matches(title: &str, project_folder: &ProjectFolderRecord) -> bool {
    match title {
        "Local folder" => {
            project_folder.label == "Local workspace"
                && project_folder.target_kind == "local"
                && project_folder.path == "."
                && project_folder.remote_target_id.is_none()
        }
        "SSH folder" => {
            project_folder.label == "SSH workspace"
                && project_folder.target_kind == "ssh"
                && project_folder.path == "/srv/app"
                && project_folder.remote_target_id.is_some()
        }
        "SSM folder" => {
            project_folder.label == "SSM workspace"
                && project_folder.target_kind == "ssm"
                && project_folder.path == "/home/ssm-user/app"
                && project_folder.remote_target_id.is_some()
        }
        _ => false,
    }
}

fn transcript_event_type(kind: &TranscriptEventKind) -> &'static str {
    match kind {
        TranscriptEventKind::UserMessage { .. } => "transcript.user_message",
        TranscriptEventKind::AssistantTextDelta { .. } => "transcript.assistant_text_delta",
        TranscriptEventKind::ModelRequest { .. } => "transcript.model_request",
        TranscriptEventKind::ModelStreamEvent { .. } => "transcript.model_stream_event",
        TranscriptEventKind::PlanCreated { .. } => "transcript.plan_created",
        TranscriptEventKind::PlanExecuted { .. } => "transcript.plan_executed",
        TranscriptEventKind::ConversationSettingChanged { .. } => {
            "transcript.conversation_setting_changed"
        }
        TranscriptEventKind::Error { .. } => "transcript.error",
        TranscriptEventKind::StatusChanged { .. } => "transcript.status_changed",
    }
}

fn transcript_event_from_record(
    record: StoredConversationEvent,
) -> Result<TranscriptEvent, TranscriptStoreError> {
    serde_json::from_str(&record.payload_json).map_err(transcript_json_error)
}

fn transcript_event_from_stored_record(
    record: &StoredConversationEvent,
) -> PersistenceResult<TranscriptEvent> {
    serde_json::from_str(&record.payload_json).map_err(|error| {
        PersistenceError::Startup(format!("invalid persisted transcript event: {error}"))
    })
}

fn transcript_persistence_error(error: PersistenceError) -> TranscriptStoreError {
    TranscriptStoreError::Unavailable(error.to_string())
}

fn transcript_json_error(error: serde_json::Error) -> TranscriptStoreError {
    TranscriptStoreError::Unavailable(format!("invalid persisted transcript JSON: {error}"))
}

fn profile_provider_label(provider: UiLlmProvider) -> &'static str {
    match provider {
        UiLlmProvider::BedrockMantle => "bedrock_mantle",
        UiLlmProvider::BedrockRuntime => "bedrock_runtime",
    }
}

fn goal_status_label(status: &ThreadGoalStatus) -> &'static str {
    match status {
        ThreadGoalStatus::Active => "Active",
        ThreadGoalStatus::Paused => "Paused",
        ThreadGoalStatus::Blocked => "Blocked",
        ThreadGoalStatus::UsageLimited => "UsageLimited",
        ThreadGoalStatus::BudgetLimited => "BudgetLimited",
        ThreadGoalStatus::Complete => "Complete",
    }
}

fn goal_status_from_label(value: &str) -> ThreadGoalStatus {
    match value {
        "Paused" => ThreadGoalStatus::Paused,
        "Blocked" => ThreadGoalStatus::Blocked,
        "UsageLimited" => ThreadGoalStatus::UsageLimited,
        "BudgetLimited" => ThreadGoalStatus::BudgetLimited,
        "Complete" => ThreadGoalStatus::Complete,
        _ => ThreadGoalStatus::Active,
    }
}

fn profile_from_record(record: LlmProfileRecord) -> PersistenceResult<LlmProfileForm> {
    let profile: LlmProfileForm =
        serde_json::from_str(&record.document_json).map_err(json_persistence_error)?;
    if profile.id != record.id {
        return Err(PersistenceError::Startup(format!(
            "persisted profile id mismatch: record={} document={}",
            record.id, profile.id
        )));
    }
    Ok(profile)
}

fn local_project_metadata_snapshot(
    folder_id: &str,
    metadata: LocalProjectMetadata,
) -> LocalProjectMetadataSnapshot {
    LocalProjectMetadataSnapshot {
        folder_id: folder_id.to_owned(),
        label: metadata.label,
        root: path_to_ui_string(metadata.root),
        settings_path: path_to_ui_string(metadata.settings_path),
        skills_dir: path_to_ui_string(metadata.skills_dir),
        mcp_config_path: path_to_ui_string(metadata.mcp_config_path),
        compatible_mcp_config_path: path_to_ui_string(metadata.compatible_mcp_config_path),
        rtk_status: rtk_status_label(metadata.rtk_diagnostic.status).to_owned(),
        rtk_detail: metadata.rtk_diagnostic.detail,
        rtk_resolved_path: metadata.rtk_diagnostic.resolved_path.map(path_to_ui_string),
        instructions: metadata
            .instructions
            .into_iter()
            .map(|instruction| ProjectInstructionSnapshot {
                kind: project_instruction_snapshot_kind(instruction.kind),
                path: path_to_ui_string(instruction.path),
                contents: instruction.contents,
            })
            .collect(),
        git: metadata.git.map(|git| GitMetadataSnapshot {
            worktree_root: path_to_ui_string(git.worktree_root),
            git_dir: path_to_ui_string(git.git_dir),
        }),
        default_profile_id: metadata.default_profile_id,
        recent_conversation_ids: metadata.recent_conversation_ids,
    }
}

fn rtk_status_label(status: RtkBinaryStatus) -> &'static str {
    match status {
        RtkBinaryStatus::Available => "available",
        RtkBinaryStatus::Missing => "missing",
    }
}

fn project_instruction_snapshot_kind(
    kind: ProjectInstructionKind,
) -> ProjectInstructionSnapshotKind {
    match kind {
        ProjectInstructionKind::Agents => ProjectInstructionSnapshotKind::Agents,
        ProjectInstructionKind::Claude => ProjectInstructionSnapshotKind::Claude,
        ProjectInstructionKind::RooRules => ProjectInstructionSnapshotKind::RooRules,
    }
}

fn path_to_ui_string(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().into_owned()
}

fn project_default_profile_id(record: &ProjectFolderRecord) -> PersistenceResult<Option<String>> {
    let document = project_folder_document(record)?;
    match document.get(PROJECT_DOCUMENT_DEFAULT_PROFILE_ID_KEY) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(profile_id)) => Ok(Some(profile_id.clone())),
        Some(_) => Err(project_folder_document_error(
            record,
            "`default_profile_id` must be a string",
        )),
    }
}

fn project_recent_conversation_ids(record: &ProjectFolderRecord) -> PersistenceResult<Vec<String>> {
    let document = project_folder_document(record)?;
    project_recent_conversation_ids_from_document(record, &document)
}

fn project_folder_watch_enabled(record: &ProjectFolderRecord) -> PersistenceResult<bool> {
    let document = project_folder_document(record)?;
    match document.get("watch") {
        None | Some(serde_json::Value::Null) => Ok(true),
        Some(serde_json::Value::Bool(enabled)) => Ok(*enabled),
        Some(_) => Err(project_folder_document_error(
            record,
            "`watch` must be a boolean",
        )),
    }
}

fn update_project_recent_conversation_document(
    record: &mut ProjectFolderRecord,
    conversation_id: &str,
    now_ms: i64,
) -> PersistenceResult<bool> {
    let mut document = project_folder_document(record)?;
    let mut recent = project_recent_conversation_ids_from_document(record, &document)?;
    let before = recent.clone();
    record_project_recent_conversation(
        &mut recent,
        conversation_id,
        MAX_PROJECT_RECENT_CONVERSATIONS,
    );
    if recent == before {
        return Ok(false);
    }

    let object = document
        .as_object_mut()
        .expect("project_folder_document returns an object");
    object.insert(
        PROJECT_DOCUMENT_RECENT_CONVERSATION_IDS_KEY.to_owned(),
        serde_json::Value::Array(
            recent
                .into_iter()
                .map(serde_json::Value::String)
                .collect::<Vec<_>>(),
        ),
    );
    record.document_json = serde_json::to_string(&document).map_err(json_persistence_error)?;
    record.updated_at_ms = now_ms;
    Ok(true)
}

fn project_folder_document(record: &ProjectFolderRecord) -> PersistenceResult<serde_json::Value> {
    if record.document_json.trim().is_empty() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }

    let document: serde_json::Value =
        serde_json::from_str(&record.document_json).map_err(json_persistence_error)?;
    if !document.is_object() {
        return Err(project_folder_document_error(
            record,
            "document_json must be a JSON object",
        ));
    }
    Ok(document)
}

fn project_recent_conversation_ids_from_document(
    record: &ProjectFolderRecord,
    document: &serde_json::Value,
) -> PersistenceResult<Vec<String>> {
    match document.get(PROJECT_DOCUMENT_RECENT_CONVERSATION_IDS_KEY) {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .map(|value| match value {
                serde_json::Value::String(id) => Ok(id.clone()),
                _ => Err(project_folder_document_error(
                    record,
                    "`recent_conversation_ids` must contain only strings",
                )),
            })
            .collect(),
        Some(_) => Err(project_folder_document_error(
            record,
            "`recent_conversation_ids` must be an array",
        )),
    }
}

fn goal_state_from_record(record: &GoalRecord) -> PersistenceResult<GoalLoopState> {
    let json: serde_json::Value =
        serde_json::from_str(&record.document_json).map_err(json_persistence_error)?;
    let status = json
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(goal_status_from_label)
        .unwrap_or_else(|| goal_status_from_label(&record.status));
    let charged_turn_ids = charged_turn_ids_from_goal_document(record, &json)?;
    Ok(GoalLoopState::from_persisted_goal_with_charged_turn_ids(
        ThreadGoal {
            objective: json
                .get("objective")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            status,
            token_budget: json.get("token_budget").and_then(serde_json::Value::as_u64),
            tokens_used: json
                .get("tokens_used")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            elapsed_ms: json
                .get("elapsed_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            created_at_ms: json
                .get("created_at_ms")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(record.updated_at_ms),
            updated_at_ms: json
                .get("updated_at_ms")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(record.updated_at_ms),
        },
        charged_turn_ids,
    ))
}

fn latest_conversation_mode(
    events: &[StoredConversationEvent],
) -> PersistenceResult<Option<AgentMode>> {
    events
        .iter()
        .filter(|event| event.event_type == "conversation.mode_changed")
        .try_fold(None, |_, event| {
            let payload: serde_json::Value =
                serde_json::from_str(&event.payload_json).map_err(json_persistence_error)?;
            let mode = payload
                .get("mode")
                .and_then(serde_json::Value::as_str)
                .and_then(agent_mode_from_label)
                .ok_or_else(|| {
                    PersistenceError::Startup(format!(
                        "invalid persisted conversation mode event {}",
                        event.id
                    ))
                })?;
            Ok(Some(mode))
        })
}

fn charged_turn_ids_from_goal_document(
    record: &GoalRecord,
    document: &serde_json::Value,
) -> PersistenceResult<BTreeSet<String>> {
    match document.get("charged_turn_ids") {
        None | Some(serde_json::Value::Null) => Ok(BTreeSet::new()),
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .map(|value| match value {
                serde_json::Value::String(id) => Ok(id.clone()),
                _ => Err(PersistenceError::Startup(format!(
                    "invalid persisted workflow JSON: charged_turn_ids for goal {} must contain only strings",
                    record.conversation_id
                ))),
            })
            .collect(),
        Some(_) => Err(PersistenceError::Startup(format!(
            "invalid persisted workflow JSON: charged_turn_ids for goal {} must be an array",
            record.conversation_id
        ))),
    }
}

fn json_persistence_error(error: serde_json::Error) -> PersistenceError {
    PersistenceError::Startup(format!("invalid persisted workflow JSON: {error}"))
}

fn validation_persistence_error(
    errors: Vec<fastrock_ui::ProfileValidationError>,
) -> PersistenceError {
    let messages = errors
        .into_iter()
        .map(|error| format!("{}: {}", error.field, error.message))
        .collect::<Vec<_>>()
        .join(", ");
    PersistenceError::Startup(format!("invalid persisted profile settings: {messages}"))
}

fn mcp_config_persistence_error(error: McpConfigError) -> PersistenceError {
    PersistenceError::Startup(format!("invalid persisted MCP config: {error}"))
}

fn cache_metadata_persistence_error(message: String) -> PersistenceError {
    PersistenceError::Startup(format!("invalid persisted cache metadata: {message}"))
}

fn project_metadata_persistence_error(error: fastrock_projects::ProjectError) -> PersistenceError {
    PersistenceError::Startup(format!("invalid project metadata: {error}"))
}

fn project_folder_document_error(record: &ProjectFolderRecord, message: &str) -> PersistenceError {
    PersistenceError::Startup(format!(
        "invalid project folder document for {}: {message}",
        record.id
    ))
}

fn project_target_kind(record: &ProjectFolderRecord) -> UiProjectTargetKind {
    match record.target_kind.as_str() {
        "local" => UiProjectTargetKind::Local,
        "ssh" => UiProjectTargetKind::Ssh,
        "ssm" | "aws_session_manager" => UiProjectTargetKind::AwsSessionManager,
        _ => UiProjectTargetKind::Remote,
    }
}

fn project_target_label(record: Option<&ProjectFolderRecord>) -> String {
    let Some(record) = record else {
        return "local".to_owned();
    };
    match (
        record.target_kind.as_str(),
        record.remote_target_id.as_deref(),
    ) {
        ("local", _) => "local".to_owned(),
        ("ssh", Some(remote_target_id)) => format!("ssh:{remote_target_id}"),
        ("ssh", None) => "ssh".to_owned(),
        ("ssm" | "aws_session_manager", Some(remote_target_id)) => {
            format!("ssm:{remote_target_id}")
        }
        ("ssm" | "aws_session_manager", None) => "ssm".to_owned(),
        (_, Some(remote_target_id)) => format!("remote:{remote_target_id}"),
        _ => "remote".to_owned(),
    }
}

fn project_folder_target_display(record: &ProjectFolderRecord) -> String {
    match (
        record.target_kind.as_str(),
        record.remote_target_id.as_deref(),
    ) {
        ("local", _) => "local".to_owned(),
        ("ssh", Some(remote_target_id)) => format!("ssh:{remote_target_id}"),
        ("ssh", None) => "ssh".to_owned(),
        ("ssm" | "aws_session_manager", Some(remote_target_id)) => {
            format!("ssm:{remote_target_id}")
        }
        ("ssm" | "aws_session_manager", None) => "ssm".to_owned(),
        (kind, Some(remote_target_id)) => format!("{kind}:{remote_target_id}"),
        (kind, None) => kind.to_owned(),
    }
}

fn mcp_scope_display(scope: &McpConfigScope) -> String {
    match scope {
        McpConfigScope::Global => "global".to_owned(),
        McpConfigScope::Project { project_folder_id } => format!("project:{project_folder_id}"),
    }
}

fn mcp_transport_display(transport: &McpTransportConfig) -> String {
    match transport {
        McpTransportConfig::Stdio { command, .. } => format!("stdio:{command}"),
        McpTransportConfig::StreamableHttp { url, .. } => format!("http:{url}"),
    }
}

fn mcp_oauth_state_display(state: &McpOAuthState) -> &'static str {
    match state {
        McpOAuthState::NotConfigured => "oauth none",
        McpOAuthState::AuthorizationRequired => "oauth required",
        McpOAuthState::Authorized { .. } => "oauth authorized",
        McpOAuthState::Expired { .. } => "oauth expired",
        McpOAuthState::Failed(_) => "oauth failed",
    }
}

fn workflow_profile_row(
    profile: &LlmProfileForm,
    selected: bool,
    validation: &str,
) -> WorkflowProfileRow {
    WorkflowProfileRow {
        profile_id: profile.id.clone(),
        name: profile.name.clone(),
        provider: profile_provider_display(profile.provider).to_owned(),
        model: profile.model_id.clone(),
        region: profile.region.clone(),
        enabled: profile.enabled,
        default_for_new_conversations: profile.default_for_new_conversations,
        selected,
        validation: validation.to_owned(),
    }
}

fn skill_scope_display(scope: &SkillScope) -> String {
    match scope {
        SkillScope::Global => "global".to_owned(),
        SkillScope::Project(project_folder_id) => format!("project:{project_folder_id}"),
    }
}

fn memory_kind_display(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Observation => "observation",
        MemoryKind::ProjectFact => "project fact",
        MemoryKind::UserPreference => "user preference",
        MemoryKind::PriorFix => "prior fix",
    }
}

fn memory_scope_display(scope: &MemoryScope) -> String {
    match scope {
        MemoryScope::Global => "global".to_owned(),
        MemoryScope::Project { project_folder_id } => format!("project:{project_folder_id}"),
    }
}

fn memory_preview(text: &str) -> String {
    let mut preview = text
        .split_whitespace()
        .take(24)
        .collect::<Vec<_>>()
        .join(" ");
    if preview.len() < text.trim().len() {
        preview.push_str("...");
    }
    preview
}

fn agent_mode_display(mode: &AgentMode) -> &str {
    match mode {
        AgentMode::Code => "Code",
        AgentMode::Architect => "Architect",
        AgentMode::Ask => "Ask",
        AgentMode::Debug => "Debug",
        AgentMode::Plan => "Plan",
        AgentMode::Custom(slug) => slug.as_str(),
    }
}

fn agent_mode_from_label(value: &str) -> Option<AgentMode> {
    match value {
        "Code" => Some(AgentMode::Code),
        "Architect" => Some(AgentMode::Architect),
        "Ask" => Some(AgentMode::Ask),
        "Debug" => Some(AgentMode::Debug),
        "Plan" => Some(AgentMode::Plan),
        custom if !custom.trim().is_empty() => Some(AgentMode::Custom(custom.to_owned())),
        _ => None,
    }
}

fn shell_folder_label(row: &ConversationListRow) -> String {
    if row.target_label.is_empty() || row.target_label == "local" {
        row.folder_label.clone()
    } else {
        format!("{} · {}", row.folder_label, row.target_label)
    }
}

fn trim_editor_line_for_shell(line: String) -> String {
    line.trim_end_matches(['\r', '\n']).to_owned()
}

fn project_event_path(event: &ProjectFileEvent) -> Result<&Path, ProjectHotReloadError> {
    let path = match event {
        ProjectFileEvent::Created { path }
        | ProjectFileEvent::Modified { path }
        | ProjectFileEvent::Deleted { path } => path.as_path(),
    };
    validate_project_event_path(path)?;
    Ok(path)
}

fn validate_project_event_path(path: &Path) -> Result<(), ProjectHotReloadError> {
    let mut saw_component = false;
    for component in path.components() {
        saw_component = true;
        if !matches!(component, Component::Normal(_)) {
            return Err(ProjectHotReloadError::InvalidEventPath(
                path.display().to_string(),
            ));
        }
    }
    if saw_component {
        Ok(())
    } else {
        Err(ProjectHotReloadError::InvalidEventPath(
            path.display().to_string(),
        ))
    }
}

fn is_project_mcp_event_path(path: &Path) -> bool {
    path == Path::new(PROJECT_MCP_RELATIVE_PATH) || path == Path::new(COMPAT_MCP_RELATIVE_PATH)
}

fn is_project_skill_event_path(path: &Path) -> bool {
    path.starts_with(PROJECT_SKILLS_RELATIVE_PATH)
        && path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
}

fn project_scoped_mcp_config(
    folder_id: &str,
    mut config: McpConfigSet,
) -> Result<McpConfigSet, McpConfigError> {
    for server in &mut config.servers {
        server.scope = McpConfigScope::Project {
            project_folder_id: folder_id.to_owned(),
        };
    }
    config.validate()?;
    Ok(config)
}

fn shell_settings_row(name: impl Into<String>, value: impl Into<String>) -> WorkflowSettingsRow {
    WorkflowSettingsRow {
        name: name.into(),
        value: value.into(),
    }
}

fn command_policy_display(policy: &CommandPolicy) -> String {
    if !policy.commands_enabled {
        return "commands disabled".to_owned();
    }
    let mut parts = vec!["commands enabled".to_owned()];
    if !policy.allow_prefixes.is_empty() {
        parts.push(format!(
            "allowlist {} prefixes",
            policy.allow_prefixes.len()
        ));
    }
    if !policy.deny_exact.is_empty() || !policy.deny_prefixes.is_empty() {
        parts.push(format!(
            "denylist {} exact / {} prefixes",
            policy.deny_exact.len(),
            policy.deny_prefixes.len()
        ));
    }
    if let Some(max_runtime_ms) = policy.max_runtime_ms {
        parts.push(format!("runtime cap {max_runtime_ms} ms"));
    }
    if let Some(max_output_bytes) = policy.max_output_bytes {
        parts.push(format!("output cap {max_output_bytes} bytes"));
    }
    if let Some(max_concurrent_commands) = policy.max_concurrent_commands {
        parts.push(format!("concurrency cap {max_concurrent_commands}"));
    }
    if policy.label_outside_project_mutations {
        parts.push("outside-project labels on".to_owned());
    }
    parts.join("; ")
}

fn request_tuning_display(tuning: &fastrock_ui::UiRequestTuning) -> String {
    let mut parts = Vec::new();
    if let Some(tokens) = tuning.max_output_tokens {
        parts.push(format!("max {tokens}"));
    }
    if let Some(temperature) = tuning.temperature_milli {
        parts.push(format!("temp {:.3}", f64::from(temperature) / 1000.0));
    }
    if let Some(top_p) = tuning.top_p_milli {
        parts.push(format!("top-p {:.3}", f64::from(top_p) / 1000.0));
    }
    if let Some(timeout_ms) = tuning.timeout_ms {
        parts.push(format!("timeout {timeout_ms} ms"));
    }
    if let Some(attempts) = tuning.retry_max_attempts {
        parts.push(format!("retry {attempts}"));
    }
    if parts.is_empty() {
        "provider defaults".to_owned()
    } else {
        parts.join(", ")
    }
}

fn default_profile_name(provider: UiLlmProvider) -> &'static str {
    match provider {
        UiLlmProvider::BedrockMantle => "Bedrock Mantle",
        UiLlmProvider::BedrockRuntime => "Bedrock Runtime",
    }
}

fn unique_profile_name(profiles: &[LlmProfileForm], base: &str) -> String {
    unique_profile_name_with_suffix(profiles, base, "")
}

fn unique_profile_name_excluding(
    profiles: &[LlmProfileForm],
    current_name: &str,
    suffix: &str,
) -> String {
    unique_profile_name_with_suffix(profiles, current_name, suffix)
}

fn unique_profile_name_with_suffix(
    profiles: &[LlmProfileForm],
    base: &str,
    suffix: &str,
) -> String {
    let candidate_base = format!("{}{}", base.trim(), suffix);
    if profile_name_is_available(profiles, &candidate_base) {
        return candidate_base;
    }
    (2..)
        .map(|index| format!("{candidate_base} {index}"))
        .find(|candidate| profile_name_is_available(profiles, candidate))
        .expect("unbounded iterator returns a profile name")
}

fn profile_name_is_available(profiles: &[LlmProfileForm], candidate: &str) -> bool {
    !profiles
        .iter()
        .any(|profile| profile.name.eq_ignore_ascii_case(candidate))
}

fn aws_cli_profile_summaries(shared_config: &AwsSharedConfig) -> Vec<AwsCliProfileSummary> {
    shared_config
        .profile_names()
        .filter_map(|name| {
            shared_config
                .profile(name)
                .map(|profile| AwsCliProfileSummary {
                    name: name.to_owned(),
                    region: profile.region.clone(),
                    source_type: aws_profile_source_type(profile).to_owned(),
                    can_resolve_credentials: profile.can_resolve_credentials(),
                })
        })
        .collect()
}

fn credential_preview_for_source(
    shared_config: &AwsSharedConfig,
    source: &UiCredentialSource,
    fastrock_region: &str,
) -> (UiAwsCredentialPreview, bool) {
    let region_source = Some(format!("fastrock_profile:{fastrock_region}"));
    match source {
        UiCredentialSource::AwsCliProfile { profile_name } => {
            let Some(profile) = shared_config.profile(profile_name) else {
                return (
                    UiAwsCredentialPreview {
                        source_type: "missing_profile".to_owned(),
                        profile_name: Some(profile_name.clone()),
                        region_source,
                        account_identity: None,
                        expiration: None,
                    },
                    false,
                );
            };
            (
                UiAwsCredentialPreview {
                    source_type: aws_profile_source_type(profile).to_owned(),
                    profile_name: Some(profile_name.clone()),
                    region_source: Some(profile.region.as_deref().map_or_else(
                        || format!("fastrock_profile:{fastrock_region}"),
                        |region| format!("fastrock_profile:{fastrock_region};aws_profile:{region}"),
                    )),
                    account_identity: None,
                    expiration: None,
                },
                true,
            )
        }
        UiCredentialSource::DefaultChain => (
            UiAwsCredentialPreview {
                source_type: "default_chain".to_owned(),
                profile_name: None,
                region_source,
                account_identity: None,
                expiration: None,
            },
            true,
        ),
        UiCredentialSource::Environment => (
            UiAwsCredentialPreview {
                source_type: "environment".to_owned(),
                profile_name: None,
                region_source,
                account_identity: None,
                expiration: None,
            },
            true,
        ),
        UiCredentialSource::SecretRef { secret_ref } => (
            UiAwsCredentialPreview {
                source_type: "explicit_static_secret_ref".to_owned(),
                profile_name: Some(secret_ref.clone()),
                region_source,
                account_identity: None,
                expiration: None,
            },
            true,
        ),
    }
}

fn resolve_profile_auth(
    profile: &LlmProfileForm,
    shared_config: &AwsSharedConfig,
) -> Result<fastrock_bedrock::ResolvedBedrockAuth, BedrockProfileProbeError> {
    shared_config
        .resolve_bedrock_auth(
            &aws_credential_source_from_ui(&profile.credential_source),
            Some(profile.region.trim()),
        )
        .map_err(|error| BedrockProfileProbeError::AwsConfig(error.to_string()))
}

fn mantle_probe_auth(
    profile: &LlmProfileForm,
    shared_config: &AwsSharedConfig,
    bearer_api_key: Option<String>,
) -> Result<MantleClientAuth, BedrockProfileProbeError> {
    match &profile.mantle_settings.auth_mode {
        UiMantleAuthMode::AwsSigV4 => Ok(MantleClientAuth::AwsSigV4 {
            resolved_auth: resolve_profile_auth(profile, shared_config)?,
            project_id: profile.mantle_settings.project_id.clone(),
        }),
        UiMantleAuthMode::BearerApiKey { secret_ref } => {
            let api_key =
                bearer_api_key.ok_or_else(|| BedrockProfileProbeError::MissingBearerApiKey {
                    profile_id: profile.id.clone(),
                    secret_ref: secret_ref.clone(),
                })?;
            Ok(MantleClientAuth::BearerApiKey {
                api_key,
                project_id: profile.mantle_settings.project_id.clone(),
            })
        }
    }
}

async fn run_mantle_model_message<T, S>(
    request: BedrockModelRunRequest,
    profile: LlmProfileForm,
    shared_config: &AwsSharedConfig,
    bearer_api_key: Option<String>,
    transport: T,
    signer: S,
) -> Result<BedrockModelRunResult, BedrockModelRunError>
where
    T: MantleHttpTransport,
    S: MantleSigV4Signer,
{
    let endpoint = profile_endpoint_or(&profile, mantle_base_url(profile.region.trim()).as_str());
    let api_shape = mantle_api_shape_for_run(profile.mantle_settings.api_shape);
    let auth = mantle_probe_auth(&profile, shared_config, bearer_api_key)
        .map_err(BedrockModelRunError::Profile)?;
    let client = MantleClient::new(endpoint.clone(), auth, transport, signer);
    let model_message = model_facing_message(&request);
    // SPEC §V5: Bedrock Mantle store is permanently disabled. Every request sends
    // store=false so AWS never retains Fastrock response state, and the stored-
    // response policy drops any previous_response_id chaining.
    let store = false;
    let stored_response_policy =
        MantleStoredResponsePolicy::new(profile.mantle_settings.project_id.clone(), store);
    let stored_response_state = MantleStoredResponseState {
        project_id: profile.mantle_settings.project_id.clone(),
        last_response_id: request.mantle_previous_response_id.clone(),
    };
    let events = match api_shape {
        MantleApiShape::Responses => {
            client
                .stream_response(stored_response_policy.prepare_request(
                    MantleResponseRequest {
                        model: profile.model_id.clone(),
                        input: model_message.clone(),
                        previous_response_id: None,
                        store,
                        stream: false,
                        max_output_tokens: profile.request_tuning.max_output_tokens,
                        temperature: request_tuning_temperature(&profile.request_tuning),
                        top_p: request_tuning_top_p(&profile.request_tuning),
                    },
                    &stored_response_state,
                ))
                .await
        }
        MantleApiShape::ChatCompletions => {
            client
                .stream_chat_completion(MantleChatCompletionRequest {
                    model: profile.model_id.clone(),
                    messages: vec![MantleChatMessage {
                        role: "user".to_owned(),
                        content: model_message.clone(),
                    }],
                    store: Some(store),
                    stream: false,
                    max_tokens: profile.request_tuning.max_output_tokens,
                    temperature: request_tuning_temperature(&profile.request_tuning),
                    top_p: request_tuning_top_p(&profile.request_tuning),
                })
                .await
        }
        MantleApiShape::AnthropicMessages => {
            client
                .stream_anthropic_message(MantleAnthropicMessagesRequest {
                    model: profile.model_id.clone(),
                    messages: vec![MantleChatMessage {
                        role: "user".to_owned(),
                        content: model_message.clone(),
                    }],
                    max_tokens: profile
                        .request_tuning
                        .max_output_tokens
                        .map(u64::from)
                        .unwrap_or(4096),
                    stream: false,
                    temperature: request_tuning_temperature(&profile.request_tuning),
                    top_p: request_tuning_top_p(&profile.request_tuning),
                })
                .await
        }
    }
    .map_err(mantle_model_run_error)?;
    let profile_id = profile.id;
    let model_id = profile.model_id;
    let region = profile.region;
    let project_id = profile.mantle_settings.project_id;

    Ok(BedrockModelRunResult {
        conversation_id: request.conversation_id,
        metadata: ModelRequestMetadata {
            provider: ModelProviderKind::BedrockMantle,
            profile_id: Some(profile_id),
            model_id,
            endpoint: format!("{}{}", endpoint, api_shape.endpoint_path()),
            region: Some(region),
            project_id,
            api_shape: Some(mantle_api_shape_wire_label(api_shape).to_owned()),
            store: Some(store),
            streaming: true,
            request_kind: mantle_request_kind(api_shape).to_owned(),
        },
        events: normalize_mantle_stream_events(events),
    })
}

async fn run_runtime_model_message<T, S>(
    request: BedrockModelRunRequest,
    profile: LlmProfileForm,
    shared_config: &AwsSharedConfig,
    transport: T,
    signer: S,
) -> Result<BedrockModelRunResult, BedrockModelRunError>
where
    T: RuntimeHttpTransport,
    S: RuntimeSigV4Signer,
{
    let endpoint = profile_endpoint_or(&profile, runtime_endpoint(profile.region.trim()).as_str());
    let resolved_auth =
        resolve_profile_auth(&profile, shared_config).map_err(BedrockModelRunError::Profile)?;
    let model_id = runtime_probe_model_id(&profile);
    let client = BedrockRuntimeClient::new(endpoint.clone(), resolved_auth, transport, signer);
    let model_message = model_facing_message(&request);
    let response = client
        .converse_stream_with_invoke_fallback(
            runtime_user_message_request(
                &model_id,
                &model_message,
                &profile.runtime_settings,
                &profile.request_tuning,
            ),
            runtime_invoke_fallback_body(&model_message, &profile.request_tuning),
        )
        .await
        .map_err(runtime_model_run_error)?;
    let events = runtime_model_response_events(response);
    let profile_id = profile.id;
    let region = profile.region;

    Ok(BedrockModelRunResult {
        conversation_id: request.conversation_id,
        metadata: ModelRequestMetadata {
            provider: ModelProviderKind::BedrockRuntime,
            profile_id: Some(profile_id),
            model_id: model_id.clone(),
            endpoint: format!("{}/model/{model_id}/converse-stream", endpoint),
            region: Some(region),
            project_id: None,
            api_shape: None,
            store: None,
            streaming: true,
            request_kind: "converse_stream_with_invoke_fallback".to_owned(),
        },
        events,
    })
}

fn aws_credential_source_from_ui(source: &UiCredentialSource) -> AwsCredentialSource {
    match source {
        UiCredentialSource::DefaultChain => AwsCredentialSource::DefaultChain,
        UiCredentialSource::AwsCliProfile { profile_name } => AwsCredentialSource::CliProfile {
            profile_name: profile_name.clone(),
        },
        UiCredentialSource::Environment => AwsCredentialSource::Environment,
        UiCredentialSource::SecretRef { secret_ref } => AwsCredentialSource::ExplicitStaticSecret {
            secret_ref: secret_ref.clone(),
        },
    }
}

fn profile_endpoint_or(profile: &LlmProfileForm, default_endpoint: &str) -> String {
    profile
        .endpoint_override
        .as_deref()
        .map(str::trim)
        .filter(|endpoint| !endpoint.is_empty())
        .unwrap_or(default_endpoint)
        .to_owned()
}

fn discovered_model_from_mantle(model: fastrock_bedrock::MantleModel) -> BedrockDiscoveredModel {
    BedrockDiscoveredModel {
        id: model.id,
        owned_by: model.owned_by,
        api_shapes: model
            .api_shapes
            .into_iter()
            .map(ui_mantle_api_shape_from_bedrock)
            .collect(),
    }
}

fn ui_mantle_api_shape_from_bedrock(shape: MantleApiShape) -> UiMantleApiShape {
    match shape {
        MantleApiShape::Responses => UiMantleApiShape::Responses,
        MantleApiShape::ChatCompletions => UiMantleApiShape::ChatCompletions,
        MantleApiShape::AnthropicMessages => UiMantleApiShape::AnthropicMessages,
    }
}

fn runtime_probe_model_id(profile: &LlmProfileForm) -> String {
    let target = profile.runtime_settings.target.trim();
    if target.is_empty() {
        profile.model_id.clone()
    } else {
        target.to_owned()
    }
}

fn model_facing_message(request: &BedrockModelRunRequest) -> String {
    if !request.prompt_compression_enabled {
        return request.message.clone();
    }
    let level = PromptCompressionLevel::from_stored_label(&request.prompt_compression_level)
        .unwrap_or(PromptCompressionLevel::Full);
    let style = match level {
        PromptCompressionLevel::Lite => {
            "Use concise technical prose. Preserve identifiers, paths, commands, and exact errors."
        }
        PromptCompressionLevel::Full => {
            "Use terse technical prose. Drop filler. Preserve identifiers, paths, commands, exact errors, and decisions."
        }
        PromptCompressionLevel::Ultra => {
            "Use maximally terse technical fragments. Preserve identifiers, paths, commands, exact errors, invariants, and decisions."
        }
    };
    format!(
        "Fastrock prompt compression is enabled at {} level. {style}\n\nUser message:\n{}",
        level.stored_label(),
        request.message
    )
}

fn runtime_connection_probe_request(model_id: &str) -> RuntimeConverseRequest {
    RuntimeConverseRequest {
        model_id: model_id.to_owned(),
        additional_model_request_fields: None,
        inference_config: None,
        system: vec![RuntimeSystemContentBlock::Text {
            text: "Fastrock connection probe. Respond briefly.".to_owned(),
        }],
        messages: vec![RuntimeMessage {
            role: "user".to_owned(),
            content: vec![RuntimeContentBlock::Text {
                text: "Return ok.".to_owned(),
            }],
        }],
        service_tier: None,
        tool_config: None,
    }
}

fn runtime_user_message_request(
    model_id: &str,
    message: &str,
    settings: &fastrock_ui::UiRuntimeProfileSettings,
    request_tuning: &fastrock_ui::UiRequestTuning,
) -> RuntimeConverseRequest {
    RuntimeConverseRequest {
        model_id: model_id.to_owned(),
        additional_model_request_fields: runtime_additional_model_request_fields(settings),
        inference_config: runtime_inference_configuration(request_tuning),
        system: Vec::new(),
        messages: vec![RuntimeMessage {
            role: "user".to_owned(),
            content: runtime_user_content_blocks(message, settings),
        }],
        service_tier: runtime_service_tier(settings),
        tool_config: None,
    }
}

fn runtime_inference_configuration(
    request_tuning: &fastrock_ui::UiRequestTuning,
) -> Option<RuntimeInferenceConfiguration> {
    let inference_config = RuntimeInferenceConfiguration {
        max_tokens: request_tuning.max_output_tokens,
        temperature: request_tuning_temperature(request_tuning),
        top_p: request_tuning_top_p(request_tuning),
    };
    (inference_config.max_tokens.is_some()
        || inference_config.temperature.is_some()
        || inference_config.top_p.is_some())
    .then_some(inference_config)
}

fn request_tuning_temperature(
    request_tuning: &fastrock_ui::UiRequestTuning,
) -> Option<BedrockFloatMilli> {
    request_tuning.temperature_milli.map(BedrockFloatMilli::new)
}

fn request_tuning_top_p(
    request_tuning: &fastrock_ui::UiRequestTuning,
) -> Option<BedrockFloatMilli> {
    request_tuning.top_p_milli.map(BedrockFloatMilli::new)
}

fn runtime_user_content_blocks(
    message: &str,
    settings: &fastrock_ui::UiRuntimeProfileSettings,
) -> Vec<RuntimeContentBlock> {
    let mut content = vec![RuntimeContentBlock::Text {
        text: message.to_owned(),
    }];
    if let Some(ttl_seconds) = settings.prompt_cache_ttl_seconds {
        content.push(RuntimeContentBlock::CachePoint {
            cache_point: RuntimeCachePointBlock {
                cache_point_type: "default".to_owned(),
                ttl: Some(runtime_cache_point_ttl(ttl_seconds)),
            },
        });
    }
    content
}

fn runtime_cache_point_ttl(ttl_seconds: u64) -> String {
    if ttl_seconds >= 3600 {
        "1h".to_owned()
    } else {
        "5m".to_owned()
    }
}

fn runtime_additional_model_request_fields(
    settings: &fastrock_ui::UiRuntimeProfileSettings,
) -> Option<serde_json::Value> {
    settings.reasoning_budget_tokens.map(|budget_tokens| {
        serde_json::json!({
            "thinking": {
                "type": "enabled",
                "budget_tokens": budget_tokens
            }
        })
    })
}

fn runtime_service_tier(
    settings: &fastrock_ui::UiRuntimeProfileSettings,
) -> Option<RuntimeServiceTier> {
    let tier_type = settings.service_tier.as_deref()?.trim();
    (!tier_type.is_empty()).then(|| RuntimeServiceTier {
        tier_type: tier_type.to_owned(),
    })
}

fn runtime_invoke_fallback_body(
    message: &str,
    request_tuning: &fastrock_ui::UiRequestTuning,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "prompt": message,
        "inputText": message,
        "messages": [
            {
                "role": "user",
                "content": message
            }
        ]
    });
    let Some(object) = body.as_object_mut() else {
        return body;
    };
    if let Some(max_tokens) = request_tuning.max_output_tokens {
        object.insert("max_tokens".to_owned(), serde_json::json!(max_tokens));
    }
    if let Some(temperature) = request_tuning_temperature(request_tuning) {
        object.insert(
            "temperature".to_owned(),
            serde_json::json!(temperature.as_f64()),
        );
    }
    if let Some(top_p) = request_tuning_top_p(request_tuning) {
        object.insert("top_p".to_owned(), serde_json::json!(top_p.as_f64()));
    }
    body
}

fn runtime_model_response_events(
    response: RuntimeModelResponse,
) -> Vec<NormalizedModelStreamEvent> {
    let mut events = vec![NormalizedModelStreamEvent {
        provider: ModelProviderKind::BedrockRuntime,
        kind: NormalizedModelStreamEventKind::Started {
            response_id: response.invoked_model_id.clone(),
            role: Some("assistant".to_owned()),
        },
    }];
    if !response.output_text.is_empty() {
        events.push(NormalizedModelStreamEvent::text_delta(
            ModelProviderKind::BedrockRuntime,
            response.output_text,
        ));
    }
    if let Some(usage) = response.usage {
        events.push(NormalizedModelStreamEvent {
            provider: ModelProviderKind::BedrockRuntime,
            kind: NormalizedModelStreamEventKind::Usage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                total_tokens: usage.total_tokens,
            },
        });
    }
    events.push(NormalizedModelStreamEvent::completed(
        ModelProviderKind::BedrockRuntime,
    ));
    events
}

fn mantle_api_shape_for_run(shape: UiMantleApiShape) -> MantleApiShape {
    match shape {
        UiMantleApiShape::Auto | UiMantleApiShape::Responses => MantleApiShape::Responses,
        UiMantleApiShape::ChatCompletions => MantleApiShape::ChatCompletions,
        UiMantleApiShape::AnthropicMessages => MantleApiShape::AnthropicMessages,
    }
}

fn mantle_api_shape_wire_label(shape: MantleApiShape) -> &'static str {
    match shape {
        MantleApiShape::Responses => "responses",
        MantleApiShape::ChatCompletions => "chat_completions",
        MantleApiShape::AnthropicMessages => "anthropic_messages",
    }
}

fn mantle_request_kind(shape: MantleApiShape) -> &'static str {
    match shape {
        MantleApiShape::Responses => "responses.create",
        MantleApiShape::ChatCompletions => "chat.completions.create",
        MantleApiShape::AnthropicMessages => "anthropic.messages.create",
    }
}

fn mantle_model_run_error(error: MantleClientError) -> BedrockModelRunError {
    BedrockModelRunError::MantleClient {
        diagnostic: error.diagnostic(),
        message: error.to_string(),
    }
}

fn runtime_model_run_error(error: RuntimeClientError) -> BedrockModelRunError {
    BedrockModelRunError::RuntimeClient {
        diagnostic: error.diagnostic(),
        message: error.to_string(),
    }
}

fn mantle_probe_client_error(error: MantleClientError) -> BedrockProfileProbeError {
    BedrockProfileProbeError::MantleClient {
        diagnostic: error.diagnostic(),
        message: error.to_string(),
    }
}

fn runtime_probe_client_error(error: RuntimeClientError) -> BedrockProfileProbeError {
    BedrockProfileProbeError::RuntimeClient {
        diagnostic: error.diagnostic(),
        message: error.to_string(),
    }
}

fn aws_profile_source_type(profile: &AwsSharedProfile) -> &'static str {
    if profile.sso_session.is_some() {
        "sso"
    } else if profile.role_arn.is_some() {
        "assume_role"
    } else if profile.credential_process.is_some() {
        "credential_process"
    } else if profile.has_static_credentials {
        "static_credentials"
    } else {
        "profile"
    }
}

fn profile_provider_display(provider: UiLlmProvider) -> &'static str {
    match provider {
        UiLlmProvider::BedrockMantle => "Bedrock Mantle",
        UiLlmProvider::BedrockRuntime => "Bedrock Runtime",
    }
}

fn credential_source_display(source: &UiCredentialSource) -> String {
    match source {
        UiCredentialSource::DefaultChain => "default chain".to_owned(),
        UiCredentialSource::AwsCliProfile { profile_name } => profile_name.clone(),
        UiCredentialSource::Environment => "environment".to_owned(),
        UiCredentialSource::SecretRef { secret_ref } => format!("secret:{secret_ref}"),
    }
}

fn mantle_api_shape_display(shape: UiMantleApiShape) -> &'static str {
    match shape {
        UiMantleApiShape::Auto => "auto",
        UiMantleApiShape::Responses => "responses",
        UiMantleApiShape::ChatCompletions => "chat completions",
        UiMantleApiShape::AnthropicMessages => "anthropic messages",
    }
}

fn mantle_auth_display(auth_mode: &UiMantleAuthMode) -> &'static str {
    match auth_mode {
        UiMantleAuthMode::BearerApiKey { .. } => "bearer api key",
        UiMantleAuthMode::AwsSigV4 => "aws sigv4",
    }
}

fn runtime_target_display(target_type: UiRuntimeTargetType) -> &'static str {
    match target_type {
        UiRuntimeTargetType::FoundationModel => "foundation model",
        UiRuntimeTargetType::InferenceProfileArn => "inference profile arn",
        UiRuntimeTargetType::ApplicationInferenceProfileArn => "application inference profile arn",
        UiRuntimeTargetType::PromptRouterArn => "prompt router arn",
        UiRuntimeTargetType::CustomArn => "custom arn",
    }
}

fn profile_probe_capability_label(profile: &LlmProfileForm) -> &'static str {
    match profile.provider {
        UiLlmProvider::BedrockMantle => "models + test",
        UiLlmProvider::BedrockRuntime => "capabilities + test",
    }
}

fn command_exit_label(exit_code: Option<i32>, timed_out: bool, cancelled: bool) -> String {
    if timed_out {
        return "timed out".to_owned();
    }
    if cancelled {
        return "cancelled".to_owned();
    }
    exit_code
        .map(|code| format!("exit {code}"))
        .unwrap_or_else(|| "running".to_owned())
}

fn evaluate_workflow_command_policy(
    policy: &CommandPolicy,
    request: &RtkCommandRequest,
    project_root: &str,
    active_commands: usize,
) -> Result<Vec<String>, String> {
    match evaluate_command_policy(
        policy,
        &command_policy_request(request, project_root, active_commands),
    ) {
        CommandPolicyDecision::Allow { labels } => {
            Ok(labels.into_iter().map(|label| label.to_string()).collect())
        }
        CommandPolicyDecision::Deny { reason } => Err(reason.to_string()),
    }
}

fn command_policy_request(
    request: &RtkCommandRequest,
    project_root: &str,
    active_commands: usize,
) -> CommandPolicyRequest {
    CommandPolicyRequest {
        command: request.command.clone(),
        cwd: request.cwd.clone(),
        project_root: Some(project_root.to_owned()),
        active_commands,
    }
}

fn clamp_command_timeout(request: &mut RtkCommandRequest, max_runtime_ms: Option<u64>) {
    if let Some(max_runtime_ms) = max_runtime_ms {
        request.timeout_ms = Some(
            request
                .timeout_ms
                .map_or(max_runtime_ms, |timeout_ms| timeout_ms.min(max_runtime_ms)),
        );
    }
}

fn apply_remote_command_policy_metadata(
    transcript: &mut RemoteCommandTranscript,
    labels: Vec<String>,
    max_output_bytes: Option<usize>,
) {
    transcript.policy_labels = labels;
    if let Some(max_output_bytes) = max_output_bytes {
        truncate_capture(
            &mut transcript.stdout,
            max_output_bytes,
            &mut transcript.stdout_truncated,
        );
        truncate_capture(
            &mut transcript.stderr,
            max_output_bytes,
            &mut transcript.stderr_truncated,
        );
    }
}

fn truncate_capture(bytes: &mut Vec<u8>, max_output_bytes: usize, truncated: &mut bool) {
    if bytes.len() > max_output_bytes {
        bytes.truncate(max_output_bytes);
        *truncated = true;
    }
}

fn local_command_io_label(command: &RtkCommandTranscript) -> String {
    let mut parts = vec![
        format!("stdout {} bytes", command.stdout_bytes),
        format!("stderr {} bytes", command.stderr_bytes),
    ];
    if command.stdout_truncated || command.stderr_truncated {
        parts.push("captured output truncated".to_owned());
    }
    if let Some(savings) = &command.rtk_savings {
        parts.push(format!(
            "rtk saved {} tokens from {} input / {} output",
            savings.saved_tokens, savings.input_tokens, savings.output_tokens
        ));
    }
    if !command.policy_labels.is_empty() {
        parts.push(format!("policy {}", command.policy_labels.join("; ")));
    }
    parts.join(", ")
}

fn remote_command_io_label(command: &RemoteCommandTranscript) -> String {
    let mut parts = vec![
        format!("stdout {} bytes", command.stdout_bytes),
        format!("stderr {} bytes", command.stderr_bytes),
    ];
    if command.stdout_truncated || command.stderr_truncated {
        parts.push("captured output truncated".to_owned());
    }
    if let Some(savings) = &command.rtk_savings {
        parts.push(format!(
            "rtk saved {} tokens from {} input / {} output",
            savings.saved_tokens, savings.input_tokens, savings.output_tokens
        ));
    }
    if !command.policy_labels.is_empty() {
        parts.push(format!("policy {}", command.policy_labels.join("; ")));
    }
    parts.join(", ")
}

fn cache_usage_display(usage: CacheUsage) -> String {
    format!(
        "{} prompt / {} read / {} write / {} hit / {} miss",
        usage.prompt_tokens,
        usage.cache_read_tokens,
        usage.cache_write_tokens,
        usage.cache_hits,
        usage.cache_misses
    )
}

fn mcp_status_snapshot_record(
    snapshot: &McpRuntimeSnapshot,
) -> PersistenceResult<McpStatusSnapshot> {
    Ok(McpStatusSnapshot {
        server_id: snapshot.server_id.clone(),
        status: snapshot.status.label().to_owned(),
        document_json: serde_json::to_string(snapshot).map_err(json_persistence_error)?,
        updated_at_ms: snapshot.updated_at_ms,
    })
}

fn mcp_runtime_snapshot_from_record(
    record: McpStatusSnapshot,
) -> PersistenceResult<McpRuntimeSnapshot> {
    let snapshot: McpRuntimeSnapshot =
        serde_json::from_str(&record.document_json).map_err(json_persistence_error)?;
    if snapshot.server_id != record.server_id {
        return Err(PersistenceError::Startup(format!(
            "persisted MCP status id mismatch: record={} document={}",
            record.server_id, snapshot.server_id
        )));
    }
    if snapshot.status.label() != record.status {
        return Err(PersistenceError::Startup(format!(
            "persisted MCP status label mismatch for {}: record={} document={}",
            record.server_id,
            record.status,
            snapshot.status.label()
        )));
    }
    Ok(snapshot)
}

fn cache_usage_key(provider_plane: CacheProviderPlane) -> String {
    format!("{CACHE_USAGE_KEY_PREFIX}{}", provider_plane.label())
}

fn context_fragment_cache_key(record: &ContextFragmentRecord) -> String {
    format!(
        "{CONTEXT_FRAGMENT_CACHE_KEY_PREFIX}{}:{}",
        context_fragment_kind_label(record.kind),
        record.source_id
    )
}

fn context_fragment_cache_metadata_record(
    record: &ContextFragmentRecord,
) -> PersistenceResult<CacheMetadataRecord> {
    Ok(CacheMetadataRecord {
        cache_key: context_fragment_cache_key(record),
        provider_plane: LOCAL_CONTEXT_PROVIDER_PLANE.to_owned(),
        document_json: serde_json::to_string(record).map_err(json_persistence_error)?,
        updated_at_ms: record.last_seen_ms,
    })
}

fn context_fragment_kind_label(kind: ContextFragmentKind) -> &'static str {
    match kind {
        ContextFragmentKind::Instruction => "instruction",
        ContextFragmentKind::FileFragment => "file_fragment",
        ContextFragmentKind::ToolSchema => "tool_schema",
        ContextFragmentKind::McpManifest => "mcp_manifest",
        ContextFragmentKind::SkillManifest => "skill_manifest",
    }
}

fn tool_output_cache_key(record: &ToolOutputRecord) -> String {
    record.reference_id.clone()
}

fn tool_output_cache_metadata_record(
    record: &ToolOutputRecord,
) -> PersistenceResult<CacheMetadataRecord> {
    Ok(CacheMetadataRecord {
        cache_key: tool_output_cache_key(record),
        provider_plane: TOOL_OUTPUT_PROVIDER_PLANE.to_owned(),
        document_json: serde_json::to_string(record).map_err(json_persistence_error)?,
        updated_at_ms: record.last_seen_ms,
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn generated_persisted_id() -> String {
    let millis = now_ms().max(0) as u64 & 0x0000_FFFF_FFFF_FFFF;
    let counter = GENERATED_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut bytes = [0_u8; 16];
    bytes[0] = (millis >> 40) as u8;
    bytes[1] = (millis >> 32) as u8;
    bytes[2] = (millis >> 24) as u8;
    bytes[3] = (millis >> 16) as u8;
    bytes[4] = (millis >> 8) as u8;
    bytes[5] = millis as u8;
    bytes[6..14].copy_from_slice(&counter.to_be_bytes());
    bytes[14..16].copy_from_slice(&(counter as u16).wrapping_mul(31).to_be_bytes());

    let mut value = u128::from_be_bytes(bytes);
    let mut output = [b'0'; 26];
    for index in (0..output.len()).rev() {
        output[index] = CROCKFORD_BASE32[(value & 0b1_1111) as usize];
        value >>= 5;
    }
    String::from_utf8(output.to_vec()).expect("Crockford Base32 output is valid UTF-8")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseEvidence {
    pub mantle_profile: bool,
    pub runtime_profile: bool,
    pub parallel_conversations: bool,
    pub local_rtk_command: bool,
    pub remote_rtk_commands: bool,
    pub remote_file_operations: bool,
    pub configured_mcp: bool,
    pub configured_skills: bool,
    pub memory_present: bool,
    pub open_editor_tab: bool,
    pub saved_editor_file: bool,
    pub goal_present: bool,
}

impl ReleaseEvidence {
    pub fn all_core_workflows_present(&self) -> bool {
        self.mantle_profile
            && self.runtime_profile
            && self.parallel_conversations
            && self.local_rtk_command
            && self.remote_rtk_commands
            && self.remote_file_operations
            && self.configured_mcp
            && self.configured_skills
            && self.memory_present
            && self.open_editor_tab
            && self.saved_editor_file
            && self.goal_present
    }
}
