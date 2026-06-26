#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::SystemTime;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use crc32fast::Hasher as Crc32Hasher;
use fastrock_core::{
    ModelProviderKind, NormalizedModelStreamEvent, NormalizedModelStreamEventKind,
};
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use url::Url;

type HmacSha256 = Hmac<Sha256>;

const AWS_PERCENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockProfile {
    pub region: String,
    pub endpoint_override: Option<String>,
    pub credential_source: AwsCredentialSource,
    pub mantle: Option<MantleSettings>,
    pub runtime: Option<RuntimeSettings>,
}

impl BedrockProfile {
    pub fn resolve_shared_aws_auth(
        &self,
        shared_config: &AwsSharedConfig,
    ) -> Result<ResolvedBedrockAuth, AwsSharedConfigError> {
        shared_config.resolve_bedrock_auth(&self.credential_source, Some(&self.region))
    }

    pub fn mantle_diagnostics(
        &self,
        resolved_auth: &ResolvedBedrockAuth,
    ) -> Option<MantleDiagnostics> {
        let mantle = self.mantle.as_ref()?;
        Some(MantleDiagnostics {
            region: resolved_auth.region.clone(),
            endpoint: resolved_auth.mantle_sigv4_base_url.clone(),
            project_id: mantle.project_id.clone(),
            store_default: mantle.store_default,
            api_shape: mantle.api_shape,
            auth_mode: mantle.auth_mode.clone(),
            quota: MantleQuotaDiagnostics::default(),
            cloudtrail: MantleCloudTrailDiagnostics::default(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AwsCredentialSource {
    DefaultChain,
    CliProfile { profile_name: String },
    Environment,
    ExplicitStaticSecret { secret_ref: String },
}

impl AwsCredentialSource {
    pub fn uses_aws_cli_profile(&self) -> bool {
        matches!(self, Self::CliProfile { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MantleSettings {
    pub project_id: Option<String>,
    pub store_default: bool,
    pub api_shape: MantleApiShape,
    pub auth_mode: MantleAuthMode,
    pub use_background: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MantleApiShape {
    #[serde(rename = "responses", alias = "responses_api")]
    Responses,
    #[serde(
        rename = "chat_completions",
        alias = "chat-completions",
        alias = "chat.completions",
        alias = "chat",
        alias = "chat_completions_api"
    )]
    ChatCompletions,
    #[serde(
        rename = "anthropic_messages",
        alias = "anthropic-messages",
        alias = "anthropic.messages",
        alias = "messages",
        alias = "anthropic_messages_api"
    )]
    AnthropicMessages,
}

impl MantleApiShape {
    pub const fn endpoint_path(self) -> &'static str {
        match self {
            Self::Responses => "/responses",
            Self::ChatCompletions => "/chat/completions",
            Self::AnthropicMessages => "/messages",
        }
    }
}

pub const MANTLE_API_SHAPE_FALLBACK_ORDER: [MantleApiShape; 3] = [
    MantleApiShape::Responses,
    MantleApiShape::ChatCompletions,
    MantleApiShape::AnthropicMessages,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MantleAuthMode {
    BearerApiKey { secret_ref: String },
    AwsSigV4,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSettings {
    pub invoke_target: BedrockInvokeTarget,
    pub use_converse_stream: bool,
    pub service_tier: Option<String>,
    pub prompt_cache_ttl_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BedrockInvokeTarget {
    FoundationModel(String),
    InferenceProfileArn(String),
    ApplicationInferenceProfileArn(String),
    PromptRouterArn(String),
    CustomArn(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BedrockUsagePlane {
    Mantle,
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BedrockProviderErrorKind {
    AuthOrPermission,
    ModelOrRegionMismatch,
    IncompatibleRequest,
    QuotaOrThrottle,
    ServerOrUnavailable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockProviderErrorDiagnostic {
    pub plane: BedrockUsagePlane,
    pub status: u16,
    pub kind: BedrockProviderErrorKind,
    pub retryable: bool,
    pub summary: &'static str,
    pub suggested_action: &'static str,
    pub body_excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MantleDiagnostics {
    pub region: String,
    pub endpoint: String,
    pub project_id: Option<String>,
    pub store_default: bool,
    pub api_shape: MantleApiShape,
    pub auth_mode: MantleAuthMode,
    pub quota: MantleQuotaDiagnostics,
    pub cloudtrail: MantleCloudTrailDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MantleQuotaDiagnostics {
    pub usage_plane: BedrockUsagePlane,
    pub endpoint_family: &'static str,
    pub accounting_separate_from_runtime: bool,
    pub dimensions: Vec<&'static str>,
}

impl Default for MantleQuotaDiagnostics {
    fn default() -> Self {
        Self {
            usage_plane: BedrockUsagePlane::Mantle,
            endpoint_family: "bedrock-mantle",
            accounting_separate_from_runtime: true,
            dimensions: vec!["region", "project", "model", "api_shape", "service_tier"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MantleCloudTrailDiagnostics {
    pub event_source: &'static str,
    pub management_events: Vec<&'static str>,
    pub data_events: Vec<&'static str>,
    pub resource_types: Vec<&'static str>,
    pub inference_is_data_event: bool,
}

impl Default for MantleCloudTrailDiagnostics {
    fn default() -> Self {
        Self {
            event_source: "bedrock-mantle.amazonaws.com",
            management_events: vec![
                "ListModels",
                "GetModel",
                "ListProjects",
                "CreateProject",
                "GetProject",
                "UpdateProject",
                "ArchiveProject",
            ],
            data_events: vec![
                "CreateInference",
                "GetInference",
                "CancelInference",
                "DeleteInference",
                "CountTokens",
            ],
            resource_types: vec![
                "AWS::BedrockMantle::Project",
                "AWS::BedrockMantle::Reservation",
                "AWS::BedrockMantle::CustomizedModel",
                "AWS::BedrockMantle::Environment",
                "AWS::BedrockMantle::Runtime",
                "AWS::BedrockMantle::Skill",
            ],
            inference_is_data_event: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeQuotaDiagnostics {
    pub usage_plane: BedrockUsagePlane,
    pub endpoint_family: &'static str,
    pub accounting_separate_from_mantle: bool,
    pub operations: Vec<&'static str>,
}

impl Default for RuntimeQuotaDiagnostics {
    fn default() -> Self {
        Self {
            usage_plane: BedrockUsagePlane::Runtime,
            endpoint_family: "bedrock-runtime",
            accounting_separate_from_mantle: true,
            operations: vec!["Converse", "ConverseStream", "InvokeModel"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsSharedConfigFiles {
    pub config_path: PathBuf,
    pub credentials_path: PathBuf,
}

impl AwsSharedConfigFiles {
    pub fn new(config_path: impl Into<PathBuf>, credentials_path: impl Into<PathBuf>) -> Self {
        Self {
            config_path: config_path.into(),
            credentials_path: credentials_path.into(),
        }
    }

    pub fn load(&self) -> Result<AwsSharedConfig, AwsSharedConfigError> {
        let config = read_optional_to_string(&self.config_path)?;
        let credentials = read_optional_to_string(&self.credentials_path)?;
        Ok(AwsSharedConfig::from_ini_documents(&config, &credentials))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AwsSharedConfig {
    profiles: BTreeMap<String, AwsSharedProfile>,
}

impl AwsSharedConfig {
    pub fn from_ini_documents(config: &str, credentials: &str) -> Self {
        let mut profiles = parse_aws_ini(config, IniDocumentKind::Config);
        for (name, credential_profile) in parse_aws_ini(credentials, IniDocumentKind::Credentials) {
            profiles
                .entry(name)
                .and_modify(|profile| profile.merge(credential_profile.clone()))
                .or_insert(credential_profile);
        }
        Self { profiles }
    }

    pub fn profile(&self, name: &str) -> Option<&AwsSharedProfile> {
        self.profiles.get(name)
    }

    pub fn profile_names(&self) -> impl Iterator<Item = &str> {
        self.profiles.keys().map(String::as_str)
    }

    pub fn static_credentials_for_profile(&self, name: &str) -> Option<AwsStaticCredentials> {
        self.profile(name)?.static_credentials()
    }

    pub fn resolve_bedrock_auth(
        &self,
        source: &AwsCredentialSource,
        explicit_region: Option<&str>,
    ) -> Result<ResolvedBedrockAuth, AwsSharedConfigError> {
        let resolved_region = match source {
            AwsCredentialSource::CliProfile { profile_name } => {
                let profile = self
                    .profile(profile_name)
                    .ok_or_else(|| AwsSharedConfigError::ProfileNotFound(profile_name.clone()))?;
                explicit_region
                    .map(str::to_owned)
                    .or_else(|| profile.region.clone())
                    .ok_or_else(|| AwsSharedConfigError::RegionMissing(profile_name.clone()))?
            }
            AwsCredentialSource::DefaultChain
            | AwsCredentialSource::Environment
            | AwsCredentialSource::ExplicitStaticSecret { .. } => explicit_region
                .map(str::to_owned)
                .ok_or_else(|| AwsSharedConfigError::RegionMissing("explicit".to_owned()))?,
        };
        let credential_profile = match source {
            AwsCredentialSource::CliProfile { profile_name } => self
                .profile(profile_name)
                .map(|profile| profile.resolved_credential_profile()),
            AwsCredentialSource::DefaultChain
            | AwsCredentialSource::Environment
            | AwsCredentialSource::ExplicitStaticSecret { .. } => None,
        };

        Ok(ResolvedBedrockAuth {
            credential_source: source.clone(),
            credential_profile,
            region: resolved_region.clone(),
            mantle_sigv4_base_url: mantle_base_url(&resolved_region),
            runtime_endpoint: runtime_endpoint(&resolved_region),
        })
    }
}

#[derive(Clone, PartialEq, Eq, Default)]
pub struct AwsSharedProfile {
    pub name: String,
    pub region: Option<String>,
    pub has_static_credentials: bool,
    pub credential_process: Option<String>,
    pub sso_session: Option<String>,
    pub role_arn: Option<String>,
    pub source_profile: Option<String>,
    static_access_key_id: Option<String>,
    static_secret_access_key: Option<String>,
    static_session_token: Option<String>,
}

impl fmt::Debug for AwsSharedProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AwsSharedProfile")
            .field("name", &self.name)
            .field("region", &self.region)
            .field("has_static_credentials", &self.has_static_credentials)
            .field("credential_process", &self.credential_process)
            .field("sso_session", &self.sso_session)
            .field("role_arn", &self.role_arn)
            .field("source_profile", &self.source_profile)
            .finish_non_exhaustive()
    }
}

impl AwsSharedProfile {
    pub fn can_resolve_credentials(&self) -> bool {
        self.has_static_credentials
            || self.credential_process.is_some()
            || self.sso_session.is_some()
            || self.role_arn.is_some()
    }

    fn static_credentials(&self) -> Option<AwsStaticCredentials> {
        Some(AwsStaticCredentials::new(
            self.static_access_key_id.clone()?,
            self.static_secret_access_key.clone()?,
            self.static_session_token.clone(),
        ))
    }

    fn resolved_credential_profile(&self) -> ResolvedAwsCredentialProfile {
        ResolvedAwsCredentialProfile {
            profile_name: self.name.clone(),
            resolution_kind: self.credential_resolution_kind(),
            has_static_credentials: self.has_static_credentials,
            credential_process_configured: self.credential_process.is_some(),
            sso_session: self.sso_session.clone(),
            role_arn: self.role_arn.clone(),
            source_profile: self.source_profile.clone(),
        }
    }

    fn credential_resolution_kind(&self) -> AwsCredentialResolutionKind {
        if self.role_arn.is_some() {
            AwsCredentialResolutionKind::AssumeRole
        } else if self.sso_session.is_some() {
            AwsCredentialResolutionKind::SsoSession
        } else if self.credential_process.is_some() {
            AwsCredentialResolutionKind::CredentialProcess
        } else if self.has_static_credentials {
            AwsCredentialResolutionKind::StaticCredentials
        } else {
            AwsCredentialResolutionKind::ProfileOnly
        }
    }

    fn merge(&mut self, other: Self) {
        self.region = self.region.take().or(other.region);
        self.has_static_credentials |= other.has_static_credentials;
        self.credential_process = self.credential_process.take().or(other.credential_process);
        self.sso_session = self.sso_session.take().or(other.sso_session);
        self.role_arn = self.role_arn.take().or(other.role_arn);
        self.source_profile = self.source_profile.take().or(other.source_profile);
        self.static_access_key_id = self
            .static_access_key_id
            .take()
            .or(other.static_access_key_id);
        self.static_secret_access_key = self
            .static_secret_access_key
            .take()
            .or(other.static_secret_access_key);
        self.static_session_token = self
            .static_session_token
            .take()
            .or(other.static_session_token);
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AwsStaticCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
}

impl AwsStaticCredentials {
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        session_token: Option<String>,
    ) -> Self {
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            session_token,
        }
    }

    pub fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    pub fn secret_access_key(&self) -> &str {
        &self.secret_access_key
    }

    pub fn session_token(&self) -> Option<&str> {
        self.session_token.as_deref()
    }
}

impl fmt::Debug for AwsStaticCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AwsStaticCredentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"<redacted>")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBedrockAuth {
    pub credential_source: AwsCredentialSource,
    pub credential_profile: Option<ResolvedAwsCredentialProfile>,
    pub region: String,
    pub mantle_sigv4_base_url: String,
    pub runtime_endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAwsCredentialProfile {
    pub profile_name: String,
    pub resolution_kind: AwsCredentialResolutionKind,
    pub has_static_credentials: bool,
    pub credential_process_configured: bool,
    pub sso_session: Option<String>,
    pub role_arn: Option<String>,
    pub source_profile: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwsCredentialResolutionKind {
    StaticCredentials,
    CredentialProcess,
    SsoSession,
    AssumeRole,
    ProfileOnly,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AwsSharedConfigError {
    #[error("failed to read AWS shared config file {path}: {message}")]
    ReadFailed { path: PathBuf, message: String },
    #[error("AWS CLI profile not found: {0}")]
    ProfileNotFound(String),
    #[error("AWS region is missing for profile/source: {0}")]
    RegionMissing(String),
}

pub fn mantle_base_url(region: &str) -> String {
    format!("https://bedrock-mantle.{region}.api.aws/v1")
}

pub fn runtime_endpoint(region: &str) -> String {
    format!("https://bedrock-runtime.{region}.amazonaws.com")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MantleClient<T, S> {
    base_url: String,
    auth: MantleClientAuth,
    transport: T,
    signer: S,
}

impl<T, S> MantleClient<T, S>
where
    T: MantleHttpTransport,
    S: MantleSigV4Signer,
{
    pub fn new(
        base_url: impl Into<String>,
        auth: MantleClientAuth,
        transport: T,
        signer: S,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            auth,
            transport,
            signer,
        }
    }

    pub async fn list_models(&self) -> Result<Vec<MantleModel>, MantleClientError> {
        let request = self.authed_request("GET", "/models", Vec::new())?;
        let response = self.transport.send(request).await?;
        ensure_success(response.status, &response.body)?;
        let models: MantleModelsResponse = serde_json::from_slice(&response.body)?;
        Ok(models.data)
    }

    pub async fn create_response(
        &self,
        mut request_body: MantleResponseRequest,
    ) -> Result<MantleResponse, MantleClientError> {
        request_body.stream = false;
        let body = serde_json::to_vec(&request_body)?;
        let request = self.authed_request("POST", "/responses", body)?;
        let response = self.transport.send(request).await?;
        ensure_success(response.status, &response.body)?;
        let raw_json: serde_json::Value = serde_json::from_slice(&response.body)?;
        Ok(mantle_response_from_raw_json(raw_json))
    }

    pub async fn get_response(
        &self,
        response_id: &str,
    ) -> Result<MantleResponse, MantleClientError> {
        let path = format!("/responses/{}", encode_path_segment(response_id));
        let request = self.authed_request("GET", &path, Vec::new())?;
        let response = self.transport.send(request).await?;
        ensure_success(response.status, &response.body)?;
        let raw_json: serde_json::Value = serde_json::from_slice(&response.body)?;
        Ok(mantle_response_from_raw_json(raw_json))
    }

    pub async fn create_chat_completion(
        &self,
        mut request_body: MantleChatCompletionRequest,
    ) -> Result<MantleResponse, MantleClientError> {
        request_body.stream = false;
        let response = self
            .send_json(
                MantleApiShape::ChatCompletions.endpoint_path(),
                &request_body,
            )
            .await?;
        let raw_json: serde_json::Value = serde_json::from_slice(&response.body)?;
        Ok(MantleResponse {
            id: extract_json_id(&raw_json),
            output_text: extract_chat_completion_output_text(&raw_json),
            raw_json,
        })
    }

    pub async fn create_anthropic_message(
        &self,
        mut request_body: MantleAnthropicMessagesRequest,
    ) -> Result<MantleResponse, MantleClientError> {
        request_body.stream = false;
        let response = self
            .send_json(
                MantleApiShape::AnthropicMessages.endpoint_path(),
                &request_body,
            )
            .await?;
        let raw_json: serde_json::Value = serde_json::from_slice(&response.body)?;
        Ok(MantleResponse {
            id: extract_json_id(&raw_json),
            output_text: extract_anthropic_message_output_text(&raw_json),
            raw_json,
        })
    }

    pub async fn stream_response(
        &self,
        mut request_body: MantleResponseRequest,
    ) -> Result<Vec<MantleStreamEvent>, MantleClientError> {
        request_body.stream = true;
        let body = serde_json::to_vec(&request_body)?;
        let request = self.authed_request("POST", "/responses", body)?;
        let response = self.transport.send(request).await?;
        ensure_success(response.status, &response.body)?;
        let stream_body = String::from_utf8_lossy(&response.body);
        parse_mantle_sse(&stream_body)
    }

    pub async fn stream_chat_completion(
        &self,
        mut request_body: MantleChatCompletionRequest,
    ) -> Result<Vec<MantleStreamEvent>, MantleClientError> {
        request_body.stream = true;
        let response = self
            .send_json(
                MantleApiShape::ChatCompletions.endpoint_path(),
                &request_body,
            )
            .await?;
        let stream_body = String::from_utf8_lossy(&response.body);
        parse_mantle_sse(&stream_body)
    }

    pub async fn stream_anthropic_message(
        &self,
        mut request_body: MantleAnthropicMessagesRequest,
    ) -> Result<Vec<MantleStreamEvent>, MantleClientError> {
        request_body.stream = true;
        let response = self
            .send_json(
                MantleApiShape::AnthropicMessages.endpoint_path(),
                &request_body,
            )
            .await?;
        let stream_body = String::from_utf8_lossy(&response.body);
        parse_mantle_sse(&stream_body)
    }

    async fn send_json<B: Serialize + ?Sized>(
        &self,
        path: &str,
        request_body: &B,
    ) -> Result<MantleHttpResponse, MantleClientError> {
        let request = self.authed_request("POST", path, serde_json::to_vec(request_body)?)?;
        let response = self.transport.send(request).await?;
        ensure_success(response.status, &response.body)?;
        Ok(response)
    }

    fn authed_request(
        &self,
        method: &str,
        path: &str,
        body: Vec<u8>,
    ) -> Result<MantleHttpRequest, MantleClientError> {
        let mut request = MantleHttpRequest {
            method: method.to_owned(),
            url: format!("{}{}", self.base_url, path),
            headers: BTreeMap::from([
                ("accept".to_owned(), "application/json".to_owned()),
                ("content-type".to_owned(), "application/json".to_owned()),
            ]),
            body,
        };

        match &self.auth {
            MantleClientAuth::BearerApiKey {
                api_key,
                project_id,
            } => {
                request
                    .headers
                    .insert("authorization".to_owned(), format!("Bearer {api_key}"));
                if let Some(project_id) = project_id {
                    request
                        .headers
                        .insert("openai-project".to_owned(), project_id.clone());
                }
            }
            MantleClientAuth::AwsSigV4 {
                resolved_auth,
                project_id,
            } => {
                if let Some(project_id) = project_id {
                    request
                        .headers
                        .insert("openai-project".to_owned(), project_id.clone());
                }
                self.signer.sign(&mut request, resolved_auth)?;
            }
        }

        Ok(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MantleClientAuth {
    BearerApiKey {
        api_key: String,
        project_id: Option<String>,
    },
    AwsSigV4 {
        resolved_auth: ResolvedBedrockAuth,
        project_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MantleHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MantleHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub trait MantleHttpTransport {
    fn send(
        &self,
        request: MantleHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<MantleHttpResponse, MantleClientError>> + Send + '_>>;
}

#[derive(Debug, Clone)]
pub struct ReqwestBedrockHttpTransport {
    client: reqwest::Client,
}

impl ReqwestBedrockHttpTransport {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

impl Default for ReqwestBedrockHttpTransport {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl MantleHttpTransport for ReqwestBedrockHttpTransport {
    fn send(
        &self,
        request: MantleHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<MantleHttpResponse, MantleClientError>> + Send + '_>>
    {
        Box::pin(async move {
            let MantleHttpRequest {
                method,
                url,
                headers,
                body,
            } = request;
            let (status, headers, body) =
                send_reqwest_request(&self.client, &method, &url, headers, body)
                    .await
                    .map_err(MantleClientError::Transport)?;
            Ok(MantleHttpResponse {
                status,
                headers,
                body,
            })
        })
    }
}

pub trait MantleSigV4Signer {
    fn sign(
        &self,
        request: &mut MantleHttpRequest,
        resolved_auth: &ResolvedBedrockAuth,
    ) -> Result<(), MantleClientError>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MissingMantleSigV4Signer;

impl MantleSigV4Signer for MissingMantleSigV4Signer {
    fn sign(
        &self,
        _request: &mut MantleHttpRequest,
        _resolved_auth: &ResolvedBedrockAuth,
    ) -> Result<(), MantleClientError> {
        Err(MantleClientError::SigV4SignerUnavailable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderMantleSigV4Signer {
    pub authorization_value: String,
}

impl MantleSigV4Signer for HeaderMantleSigV4Signer {
    fn sign(
        &self,
        request: &mut MantleHttpRequest,
        resolved_auth: &ResolvedBedrockAuth,
    ) -> Result<(), MantleClientError> {
        request
            .headers
            .insert("authorization".to_owned(), self.authorization_value.clone());
        request
            .headers
            .insert("x-amz-region".to_owned(), resolved_auth.region.clone());
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticCredentialsMantleSigV4Signer {
    credentials: AwsStaticCredentials,
    signing_time: SystemTime,
    service: String,
}

impl StaticCredentialsMantleSigV4Signer {
    pub fn new(credentials: AwsStaticCredentials) -> Self {
        Self {
            credentials,
            signing_time: SystemTime::now(),
            service: "bedrock".to_owned(),
        }
    }

    pub fn with_time(mut self, signing_time: SystemTime) -> Self {
        self.signing_time = signing_time;
        self
    }

    pub fn with_service(mut self, service: impl Into<String>) -> Self {
        self.service = service.into();
        self
    }
}

impl MantleSigV4Signer for StaticCredentialsMantleSigV4Signer {
    fn sign(
        &self,
        request: &mut MantleHttpRequest,
        resolved_auth: &ResolvedBedrockAuth,
    ) -> Result<(), MantleClientError> {
        let parsed_url = Url::parse(&request.url)
            .map_err(|error| MantleClientError::SigV4(format!("invalid URL: {error}")))?;
        let host = parsed_url
            .host_str()
            .ok_or_else(|| MantleClientError::SigV4("request URL is missing host".to_owned()))?;
        let datestamp = sigv4_datestamp(self.signing_time);
        let amz_date = sigv4_amz_date(self.signing_time);
        let payload_hash = sha256_hex(&request.body);

        request.headers.insert("host".to_owned(), host.to_owned());
        request
            .headers
            .insert("x-amz-content-sha256".to_owned(), payload_hash.clone());
        request
            .headers
            .insert("x-amz-date".to_owned(), amz_date.clone());
        if let Some(session_token) = self.credentials.session_token() {
            request
                .headers
                .insert("x-amz-security-token".to_owned(), session_token.to_owned());
        }

        let canonical_uri = canonical_uri(parsed_url.path());
        let canonical_query = canonical_query(&parsed_url);
        let (canonical_headers, signed_headers) = canonical_headers(&request.headers);
        let canonical_request = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            request.method.to_uppercase(),
            canonical_uri,
            canonical_query,
            canonical_headers,
            signed_headers,
            payload_hash
        );
        let credential_scope = format!(
            "{}/{}/{}/aws4_request",
            datestamp, resolved_auth.region, self.service
        );
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date,
            credential_scope,
            sha256_hex(canonical_request.as_bytes())
        );
        let signing_key = sigv4_signing_key(
            self.credentials.secret_access_key(),
            &datestamp,
            &resolved_auth.region,
            &self.service,
        );
        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.credentials.access_key_id(),
            credential_scope,
            signed_headers,
            signature
        );

        request
            .headers
            .insert("authorization".to_owned(), authorization);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MantleResponseRequest {
    pub model: String,
    pub input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    pub store: bool,
    pub stream: bool,
    #[serde(rename = "max_output_tokens", skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<BedrockFloatMilli>,
    #[serde(rename = "top_p", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<BedrockFloatMilli>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MantleStoredResponseState {
    pub project_id: Option<String>,
    pub last_response_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MantleStoredResponsePolicy {
    pub project_id: Option<String>,
    pub store: bool,
}

impl MantleStoredResponsePolicy {
    pub fn new(project_id: Option<String>, store: bool) -> Self {
        Self { project_id, store }
    }

    pub fn from_settings(settings: &MantleSettings, store_override: Option<bool>) -> Self {
        Self {
            project_id: settings.project_id.clone(),
            store: store_override.unwrap_or(settings.store_default),
        }
    }

    pub fn prepare_request(
        &self,
        mut request: MantleResponseRequest,
        state: &MantleStoredResponseState,
    ) -> MantleResponseRequest {
        request.store = self.store;

        if !self.store || state.project_id != self.project_id {
            request.previous_response_id = None;
        } else if request.previous_response_id.is_none() {
            request.previous_response_id = state.last_response_id.clone();
        }

        request
    }

    pub fn state_after_response(&self, response: &MantleResponse) -> MantleStoredResponseState {
        self.state_after_response_id(Some(response.id.clone()))
    }

    pub fn state_after_response_id(
        &self,
        response_id: Option<String>,
    ) -> MantleStoredResponseState {
        if self.store {
            MantleStoredResponseState {
                project_id: self.project_id.clone(),
                last_response_id: response_id,
            }
        } else {
            MantleStoredResponseState::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MantleChatCompletionRequest {
    pub model: String,
    pub messages: Vec<MantleChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<BedrockFloatMilli>,
    #[serde(rename = "top_p", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<BedrockFloatMilli>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MantleChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MantleAnthropicMessagesRequest {
    pub model: String,
    pub messages: Vec<MantleChatMessage>,
    pub max_tokens: u64,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<BedrockFloatMilli>,
    #[serde(rename = "top_p", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<BedrockFloatMilli>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BedrockFloatMilli(pub u16);

impl BedrockFloatMilli {
    pub const fn new(value_milli: u16) -> Self {
        Self(value_milli)
    }

    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / 1000.0
    }
}

impl Serialize for BedrockFloatMilli {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f64(self.as_f64())
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MantleModelsResponse {
    pub data: Vec<MantleModel>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MantleModel {
    pub id: String,
    #[serde(default)]
    pub owned_by: Option<String>,
    #[serde(
        default,
        alias = "apiShapes",
        alias = "supportedApiShapes",
        alias = "supported_api_shapes",
        alias = "supportedApis",
        alias = "supported_apis"
    )]
    pub api_shapes: Vec<MantleApiShape>,
}

impl MantleModel {
    pub fn api_shapes_known(&self) -> bool {
        !self.api_shapes.is_empty()
    }

    pub fn supports_api_shape(&self, shape: MantleApiShape) -> bool {
        self.api_shapes.contains(&shape)
    }

    pub fn choose_api_shape(&self, preferred: MantleApiShape) -> Option<MantleApiShape> {
        choose_mantle_api_shape(self, preferred)
    }
}

pub fn choose_mantle_api_shape(
    model: &MantleModel,
    preferred: MantleApiShape,
) -> Option<MantleApiShape> {
    if !model.api_shapes_known() {
        return None;
    }
    if model.supports_api_shape(preferred) {
        return Some(preferred);
    }
    MANTLE_API_SHAPE_FALLBACK_ORDER
        .iter()
        .copied()
        .find(|shape| model.supports_api_shape(*shape))
}

#[derive(Debug, Clone, PartialEq)]
pub struct MantleResponse {
    pub id: String,
    pub output_text: Option<String>,
    pub raw_json: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MantleStreamEvent {
    ResponseCreated { id: String },
    OutputTextDelta { delta: String },
    Completed,
    Error { message: String },
    Raw { event: String, data: String },
}

pub fn normalize_mantle_stream_events(
    events: impl IntoIterator<Item = MantleStreamEvent>,
) -> Vec<NormalizedModelStreamEvent> {
    events
        .into_iter()
        .map(|event| match event {
            MantleStreamEvent::ResponseCreated { id } => NormalizedModelStreamEvent {
                provider: ModelProviderKind::BedrockMantle,
                kind: NormalizedModelStreamEventKind::Started {
                    response_id: Some(id),
                    role: None,
                },
            },
            MantleStreamEvent::OutputTextDelta { delta } => {
                NormalizedModelStreamEvent::text_delta(ModelProviderKind::BedrockMantle, delta)
            }
            MantleStreamEvent::Completed => {
                NormalizedModelStreamEvent::completed(ModelProviderKind::BedrockMantle)
            }
            MantleStreamEvent::Error { message } => NormalizedModelStreamEvent {
                provider: ModelProviderKind::BedrockMantle,
                kind: NormalizedModelStreamEventKind::Error { message },
            },
            MantleStreamEvent::Raw { event, data } => NormalizedModelStreamEvent {
                provider: ModelProviderKind::BedrockMantle,
                kind: NormalizedModelStreamEventKind::Raw { event, data },
            },
        })
        .collect()
}

#[derive(Debug, Error)]
pub enum MantleClientError {
    #[error("Mantle HTTP transport error: {0}")]
    Transport(String),
    #[error("Mantle HTTP status {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("Mantle JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Mantle SSE error: {0}")]
    Sse(String),
    #[error("Mantle SigV4 error: {0}")]
    SigV4(String),
    #[error("Mantle SigV4 signer is unavailable")]
    SigV4SignerUnavailable,
}

impl MantleClientError {
    pub fn diagnostic(&self) -> Option<BedrockProviderErrorDiagnostic> {
        let Self::HttpStatus { status, body } = self else {
            return None;
        };
        Some(mantle_status_diagnostic(*status, body))
    }
}

pub fn parse_mantle_sse(body: &str) -> Result<Vec<MantleStreamEvent>, MantleClientError> {
    let mut events = Vec::new();
    let mut event_name = String::new();
    let mut data_lines = Vec::new();

    for line in body.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            flush_sse_event(&mut events, &mut event_name, &mut data_lines)?;
            continue;
        }
        if let Some(name) = line.strip_prefix("event:") {
            event_name = name.trim().to_owned();
        } else if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.trim().to_owned());
        }
    }

    flush_sse_event(&mut events, &mut event_name, &mut data_lines)?;
    Ok(events)
}

fn flush_sse_event(
    events: &mut Vec<MantleStreamEvent>,
    event_name: &mut String,
    data_lines: &mut Vec<String>,
) -> Result<(), MantleClientError> {
    if event_name.is_empty() && data_lines.is_empty() {
        return Ok(());
    }

    let event = std::mem::take(event_name);
    let data = data_lines.join("\n");
    data_lines.clear();

    if data == "[DONE]" {
        events.push(MantleStreamEvent::Completed);
        return Ok(());
    }

    match event.as_str() {
        "" => {
            let json: serde_json::Value = serde_json::from_str(&data)?;
            if let Some(delta) =
                extract_chat_stream_delta(&json).or_else(|| extract_anthropic_stream_delta(&json))
            {
                events.push(MantleStreamEvent::OutputTextDelta { delta });
            } else {
                events.push(MantleStreamEvent::Raw { event, data });
            }
        }
        "response.created" => {
            let json: serde_json::Value = serde_json::from_str(&data)?;
            events.push(MantleStreamEvent::ResponseCreated {
                id: json
                    .get("response")
                    .and_then(|response| response.get("id"))
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| json.get("id").and_then(serde_json::Value::as_str))
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        "response.output_text.delta" => {
            let json: serde_json::Value = serde_json::from_str(&data)?;
            events.push(MantleStreamEvent::OutputTextDelta {
                delta: json
                    .get("delta")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        "content_block_delta" | "message_delta" => {
            let json: serde_json::Value = serde_json::from_str(&data)?;
            if let Some(delta) = extract_anthropic_stream_delta(&json) {
                events.push(MantleStreamEvent::OutputTextDelta { delta });
            } else {
                events.push(MantleStreamEvent::Raw { event, data });
            }
        }
        "response.completed" => events.push(MantleStreamEvent::Completed),
        "error" => {
            let json: serde_json::Value = serde_json::from_str(&data)?;
            events.push(MantleStreamEvent::Error {
                message: json
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        json.get("error")
                            .and_then(|error| error.get("message"))
                            .and_then(serde_json::Value::as_str)
                    })
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        _ => events.push(MantleStreamEvent::Raw { event, data }),
    }

    Ok(())
}

fn extract_json_id(raw_json: &serde_json::Value) -> String {
    raw_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn mantle_response_from_raw_json(raw_json: serde_json::Value) -> MantleResponse {
    MantleResponse {
        id: extract_json_id(&raw_json),
        output_text: extract_response_output_text(&raw_json),
        raw_json,
    }
}

fn encode_path_segment(value: &str) -> String {
    utf8_percent_encode(value, AWS_PERCENT_ENCODE_SET).to_string()
}

fn extract_response_output_text(raw_json: &serde_json::Value) -> Option<String> {
    if let Some(output_text) = raw_json
        .get("output_text")
        .and_then(serde_json::Value::as_str)
    {
        return Some(output_text.to_owned());
    }

    let output = raw_json.get("output")?.as_array()?;
    let mut text = String::new();
    for item in output {
        let Some(content) = item.get("content").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for content_item in content {
            if let Some(part) = content_item
                .get("text")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    content_item
                        .get("delta")
                        .and_then(serde_json::Value::as_str)
                })
            {
                text.push_str(part);
            }
        }
    }

    if text.is_empty() { None } else { Some(text) }
}

fn extract_chat_completion_output_text(raw_json: &serde_json::Value) -> Option<String> {
    let choices = raw_json.get("choices")?.as_array()?;
    let mut text = String::new();
    for choice in choices {
        let Some(content) = choice
            .get("message")
            .and_then(|message| message.get("content"))
            .or_else(|| choice.get("delta").and_then(|delta| delta.get("content")))
        else {
            continue;
        };
        append_mantle_content_text(content, &mut text);
    }

    if text.is_empty() { None } else { Some(text) }
}

fn extract_anthropic_message_output_text(raw_json: &serde_json::Value) -> Option<String> {
    let content = raw_json.get("content")?.as_array()?;
    let mut text = String::new();
    for item in content {
        append_mantle_content_text(item, &mut text);
    }

    if text.is_empty() { None } else { Some(text) }
}

fn extract_chat_stream_delta(raw_json: &serde_json::Value) -> Option<String> {
    let choices = raw_json.get("choices")?.as_array()?;
    let mut text = String::new();
    for choice in choices {
        if let Some(content) = choice
            .get("delta")
            .and_then(|delta| delta.get("content"))
            .or_else(|| {
                choice
                    .get("message")
                    .and_then(|message| message.get("content"))
            })
        {
            append_mantle_content_text(content, &mut text);
        }
    }

    if text.is_empty() { None } else { Some(text) }
}

fn extract_anthropic_stream_delta(raw_json: &serde_json::Value) -> Option<String> {
    raw_json
        .get("delta")
        .and_then(|delta| delta.get("text"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            raw_json
                .get("content_block")
                .and_then(|block| block.get("text"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
}

fn append_mantle_content_text(content: &serde_json::Value, text: &mut String) {
    if let Some(part) = content.as_str() {
        text.push_str(part);
        return;
    }

    if let Some(part) = content.get("text").and_then(serde_json::Value::as_str) {
        text.push_str(part);
        return;
    }

    if let Some(items) = content.as_array() {
        for item in items {
            append_mantle_content_text(item, text);
        }
    }
}

fn ensure_success(status: u16, body: &[u8]) -> Result<(), MantleClientError> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(MantleClientError::HttpStatus {
            status,
            body: String::from_utf8_lossy(body).into_owned(),
        })
    }
}

pub fn mantle_status_diagnostic(status: u16, body: &str) -> BedrockProviderErrorDiagnostic {
    let (kind, retryable, summary, suggested_action) = match status {
        401 | 403 => (
            BedrockProviderErrorKind::AuthOrPermission,
            false,
            "Mantle authentication, project, or IAM authorization failed",
            "Check bearer API key or SigV4 credentials, OpenAI-Project selection, region, and IAM permissions.",
        ),
        404 => (
            BedrockProviderErrorKind::ModelOrRegionMismatch,
            false,
            "Mantle model or stored response was not found",
            "Refresh model discovery and verify model ID, stored response ID, project, region, and selected AWS profile.",
        ),
        409 | 422 => (
            BedrockProviderErrorKind::IncompatibleRequest,
            false,
            "Mantle rejected the request shape for this model/API",
            "Check Responses vs Chat Completions vs Anthropic Messages compatibility, tools, cache, background mode, and request fields.",
        ),
        429 => (
            BedrockProviderErrorKind::QuotaOrThrottle,
            true,
            "Mantle token quota or throttle limit was reached",
            "Back off using Mantle token quota guidance, reduce concurrent streams or token volume, and keep Mantle quota accounting separate from Runtime.",
        ),
        500..=599 => (
            BedrockProviderErrorKind::ServerOrUnavailable,
            true,
            "Mantle service returned a transient server error",
            "Preserve partial transcript state and retry only if the request is idempotent or can be safely continued.",
        ),
        _ => (
            BedrockProviderErrorKind::Unknown,
            false,
            "Mantle request failed",
            "Inspect the redacted response body, endpoint, model, region, project, and profile diagnostics.",
        ),
    };
    BedrockProviderErrorDiagnostic {
        plane: BedrockUsagePlane::Mantle,
        status,
        kind,
        retryable,
        summary,
        suggested_action,
        body_excerpt: redacted_body_excerpt(body),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockRuntimeClient<T, S> {
    endpoint: String,
    resolved_auth: ResolvedBedrockAuth,
    transport: T,
    signer: S,
}

impl<T, S> BedrockRuntimeClient<T, S>
where
    T: RuntimeHttpTransport,
    S: RuntimeSigV4Signer,
{
    pub fn new(
        endpoint: impl Into<String>,
        resolved_auth: ResolvedBedrockAuth,
        transport: T,
        signer: S,
    ) -> Self {
        Self {
            endpoint: endpoint.into().trim_end_matches('/').to_owned(),
            resolved_auth,
            transport,
            signer,
        }
    }

    pub async fn converse(
        &self,
        request_body: RuntimeConverseRequest,
    ) -> Result<RuntimeModelResponse, RuntimeClientError> {
        let body = serde_json::to_vec(&request_body)?;
        let response = self
            .send_json(
                "POST",
                &format!(
                    "/model/{}/converse",
                    encode_model_id(&request_body.model_id)
                ),
                body,
            )
            .await?;
        parse_runtime_response(response)
    }

    pub async fn converse_stream(
        &self,
        request_body: RuntimeConverseRequest,
    ) -> Result<Vec<RuntimeStreamEvent>, RuntimeClientError> {
        let body = serde_json::to_vec(&request_body)?;
        let response = self
            .send_json(
                "POST",
                &format!(
                    "/model/{}/converse-stream",
                    encode_model_id(&request_body.model_id)
                ),
                body,
            )
            .await?;
        ensure_runtime_success(response.status, &response.body)?;
        parse_runtime_event_stream(&response.body)
    }

    pub async fn invoke_model(
        &self,
        model_id: &str,
        body: serde_json::Value,
    ) -> Result<RuntimeModelResponse, RuntimeClientError> {
        let response = self
            .send_json(
                "POST",
                &format!("/model/{}/invoke", encode_model_id(model_id)),
                serde_json::to_vec(&body)?,
            )
            .await?;
        parse_runtime_response(response)
    }

    pub async fn invoke_model_stream(
        &self,
        model_id: &str,
        body: serde_json::Value,
    ) -> Result<Vec<RuntimeStreamEvent>, RuntimeClientError> {
        let response = self
            .send_json(
                "POST",
                &format!(
                    "/model/{}/invoke-with-response-stream",
                    encode_model_id(model_id)
                ),
                serde_json::to_vec(&body)?,
            )
            .await?;
        ensure_runtime_success(response.status, &response.body)?;
        parse_runtime_event_stream(&response.body)
    }

    pub async fn converse_stream_with_invoke_fallback(
        &self,
        request_body: RuntimeConverseRequest,
        invoke_body: serde_json::Value,
    ) -> Result<RuntimeModelResponse, RuntimeClientError> {
        match self.converse_stream(request_body.clone()).await {
            Ok(events) => Ok(runtime_response_from_stream_events(events)),
            Err(RuntimeClientError::HttpStatus { status, .. })
                if status == 400 || status == 404 =>
            {
                match self
                    .invoke_model_stream(&request_body.model_id, invoke_body.clone())
                    .await
                {
                    Ok(events) => Ok(runtime_response_from_stream_events(events)),
                    Err(RuntimeClientError::HttpStatus { status, .. })
                        if status == 400 || status == 404 =>
                    {
                        self.invoke_model(&request_body.model_id, invoke_body).await
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    async fn send_json(
        &self,
        method: &str,
        path: &str,
        body: Vec<u8>,
    ) -> Result<RuntimeHttpResponse, RuntimeClientError> {
        let mut request = RuntimeHttpRequest {
            method: method.to_owned(),
            url: format!("{}{}", self.endpoint, path),
            headers: BTreeMap::from([
                ("accept".to_owned(), "application/json".to_owned()),
                ("content-type".to_owned(), "application/json".to_owned()),
            ]),
            body,
        };
        self.signer.sign(&mut request, &self.resolved_auth)?;
        self.transport.send(request).await
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeConverseRequest {
    pub model_id: String,
    #[serde(
        rename = "additionalModelRequestFields",
        skip_serializing_if = "Option::is_none"
    )]
    pub additional_model_request_fields: Option<serde_json::Value>,
    #[serde(rename = "inferenceConfig", skip_serializing_if = "Option::is_none")]
    pub inference_config: Option<RuntimeInferenceConfiguration>,
    pub system: Vec<RuntimeSystemContentBlock>,
    pub messages: Vec<RuntimeMessage>,
    #[serde(rename = "serviceTier", skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<RuntimeServiceTier>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<RuntimeToolConfig>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeInferenceConfiguration {
    #[serde(rename = "maxTokens", skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<BedrockFloatMilli>,
    #[serde(rename = "topP", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<BedrockFloatMilli>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeServiceTier {
    #[serde(rename = "type")]
    pub tier_type: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeMessage {
    pub role: String,
    pub content: Vec<RuntimeContentBlock>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum RuntimeContentBlock {
    Text {
        text: String,
    },
    Image {
        image: RuntimeImageBlock,
    },
    ToolUse {
        #[serde(rename = "toolUse")]
        tool_use: RuntimeToolUseBlock,
    },
    ToolResult {
        #[serde(rename = "toolResult")]
        tool_result: RuntimeToolResultBlock,
    },
    CachePoint {
        #[serde(rename = "cachePoint")]
        cache_point: RuntimeCachePointBlock,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum RuntimeSystemContentBlock {
    Text {
        text: String,
    },
    CachePoint {
        #[serde(rename = "cachePoint")]
        cache_point: RuntimeCachePointBlock,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeImageBlock {
    pub format: String,
    pub source: RuntimeImageSource,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeImageSource {
    pub bytes: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeToolUseBlock {
    #[serde(rename = "toolUseId")]
    pub tool_use_id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeToolResultBlock {
    #[serde(rename = "toolUseId")]
    pub tool_use_id: String,
    pub content: Vec<RuntimeToolResultContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum RuntimeToolResultContentBlock {
    Text { text: String },
    Json { json: serde_json::Value },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeCachePointBlock {
    #[serde(rename = "type")]
    pub cache_point_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeToolConfig {
    pub tools: Vec<RuntimeTool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeTool {
    #[serde(rename = "toolSpec")]
    pub tool_spec: RuntimeToolSpec,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeToolSpec {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: RuntimeToolInputSchema,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeToolInputSchema {
    pub json: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeModelResponse {
    pub output_text: String,
    pub usage: Option<RuntimeTokenUsage>,
    pub latency_ms: Option<u64>,
    pub stop_reason: Option<String>,
    pub invoked_model_id: Option<String>,
    pub trace: Option<serde_json::Value>,
    pub raw_json: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStreamEvent {
    MessageStart {
        role: Option<String>,
    },
    ContentBlockDelta {
        text: String,
    },
    ContentBlockStop {
        content_block_index: Option<u64>,
    },
    MessageStop {
        stop_reason: Option<String>,
    },
    Metadata {
        usage: Option<RuntimeTokenUsage>,
        latency_ms: Option<u64>,
        invoked_model_id: Option<String>,
        trace: Option<serde_json::Value>,
    },
    Raw {
        event: String,
        data: String,
    },
}

pub fn normalize_runtime_stream_events(
    events: impl IntoIterator<Item = RuntimeStreamEvent>,
) -> Vec<NormalizedModelStreamEvent> {
    events
        .into_iter()
        .filter_map(|event| match event {
            RuntimeStreamEvent::MessageStart { role } => Some(NormalizedModelStreamEvent {
                provider: ModelProviderKind::BedrockRuntime,
                kind: NormalizedModelStreamEventKind::Started {
                    response_id: None,
                    role,
                },
            }),
            RuntimeStreamEvent::ContentBlockDelta { text } => Some(
                NormalizedModelStreamEvent::text_delta(ModelProviderKind::BedrockRuntime, text),
            ),
            RuntimeStreamEvent::ContentBlockStop { .. } => None,
            RuntimeStreamEvent::MessageStop { .. } => Some(NormalizedModelStreamEvent::completed(
                ModelProviderKind::BedrockRuntime,
            )),
            RuntimeStreamEvent::Metadata { usage, .. } => {
                usage.map(|usage| NormalizedModelStreamEvent {
                    provider: ModelProviderKind::BedrockRuntime,
                    kind: NormalizedModelStreamEventKind::Usage {
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        total_tokens: usage.total_tokens,
                    },
                })
            }
            RuntimeStreamEvent::Raw { event, data } => Some(NormalizedModelStreamEvent {
                provider: ModelProviderKind::BedrockRuntime,
                kind: NormalizedModelStreamEventKind::Raw { event, data },
            }),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeTokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub trait RuntimeHttpTransport {
    fn send(
        &self,
        request: RuntimeHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RuntimeHttpResponse, RuntimeClientError>> + Send + '_>>;
}

impl RuntimeHttpTransport for ReqwestBedrockHttpTransport {
    fn send(
        &self,
        request: RuntimeHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RuntimeHttpResponse, RuntimeClientError>> + Send + '_>>
    {
        Box::pin(async move {
            let RuntimeHttpRequest {
                method,
                url,
                headers,
                body,
            } = request;
            let (status, headers, body) =
                send_reqwest_request(&self.client, &method, &url, headers, body)
                    .await
                    .map_err(RuntimeClientError::Transport)?;
            Ok(RuntimeHttpResponse {
                status,
                headers,
                body,
            })
        })
    }
}

pub trait RuntimeSigV4Signer {
    fn sign(
        &self,
        request: &mut RuntimeHttpRequest,
        resolved_auth: &ResolvedBedrockAuth,
    ) -> Result<(), RuntimeClientError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticCredentialsRuntimeSigV4Signer {
    inner: StaticCredentialsMantleSigV4Signer,
}

impl StaticCredentialsRuntimeSigV4Signer {
    pub fn new(credentials: AwsStaticCredentials) -> Self {
        Self {
            inner: StaticCredentialsMantleSigV4Signer::new(credentials),
        }
    }

    pub fn with_time(mut self, signing_time: SystemTime) -> Self {
        self.inner = self.inner.with_time(signing_time);
        self
    }
}

impl RuntimeSigV4Signer for StaticCredentialsRuntimeSigV4Signer {
    fn sign(
        &self,
        request: &mut RuntimeHttpRequest,
        resolved_auth: &ResolvedBedrockAuth,
    ) -> Result<(), RuntimeClientError> {
        let mut mantle_request = MantleHttpRequest {
            method: request.method.clone(),
            url: request.url.clone(),
            headers: request.headers.clone(),
            body: request.body.clone(),
        };
        self.inner
            .sign(&mut mantle_request, resolved_auth)
            .map_err(|error| RuntimeClientError::SigV4(error.to_string()))?;
        request.headers = mantle_request.headers;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum RuntimeClientError {
    #[error("Bedrock Runtime HTTP transport error: {0}")]
    Transport(String),
    #[error("Bedrock Runtime HTTP status {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("Bedrock Runtime JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Bedrock Runtime stream error: {0}")]
    Stream(String),
    #[error("Bedrock Runtime SigV4 error: {0}")]
    SigV4(String),
}

impl RuntimeClientError {
    pub fn diagnostic(&self) -> Option<BedrockProviderErrorDiagnostic> {
        let Self::HttpStatus { status, body } = self else {
            return None;
        };
        Some(runtime_status_diagnostic(*status, body))
    }
}

fn parse_runtime_response(
    response: RuntimeHttpResponse,
) -> Result<RuntimeModelResponse, RuntimeClientError> {
    ensure_runtime_success(response.status, &response.body)?;
    let raw_json: serde_json::Value = serde_json::from_slice(&response.body)?;
    Ok(RuntimeModelResponse {
        output_text: extract_runtime_output_text(&raw_json),
        usage: raw_json.get("usage").map(runtime_token_usage_from_json),
        latency_ms: extract_runtime_latency_ms(&raw_json),
        stop_reason: raw_json
            .get("stopReason")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        invoked_model_id: extract_runtime_invoked_model_id(&raw_json, &response.headers),
        trace: raw_json.get("trace").cloned(),
        raw_json,
    })
}

fn runtime_response_from_stream_events(events: Vec<RuntimeStreamEvent>) -> RuntimeModelResponse {
    let usage = events.iter().find_map(|event| match event {
        RuntimeStreamEvent::Metadata { usage, .. } => *usage,
        _ => None,
    });
    let latency_ms = events.iter().find_map(|event| match event {
        RuntimeStreamEvent::Metadata { latency_ms, .. } => *latency_ms,
        _ => None,
    });
    let stop_reason = events.iter().find_map(|event| match event {
        RuntimeStreamEvent::MessageStop { stop_reason } => stop_reason.clone(),
        _ => None,
    });
    let invoked_model_id = events.iter().find_map(|event| match event {
        RuntimeStreamEvent::Metadata {
            invoked_model_id, ..
        } => invoked_model_id.clone(),
        _ => None,
    });
    let trace = events.iter().find_map(|event| match event {
        RuntimeStreamEvent::Metadata { trace, .. } => trace.clone(),
        _ => None,
    });
    RuntimeModelResponse {
        output_text: runtime_stream_output_text(&events),
        usage,
        latency_ms,
        stop_reason,
        invoked_model_id,
        trace,
        raw_json: serde_json::json!({ "stream_events": events.len() }),
    }
}

fn runtime_stream_output_text(events: &[RuntimeStreamEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            RuntimeStreamEvent::ContentBlockDelta { text } => Some(text.as_str()),
            RuntimeStreamEvent::MessageStart { .. }
            | RuntimeStreamEvent::ContentBlockStop { .. }
            | RuntimeStreamEvent::MessageStop { .. }
            | RuntimeStreamEvent::Metadata { .. }
            | RuntimeStreamEvent::Raw { .. } => None,
        })
        .collect()
}

fn ensure_runtime_success(status: u16, body: &[u8]) -> Result<(), RuntimeClientError> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(RuntimeClientError::HttpStatus {
            status,
            body: String::from_utf8_lossy(body).into_owned(),
        })
    }
}

pub fn runtime_status_diagnostic(status: u16, body: &str) -> BedrockProviderErrorDiagnostic {
    let (kind, retryable, summary, suggested_action) = match status {
        401 | 403 => (
            BedrockProviderErrorKind::AuthOrPermission,
            false,
            "Bedrock Runtime authentication or IAM authorization failed",
            "Check AWS credentials, selected AWS CLI profile, region, model access, inference profile permissions, and IAM policy.",
        ),
        400 | 404 => (
            BedrockProviderErrorKind::ModelOrRegionMismatch,
            false,
            "Bedrock Runtime model, ARN, API, or region was not accepted",
            "Verify model ID or ARN, region, Converse support, InvokeModel fallback eligibility, and profile/region selection.",
        ),
        409 | 422 => (
            BedrockProviderErrorKind::IncompatibleRequest,
            false,
            "Bedrock Runtime rejected the request shape",
            "Check message content blocks, tool schema compatibility, cache points, service tier, reasoning budget, and model capability overrides.",
        ),
        429 | 503 => (
            BedrockProviderErrorKind::QuotaOrThrottle,
            true,
            "Bedrock Runtime quota, throttle, or service availability limit was reached",
            "Back off using Runtime quota guidance, reduce concurrency or token volume, and keep Runtime quota accounting separate from Mantle.",
        ),
        500..=599 => (
            BedrockProviderErrorKind::ServerOrUnavailable,
            true,
            "Bedrock Runtime returned a transient server error",
            "Preserve partial transcript state and retry only if the request is safe to resume or can be continued by a new turn.",
        ),
        _ => (
            BedrockProviderErrorKind::Unknown,
            false,
            "Bedrock Runtime request failed",
            "Inspect the redacted response body, endpoint, model or ARN, region, and AWS profile diagnostics.",
        ),
    };
    BedrockProviderErrorDiagnostic {
        plane: BedrockUsagePlane::Runtime,
        status,
        kind,
        retryable,
        summary,
        suggested_action,
        body_excerpt: redacted_body_excerpt(body),
    }
}

fn redacted_body_excerpt(body: &str) -> String {
    const MAX_ERROR_BODY_EXCERPT_BYTES: usize = 1024;
    let mut redacted = redact_jsonish_secret_fields(body);
    if redacted.len() > MAX_ERROR_BODY_EXCERPT_BYTES {
        let mut end = MAX_ERROR_BODY_EXCERPT_BYTES;
        while !redacted.is_char_boundary(end) {
            end -= 1;
        }
        redacted = format!("{}...[truncated]", &redacted[..end]);
    }
    redacted
}

fn redact_jsonish_secret_fields(body: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(body) else {
        return redact_plaintext_secret_fields(body);
    };
    redact_json_value(&mut value);
    serde_json::to_string(&value).unwrap_or_else(|_| body.to_owned())
}

fn redact_json_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                let lower_key = key.to_ascii_lowercase();
                if contains_secret_field_name(&lower_key) {
                    *value = serde_json::Value::String("<redacted>".to_owned());
                } else {
                    redact_json_value(value);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_json_value(value);
            }
        }
        _ => {}
    }
}

fn redact_plaintext_secret_fields(body: &str) -> String {
    body.lines()
        .map(|line| {
            let lower_line = line.to_ascii_lowercase();
            if !contains_secret_field_name(&lower_line) {
                return line.to_owned();
            }
            if let Some((key, _)) = line.split_once(':') {
                return format!("{key}: <redacted>");
            }
            if let Some((key, _)) = line.split_once('=') {
                return format!("{key}=<redacted>");
            }
            "<redacted>".to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn contains_secret_field_name(lower_key: &str) -> bool {
    lower_key.contains("authorization")
        || lower_key.contains("api_key")
        || lower_key.contains("apikey")
        || lower_key.contains("access_token")
        || lower_key.contains("secret")
        || lower_key.contains("password")
}

fn extract_runtime_output_text(raw_json: &serde_json::Value) -> String {
    raw_json
        .pointer("/output/message/content")
        .and_then(serde_json::Value::as_array)
        .map(|content| {
            content
                .iter()
                .filter_map(|item| item.get("text").and_then(serde_json::Value::as_str))
                .collect::<String>()
        })
        .filter(|text| !text.is_empty())
        .or_else(|| {
            raw_json
                .get("body")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

fn parse_runtime_event_stream(body: &[u8]) -> Result<Vec<RuntimeStreamEvent>, RuntimeClientError> {
    parse_aws_event_stream(body)?
        .into_iter()
        .map(runtime_event_from_event_stream_message)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AwsEventStreamMessage {
    headers: BTreeMap<String, AwsEventStreamHeaderValue>,
    payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AwsEventStreamHeaderValue {
    Bool(bool),
    Byte(i8),
    Short(i16),
    Integer(i32),
    Long(i64),
    Bytes(Vec<u8>),
    String(String),
    Timestamp(i64),
    Uuid([u8; 16]),
}

impl AwsEventStreamHeaderValue {
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_str()),
            _ => None,
        }
    }
}

fn parse_aws_event_stream(body: &[u8]) -> Result<Vec<AwsEventStreamMessage>, RuntimeClientError> {
    let mut offset = 0;
    let mut messages = Vec::new();

    while offset < body.len() {
        if body.len() - offset < 16 {
            return Err(RuntimeClientError::Stream(
                "event stream frame is shorter than the 16-byte overhead".to_owned(),
            ));
        }

        let total_len = read_u32(&body[offset..offset + 4]) as usize;
        let headers_len = read_u32(&body[offset + 4..offset + 8]) as usize;
        let prelude_crc = read_u32(&body[offset + 8..offset + 12]);
        if crc32(&body[offset..offset + 8]) != prelude_crc {
            return Err(RuntimeClientError::Stream(
                "event stream prelude CRC mismatch".to_owned(),
            ));
        }
        if total_len < 16 {
            return Err(RuntimeClientError::Stream(
                "event stream frame length is smaller than overhead".to_owned(),
            ));
        }
        if offset + total_len > body.len() {
            return Err(RuntimeClientError::Stream(
                "event stream frame is truncated".to_owned(),
            ));
        }

        let message_end = offset + total_len;
        let expected_message_crc = read_u32(&body[message_end - 4..message_end]);
        if crc32(&body[offset..message_end - 4]) != expected_message_crc {
            return Err(RuntimeClientError::Stream(
                "event stream message CRC mismatch".to_owned(),
            ));
        }

        let headers_start = offset + 12;
        let headers_end = headers_start + headers_len;
        if headers_end > message_end - 4 {
            return Err(RuntimeClientError::Stream(
                "event stream headers exceed frame length".to_owned(),
            ));
        }

        messages.push(AwsEventStreamMessage {
            headers: parse_event_stream_headers(&body[headers_start..headers_end])?,
            payload: body[headers_end..message_end - 4].to_vec(),
        });
        offset = message_end;
    }

    Ok(messages)
}

fn parse_event_stream_headers(
    bytes: &[u8],
) -> Result<BTreeMap<String, AwsEventStreamHeaderValue>, RuntimeClientError> {
    let mut offset = 0;
    let mut headers = BTreeMap::new();

    while offset < bytes.len() {
        let name_len = *bytes.get(offset).ok_or_else(|| {
            RuntimeClientError::Stream("event stream header is missing name length".to_owned())
        })? as usize;
        offset += 1;
        if offset + name_len > bytes.len() {
            return Err(RuntimeClientError::Stream(
                "event stream header name is truncated".to_owned(),
            ));
        }
        let name = std::str::from_utf8(&bytes[offset..offset + name_len])
            .map_err(|error| RuntimeClientError::Stream(error.to_string()))?
            .to_owned();
        offset += name_len;
        let value_type = *bytes.get(offset).ok_or_else(|| {
            RuntimeClientError::Stream("event stream header is missing value type".to_owned())
        })?;
        offset += 1;
        let (value, consumed) = parse_event_stream_header_value(value_type, &bytes[offset..])?;
        offset += consumed;

        if headers.insert(name.clone(), value).is_some() {
            return Err(RuntimeClientError::Stream(format!(
                "duplicate event stream header: {name}"
            )));
        }
    }

    Ok(headers)
}

fn parse_event_stream_header_value(
    value_type: u8,
    bytes: &[u8],
) -> Result<(AwsEventStreamHeaderValue, usize), RuntimeClientError> {
    match value_type {
        0 => Ok((AwsEventStreamHeaderValue::Bool(true), 0)),
        1 => Ok((AwsEventStreamHeaderValue::Bool(false), 0)),
        2 => {
            let value = *bytes.first().ok_or_else(|| {
                RuntimeClientError::Stream("event stream byte header is truncated".to_owned())
            })? as i8;
            Ok((AwsEventStreamHeaderValue::Byte(value), 1))
        }
        3 => Ok((
            AwsEventStreamHeaderValue::Short(read_i16_checked(bytes)?),
            2,
        )),
        4 => Ok((
            AwsEventStreamHeaderValue::Integer(read_i32_checked(bytes)?),
            4,
        )),
        5 => Ok((AwsEventStreamHeaderValue::Long(read_i64_checked(bytes)?), 8)),
        6 => {
            let (len, data_start) = read_len_prefixed_header(bytes)?;
            Ok((
                AwsEventStreamHeaderValue::Bytes(bytes[data_start..data_start + len].to_vec()),
                data_start + len,
            ))
        }
        7 => {
            let (len, data_start) = read_len_prefixed_header(bytes)?;
            let value = std::str::from_utf8(&bytes[data_start..data_start + len])
                .map_err(|error| RuntimeClientError::Stream(error.to_string()))?
                .to_owned();
            Ok((AwsEventStreamHeaderValue::String(value), data_start + len))
        }
        8 => Ok((
            AwsEventStreamHeaderValue::Timestamp(read_i64_checked(bytes)?),
            8,
        )),
        9 => {
            if bytes.len() < 16 {
                return Err(RuntimeClientError::Stream(
                    "event stream UUID header is truncated".to_owned(),
                ));
            }
            let mut uuid = [0; 16];
            uuid.copy_from_slice(&bytes[..16]);
            Ok((AwsEventStreamHeaderValue::Uuid(uuid), 16))
        }
        _ => Err(RuntimeClientError::Stream(format!(
            "unsupported event stream header value type: {value_type}"
        ))),
    }
}

fn runtime_event_from_event_stream_message(
    message: AwsEventStreamMessage,
) -> Result<RuntimeStreamEvent, RuntimeClientError> {
    match message
        .headers
        .get(":message-type")
        .and_then(AwsEventStreamHeaderValue::as_str)
    {
        Some("error") | Some("exception") => {
            let error_code = message
                .headers
                .get(":error-code")
                .and_then(AwsEventStreamHeaderValue::as_str)
                .unwrap_or("RuntimeStreamException");
            let error_message = message
                .headers
                .get(":error-message")
                .and_then(AwsEventStreamHeaderValue::as_str)
                .unwrap_or("Bedrock Runtime stream returned an exception");
            Err(RuntimeClientError::Stream(format!(
                "{error_code}: {error_message}"
            )))
        }
        _ => runtime_event_from_payload(&message.payload),
    }
}

fn runtime_event_from_payload(payload: &[u8]) -> Result<RuntimeStreamEvent, RuntimeClientError> {
    let json: serde_json::Value = serde_json::from_slice(payload)?;
    if let Some(value) = json.get("messageStart") {
        return Ok(RuntimeStreamEvent::MessageStart {
            role: value
                .get("role")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        });
    }
    if let Some(value) = json.get("contentBlockDelta") {
        return Ok(RuntimeStreamEvent::ContentBlockDelta {
            text: value
                .pointer("/delta/text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        });
    }
    if let Some(value) = json.get("contentBlockStop") {
        return Ok(RuntimeStreamEvent::ContentBlockStop {
            content_block_index: value
                .get("contentBlockIndex")
                .and_then(serde_json::Value::as_u64),
        });
    }
    if let Some(value) = json.get("messageStop") {
        return Ok(RuntimeStreamEvent::MessageStop {
            stop_reason: value
                .get("stopReason")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        });
    }
    if let Some(value) = json.get("metadata") {
        return Ok(RuntimeStreamEvent::Metadata {
            usage: value.get("usage").map(runtime_token_usage_from_json),
            latency_ms: extract_runtime_latency_ms(value),
            invoked_model_id: extract_runtime_invoked_model_id(value, &BTreeMap::new()),
            trace: value.get("trace").cloned(),
        });
    }
    if json.get("chunk").is_some() || json.get("bytes").is_some() {
        return runtime_event_from_invoke_stream_payload(&json);
    }

    let event = json
        .as_object()
        .and_then(|object| object.keys().next())
        .cloned()
        .unwrap_or_else(|| "unknown".to_owned());
    Ok(RuntimeStreamEvent::Raw {
        event,
        data: String::from_utf8_lossy(payload).into_owned(),
    })
}

fn runtime_event_from_invoke_stream_payload(
    json: &serde_json::Value,
) -> Result<RuntimeStreamEvent, RuntimeClientError> {
    let Some(encoded) = json
        .pointer("/chunk/bytes")
        .and_then(serde_json::Value::as_str)
        .or_else(|| json.get("bytes").and_then(serde_json::Value::as_str))
    else {
        return Ok(RuntimeStreamEvent::Raw {
            event: "chunk".to_owned(),
            data: json.to_string(),
        });
    };
    let decoded = BASE64_STANDARD.decode(encoded).map_err(|error| {
        RuntimeClientError::Stream(format!("invoke stream chunk base64 decode failed: {error}"))
    })?;
    let text = extract_runtime_invoke_chunk_text(&decoded)?;
    Ok(RuntimeStreamEvent::ContentBlockDelta { text })
}

fn extract_runtime_invoke_chunk_text(bytes: &[u8]) -> Result<String, RuntimeClientError> {
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Ok(String::from_utf8_lossy(bytes).into_owned());
    };

    if let Some(text) = extract_runtime_output_text_pointer(&json) {
        return Ok(text);
    }
    if let Some(text) = extract_runtime_content_array_text(&json) {
        return Ok(text);
    }

    Ok(extract_runtime_output_text(&json))
}

fn extract_runtime_output_text_pointer(json: &serde_json::Value) -> Option<String> {
    [
        "/delta/text",
        "/completion",
        "/generation",
        "/outputText",
        "/text",
        "/outputs/0/text",
        "/message/content/0/text",
    ]
    .iter()
    .find_map(|pointer| {
        json.pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    })
}

fn extract_runtime_content_array_text(json: &serde_json::Value) -> Option<String> {
    let content = json.get("content")?.as_array()?;
    let text = content
        .iter()
        .filter_map(|item| item.get("text").and_then(serde_json::Value::as_str))
        .collect::<String>();
    (!text.is_empty()).then_some(text)
}

fn runtime_token_usage_from_json(value: &serde_json::Value) -> RuntimeTokenUsage {
    RuntimeTokenUsage {
        input_tokens: value.get("inputTokens").and_then(serde_json::Value::as_u64),
        output_tokens: value
            .get("outputTokens")
            .and_then(serde_json::Value::as_u64),
        total_tokens: value.get("totalTokens").and_then(serde_json::Value::as_u64),
        cache_read_tokens: first_u64(
            value,
            &[
                "cacheReadInputTokens",
                "cacheReadTokens",
                "cacheReadInputTokenCount",
                "cachedInputTokens",
            ],
        ),
        cache_write_tokens: first_u64(
            value,
            &[
                "cacheWriteInputTokens",
                "cacheWriteTokens",
                "cacheWriteInputTokenCount",
            ],
        ),
    }
}

fn extract_runtime_latency_ms(value: &serde_json::Value) -> Option<u64> {
    value
        .pointer("/metrics/latencyMs")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| value.get("latencyMs").and_then(serde_json::Value::as_u64))
}

fn extract_runtime_invoked_model_id(
    value: &serde_json::Value,
    headers: &BTreeMap<String, String>,
) -> Option<String> {
    value
        .get("invokedModelId")
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("modelId").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .or_else(|| header_case_insensitive(headers, "x-amzn-bedrock-invoked-model-id"))
}

fn first_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_u64))
}

fn header_case_insensitive(headers: &BTreeMap<String, String>, name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

fn read_len_prefixed_header(bytes: &[u8]) -> Result<(usize, usize), RuntimeClientError> {
    if bytes.len() < 2 {
        return Err(RuntimeClientError::Stream(
            "event stream length-prefixed header is truncated".to_owned(),
        ));
    }
    let len = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
    if bytes.len() < 2 + len {
        return Err(RuntimeClientError::Stream(
            "event stream length-prefixed header value is truncated".to_owned(),
        ));
    }
    Ok((len, 2))
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_i16_checked(bytes: &[u8]) -> Result<i16, RuntimeClientError> {
    if bytes.len() < 2 {
        return Err(RuntimeClientError::Stream(
            "event stream i16 header is truncated".to_owned(),
        ));
    }
    Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_i32_checked(bytes: &[u8]) -> Result<i32, RuntimeClientError> {
    if bytes.len() < 4 {
        return Err(RuntimeClientError::Stream(
            "event stream i32 header is truncated".to_owned(),
        ));
    }
    Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_i64_checked(bytes: &[u8]) -> Result<i64, RuntimeClientError> {
    if bytes.len() < 8 {
        return Err(RuntimeClientError::Stream(
            "event stream i64 header is truncated".to_owned(),
        ));
    }
    Ok(i64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut hasher = Crc32Hasher::new();
    hasher.update(bytes);
    hasher.finalize()
}

fn encode_model_id(model_id: &str) -> String {
    utf8_percent_encode(model_id, AWS_PERCENT_ENCODE_SET).to_string()
}

fn sigv4_amz_date(time: SystemTime) -> String {
    let datetime = OffsetDateTime::from(time);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        datetime.year(),
        u8::from(datetime.month()),
        datetime.day(),
        datetime.hour(),
        datetime.minute(),
        datetime.second()
    )
}

fn sigv4_datestamp(time: SystemTime) -> String {
    let datetime = OffsetDateTime::from(time);
    format!(
        "{:04}{:02}{:02}",
        datetime.year(),
        u8::from(datetime.month()),
        datetime.day()
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sigv4_signing_key(secret_access_key: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let date_key = hmac_sha256(
        format!("AWS4{secret_access_key}").as_bytes(),
        date.as_bytes(),
    );
    let date_region_key = hmac_sha256(&date_key, region.as_bytes());
    let date_region_service_key = hmac_sha256(&date_region_key, service.as_bytes());
    hmac_sha256(&date_region_service_key, b"aws4_request")
}

fn canonical_uri(path: &str) -> String {
    if path.is_empty() {
        "/".to_owned()
    } else {
        path.split('/')
            .map(|segment| utf8_percent_encode(segment, AWS_PERCENT_ENCODE_SET).to_string())
            .collect::<Vec<_>>()
            .join("/")
    }
}

fn canonical_query(url: &Url) -> String {
    let mut pairs = url
        .query_pairs()
        .map(|(key, value)| {
            (
                utf8_percent_encode(&key, AWS_PERCENT_ENCODE_SET).to_string(),
                utf8_percent_encode(&value, AWS_PERCENT_ENCODE_SET).to_string(),
            )
        })
        .collect::<Vec<_>>();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn canonical_headers(headers: &BTreeMap<String, String>) -> (String, String) {
    let mut normalized = headers
        .iter()
        .map(|(name, value)| {
            (
                name.to_ascii_lowercase(),
                value.split_whitespace().collect::<Vec<_>>().join(" "),
            )
        })
        .collect::<Vec<_>>();
    normalized.sort_by(|left, right| left.0.cmp(&right.0));

    let canonical_headers = normalized
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect::<String>();
    let signed_headers = normalized
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");

    (canonical_headers, signed_headers)
}

fn read_optional_to_string(path: &Path) -> Result<String, AwsSharedConfigError> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(AwsSharedConfigError::ReadFailed {
            path: path.to_owned(),
            message: error.to_string(),
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IniDocumentKind {
    Config,
    Credentials,
}

fn parse_aws_ini(contents: &str, kind: IniDocumentKind) -> BTreeMap<String, AwsSharedProfile> {
    let mut profiles = BTreeMap::new();
    let mut current_name: Option<String> = None;

    for line in contents.lines() {
        let line = strip_ini_comment(line).trim();
        if line.is_empty() {
            continue;
        }

        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            let profile_name = normalize_profile_section(section.trim(), kind);
            profiles
                .entry(profile_name.clone())
                .or_insert_with(|| AwsSharedProfile {
                    name: profile_name.clone(),
                    ..AwsSharedProfile::default()
                });
            current_name = Some(profile_name);
            continue;
        }

        let Some(profile_name) = current_name.as_deref() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let key = key.trim();
        let value = value.trim().to_owned();
        let profile = profiles
            .entry(profile_name.to_owned())
            .or_insert_with(|| AwsSharedProfile {
                name: profile_name.to_owned(),
                ..AwsSharedProfile::default()
            });

        match key {
            "region" => profile.region = Some(value),
            "aws_access_key_id" if !value.is_empty() => {
                profile.static_access_key_id = Some(value);
                profile.has_static_credentials = profile.static_access_key_id.is_some()
                    && profile.static_secret_access_key.is_some();
            }
            "aws_secret_access_key" if !value.is_empty() => {
                profile.static_secret_access_key = Some(value);
                profile.has_static_credentials = profile.static_access_key_id.is_some()
                    && profile.static_secret_access_key.is_some();
            }
            "aws_session_token" if !value.is_empty() => profile.static_session_token = Some(value),
            "credential_process" => profile.credential_process = Some(value),
            "sso_session" => profile.sso_session = Some(value),
            "role_arn" => profile.role_arn = Some(value),
            "source_profile" => profile.source_profile = Some(value),
            _ => {}
        }
    }

    profiles
}

fn normalize_profile_section(section: &str, kind: IniDocumentKind) -> String {
    if kind == IniDocumentKind::Config && section != "default" {
        section
            .strip_prefix("profile ")
            .unwrap_or(section)
            .trim()
            .to_owned()
    } else {
        section.to_owned()
    }
}

fn strip_ini_comment(line: &str) -> &str {
    let hash = line.find('#');
    let semicolon = line.find(';');
    match (hash, semicolon) {
        (Some(hash), Some(semicolon)) => &line[..hash.min(semicolon)],
        (Some(index), None) | (None, Some(index)) => &line[..index],
        (None, None) => line,
    }
}

async fn send_reqwest_request(
    client: &reqwest::Client,
    method: &str,
    url: &str,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
) -> Result<(u16, BTreeMap<String, String>, Vec<u8>), String> {
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|error| format!("invalid HTTP method {method}: {error}"))?;
    let mut builder = client.request(method, url);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let response = builder.body(body).send().await.map_err(|error| {
        if let Some(status) = error.status() {
            format!("HTTP client error {status}: {error}")
        } else {
            error.to_string()
        }
    })?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let body = response
        .bytes()
        .await
        .map_err(|error| error.to_string())?
        .to_vec();
    Ok((status, headers, body))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;

    use super::*;

    #[test]
    fn t5_parses_aws_cli_config_and_credentials_profiles() {
        let config = r#"
            [default]
            region = us-east-1

            [profile dev]
            region = us-west-2
            sso_session = corp
        "#;
        let credentials = r#"
            [dev]
            aws_access_key_id = test-access
            aws_secret_access_key = test-secret
        "#;

        let shared = AwsSharedConfig::from_ini_documents(config, credentials);
        let profile = shared.profile("dev").unwrap();

        assert_eq!(profile.region.as_deref(), Some("us-west-2"));
        assert!(profile.has_static_credentials);
        assert!(shared.static_credentials_for_profile("dev").is_some());
        assert_eq!(profile.sso_session.as_deref(), Some("corp"));
        assert!(profile.can_resolve_credentials());
    }

    #[test]
    fn v16_cli_profile_resolves_mantle_and_runtime_endpoints_together() {
        let shared = AwsSharedConfig::from_ini_documents(
            "[profile dev]\nregion = eu-central-1\n",
            "[dev]\naws_access_key_id = x\naws_secret_access_key = y\n",
        );

        let resolved = shared
            .resolve_bedrock_auth(
                &AwsCredentialSource::CliProfile {
                    profile_name: "dev".to_owned(),
                },
                None,
            )
            .unwrap();

        assert_eq!(resolved.region, "eu-central-1");
        assert_eq!(
            resolved.mantle_sigv4_base_url,
            "https://bedrock-mantle.eu-central-1.api.aws/v1"
        );
        assert_eq!(
            resolved.runtime_endpoint,
            "https://bedrock-runtime.eu-central-1.amazonaws.com"
        );
    }

    #[test]
    fn v16_explicit_region_overrides_cli_profile_region_for_both_endpoints() {
        let shared = AwsSharedConfig::from_ini_documents("[profile dev]\nregion = us-east-1\n", "");

        let resolved = shared
            .resolve_bedrock_auth(
                &AwsCredentialSource::CliProfile {
                    profile_name: "dev".to_owned(),
                },
                Some("ap-southeast-2"),
            )
            .unwrap();

        assert_eq!(resolved.region, "ap-southeast-2");
        assert_eq!(
            resolved.mantle_sigv4_base_url,
            "https://bedrock-mantle.ap-southeast-2.api.aws/v1"
        );
        assert_eq!(
            resolved.runtime_endpoint,
            "https://bedrock-runtime.ap-southeast-2.amazonaws.com"
        );
    }

    #[test]
    fn v16_cli_profile_resolution_metadata_covers_aws_profile_flows() {
        let shared = AwsSharedConfig::from_ini_documents(
            r#"
            [profile role]
            region = us-east-1
            role_arn = arn:aws:iam::123456789012:role/Fastrock
            source_profile = base

            [profile sso]
            region = us-west-2
            sso_session = corp
            sso_account_id = 123456789012
            sso_role_name = Developer

            [profile process]
            region = eu-west-1
            credential_process = /usr/local/bin/aws-creds
            "#,
            r#"
            [base]
            aws_access_key_id = base-key
            aws_secret_access_key = base-secret

            [static]
            region = ap-southeast-2
            aws_access_key_id = static-key
            aws_secret_access_key = static-secret
            "#,
        );

        let role = shared
            .resolve_bedrock_auth(
                &AwsCredentialSource::CliProfile {
                    profile_name: "role".to_owned(),
                },
                None,
            )
            .unwrap()
            .credential_profile
            .unwrap();
        assert_eq!(
            role.resolution_kind,
            AwsCredentialResolutionKind::AssumeRole
        );
        assert_eq!(role.source_profile.as_deref(), Some("base"));
        assert_eq!(
            role.role_arn.as_deref(),
            Some("arn:aws:iam::123456789012:role/Fastrock")
        );
        assert!(!role.has_static_credentials);

        let sso = shared
            .resolve_bedrock_auth(
                &AwsCredentialSource::CliProfile {
                    profile_name: "sso".to_owned(),
                },
                None,
            )
            .unwrap()
            .credential_profile
            .unwrap();
        assert_eq!(sso.resolution_kind, AwsCredentialResolutionKind::SsoSession);
        assert_eq!(sso.sso_session.as_deref(), Some("corp"));

        let process = shared
            .resolve_bedrock_auth(
                &AwsCredentialSource::CliProfile {
                    profile_name: "process".to_owned(),
                },
                None,
            )
            .unwrap()
            .credential_profile
            .unwrap();
        assert_eq!(
            process.resolution_kind,
            AwsCredentialResolutionKind::CredentialProcess
        );
        assert!(process.credential_process_configured);

        let static_profile = shared
            .resolve_bedrock_auth(
                &AwsCredentialSource::CliProfile {
                    profile_name: "static".to_owned(),
                },
                Some("ap-southeast-2"),
            )
            .unwrap()
            .credential_profile
            .unwrap();
        assert_eq!(
            static_profile.resolution_kind,
            AwsCredentialResolutionKind::StaticCredentials
        );
        assert!(static_profile.has_static_credentials);
    }

    #[test]
    fn t5_loads_fake_shared_config_files_without_real_aws() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("config");
        let credentials_path = temp_dir.path().join("credentials");
        fs::write(&config_path, "[profile test]\nregion = us-west-2\n").unwrap();
        fs::write(&credentials_path, "[test]\ncredential_process = echo {}\n").unwrap();

        let files = AwsSharedConfigFiles::new(config_path, credentials_path);
        let shared = files.load().unwrap();
        let profile = shared.profile("test").unwrap();

        assert_eq!(profile.region.as_deref(), Some("us-west-2"));
        assert_eq!(profile.credential_process.as_deref(), Some("echo {}"));
    }

    #[tokio::test]
    async fn t7_mantle_models_uses_bearer_auth_and_project_header() {
        let transport = FakeTransport::new(vec![MantleHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"data":[{"id":"anthropic.test","owned_by":"aws"}]}"#.to_vec(),
        }]);
        let client = MantleClient::new(
            "https://bedrock-mantle.us-east-1.api.aws/v1",
            MantleClientAuth::BearerApiKey {
                api_key: "test-key".to_owned(),
                project_id: Some("project-1".to_owned()),
            },
            transport.clone(),
            MissingMantleSigV4Signer,
        );

        let models = client.list_models().await.unwrap();

        assert_eq!(models[0].id, "anthropic.test");
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests[0].method, "GET");
        assert_eq!(
            requests[0].url,
            "https://bedrock-mantle.us-east-1.api.aws/v1/models"
        );
        assert_eq!(
            requests[0].headers.get("authorization").unwrap(),
            "Bearer test-key"
        );
        assert_eq!(
            requests[0].headers.get("openai-project").unwrap(),
            "project-1"
        );
    }

    #[tokio::test]
    async fn t7_mantle_response_posts_json_and_extracts_output_text() {
        let transport = FakeTransport::new(vec![MantleHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"id":"resp-1","output_text":"done"}"#.to_vec(),
        }]);
        let client = MantleClient::new(
            "https://bedrock-mantle.us-east-1.api.aws/v1",
            MantleClientAuth::BearerApiKey {
                api_key: "test-key".to_owned(),
                project_id: None,
            },
            transport.clone(),
            MissingMantleSigV4Signer,
        );

        let response = client
            .create_response(MantleResponseRequest {
                model: "anthropic.test".to_owned(),
                input: "hello".to_owned(),
                previous_response_id: Some("resp-parent".to_owned()),
                store: true,
                stream: true,
                max_output_tokens: None,
                temperature: None,
                top_p: None,
            })
            .await
            .unwrap();

        assert_eq!(response.id, "resp-1");
        assert_eq!(response.output_text.as_deref(), Some("done"));
        let requests = transport.requests.lock().unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "anthropic.test");
        assert_eq!(body["input"], "hello");
        assert_eq!(body["previous_response_id"], "resp-parent");
        assert_eq!(body["store"], true);
        assert_eq!(body["stream"], false);
    }

    #[tokio::test]
    async fn t7_mantle_response_can_disable_store_for_local_state_only_request() {
        let transport = FakeTransport::new(vec![MantleHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"id":"resp-local","output_text":"local done"}"#.to_vec(),
        }]);
        let client = MantleClient::new(
            "https://bedrock-mantle.us-east-1.api.aws/v1",
            MantleClientAuth::BearerApiKey {
                api_key: "test-key".to_owned(),
                project_id: None,
            },
            transport.clone(),
            MissingMantleSigV4Signer,
        );

        client
            .create_response(MantleResponseRequest {
                model: "anthropic.test".to_owned(),
                input: "hello".to_owned(),
                previous_response_id: None,
                store: false,
                stream: false,
                max_output_tokens: None,
                temperature: None,
                top_p: None,
            })
            .await
            .unwrap();

        let requests = transport.requests.lock().unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["store"], false);
        assert!(body.get("previous_response_id").is_none());
    }

    #[test]
    fn t7_mantle_store_policy_chains_previous_response_inside_same_project() {
        let settings = MantleSettings {
            project_id: Some("project-1".to_owned()),
            store_default: true,
            api_shape: MantleApiShape::Responses,
            auth_mode: MantleAuthMode::AwsSigV4,
            use_background: false,
        };
        let policy = MantleStoredResponsePolicy::from_settings(&settings, None);
        let state = MantleStoredResponseState {
            project_id: Some("project-1".to_owned()),
            last_response_id: Some("resp-parent".to_owned()),
        };

        let request = policy.prepare_request(
            MantleResponseRequest {
                model: "anthropic.test".to_owned(),
                input: "continue".to_owned(),
                previous_response_id: None,
                store: false,
                stream: false,
                max_output_tokens: None,
                temperature: None,
                top_p: None,
            },
            &state,
        );
        let next_state = policy.state_after_response_id(Some("resp-child".to_owned()));

        assert!(request.store);
        assert_eq!(request.previous_response_id.as_deref(), Some("resp-parent"));
        assert_eq!(
            next_state,
            MantleStoredResponseState {
                project_id: Some("project-1".to_owned()),
                last_response_id: Some("resp-child".to_owned()),
            }
        );
    }

    #[test]
    fn t7_mantle_store_policy_clears_previous_response_when_store_disabled() {
        let settings = MantleSettings {
            project_id: Some("project-1".to_owned()),
            store_default: true,
            api_shape: MantleApiShape::Responses,
            auth_mode: MantleAuthMode::AwsSigV4,
            use_background: false,
        };
        let policy = MantleStoredResponsePolicy::from_settings(&settings, Some(false));
        let state = MantleStoredResponseState {
            project_id: Some("project-1".to_owned()),
            last_response_id: Some("resp-parent".to_owned()),
        };

        let request = policy.prepare_request(
            MantleResponseRequest {
                model: "anthropic.test".to_owned(),
                input: "local only".to_owned(),
                previous_response_id: Some("resp-parent".to_owned()),
                store: true,
                stream: false,
                max_output_tokens: None,
                temperature: None,
                top_p: None,
            },
            &state,
        );

        assert!(!request.store);
        assert!(request.previous_response_id.is_none());
        assert_eq!(
            policy.state_after_response_id(Some("resp-ignored".to_owned())),
            MantleStoredResponseState::default()
        );
    }

    #[test]
    fn t7_mantle_store_policy_never_crosses_project_boundary() {
        let policy = MantleStoredResponsePolicy::new(Some("project-2".to_owned()), true);
        let state = MantleStoredResponseState {
            project_id: Some("project-1".to_owned()),
            last_response_id: Some("resp-project-1".to_owned()),
        };

        let request = policy.prepare_request(
            MantleResponseRequest {
                model: "anthropic.test".to_owned(),
                input: "new project".to_owned(),
                previous_response_id: Some("resp-project-1".to_owned()),
                store: true,
                stream: false,
                max_output_tokens: None,
                temperature: None,
                top_p: None,
            },
            &state,
        );
        let response = MantleResponse {
            id: "resp-project-2".to_owned(),
            output_text: None,
            raw_json: serde_json::json!({ "id": "resp-project-2" }),
        };

        assert!(request.previous_response_id.is_none());
        assert_eq!(
            policy.state_after_response(&response),
            MantleStoredResponseState {
                project_id: Some("project-2".to_owned()),
                last_response_id: Some("resp-project-2".to_owned()),
            }
        );
    }

    #[tokio::test]
    async fn t7_mantle_get_response_retrieves_stored_response_with_project_header() {
        let transport = FakeTransport::new(vec![MantleHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"id":"resp:1/child","output":[{"content":[{"text":"stored done"}]}]}"#
                .to_vec(),
        }]);
        let client = MantleClient::new(
            "https://bedrock-mantle.us-east-1.api.aws/v1",
            MantleClientAuth::BearerApiKey {
                api_key: "test-key".to_owned(),
                project_id: Some("project-1".to_owned()),
            },
            transport.clone(),
            MissingMantleSigV4Signer,
        );

        let response = client.get_response("resp:1/child").await.unwrap();

        assert_eq!(response.id, "resp:1/child");
        assert_eq!(response.output_text.as_deref(), Some("stored done"));
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests[0].method, "GET");
        assert_eq!(
            requests[0].url,
            "https://bedrock-mantle.us-east-1.api.aws/v1/responses/resp%3A1%2Fchild"
        );
        assert!(requests[0].body.is_empty());
        assert_eq!(
            requests[0].headers.get("authorization").unwrap(),
            "Bearer test-key"
        );
        assert_eq!(
            requests[0].headers.get("openai-project").unwrap(),
            "project-1"
        );
    }

    #[tokio::test]
    async fn v10_mantle_model_discovery_records_api_shapes_and_fallback_order() {
        let transport = FakeTransport::new(vec![MantleHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"data":[{"id":"chat-only","supported_api_shapes":["chat_completions","anthropic_messages"]},{"id":"unknown"}]}"#.to_vec(),
        }]);
        let client = MantleClient::new(
            "https://bedrock-mantle.us-east-1.api.aws/v1",
            MantleClientAuth::BearerApiKey {
                api_key: "test-key".to_owned(),
                project_id: None,
            },
            transport,
            MissingMantleSigV4Signer,
        );

        let models = client.list_models().await.unwrap();

        assert_eq!(
            models[0].api_shapes,
            vec![
                MantleApiShape::ChatCompletions,
                MantleApiShape::AnthropicMessages
            ]
        );
        assert_eq!(
            models[0].choose_api_shape(MantleApiShape::Responses),
            Some(MantleApiShape::ChatCompletions)
        );
        assert_eq!(models[1].choose_api_shape(MantleApiShape::Responses), None);
    }

    #[tokio::test]
    async fn v10_mantle_chat_completion_posts_fallback_endpoint_and_extracts_text() {
        let transport = FakeTransport::new(vec![MantleHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"id":"chatcmpl-1","choices":[{"message":{"role":"assistant","content":"chat done"}}]}"#.to_vec(),
        }]);
        let client = MantleClient::new(
            "https://bedrock-mantle.us-east-1.api.aws/v1",
            MantleClientAuth::BearerApiKey {
                api_key: "test-key".to_owned(),
                project_id: None,
            },
            transport.clone(),
            MissingMantleSigV4Signer,
        );

        let response = client
            .create_chat_completion(MantleChatCompletionRequest {
                model: "chat-only".to_owned(),
                messages: vec![MantleChatMessage {
                    role: "user".to_owned(),
                    content: "hello".to_owned(),
                }],
                store: Some(true),
                stream: true,
                max_tokens: None,
                temperature: None,
                top_p: None,
            })
            .await
            .unwrap();

        assert_eq!(response.id, "chatcmpl-1");
        assert_eq!(response.output_text.as_deref(), Some("chat done"));
        let requests = transport.requests.lock().unwrap();
        assert_eq!(
            requests[0].url,
            "https://bedrock-mantle.us-east-1.api.aws/v1/chat/completions"
        );
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["store"], true);
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][0]["content"], "hello");
    }

    #[tokio::test]
    async fn v10_mantle_anthropic_messages_posts_fallback_endpoint_and_extracts_text() {
        let transport = FakeTransport::new(vec![MantleHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"id":"msg-1","content":[{"type":"text","text":"message done"}]}"#.to_vec(),
        }]);
        let client = MantleClient::new(
            "https://bedrock-mantle.us-east-1.api.aws/v1",
            MantleClientAuth::BearerApiKey {
                api_key: "test-key".to_owned(),
                project_id: None,
            },
            transport.clone(),
            MissingMantleSigV4Signer,
        );

        let response = client
            .create_anthropic_message(MantleAnthropicMessagesRequest {
                model: "anthropic-only".to_owned(),
                messages: vec![MantleChatMessage {
                    role: "user".to_owned(),
                    content: "hello".to_owned(),
                }],
                max_tokens: 128,
                stream: true,
                temperature: None,
                top_p: None,
            })
            .await
            .unwrap();

        assert_eq!(response.id, "msg-1");
        assert_eq!(response.output_text.as_deref(), Some("message done"));
        let requests = transport.requests.lock().unwrap();
        assert_eq!(
            requests[0].url,
            "https://bedrock-mantle.us-east-1.api.aws/v1/messages"
        );
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["stream"], false);
        assert_eq!(body["max_tokens"], 128);
    }

    #[test]
    fn v10_mantle_sse_parser_accepts_chat_and_anthropic_delta_shapes() {
        let events = parse_mantle_sse(
            "data: {\"choices\":[{\"delta\":{\"content\":\"chat\"}}]}\n\n\
             event: content_block_delta\n\
             data: {\"delta\":{\"type\":\"text_delta\",\"text\":\" message\"}}\n\n\
             data: [DONE]\n\n",
        )
        .unwrap();

        assert_eq!(
            events,
            vec![
                MantleStreamEvent::OutputTextDelta {
                    delta: "chat".to_owned()
                },
                MantleStreamEvent::OutputTextDelta {
                    delta: " message".to_owned()
                },
                MantleStreamEvent::Completed,
            ]
        );
    }

    #[tokio::test]
    async fn t7_mantle_stream_parses_sse_events() {
        let transport = FakeTransport::new(vec![MantleHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: b"event: response.created\ndata: {\"id\":\"resp-1\"}\n\nevent: response.output_text.delta\ndata: {\"delta\":\"hi\"}\n\nevent: response.completed\ndata: {}\n\n".to_vec(),
        }]);
        let client = MantleClient::new(
            "https://bedrock-mantle.us-east-1.api.aws/v1",
            MantleClientAuth::BearerApiKey {
                api_key: "test-key".to_owned(),
                project_id: None,
            },
            transport,
            MissingMantleSigV4Signer,
        );

        let events = client
            .stream_response(MantleResponseRequest {
                model: "anthropic.test".to_owned(),
                input: "hello".to_owned(),
                previous_response_id: None,
                store: true,
                stream: false,
                max_output_tokens: None,
                temperature: None,
                top_p: None,
            })
            .await
            .unwrap();

        assert_eq!(
            events,
            vec![
                MantleStreamEvent::ResponseCreated {
                    id: "resp-1".to_owned()
                },
                MantleStreamEvent::OutputTextDelta {
                    delta: "hi".to_owned()
                },
                MantleStreamEvent::Completed,
            ]
        );
    }

    #[tokio::test]
    async fn v16_mantle_sigv4_auth_uses_shared_aws_profile_resolution() {
        let shared = AwsSharedConfig::from_ini_documents(
            "[profile mantle]\nregion = us-east-2\n",
            "[mantle]\naws_access_key_id = x\naws_secret_access_key = y\n",
        );
        let resolved = shared
            .resolve_bedrock_auth(
                &AwsCredentialSource::CliProfile {
                    profile_name: "mantle".to_owned(),
                },
                None,
            )
            .unwrap();
        let transport = FakeTransport::new(vec![MantleHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"data":[]}"#.to_vec(),
        }]);
        let credentials = shared.static_credentials_for_profile("mantle").unwrap();
        let client = MantleClient::new(
            resolved.mantle_sigv4_base_url.clone(),
            MantleClientAuth::AwsSigV4 {
                resolved_auth: resolved,
                project_id: None,
            },
            transport.clone(),
            StaticCredentialsMantleSigV4Signer::new(credentials)
                .with_time(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
        );

        client.list_models().await.unwrap();

        let requests = transport.requests.lock().unwrap();
        assert_eq!(
            requests[0].url,
            "https://bedrock-mantle.us-east-2.api.aws/v1/models"
        );
        let authorization = requests[0].headers.get("authorization").unwrap();
        assert!(authorization.starts_with("AWS4-HMAC-SHA256 "));
        assert!(authorization.contains("Credential=x/20231114/us-east-2/bedrock/aws4_request"));
        assert!(authorization.contains("SignedHeaders="));
        assert!(authorization.contains("Signature="));
        assert_eq!(
            requests[0].headers.get("x-amz-date").unwrap(),
            "20231114T221320Z"
        );
        assert!(requests[0].headers.contains_key("x-amz-content-sha256"));
    }

    #[tokio::test]
    async fn t7_reqwest_transport_calls_mantle_models_endpoint() {
        let server = OneShotHttpServer::start(
            200,
            "application/json",
            r#"{"data":[{"id":"anthropic.live","owned_by":"aws"}]}"#,
        )
        .await;
        let client = MantleClient::new(
            format!("{}/v1", server.base_url),
            MantleClientAuth::BearerApiKey {
                api_key: "live-key".to_owned(),
                project_id: Some("project-live".to_owned()),
            },
            ReqwestBedrockHttpTransport::default(),
            MissingMantleSigV4Signer,
        );

        let models = client.list_models().await.unwrap();
        let request = server.request().await;

        assert_eq!(models[0].id, "anthropic.live");
        assert!(request.starts_with("GET /v1/models HTTP/1.1"));
        assert!(request.contains("authorization: Bearer live-key"));
        assert!(request.contains("openai-project: project-live"));
    }

    #[test]
    fn v9_static_credentials_debug_output_redacts_secret_material() {
        let credentials = AwsStaticCredentials::new(
            "access-key",
            "not-a-real-secret-value",
            Some("not-a-real-session-token".to_owned()),
        );
        let debug = format!("{credentials:?}");

        assert!(debug.contains("access-key"));
        assert!(!debug.contains("not-a-real-secret-value"));
        assert!(!debug.contains("not-a-real-session-token"));
    }

    #[test]
    fn t8_mantle_diagnostics_expose_project_store_quota_and_cloudtrail() {
        let profile = BedrockProfile {
            region: "us-east-1".to_owned(),
            endpoint_override: None,
            credential_source: AwsCredentialSource::CliProfile {
                profile_name: "default".to_owned(),
            },
            mantle: Some(MantleSettings {
                project_id: Some("proj_123".to_owned()),
                store_default: true,
                api_shape: MantleApiShape::Responses,
                auth_mode: MantleAuthMode::AwsSigV4,
                use_background: true,
            }),
            runtime: None,
        };
        let resolved = ResolvedBedrockAuth {
            credential_source: profile.credential_source.clone(),
            credential_profile: None,
            region: "us-east-1".to_owned(),
            mantle_sigv4_base_url: mantle_base_url("us-east-1"),
            runtime_endpoint: runtime_endpoint("us-east-1"),
        };

        let diagnostics = profile.mantle_diagnostics(&resolved).unwrap();

        assert_eq!(diagnostics.project_id.as_deref(), Some("proj_123"));
        assert!(diagnostics.store_default);
        assert_eq!(diagnostics.quota.usage_plane, BedrockUsagePlane::Mantle);
        assert!(diagnostics.quota.accounting_separate_from_runtime);
        assert_eq!(
            diagnostics.cloudtrail.event_source,
            "bedrock-mantle.amazonaws.com"
        );
        assert!(diagnostics.cloudtrail.inference_is_data_event);
        assert!(
            diagnostics
                .cloudtrail
                .data_events
                .contains(&"CreateInference")
        );
        assert!(
            diagnostics
                .cloudtrail
                .resource_types
                .contains(&"AWS::BedrockMantle::Project")
        );
    }

    #[test]
    fn v6_mantle_quota_diagnostics_are_not_runtime_accounting() {
        let quota = MantleQuotaDiagnostics::default();

        assert_eq!(quota.usage_plane, BedrockUsagePlane::Mantle);
        assert_eq!(quota.endpoint_family, "bedrock-mantle");
        assert!(quota.accounting_separate_from_runtime);
        assert!(quota.dimensions.contains(&"api_shape"));
    }

    #[test]
    fn t25_mantle_status_diagnostic_maps_guidance_and_redacts_body() {
        let auth = mantle_status_diagnostic(
            403,
            r#"{"message":"denied","authorization":"Bearer secret","nested":{"api_key":"abc"}}"#,
        );

        assert_eq!(auth.plane, BedrockUsagePlane::Mantle);
        assert_eq!(auth.status, 403);
        assert_eq!(auth.kind, BedrockProviderErrorKind::AuthOrPermission);
        assert!(!auth.retryable);
        assert!(auth.suggested_action.contains("OpenAI-Project"));
        assert!(auth.suggested_action.contains("IAM"));
        assert!(auth.body_excerpt.contains("<redacted>"));
        assert!(!auth.body_excerpt.contains("Bearer secret"));
        assert!(!auth.body_excerpt.contains("abc"));

        let quota = mantle_status_diagnostic(429, r#"{"message":"quota"}"#);
        assert_eq!(quota.kind, BedrockProviderErrorKind::QuotaOrThrottle);
        assert!(quota.retryable);
        assert!(quota.suggested_action.contains("Mantle token quota"));
        assert!(quota.suggested_action.contains("separate from Runtime"));

        let incompatible = mantle_status_diagnostic(422, "{}");
        assert_eq!(
            incompatible.kind,
            BedrockProviderErrorKind::IncompatibleRequest
        );
        assert!(incompatible.suggested_action.contains("Responses"));
    }

    #[test]
    fn t25_runtime_status_diagnostic_maps_guidance_and_redacts_body() {
        let missing = runtime_status_diagnostic(404, r#"{"message":"model missing"}"#);

        assert_eq!(missing.plane, BedrockUsagePlane::Runtime);
        assert_eq!(missing.status, 404);
        assert_eq!(
            missing.kind,
            BedrockProviderErrorKind::ModelOrRegionMismatch
        );
        assert!(!missing.retryable);
        assert!(missing.suggested_action.contains("Converse"));
        assert!(missing.suggested_action.contains("InvokeModel"));

        let quota = runtime_status_diagnostic(503, "{}");
        assert_eq!(quota.kind, BedrockProviderErrorKind::QuotaOrThrottle);
        assert!(quota.retryable);
        assert!(quota.suggested_action.contains("Runtime quota"));
        assert!(quota.suggested_action.contains("separate from Mantle"));

        let auth = runtime_status_diagnostic(401, "authorization: Bearer runtime-secret");
        assert_eq!(auth.kind, BedrockProviderErrorKind::AuthOrPermission);
        assert!(auth.body_excerpt.contains("<redacted>"));
        assert!(!auth.body_excerpt.contains("runtime-secret"));
    }

    #[test]
    fn t25_client_errors_expose_http_status_diagnostics_only() {
        let mantle = MantleClientError::HttpStatus {
            status: 429,
            body: r#"{"message":"quota"}"#.to_owned(),
        };
        let runtime = RuntimeClientError::HttpStatus {
            status: 401,
            body: r#"{"message":"auth"}"#.to_owned(),
        };

        assert_eq!(
            mantle.diagnostic().unwrap().kind,
            BedrockProviderErrorKind::QuotaOrThrottle
        );
        assert_eq!(
            runtime.diagnostic().unwrap().kind,
            BedrockProviderErrorKind::AuthOrPermission
        );
        assert!(
            MantleClientError::Transport("offline".to_owned())
                .diagnostic()
                .is_none()
        );
        assert!(
            RuntimeClientError::Transport("offline".to_owned())
                .diagnostic()
                .is_none()
        );
    }

    #[test]
    fn t9_runtime_converse_request_serializes_tools_images_and_cache_points() {
        let request = RuntimeConverseRequest {
            model_id: "anthropic.test".to_owned(),
            additional_model_request_fields: Some(serde_json::json!({
                "thinking": {
                    "type": "enabled",
                    "budget_tokens": 2000
                }
            })),
            inference_config: Some(RuntimeInferenceConfiguration {
                max_tokens: Some(4096),
                temperature: Some(BedrockFloatMilli::new(200)),
                top_p: Some(BedrockFloatMilli::new(950)),
            }),
            system: vec![
                RuntimeSystemContentBlock::Text {
                    text: "system".to_owned(),
                },
                RuntimeSystemContentBlock::CachePoint {
                    cache_point: RuntimeCachePointBlock {
                        cache_point_type: "default".to_owned(),
                        ttl: Some("1h".to_owned()),
                    },
                },
            ],
            messages: vec![
                RuntimeMessage {
                    role: "user".to_owned(),
                    content: vec![
                        RuntimeContentBlock::Text {
                            text: "hello".to_owned(),
                        },
                        RuntimeContentBlock::Image {
                            image: RuntimeImageBlock {
                                format: "png".to_owned(),
                                source: RuntimeImageSource {
                                    bytes: "base64-image".to_owned(),
                                },
                            },
                        },
                        RuntimeContentBlock::CachePoint {
                            cache_point: RuntimeCachePointBlock {
                                cache_point_type: "default".to_owned(),
                                ttl: None,
                            },
                        },
                    ],
                },
                RuntimeMessage {
                    role: "assistant".to_owned(),
                    content: vec![RuntimeContentBlock::ToolUse {
                        tool_use: RuntimeToolUseBlock {
                            tool_use_id: "toolu_1".to_owned(),
                            name: "read_file".to_owned(),
                            input: serde_json::json!({"path": "src/lib.rs"}),
                        },
                    }],
                },
                RuntimeMessage {
                    role: "user".to_owned(),
                    content: vec![RuntimeContentBlock::ToolResult {
                        tool_result: RuntimeToolResultBlock {
                            tool_use_id: "toolu_1".to_owned(),
                            content: vec![
                                RuntimeToolResultContentBlock::Text {
                                    text: "contents".to_owned(),
                                },
                                RuntimeToolResultContentBlock::Json {
                                    json: serde_json::json!({"ok": true}),
                                },
                            ],
                            status: Some("success".to_owned()),
                        },
                    }],
                },
            ],
            tool_config: Some(RuntimeToolConfig {
                tools: vec![RuntimeTool {
                    tool_spec: RuntimeToolSpec {
                        name: "read_file".to_owned(),
                        description: "Read a file".to_owned(),
                        input_schema: RuntimeToolInputSchema {
                            json: serde_json::json!({
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" }
                                },
                                "required": ["path"]
                            }),
                        },
                    },
                }],
            }),
            service_tier: Some(RuntimeServiceTier {
                tier_type: "priority".to_owned(),
            }),
        };

        let json = serde_json::to_value(&request).unwrap();

        assert_eq!(json["system"][1]["cachePoint"]["type"], "default");
        assert_eq!(json["system"][1]["cachePoint"]["ttl"], "1h");
        assert_eq!(json["messages"][0]["content"][1]["image"]["format"], "png");
        assert_eq!(json["serviceTier"]["type"], "priority");
        assert_eq!(json["inferenceConfig"]["maxTokens"], 4096);
        assert_eq!(json["inferenceConfig"]["temperature"], 0.2);
        assert_eq!(json["inferenceConfig"]["topP"], 0.95);
        assert_eq!(
            json["additionalModelRequestFields"]["thinking"]["budget_tokens"],
            2000
        );
        assert_eq!(
            json["messages"][1]["content"][0]["toolUse"]["toolUseId"],
            "toolu_1"
        );
        assert_eq!(
            json["messages"][2]["content"][0]["toolResult"]["content"][1]["json"]["ok"],
            true
        );
        assert_eq!(
            json["toolConfig"]["tools"][0]["toolSpec"]["inputSchema"]["json"]["required"][0],
            "path"
        );
    }

    #[tokio::test]
    async fn t9_runtime_converse_uses_cli_profile_sigv4_auth() {
        let shared = AwsSharedConfig::from_ini_documents(
            "[profile runtime]\nregion = us-west-2\n",
            "[runtime]\naws_access_key_id = runtime-key\naws_secret_access_key = runtime-secret\n",
        );
        let resolved = shared
            .resolve_bedrock_auth(
                &AwsCredentialSource::CliProfile {
                    profile_name: "runtime".to_owned(),
                },
                None,
            )
            .unwrap();
        let credentials = shared.static_credentials_for_profile("runtime").unwrap();
        let transport = FakeRuntimeTransport::new(vec![RuntimeHttpResponse {
            status: 200,
            headers: BTreeMap::from([(
                "x-amzn-bedrock-invoked-model-id".to_owned(),
                "anthropic.test:1:resolved".to_owned(),
            )]),
            body: br#"{"output":{"message":{"content":[{"text":"hello"}]}},"stopReason":"end_turn","usage":{"inputTokens":12,"outputTokens":3,"totalTokens":15,"cacheReadInputTokens":8,"cacheWriteInputTokens":4},"metrics":{"latencyMs":42},"trace":{"guardrail":"none"}}"#.to_vec(),
        }]);
        let client = BedrockRuntimeClient::new(
            resolved.runtime_endpoint.clone(),
            resolved,
            transport.clone(),
            StaticCredentialsRuntimeSigV4Signer::new(credentials)
                .with_time(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
        );

        let response = client
            .converse(runtime_request("anthropic.test:1"))
            .await
            .unwrap();

        assert_eq!(response.output_text, "hello");
        assert_eq!(
            response.usage,
            Some(RuntimeTokenUsage {
                input_tokens: Some(12),
                output_tokens: Some(3),
                total_tokens: Some(15),
                cache_read_tokens: Some(8),
                cache_write_tokens: Some(4),
            })
        );
        assert_eq!(response.latency_ms, Some(42));
        assert_eq!(response.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(
            response.invoked_model_id.as_deref(),
            Some("anthropic.test:1:resolved")
        );
        assert_eq!(response.trace.as_ref().unwrap()["guardrail"], "none");
        let requests = transport.requests.lock().unwrap();
        assert_eq!(
            requests[0].url,
            "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.test%3A1/converse"
        );
        assert!(
            requests[0]
                .headers
                .get("authorization")
                .unwrap()
                .contains("Credential=runtime-key/20231114/us-west-2/bedrock/aws4_request")
        );
    }

    #[tokio::test]
    async fn t9_reqwest_transport_calls_runtime_converse_with_sigv4() {
        let server = OneShotHttpServer::start(
            200,
            "application/json",
            r#"{"output":{"message":{"content":[{"text":"from runtime"}]}}}"#,
        )
        .await;
        let resolved = ResolvedBedrockAuth {
            credential_source: AwsCredentialSource::CliProfile {
                profile_name: "runtime".to_owned(),
            },
            credential_profile: None,
            region: "us-east-1".to_owned(),
            mantle_sigv4_base_url: mantle_base_url("us-east-1"),
            runtime_endpoint: runtime_endpoint("us-east-1"),
        };
        let credentials = AwsStaticCredentials::new("runtime-key", "runtime-secret", None);
        let client = BedrockRuntimeClient::new(
            server.base_url.clone(),
            resolved,
            ReqwestBedrockHttpTransport::default(),
            StaticCredentialsRuntimeSigV4Signer::new(credentials)
                .with_time(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
        );

        let response = client
            .converse(runtime_request("anthropic.live"))
            .await
            .unwrap();
        let request = server.request().await;
        let request_lower = request.to_ascii_lowercase();

        assert_eq!(response.output_text, "from runtime");
        assert!(request.starts_with("POST /model/anthropic.live/converse HTTP/1.1"));
        assert!(request_lower.contains(
            "authorization: aws4-hmac-sha256 credential=runtime-key/20231114/us-east-1/bedrock/aws4_request"
        ));
        assert!(request_lower.contains("x-amz-date: 20231114t221320z"));
    }

    #[tokio::test]
    async fn t9_runtime_converse_stream_decodes_aws_eventstream_frames() {
        let resolved = ResolvedBedrockAuth {
            credential_source: AwsCredentialSource::DefaultChain,
            credential_profile: None,
            region: "us-east-1".to_owned(),
            mantle_sigv4_base_url: mantle_base_url("us-east-1"),
            runtime_endpoint: runtime_endpoint("us-east-1"),
        };
        let credentials = AwsStaticCredentials::new("key", "secret", None);
        let stream_body = encode_event_stream_messages(&[
            br#"{"messageStart":{"role":"assistant"}}"#,
            br#"{"contentBlockDelta":{"contentBlockIndex":0,"delta":{"text":"hi"}}}"#,
            br#"{"contentBlockStop":{"contentBlockIndex":0}}"#,
            br#"{"messageStop":{"stopReason":"end_turn"}}"#,
            br#"{"metadata":{"usage":{"inputTokens":3,"outputTokens":2,"totalTokens":5}}}"#,
        ]);
        let transport = FakeRuntimeTransport::new(vec![RuntimeHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: stream_body,
        }]);
        let client = BedrockRuntimeClient::new(
            resolved.runtime_endpoint.clone(),
            resolved,
            transport,
            StaticCredentialsRuntimeSigV4Signer::new(credentials),
        );

        let events = client
            .converse_stream(runtime_request("anthropic.test"))
            .await
            .unwrap();

        assert_eq!(
            events,
            vec![
                RuntimeStreamEvent::MessageStart {
                    role: Some("assistant".to_owned())
                },
                RuntimeStreamEvent::ContentBlockDelta {
                    text: "hi".to_owned()
                },
                RuntimeStreamEvent::ContentBlockStop {
                    content_block_index: Some(0)
                },
                RuntimeStreamEvent::MessageStop {
                    stop_reason: Some("end_turn".to_owned())
                },
                RuntimeStreamEvent::Metadata {
                    usage: Some(RuntimeTokenUsage {
                        input_tokens: Some(3),
                        output_tokens: Some(2),
                        total_tokens: Some(5),
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                    }),
                    latency_ms: None,
                    invoked_model_id: None,
                    trace: None,
                },
            ]
        );
    }

    #[test]
    fn t9_runtime_eventstream_parser_rejects_bad_crc() {
        let mut body = encode_event_stream_messages(&[br#"{"messageStop":{}}"#]);
        let last = body.len() - 1;
        body[last] ^= 0x01;

        let error = parse_runtime_event_stream(&body).unwrap_err();

        assert!(error.to_string().contains("CRC mismatch"));
    }

    #[tokio::test]
    async fn t9_runtime_invoke_model_stream_decodes_bedrock_chunk_bytes() {
        let resolved = ResolvedBedrockAuth {
            credential_source: AwsCredentialSource::DefaultChain,
            credential_profile: None,
            region: "us-east-1".to_owned(),
            mantle_sigv4_base_url: mantle_base_url("us-east-1"),
            runtime_endpoint: runtime_endpoint("us-east-1"),
        };
        let credentials = AwsStaticCredentials::new("key", "secret", None);
        let chunk = serde_json::json!({"delta":{"text":"stream chunk"}}).to_string();
        let encoded = BASE64_STANDARD.encode(chunk);
        let payload = format!(r#"{{"chunk":{{"bytes":"{encoded}"}}}}"#);
        let stream_body = encode_event_stream_messages(&[payload.as_bytes()]);
        let transport = FakeRuntimeTransport::new(vec![RuntimeHttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: stream_body,
        }]);
        let client = BedrockRuntimeClient::new(
            resolved.runtime_endpoint.clone(),
            resolved,
            transport.clone(),
            StaticCredentialsRuntimeSigV4Signer::new(credentials),
        );

        let events = client
            .invoke_model_stream("anthropic.test", serde_json::json!({"prompt":"hello"}))
            .await
            .unwrap();

        assert_eq!(
            events,
            vec![RuntimeStreamEvent::ContentBlockDelta {
                text: "stream chunk".to_owned()
            }]
        );
        let requests = transport.requests.lock().unwrap();
        assert!(
            requests[0]
                .url
                .ends_with("/model/anthropic.test/invoke-with-response-stream")
        );
    }

    #[tokio::test]
    async fn t9_runtime_converse_stream_falls_back_to_invoke_model_stream() {
        let resolved = ResolvedBedrockAuth {
            credential_source: AwsCredentialSource::DefaultChain,
            credential_profile: None,
            region: "us-east-1".to_owned(),
            mantle_sigv4_base_url: mantle_base_url("us-east-1"),
            runtime_endpoint: runtime_endpoint("us-east-1"),
        };
        let credentials = AwsStaticCredentials::new("key", "secret", None);
        let chunk = serde_json::json!({"outputText":"stream fallback"}).to_string();
        let encoded = BASE64_STANDARD.encode(chunk);
        let payload = format!(r#"{{"bytes":"{encoded}"}}"#);
        let stream_body = encode_event_stream_messages(&[payload.as_bytes()]);
        let transport = FakeRuntimeTransport::new(vec![
            RuntimeHttpResponse {
                status: 404,
                headers: BTreeMap::new(),
                body: br#"{"message":"stream unsupported"}"#.to_vec(),
            },
            RuntimeHttpResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: stream_body,
            },
        ]);
        let client = BedrockRuntimeClient::new(
            resolved.runtime_endpoint.clone(),
            resolved,
            transport.clone(),
            StaticCredentialsRuntimeSigV4Signer::new(credentials),
        );

        let response = client
            .converse_stream_with_invoke_fallback(
                runtime_request("anthropic.test"),
                serde_json::json!({"prompt":"hello"}),
            )
            .await
            .unwrap();

        assert_eq!(response.output_text, "stream fallback");
        let requests = transport.requests.lock().unwrap();
        assert!(requests[0].url.ends_with("/converse-stream"));
        assert!(requests[1].url.ends_with("/invoke-with-response-stream"));
    }

    #[tokio::test]
    async fn t9_runtime_converse_stream_falls_back_to_plain_invoke_model() {
        let resolved = ResolvedBedrockAuth {
            credential_source: AwsCredentialSource::DefaultChain,
            credential_profile: None,
            region: "us-east-1".to_owned(),
            mantle_sigv4_base_url: mantle_base_url("us-east-1"),
            runtime_endpoint: runtime_endpoint("us-east-1"),
        };
        let credentials = AwsStaticCredentials::new("key", "secret", None);
        let transport = FakeRuntimeTransport::new(vec![
            RuntimeHttpResponse {
                status: 404,
                headers: BTreeMap::new(),
                body: br#"{"message":"stream unsupported"}"#.to_vec(),
            },
            RuntimeHttpResponse {
                status: 404,
                headers: BTreeMap::new(),
                body: br#"{"message":"invoke stream unsupported"}"#.to_vec(),
            },
            RuntimeHttpResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: br#"{"body":"invoke fallback"}"#.to_vec(),
            },
        ]);
        let client = BedrockRuntimeClient::new(
            resolved.runtime_endpoint.clone(),
            resolved,
            transport.clone(),
            StaticCredentialsRuntimeSigV4Signer::new(credentials),
        );

        let response = client
            .converse_stream_with_invoke_fallback(
                runtime_request("anthropic.test"),
                serde_json::json!({"prompt":"hello"}),
            )
            .await
            .unwrap();

        assert_eq!(response.output_text, "invoke fallback");
        let requests = transport.requests.lock().unwrap();
        assert!(requests[0].url.ends_with("/converse-stream"));
        assert!(requests[1].url.ends_with("/invoke-with-response-stream"));
        assert!(requests[2].url.ends_with("/invoke"));
    }

    #[test]
    fn v6_runtime_quota_diagnostics_are_not_mantle_accounting() {
        let quota = RuntimeQuotaDiagnostics::default();

        assert_eq!(quota.usage_plane, BedrockUsagePlane::Runtime);
        assert_eq!(quota.endpoint_family, "bedrock-runtime");
        assert!(quota.accounting_separate_from_mantle);
        assert!(quota.operations.contains(&"ConverseStream"));
    }

    #[test]
    fn t10_mantle_stream_events_normalize_to_provider_neutral_shape() {
        let normalized = normalize_mantle_stream_events(vec![
            MantleStreamEvent::ResponseCreated {
                id: "resp-1".to_owned(),
            },
            MantleStreamEvent::OutputTextDelta {
                delta: "hello".to_owned(),
            },
            MantleStreamEvent::Error {
                message: "bad request".to_owned(),
            },
            MantleStreamEvent::Completed,
        ]);

        assert_eq!(
            normalized,
            vec![
                NormalizedModelStreamEvent {
                    provider: ModelProviderKind::BedrockMantle,
                    kind: NormalizedModelStreamEventKind::Started {
                        response_id: Some("resp-1".to_owned()),
                        role: None,
                    },
                },
                NormalizedModelStreamEvent::text_delta(ModelProviderKind::BedrockMantle, "hello"),
                NormalizedModelStreamEvent {
                    provider: ModelProviderKind::BedrockMantle,
                    kind: NormalizedModelStreamEventKind::Error {
                        message: "bad request".to_owned()
                    },
                },
                NormalizedModelStreamEvent::completed(ModelProviderKind::BedrockMantle),
            ]
        );
    }

    #[test]
    fn t10_runtime_stream_events_normalize_to_provider_neutral_shape() {
        let normalized = normalize_runtime_stream_events(vec![
            RuntimeStreamEvent::MessageStart {
                role: Some("assistant".to_owned()),
            },
            RuntimeStreamEvent::ContentBlockDelta {
                text: "hello".to_owned(),
            },
            RuntimeStreamEvent::ContentBlockStop {
                content_block_index: Some(0),
            },
            RuntimeStreamEvent::Metadata {
                usage: Some(RuntimeTokenUsage {
                    input_tokens: Some(1),
                    output_tokens: Some(2),
                    total_tokens: Some(3),
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                }),
                latency_ms: None,
                invoked_model_id: None,
                trace: None,
            },
            RuntimeStreamEvent::MessageStop { stop_reason: None },
        ]);

        assert_eq!(
            normalized,
            vec![
                NormalizedModelStreamEvent {
                    provider: ModelProviderKind::BedrockRuntime,
                    kind: NormalizedModelStreamEventKind::Started {
                        response_id: None,
                        role: Some("assistant".to_owned()),
                    },
                },
                NormalizedModelStreamEvent::text_delta(ModelProviderKind::BedrockRuntime, "hello"),
                NormalizedModelStreamEvent {
                    provider: ModelProviderKind::BedrockRuntime,
                    kind: NormalizedModelStreamEventKind::Usage {
                        input_tokens: Some(1),
                        output_tokens: Some(2),
                        total_tokens: Some(3),
                    },
                },
                NormalizedModelStreamEvent::completed(ModelProviderKind::BedrockRuntime),
            ]
        );
    }

    struct OneShotHttpServer {
        base_url: String,
        request_rx: oneshot::Receiver<String>,
    }

    impl OneShotHttpServer {
        async fn start(status: u16, content_type: &'static str, body: &'static str) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (request_tx, request_rx) = oneshot::channel();
            tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_http_request(&mut stream).await;
                let _ = request_tx.send(request);
                let reason = match status {
                    200 => "OK",
                    400 => "Bad Request",
                    404 => "Not Found",
                    _ => "Status",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            });
            Self {
                base_url: format!("http://{address}"),
                request_rx,
            }
        }

        async fn request(self) -> String {
            self.request_rx.await.unwrap()
        }
    }

    async fn read_http_request(stream: &mut TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0; 1024];
        loop {
            let bytes_read = stream.read(&mut chunk).await.unwrap();
            if bytes_read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..bytes_read]);
            let Some(headers_end) = http_headers_end(&buffer) else {
                continue;
            };
            let headers = String::from_utf8_lossy(&buffer[..headers_end]);
            let content_length = http_content_length(&headers);
            if buffer.len() >= headers_end + 4 + content_length {
                break;
            }
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }

    fn http_headers_end(buffer: &[u8]) -> Option<usize> {
        buffer.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn http_content_length(headers: &str) -> usize {
        headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())
                    .flatten()
            })
            .unwrap_or(0)
    }

    #[derive(Debug, Clone)]
    struct FakeTransport {
        responses: Arc<Mutex<Vec<MantleHttpResponse>>>,
        requests: Arc<Mutex<Vec<MantleHttpRequest>>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<MantleHttpResponse>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses.into_iter().rev().collect())),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl MantleHttpTransport for FakeTransport {
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
    struct FakeRuntimeTransport {
        responses: Arc<Mutex<Vec<RuntimeHttpResponse>>>,
        requests: Arc<Mutex<Vec<RuntimeHttpRequest>>>,
    }

    impl FakeRuntimeTransport {
        fn new(responses: Vec<RuntimeHttpResponse>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses.into_iter().rev().collect())),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl RuntimeHttpTransport for FakeRuntimeTransport {
        fn send(
            &self,
            request: RuntimeHttpRequest,
        ) -> Pin<
            Box<dyn Future<Output = Result<RuntimeHttpResponse, RuntimeClientError>> + Send + '_>,
        > {
            self.requests.lock().unwrap().push(request);
            let response = self.responses.lock().unwrap().pop().unwrap();
            Box::pin(async move { Ok(response) })
        }
    }

    fn runtime_request(model_id: &str) -> RuntimeConverseRequest {
        RuntimeConverseRequest {
            model_id: model_id.to_owned(),
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
        }
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
}
