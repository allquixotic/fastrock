#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_APPROVAL_POLICY_LABEL: &str = "Never";
pub const DEFAULT_COMMAND_CHANNEL_CAPACITY: usize = 256;
pub const DEFAULT_EVENT_CHANNEL_CAPACITY: usize = 1024;
pub const DEFAULT_CONVERSATION_CHANNEL_CAPACITY: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConversationId(pub String);

impl ConversationId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ConversationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProjectFolderId(pub String);

impl ProjectFolderId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConversationStatus {
    Idle,
    Running,
    WaitingForModel,
    WaitingForTool,
    Cancelling,
    Paused,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentMode {
    Code,
    Architect,
    Ask,
    Debug,
    Plan,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomModeDefinition {
    pub slug: String,
    pub display_name: String,
    pub role_definition: String,
    pub instructions: String,
    pub tool_groups: BTreeSet<String>,
    pub file_restrictions: Vec<ModeFileRestriction>,
    pub default_profile_id: Option<String>,
    pub scope: CustomModeScope,
}

impl CustomModeDefinition {
    pub fn validate(&self) -> Result<(), CustomModeError> {
        validate_custom_mode_slug(&self.slug)?;
        if self.display_name.trim().is_empty() {
            return Err(CustomModeError::MissingDisplayName {
                slug: self.slug.clone(),
            });
        }
        if self.role_definition.trim().is_empty() {
            return Err(CustomModeError::MissingRoleDefinition {
                slug: self.slug.clone(),
            });
        }
        if self.instructions.trim().is_empty() {
            return Err(CustomModeError::MissingInstructions {
                slug: self.slug.clone(),
            });
        }
        for group in &self.tool_groups {
            if group.trim().is_empty() {
                return Err(CustomModeError::InvalidToolGroup {
                    slug: self.slug.clone(),
                });
            }
        }
        for restriction in &self.file_restrictions {
            restriction.validate(&self.slug)?;
        }
        Ok(())
    }

    pub fn system_instruction(&self, project_folder: Option<&str>) -> String {
        let folder = project_folder.unwrap_or("the selected project folder");
        format!(
            "You are Fastrock in {} mode. Project folder: {folder}.\nRole: {}\nInstructions: {}",
            self.display_name, self.role_definition, self.instructions
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CustomModeScope {
    Global,
    Project(ProjectFolderId),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModeFileRestriction {
    pub pattern: String,
    pub access: ModeFileAccess,
}

impl ModeFileRestriction {
    fn validate(&self, slug: &str) -> Result<(), CustomModeError> {
        if self.pattern.trim().is_empty() {
            return Err(CustomModeError::InvalidFileRestriction {
                slug: slug.to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ModeFileAccess {
    ReadOnly,
    ReadWrite,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CustomModeRegistry {
    modes: BTreeMap<String, CustomModeDefinition>,
}

impl CustomModeRegistry {
    pub fn new(modes: Vec<CustomModeDefinition>) -> Result<Self, CustomModeError> {
        let mut registry = Self::default();
        for mode in modes {
            registry.add(mode)?;
        }
        Ok(registry)
    }

    pub fn import_json(contents: &str) -> Result<Self, CustomModeError> {
        let modes: Vec<CustomModeDefinition> =
            serde_json::from_str(contents).map_err(|error| CustomModeError::Json {
                message: error.to_string(),
            })?;
        Self::new(modes)
    }

    pub fn export_json_pretty(&self) -> Result<String, CustomModeError> {
        serde_json::to_string_pretty(&self.list()).map_err(|error| CustomModeError::Json {
            message: error.to_string(),
        })
    }

    pub fn add(&mut self, mode: CustomModeDefinition) -> Result<(), CustomModeError> {
        mode.validate()?;
        if self.modes.contains_key(&mode.slug) {
            return Err(CustomModeError::DuplicateSlug(mode.slug));
        }
        self.modes.insert(mode.slug.clone(), mode);
        Ok(())
    }

    pub fn edit(&mut self, mode: CustomModeDefinition) -> Result<(), CustomModeError> {
        mode.validate()?;
        if !self.modes.contains_key(&mode.slug) {
            return Err(CustomModeError::NotFound(mode.slug));
        }
        self.modes.insert(mode.slug.clone(), mode);
        Ok(())
    }

    pub fn delete(&mut self, slug: &str) -> Result<CustomModeDefinition, CustomModeError> {
        self.modes
            .remove(slug)
            .ok_or_else(|| CustomModeError::NotFound(slug.to_owned()))
    }

    pub fn get(&self, slug: &str) -> Option<&CustomModeDefinition> {
        self.modes.get(slug)
    }

    pub fn list(&self) -> Vec<CustomModeDefinition> {
        self.modes.values().cloned().collect()
    }

    pub fn instruction_for_mode(
        &self,
        mode: &AgentMode,
        project_folder: Option<&str>,
    ) -> Result<String, CustomModeError> {
        match mode {
            AgentMode::Custom(slug) => self
                .get(slug)
                .map(|definition| definition.system_instruction(project_folder))
                .ok_or_else(|| CustomModeError::NotFound(slug.clone())),
            _ => Ok(builtin_mode_instruction(mode, project_folder)),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CustomModeError {
    #[error("custom mode slug is invalid: {0}")]
    InvalidSlug(String),
    #[error("custom mode slug is duplicated: {0}")]
    DuplicateSlug(String),
    #[error("custom mode is not found: {0}")]
    NotFound(String),
    #[error("custom mode display name is required: {slug}")]
    MissingDisplayName { slug: String },
    #[error("custom mode role definition is required: {slug}")]
    MissingRoleDefinition { slug: String },
    #[error("custom mode instructions are required: {slug}")]
    MissingInstructions { slug: String },
    #[error("custom mode tool group is invalid: {slug}")]
    InvalidToolGroup { slug: String },
    #[error("custom mode file restriction is invalid: {slug}")]
    InvalidFileRestriction { slug: String },
    #[error("custom mode JSON is invalid: {message}")]
    Json { message: String },
}

fn validate_custom_mode_slug(slug: &str) -> Result<(), CustomModeError> {
    let valid = !slug.is_empty()
        && slug.len() <= 64
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(CustomModeError::InvalidSlug(slug.to_owned()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ApprovalPolicy {
    #[default]
    Never,
    OnFailure,
    OnRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandPolicy {
    pub commands_enabled: bool,
    pub deny_exact: Vec<Vec<String>>,
    pub deny_prefixes: Vec<Vec<String>>,
    pub allow_prefixes: Vec<Vec<String>>,
    pub max_runtime_ms: Option<u64>,
    pub max_output_bytes: Option<usize>,
    pub max_concurrent_commands: Option<usize>,
    pub label_outside_project_mutations: bool,
}

impl Default for CommandPolicy {
    fn default() -> Self {
        Self {
            commands_enabled: true,
            deny_exact: Vec::new(),
            deny_prefixes: Vec::new(),
            allow_prefixes: Vec::new(),
            max_runtime_ms: None,
            max_output_bytes: None,
            max_concurrent_commands: None,
            label_outside_project_mutations: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandPolicyRequest {
    pub command: Vec<String>,
    pub cwd: String,
    pub project_root: Option<String>,
    pub active_commands: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandPolicyDecision {
    Allow { labels: Vec<CommandPolicyLabel> },
    Deny { reason: CommandPolicyDenialReason },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandPolicyLabel {
    OutsideProjectMutation { argument: String },
}

impl fmt::Display for CommandPolicyLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutsideProjectMutation { argument } => {
                write!(formatter, "modifies outside project: {argument}")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandPolicyDenialReason {
    CommandsDisabled,
    DenyExact { command: Vec<String> },
    DenyPrefix { prefix: Vec<String> },
    NotAllowed { command: Vec<String> },
    MaxConcurrentCommands { limit: usize, active: usize },
}

impl fmt::Display for CommandPolicyDenialReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandsDisabled => formatter.write_str("command execution is disabled"),
            Self::DenyExact { command } => {
                write!(
                    formatter,
                    "command denied by exact policy: {}",
                    command.join(" ")
                )
            }
            Self::DenyPrefix { prefix } => {
                write!(
                    formatter,
                    "command denied by prefix policy: {}",
                    prefix.join(" ")
                )
            }
            Self::NotAllowed { command } => {
                write!(
                    formatter,
                    "command not allowed by allowlist: {}",
                    command.join(" ")
                )
            }
            Self::MaxConcurrentCommands { limit, active } => write!(
                formatter,
                "max concurrent commands reached: active {active}, limit {limit}"
            ),
        }
    }
}

pub fn evaluate_command_policy(
    policy: &CommandPolicy,
    request: &CommandPolicyRequest,
) -> CommandPolicyDecision {
    if !policy.commands_enabled {
        return CommandPolicyDecision::Deny {
            reason: CommandPolicyDenialReason::CommandsDisabled,
        };
    }
    if let Some(limit) = policy.max_concurrent_commands
        && request.active_commands >= limit
    {
        return CommandPolicyDecision::Deny {
            reason: CommandPolicyDenialReason::MaxConcurrentCommands {
                limit,
                active: request.active_commands,
            },
        };
    }
    if policy
        .deny_exact
        .iter()
        .any(|denied| denied == &request.command)
    {
        return CommandPolicyDecision::Deny {
            reason: CommandPolicyDenialReason::DenyExact {
                command: request.command.clone(),
            },
        };
    }
    if let Some(prefix) = policy
        .deny_prefixes
        .iter()
        .find(|prefix| command_starts_with(&request.command, prefix))
    {
        return CommandPolicyDecision::Deny {
            reason: CommandPolicyDenialReason::DenyPrefix {
                prefix: prefix.clone(),
            },
        };
    }
    if !policy.allow_prefixes.is_empty()
        && !policy
            .allow_prefixes
            .iter()
            .any(|prefix| command_starts_with(&request.command, prefix))
    {
        return CommandPolicyDecision::Deny {
            reason: CommandPolicyDenialReason::NotAllowed {
                command: request.command.clone(),
            },
        };
    }

    CommandPolicyDecision::Allow {
        labels: if policy.label_outside_project_mutations {
            outside_project_mutation_labels(request)
        } else {
            Vec::new()
        },
    }
}

fn command_starts_with(command: &[String], prefix: &[String]) -> bool {
    !prefix.is_empty()
        && command.len() >= prefix.len()
        && command
            .iter()
            .zip(prefix)
            .all(|(command, prefix)| command == prefix)
}

fn outside_project_mutation_labels(request: &CommandPolicyRequest) -> Vec<CommandPolicyLabel> {
    let Some(program) = request.command.first().map(String::as_str) else {
        return Vec::new();
    };
    if !matches!(
        program,
        "rm" | "mv" | "cp" | "touch" | "mkdir" | "rmdir" | "tee" | "chmod" | "chown" | "ln"
    ) {
        return Vec::new();
    }
    let Some(project_root) = request.project_root.as_deref() else {
        return Vec::new();
    };
    request
        .command
        .iter()
        .skip(1)
        .filter(|argument| path_like_argument_outside_project(argument, project_root))
        .map(|argument| CommandPolicyLabel::OutsideProjectMutation {
            argument: argument.clone(),
        })
        .collect()
}

fn path_like_argument_outside_project(argument: &str, project_root: &str) -> bool {
    if argument.starts_with('-') {
        return false;
    }
    let project_root = project_root.trim_end_matches('/');
    let inside_project = argument == project_root
        || argument
            .strip_prefix(project_root)
            .is_some_and(|suffix| suffix.starts_with('/'));
    (argument.starts_with('/') && !inside_project)
        || argument == ".."
        || argument.starts_with("../")
        || argument.contains("/../")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThreadGoalStatus {
    Active,
    Paused,
    Blocked,
    UsageLimited,
    BudgetLimited,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadGoal {
    pub objective: String,
    pub status: ThreadGoalStatus,
    pub token_budget: Option<u64>,
    pub tokens_used: u64,
    pub elapsed_ms: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadGoalSnapshot {
    pub objective: String,
    pub status: ThreadGoalStatus,
    pub token_budget: Option<u64>,
    pub tokens_used: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalLoopState {
    goal: Option<ThreadGoal>,
    charged_turn_ids: BTreeSet<String>,
    blocking_error_counts: BTreeMap<String, u32>,
}

impl GoalLoopState {
    pub fn new_goal(objective: impl Into<String>, token_budget: Option<u64>, now_ms: i64) -> Self {
        Self {
            goal: Some(ThreadGoal {
                objective: objective.into(),
                status: ThreadGoalStatus::Active,
                token_budget,
                tokens_used: 0,
                elapsed_ms: 0,
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            }),
            charged_turn_ids: BTreeSet::new(),
            blocking_error_counts: BTreeMap::new(),
        }
    }

    pub fn from_persisted_goal(goal: ThreadGoal) -> Self {
        Self {
            goal: Some(goal),
            charged_turn_ids: BTreeSet::new(),
            blocking_error_counts: BTreeMap::new(),
        }
    }

    pub fn from_persisted_goal_with_charged_turn_ids<I, S>(
        goal: ThreadGoal,
        charged_turn_ids: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            goal: Some(goal),
            charged_turn_ids: charged_turn_ids
                .into_iter()
                .map(Into::into)
                .collect::<BTreeSet<_>>(),
            blocking_error_counts: BTreeMap::new(),
        }
    }

    pub fn goal(&self) -> Option<&ThreadGoal> {
        self.goal.as_ref()
    }

    pub fn snapshot(&self) -> Option<ThreadGoalSnapshot> {
        self.goal.as_ref().map(|goal| ThreadGoalSnapshot {
            objective: goal.objective.clone(),
            status: goal.status.clone(),
            token_budget: goal.token_budget,
            tokens_used: goal.tokens_used,
            elapsed_ms: goal.elapsed_ms,
        })
    }

    pub fn charged_turn_ids(&self) -> impl Iterator<Item = &str> {
        self.charged_turn_ids.iter().map(String::as_str)
    }

    pub fn update_status(&mut self, status: ThreadGoalStatus, now_ms: i64) {
        if let Some(goal) = &mut self.goal {
            goal.status = status;
            goal.updated_at_ms = now_ms;
        }
    }

    pub fn clear(&mut self) -> Option<ThreadGoal> {
        self.charged_turn_ids.clear();
        self.blocking_error_counts.clear();
        self.goal.take()
    }

    pub fn resume(&mut self, now_ms: i64) -> bool {
        let Some(goal) = &mut self.goal else {
            return false;
        };
        if !matches!(
            goal.status,
            ThreadGoalStatus::Paused
                | ThreadGoalStatus::Blocked
                | ThreadGoalStatus::UsageLimited
                | ThreadGoalStatus::BudgetLimited
        ) {
            return false;
        }
        goal.status = ThreadGoalStatus::Active;
        goal.updated_at_ms = now_ms;
        self.blocking_error_counts.clear();
        true
    }

    pub fn mark_usage_limited(&mut self, now_ms: i64) -> bool {
        self.update_non_terminal_status(ThreadGoalStatus::UsageLimited, now_ms)
    }

    pub fn record_blocking_error(
        &mut self,
        error_key: impl Into<String>,
        now_ms: i64,
        threshold: u32,
    ) -> bool {
        let Some(goal) = &mut self.goal else {
            return false;
        };
        if matches!(goal.status, ThreadGoalStatus::Complete) {
            return false;
        }
        let threshold = threshold.max(1);
        let count = self
            .blocking_error_counts
            .entry(error_key.into())
            .and_modify(|count| *count += 1)
            .or_insert(1);
        goal.updated_at_ms = now_ms;
        if *count >= threshold {
            goal.status = ThreadGoalStatus::Blocked;
            return true;
        }
        false
    }

    pub fn charge_turn_usage(
        &mut self,
        turn_id: impl Into<String>,
        tokens: u64,
        elapsed_ms: u64,
        now_ms: i64,
    ) -> bool {
        let Some(goal) = &mut self.goal else {
            return false;
        };
        let turn_id = turn_id.into();
        if !self.charged_turn_ids.insert(turn_id) {
            return false;
        }
        goal.tokens_used += tokens;
        goal.elapsed_ms += elapsed_ms;
        goal.updated_at_ms = now_ms;
        if goal
            .token_budget
            .is_some_and(|budget| goal.tokens_used >= budget)
        {
            goal.status = ThreadGoalStatus::BudgetLimited;
        }
        true
    }

    fn update_non_terminal_status(&mut self, status: ThreadGoalStatus, now_ms: i64) -> bool {
        let Some(goal) = &mut self.goal else {
            return false;
        };
        if matches!(goal.status, ThreadGoalStatus::Complete) {
            return false;
        }
        goal.status = status;
        goal.updated_at_ms = now_ms;
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ModelProviderKind {
    BedrockMantle,
    BedrockRuntime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedModelStreamEvent {
    pub provider: ModelProviderKind,
    pub kind: NormalizedModelStreamEventKind,
}

impl NormalizedModelStreamEvent {
    pub fn text_delta(provider: ModelProviderKind, delta: impl Into<String>) -> Self {
        Self {
            provider,
            kind: NormalizedModelStreamEventKind::TextDelta {
                delta: delta.into(),
            },
        }
    }

    pub fn completed(provider: ModelProviderKind) -> Self {
        Self {
            provider,
            kind: NormalizedModelStreamEventKind::Completed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NormalizedModelStreamEventKind {
    Started {
        response_id: Option<String>,
        role: Option<String>,
    },
    TextDelta {
        delta: String,
    },
    Completed,
    Usage {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        total_tokens: Option<u64>,
    },
    Error {
        message: String,
    },
    Raw {
        event: String,
        data: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRequestMetadata {
    pub provider: ModelProviderKind,
    pub profile_id: Option<String>,
    pub model_id: String,
    pub endpoint: String,
    pub region: Option<String>,
    pub project_id: Option<String>,
    pub api_shape: Option<String>,
    pub store: Option<bool>,
    pub streaming: bool,
    pub request_kind: String,
}

impl ModelRequestMetadata {
    pub fn render_details(&self) -> BTreeMap<String, String> {
        let mut details = BTreeMap::from([
            ("endpoint".to_owned(), self.endpoint.clone()),
            ("model".to_owned(), self.model_id.clone()),
            ("request_kind".to_owned(), self.request_kind.clone()),
            ("streaming".to_owned(), self.streaming.to_string()),
        ]);
        if let Some(profile_id) = &self.profile_id {
            details.insert("profile_id".to_owned(), profile_id.clone());
        }
        if let Some(region) = &self.region {
            details.insert("region".to_owned(), region.clone());
        }
        if let Some(project_id) = &self.project_id {
            details.insert("project_id".to_owned(), project_id.clone());
        }
        if let Some(api_shape) = &self.api_shape {
            details.insert("api_shape".to_owned(), api_shape.clone());
        }
        if let Some(store) = self.store {
            details.insert("store".to_owned(), store.to_string());
        }
        details
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FastrockError {
    pub kind: FastrockErrorKind,
    pub code: String,
    pub summary: String,
    pub detail: String,
    pub retryable: bool,
    pub suggested_action: String,
    pub source: FastrockSubsystem,
    pub context: FastrockErrorContext,
}

impl FastrockError {
    pub fn new(kind: FastrockErrorKind, detail: impl Into<String>) -> Self {
        Self {
            code: kind.code().to_owned(),
            summary: kind.summary().to_owned(),
            detail: redact_secret_material(&detail.into()),
            retryable: kind.retryable(),
            suggested_action: kind.suggested_action().to_owned(),
            source: kind.default_source(),
            context: FastrockErrorContext::default(),
            kind,
        }
    }

    pub fn with_source(mut self, source: FastrockSubsystem) -> Self {
        self.source = source;
        self
    }

    pub fn with_conversation_id(mut self, conversation_id: impl Into<String>) -> Self {
        self.context.conversation_id = Some(conversation_id.into());
        self
    }

    pub fn with_project_folder_id(mut self, project_folder_id: impl Into<String>) -> Self {
        self.context.project_folder_id = Some(project_folder_id.into());
        self
    }

    pub fn with_profile_id(mut self, profile_id: impl Into<String>) -> Self {
        self.context.profile_id = Some(profile_id.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FastrockErrorKind {
    ProviderAuth,
    ProviderQuotaThrottle,
    ProviderRequestValidation,
    ProviderStreamDisconnect,
    RtkMissing,
    CommandFailed,
    CommandTimedOut,
    RemoteUnavailable,
    RemoteAuth,
    McpConfigInvalid,
    McpServerFailed,
    SkillInvalid,
    FileConflict,
    DatabaseMigration,
    KeyringUnavailable,
    PlanNotFound,
}

impl FastrockErrorKind {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ProviderAuth => "FR_PROVIDER_AUTH",
            Self::ProviderQuotaThrottle => "FR_PROVIDER_QUOTA_THROTTLE",
            Self::ProviderRequestValidation => "FR_PROVIDER_REQUEST_VALIDATION",
            Self::ProviderStreamDisconnect => "FR_PROVIDER_STREAM_DISCONNECT",
            Self::RtkMissing => "FR_RTK_MISSING",
            Self::CommandFailed => "FR_COMMAND_FAILED",
            Self::CommandTimedOut => "FR_COMMAND_TIMED_OUT",
            Self::RemoteUnavailable => "FR_REMOTE_UNAVAILABLE",
            Self::RemoteAuth => "FR_REMOTE_AUTH",
            Self::McpConfigInvalid => "FR_MCP_CONFIG_INVALID",
            Self::McpServerFailed => "FR_MCP_SERVER_FAILED",
            Self::SkillInvalid => "FR_SKILL_INVALID",
            Self::FileConflict => "FR_FILE_CONFLICT",
            Self::DatabaseMigration => "FR_DATABASE_MIGRATION",
            Self::KeyringUnavailable => "FR_KEYRING_UNAVAILABLE",
            Self::PlanNotFound => "FR_PLAN_NOT_FOUND",
        }
    }

    pub fn summary(&self) -> &'static str {
        match self {
            Self::ProviderAuth => "Provider authentication failed",
            Self::ProviderQuotaThrottle => "Provider quota or throttle limit reached",
            Self::ProviderRequestValidation => "Provider rejected the request",
            Self::ProviderStreamDisconnect => "Provider stream disconnected",
            Self::RtkMissing => "RTK is missing",
            Self::CommandFailed => "Command failed",
            Self::CommandTimedOut => "Command timed out",
            Self::RemoteUnavailable => "Remote target is unavailable",
            Self::RemoteAuth => "Remote authentication failed",
            Self::McpConfigInvalid => "MCP configuration is invalid",
            Self::McpServerFailed => "MCP server failed",
            Self::SkillInvalid => "Skill is invalid",
            Self::FileConflict => "File conflict detected",
            Self::DatabaseMigration => "Database migration failed",
            Self::KeyringUnavailable => "Keyring is unavailable",
            Self::PlanNotFound => "Plan artifact was not found",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::ProviderQuotaThrottle
                | Self::ProviderStreamDisconnect
                | Self::CommandTimedOut
                | Self::RemoteUnavailable
                | Self::McpServerFailed
                | Self::KeyringUnavailable
        )
    }

    pub fn suggested_action(&self) -> &'static str {
        match self {
            Self::ProviderAuth => {
                "Check credentials, selected AWS profile, region, project, and IAM permissions."
            }
            Self::ProviderQuotaThrottle => {
                "Back off, reduce concurrency or token volume, and retry when quota is available."
            }
            Self::ProviderRequestValidation => {
                "Inspect the model capability, request shape, tools, and cache settings."
            }
            Self::ProviderStreamDisconnect => {
                "Preserve the partial transcript and retry only when the request is safe to resume."
            }
            Self::RtkMissing => {
                "Install rtk and ensure the configured binary is available on PATH."
            }
            Self::CommandFailed => "Inspect stdout, stderr, cwd, environment, and exit status.",
            Self::CommandTimedOut => "Increase timeout or cancel dependent work before retrying.",
            Self::RemoteUnavailable => {
                "Check host, region, instance state, network, and remote agent/session availability."
            }
            Self::RemoteAuth => {
                "Check SSH keys, agent/keychain state, SSM IAM permissions, and target access."
            }
            Self::McpConfigInvalid => {
                "Fix the MCP server transport, URL, command, OAuth, timeout, or tool policy settings."
            }
            Self::McpServerFailed => {
                "Inspect server logs, restart the server, and retry the tool call."
            }
            Self::SkillInvalid => {
                "Fix SKILL.md frontmatter, name, description, path, or mode restrictions."
            }
            Self::FileConflict => {
                "Reload, inspect the diff, and choose whether to overwrite or preserve the external change."
            }
            Self::DatabaseMigration => {
                "Back up the database, inspect migration logs, and retry after repair."
            }
            Self::KeyringUnavailable => "Unlock or repair the OS keyring and retry secret access.",
            Self::PlanNotFound => "Refresh the conversation plan list and choose an existing plan.",
        }
    }

    pub fn default_source(&self) -> FastrockSubsystem {
        match self {
            Self::ProviderAuth
            | Self::ProviderQuotaThrottle
            | Self::ProviderRequestValidation
            | Self::ProviderStreamDisconnect => FastrockSubsystem::Bedrock,
            Self::RtkMissing | Self::CommandFailed | Self::CommandTimedOut => {
                FastrockSubsystem::Rtk
            }
            Self::RemoteUnavailable | Self::RemoteAuth => FastrockSubsystem::Remote,
            Self::McpConfigInvalid | Self::McpServerFailed => FastrockSubsystem::Mcp,
            Self::SkillInvalid => FastrockSubsystem::Skills,
            Self::FileConflict => FastrockSubsystem::Editor,
            Self::DatabaseMigration => FastrockSubsystem::Persistence,
            Self::KeyringUnavailable => FastrockSubsystem::Keyring,
            Self::PlanNotFound => FastrockSubsystem::Core,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FastrockSubsystem {
    App,
    Core,
    Bedrock,
    Rtk,
    Projects,
    Remote,
    Persistence,
    Mcp,
    Skills,
    Cache,
    Editor,
    Keyring,
    Ui,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FastrockErrorContext {
    pub conversation_id: Option<String>,
    pub project_folder_id: Option<String>,
    pub profile_id: Option<String>,
}

pub fn redact_secret_material(detail: &str) -> String {
    [
        "aws_secret_access_key=",
        "secret_access_key=",
        "api_key=",
        "token=",
        "Authorization: Bearer ",
        "authorization: bearer ",
    ]
    .into_iter()
    .fold(detail.to_owned(), redact_marker_value)
}

fn redact_marker_value(mut detail: String, marker: &str) -> String {
    let mut search_start = 0;
    while let Some(relative_start) = detail[search_start..].find(marker) {
        let value_start = search_start + relative_start + marker.len();
        let value_end = detail[value_start..]
            .char_indices()
            .find_map(|(offset, character)| {
                (character.is_whitespace() || [',', ';', '&', '"', '\''].contains(&character))
                    .then_some(value_start + offset)
            })
            .unwrap_or(detail.len());
        detail.replace_range(value_start..value_end, "[REDACTED]");
        search_start = value_start + "[REDACTED]".len();
    }
    detail
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanArtifact {
    pub id: String,
    pub prompt: String,
    pub status: PlanArtifactStatus,
    pub created_sequence: u64,
    pub executed_sequence: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanArtifactStatus {
    Draft,
    Executed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationCommand {
    AppendUserMessage { text: String },
    AppendAssistantTextDelta { delta: String },
    AppendModelRequest { metadata: ModelRequestMetadata },
    AppendModelEvent { event: NormalizedModelStreamEvent },
    StartPlan { prompt: String },
    ExecutePlan { plan_id: String },
    SetStatus { status: ConversationStatus },
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationActorEvent {
    Started {
        conversation_id: ConversationId,
    },
    StatusChanged {
        conversation_id: ConversationId,
        status: ConversationStatus,
    },
    TranscriptAppended {
        conversation_id: ConversationId,
        event: TranscriptEvent,
    },
    Stopped {
        conversation_id: ConversationId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptEvent {
    pub conversation_id: ConversationId,
    pub sequence: u64,
    pub kind: TranscriptEventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TranscriptEventKind {
    UserMessage { text: String },
    AssistantTextDelta { delta: String },
    ModelRequest { metadata: Box<ModelRequestMetadata> },
    ModelStreamEvent { event: NormalizedModelStreamEvent },
    PlanCreated { artifact: Box<PlanArtifact> },
    PlanExecuted { plan_id: String },
    ConversationSettingChanged { key: String, value: String },
    Error { error: Box<FastrockError> },
    StatusChanged { status: ConversationStatus },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAssemblyInput {
    pub mode: AgentMode,
    pub project_folder: Option<String>,
    pub transcript_events: Vec<TranscriptEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptContext {
    pub mode: AgentMode,
    pub system_instruction: String,
    pub messages: Vec<PromptMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptMessage {
    pub role: PromptRole,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptRole {
    System,
    User,
    Assistant,
}

pub fn assemble_prompt_context(input: PromptAssemblyInput) -> PromptContext {
    let mut messages = Vec::new();
    let mut pending_assistant = String::new();

    for event in input.transcript_events {
        match event.kind {
            TranscriptEventKind::UserMessage { text } => {
                flush_pending_assistant(&mut pending_assistant, &mut messages);
                messages.push(PromptMessage {
                    role: PromptRole::User,
                    content: text,
                });
            }
            TranscriptEventKind::AssistantTextDelta { delta } => {
                pending_assistant.push_str(&delta);
            }
            TranscriptEventKind::ModelStreamEvent { event } => match event.kind {
                NormalizedModelStreamEventKind::TextDelta { delta } => {
                    pending_assistant.push_str(&delta);
                }
                NormalizedModelStreamEventKind::Completed
                | NormalizedModelStreamEventKind::Started { .. }
                | NormalizedModelStreamEventKind::Usage { .. } => {}
                NormalizedModelStreamEventKind::Error { message } => {
                    flush_pending_assistant(&mut pending_assistant, &mut messages);
                    messages.push(PromptMessage {
                        role: PromptRole::Assistant,
                        content: format!("[model error] {message}"),
                    });
                }
                NormalizedModelStreamEventKind::Raw { event, data } => {
                    flush_pending_assistant(&mut pending_assistant, &mut messages);
                    messages.push(PromptMessage {
                        role: PromptRole::Assistant,
                        content: format!("[raw model event: {event}] {data}"),
                    });
                }
            },
            TranscriptEventKind::Error { error } => {
                flush_pending_assistant(&mut pending_assistant, &mut messages);
                messages.push(PromptMessage {
                    role: PromptRole::Assistant,
                    content: format!("[error {}] {}", error.code, error.summary),
                });
            }
            TranscriptEventKind::PlanCreated { artifact } => {
                flush_pending_assistant(&mut pending_assistant, &mut messages);
                messages.push(PromptMessage {
                    role: PromptRole::Assistant,
                    content: format!("[plan {}] {}", artifact.id, artifact.prompt),
                });
            }
            TranscriptEventKind::PlanExecuted { plan_id } => {
                flush_pending_assistant(&mut pending_assistant, &mut messages);
                messages.push(PromptMessage {
                    role: PromptRole::Assistant,
                    content: format!("[plan executed] {plan_id}"),
                });
            }
            TranscriptEventKind::ModelRequest { .. }
            | TranscriptEventKind::ConversationSettingChanged { .. }
            | TranscriptEventKind::StatusChanged { .. } => {}
        }
    }
    flush_pending_assistant(&mut pending_assistant, &mut messages);

    PromptContext {
        system_instruction: builtin_mode_instruction(&input.mode, input.project_folder.as_deref()),
        mode: input.mode,
        messages,
    }
}

pub fn builtin_mode_instruction(mode: &AgentMode, project_folder: Option<&str>) -> String {
    let folder = project_folder.unwrap_or("the selected project folder");
    match mode {
        AgentMode::Code => format!(
            "You are Fastrock in Code mode. Work directly in {folder}. Prefer concrete edits, tests, and concise status."
        ),
        AgentMode::Architect => format!(
            "You are Fastrock in Architect mode. Analyze {folder}, propose implementation structure, and avoid file mutations unless the user switches modes."
        ),
        AgentMode::Ask => format!(
            "You are Fastrock in Ask mode. Answer questions about {folder} with precise references and avoid changing files."
        ),
        AgentMode::Debug => format!(
            "You are Fastrock in Debug mode. Reproduce failures in {folder}, isolate causes, and propose or apply the smallest correct fix."
        ),
        AgentMode::Plan => format!(
            "You are Fastrock in Plan mode. Inspect {folder}, produce a concrete plan, and do not mutate project files."
        ),
        AgentMode::Custom(value) => value.clone(),
    }
}

fn flush_pending_assistant(pending_assistant: &mut String, messages: &mut Vec<PromptMessage>) {
    if pending_assistant.is_empty() {
        return;
    }
    messages.push(PromptMessage {
        role: PromptRole::Assistant,
        content: std::mem::take(pending_assistant),
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRouteRequest {
    pub conversation_id: ConversationId,
    pub mode: AgentMode,
    pub tool_name: String,
    pub mutation: ToolMutationClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolMutationClass {
    ReadOnly,
    WritesProjectFiles,
    RunsCommand,
    NetworkMutation,
    StartsRemoteSession,
    UpdatesSettings,
}

impl ToolMutationClass {
    pub fn is_mutating(&self) -> bool {
        !matches!(self, Self::ReadOnly)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanModeCommandClassification {
    ReadOnly,
    Mutation { reason: PlanModeCommandBlockReason },
}

impl PlanModeCommandClassification {
    pub fn mutation_class(&self) -> ToolMutationClass {
        match self {
            Self::ReadOnly => ToolMutationClass::ReadOnly,
            Self::Mutation { .. } => ToolMutationClass::RunsCommand,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanModeCommandBlockReason {
    EmptyCommand,
    ShellCommand,
    Formatter,
    Codegen,
    Migration,
    PackageInstall,
    GitMutation,
    FileMutation,
    UnknownCommand,
}

impl PlanModeCommandBlockReason {
    pub fn label(&self) -> &'static str {
        match self {
            Self::EmptyCommand => "empty command",
            Self::ShellCommand => "shell command",
            Self::Formatter => "formatter",
            Self::Codegen => "code generation",
            Self::Migration => "migration",
            Self::PackageInstall => "package install",
            Self::GitMutation => "git mutation",
            Self::FileMutation => "file mutation",
            Self::UnknownCommand => "unknown command intent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRouteDecision {
    Allow,
    Reject {
        reason: ToolRouteRejectionReason,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRouteRejectionReason {
    PlanModeMutation,
}

pub fn route_tool_request(request: &ToolRouteRequest) -> ToolRouteDecision {
    if matches!(request.mode, AgentMode::Plan) && request.mutation.is_mutating() {
        return ToolRouteDecision::Reject {
            reason: ToolRouteRejectionReason::PlanModeMutation,
            message: format!(
                "Plan mode blocks mutating tool `{}` for conversation {}",
                request.tool_name, request.conversation_id
            ),
        };
    }

    ToolRouteDecision::Allow
}

pub fn classify_plan_mode_rtk_command(command: &[String]) -> PlanModeCommandClassification {
    let command = strip_rtk_wrapper(command);
    if command.is_empty() {
        return block_plan_command(PlanModeCommandBlockReason::EmptyCommand);
    }

    let program = command_name(&command[0]);
    let args = &command[1..];

    if is_shell_command(program) {
        return block_plan_command(PlanModeCommandBlockReason::ShellCommand);
    }

    if is_file_mutation_command(program) {
        return block_plan_command(PlanModeCommandBlockReason::FileMutation);
    }

    match program {
        "git" => classify_git_plan_command(args),
        "cargo" => classify_cargo_plan_command(args),
        "rg" | "grep" | "ag" | "ripgrep" | "ls" | "pwd" | "cat" | "head" | "tail" | "wc" => {
            PlanModeCommandClassification::ReadOnly
        }
        "sed" => {
            if args.iter().any(|arg| arg == "-i" || arg.starts_with("-i")) {
                block_plan_command(PlanModeCommandBlockReason::FileMutation)
            } else {
                PlanModeCommandClassification::ReadOnly
            }
        }
        "find" => {
            if args
                .iter()
                .any(|arg| matches!(arg.as_str(), "-delete" | "-exec" | "-execdir"))
            {
                block_plan_command(PlanModeCommandBlockReason::FileMutation)
            } else {
                PlanModeCommandClassification::ReadOnly
            }
        }
        "rustfmt" | "prettier" | "black" | "gofmt" => {
            block_plan_command(PlanModeCommandBlockReason::Formatter)
        }
        "sqlx" | "diesel" | "prisma" => block_plan_command(PlanModeCommandBlockReason::Migration),
        "npm" | "pnpm" | "yarn" | "pip" | "pip3" | "uv" | "brew" | "apt" | "apt-get" => {
            classify_package_manager_plan_command(program, args)
        }
        "protoc" | "buf" | "openapi-generator" | "slint-compiler" => {
            block_plan_command(PlanModeCommandBlockReason::Codegen)
        }
        "rtk" => PlanModeCommandClassification::ReadOnly,
        "--version" | "gain" => PlanModeCommandClassification::ReadOnly,
        _ => block_plan_command(PlanModeCommandBlockReason::UnknownCommand),
    }
}

pub fn route_plan_mode_rtk_command(
    conversation_id: ConversationId,
    command: &[String],
) -> ToolRouteDecision {
    match classify_plan_mode_rtk_command(command) {
        PlanModeCommandClassification::ReadOnly => ToolRouteDecision::Allow,
        PlanModeCommandClassification::Mutation { reason } => ToolRouteDecision::Reject {
            reason: ToolRouteRejectionReason::PlanModeMutation,
            message: format!(
                "Plan mode blocks rtk command `{}` for conversation {}: {}",
                command.join(" "),
                conversation_id,
                reason.label()
            ),
        },
    }
}

fn block_plan_command(reason: PlanModeCommandBlockReason) -> PlanModeCommandClassification {
    PlanModeCommandClassification::Mutation { reason }
}

fn strip_rtk_wrapper(command: &[String]) -> &[String] {
    let Some((first, rest)) = command.split_first() else {
        return command;
    };
    if command_name(first) != "rtk" {
        return command;
    }
    if let Some((subcommand, proxied)) = rest.split_first()
        && subcommand == "proxy"
    {
        return proxied;
    }
    rest
}

fn command_name(program: &str) -> &str {
    program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .strip_suffix(".exe")
        .unwrap_or_else(|| program.rsplit(['/', '\\']).next().unwrap_or(program))
}

fn is_shell_command(program: &str) -> bool {
    matches!(
        program,
        "sh" | "bash" | "zsh" | "fish" | "cmd" | "powershell" | "pwsh"
    )
}

fn is_file_mutation_command(program: &str) -> bool {
    matches!(
        program,
        "rm" | "mv"
            | "cp"
            | "touch"
            | "mkdir"
            | "rmdir"
            | "tee"
            | "truncate"
            | "chmod"
            | "chown"
            | "ln"
    )
}

fn classify_git_plan_command(args: &[String]) -> PlanModeCommandClassification {
    let Some(subcommand) = first_non_option_arg(args) else {
        return block_plan_command(PlanModeCommandBlockReason::UnknownCommand);
    };
    match subcommand {
        "status" | "diff" | "show" | "log" | "grep" | "ls-files" | "rev-parse" | "cat-file"
        | "blame" => PlanModeCommandClassification::ReadOnly,
        "add" | "commit" | "push" | "reset" | "checkout" | "switch" | "merge" | "rebase"
        | "cherry-pick" | "stash" | "clean" | "restore" | "mv" | "rm" | "tag" => {
            block_plan_command(PlanModeCommandBlockReason::GitMutation)
        }
        _ => block_plan_command(PlanModeCommandBlockReason::UnknownCommand),
    }
}

fn classify_cargo_plan_command(args: &[String]) -> PlanModeCommandClassification {
    let Some(subcommand) = first_non_option_arg(args) else {
        return block_plan_command(PlanModeCommandBlockReason::UnknownCommand);
    };
    match subcommand {
        "check" | "test" | "build" | "clippy" | "metadata" | "tree" => {
            PlanModeCommandClassification::ReadOnly
        }
        "fmt" => block_plan_command(PlanModeCommandBlockReason::Formatter),
        "generate" => block_plan_command(PlanModeCommandBlockReason::Codegen),
        "add" | "install" | "remove" | "update" => {
            block_plan_command(PlanModeCommandBlockReason::PackageInstall)
        }
        _ => block_plan_command(PlanModeCommandBlockReason::UnknownCommand),
    }
}

fn classify_package_manager_plan_command(
    program: &str,
    args: &[String],
) -> PlanModeCommandClassification {
    let Some(subcommand) = first_non_option_arg(args) else {
        return block_plan_command(PlanModeCommandBlockReason::UnknownCommand);
    };
    let reason = match program {
        "npm" => matches!(subcommand, "install" | "i" | "add" | "update" | "uninstall"),
        "pnpm" | "yarn" => matches!(
            subcommand,
            "install" | "add" | "update" | "upgrade" | "remove"
        ),
        "pip" | "pip3" => matches!(subcommand, "install" | "uninstall"),
        "uv" => matches!(subcommand, "add" | "remove" | "pip"),
        "brew" => matches!(subcommand, "install" | "uninstall" | "upgrade"),
        "apt" | "apt-get" => matches!(
            subcommand,
            "install" | "remove" | "upgrade" | "dist-upgrade"
        ),
        _ => false,
    };
    if reason {
        block_plan_command(PlanModeCommandBlockReason::PackageInstall)
    } else {
        block_plan_command(PlanModeCommandBlockReason::UnknownCommand)
    }
}

fn first_non_option_arg(args: &[String]) -> Option<&str> {
    args.iter()
        .find(|arg| !arg.starts_with('-'))
        .map(String::as_str)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRenderConfig {
    pub max_command_capture_bytes: usize,
}

impl Default for TranscriptRenderConfig {
    fn default() -> Self {
        Self {
            max_command_capture_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationVisibility {
    Active,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptUiCoalescerConfig {
    pub active_max_updates_per_second: u32,
    pub hidden_max_updates_per_second: u32,
    pub render_config: TranscriptRenderConfig,
}

impl Default for TranscriptUiCoalescerConfig {
    fn default() -> Self {
        Self {
            active_max_updates_per_second: 30,
            hidden_max_updates_per_second: 4,
            render_config: TranscriptRenderConfig::default(),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TranscriptUiCoalescerConfigError {
    #[error("active transcript UI update rate must be greater than zero")]
    ZeroActiveRate,
    #[error("hidden transcript UI update rate must be greater than zero")]
    ZeroHiddenRate,
}

impl TranscriptUiCoalescerConfig {
    fn validate(&self) -> Result<(), TranscriptUiCoalescerConfigError> {
        if self.active_max_updates_per_second == 0 {
            return Err(TranscriptUiCoalescerConfigError::ZeroActiveRate);
        }
        if self.hidden_max_updates_per_second == 0 {
            return Err(TranscriptUiCoalescerConfigError::ZeroHiddenRate);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct TranscriptUiCoalescer {
    config: TranscriptUiCoalescerConfig,
    visibility: ConversationVisibility,
    pending: Vec<TranscriptRenderEvent>,
    last_flush_ms: u64,
}

impl TranscriptUiCoalescer {
    pub fn new(
        config: TranscriptUiCoalescerConfig,
        visibility: ConversationVisibility,
        now_ms: u64,
    ) -> Result<Self, TranscriptUiCoalescerConfigError> {
        config.validate()?;
        Ok(Self {
            config,
            visibility,
            pending: Vec::new(),
            last_flush_ms: now_ms,
        })
    }

    pub fn set_visibility(&mut self, visibility: ConversationVisibility) {
        self.visibility = visibility;
    }

    pub fn pending_event_count(&self) -> usize {
        self.pending.len()
    }

    pub fn push(
        &mut self,
        event: TranscriptRenderEvent,
        now_ms: u64,
    ) -> Option<Vec<RenderedTranscriptBlock>> {
        self.pending.push(event);
        self.should_flush(now_ms).then(|| self.flush(now_ms))
    }

    pub fn force_flush(&mut self, now_ms: u64) -> Option<Vec<RenderedTranscriptBlock>> {
        (!self.pending.is_empty()).then(|| self.flush(now_ms))
    }

    fn should_flush(&self, now_ms: u64) -> bool {
        !self.pending.is_empty()
            && now_ms.saturating_sub(self.last_flush_ms) >= self.flush_interval_ms()
    }

    fn flush(&mut self, now_ms: u64) -> Vec<RenderedTranscriptBlock> {
        self.last_flush_ms = now_ms;
        coalesce_transcript_events(
            std::mem::take(&mut self.pending),
            self.config.render_config.clone(),
        )
    }

    fn flush_interval_ms(&self) -> u64 {
        let rate = match self.visibility {
            ConversationVisibility::Active => self.config.active_max_updates_per_second,
            ConversationVisibility::Hidden => self.config.hidden_max_updates_per_second,
        };
        1_000_u64.div_ceil(u64::from(rate))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptRenderEvent {
    ModelRequest(ModelRequestMetadata),
    Model(NormalizedModelStreamEvent),
    CommandOutput {
        command_id: String,
        stream: CommandOutputStream,
        bytes: Vec<u8>,
    },
    CommandExited {
        command_id: String,
        exit_code: Option<i32>,
    },
    FileChanged {
        path: String,
        action: FileTranscriptAction,
        diff_summary: DiffSummary,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileTranscriptAction {
    Created,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DiffSummary {
    pub added_lines: usize,
    pub removed_lines: usize,
    pub modified_lines: usize,
}

impl DiffSummary {
    pub fn changed_lines(&self) -> usize {
        self.added_lines + self.removed_lines + self.modified_lines
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderedTranscriptBlock {
    AssistantText {
        text: String,
    },
    ModelRequest {
        provider: ModelProviderKind,
        details: BTreeMap<String, String>,
    },
    ModelStatus {
        provider: ModelProviderKind,
        status: String,
    },
    CommandOutput {
        command_id: String,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        truncated: bool,
    },
    CommandExit {
        command_id: String,
        exit_code: Option<i32>,
    },
    FileChange {
        path: String,
        action: FileTranscriptAction,
        diff_summary: DiffSummary,
    },
}

pub fn coalesce_transcript_events(
    events: impl IntoIterator<Item = TranscriptRenderEvent>,
    config: TranscriptRenderConfig,
) -> Vec<RenderedTranscriptBlock> {
    let mut blocks = Vec::new();
    let mut pending_model_text = String::new();
    let mut pending_command: Option<PendingCommandOutput> = None;

    for event in events {
        match event {
            TranscriptRenderEvent::ModelRequest(metadata) => {
                flush_pending_model(&mut pending_model_text, &mut blocks);
                flush_pending_command(&mut pending_command, &mut blocks);
                blocks.push(RenderedTranscriptBlock::ModelRequest {
                    provider: metadata.provider.clone(),
                    details: metadata.render_details(),
                });
            }
            TranscriptRenderEvent::Model(model_event) => {
                flush_pending_command(&mut pending_command, &mut blocks);
                match model_event.kind {
                    NormalizedModelStreamEventKind::TextDelta { delta } => {
                        pending_model_text.push_str(&delta);
                    }
                    NormalizedModelStreamEventKind::Started { .. } => {
                        flush_pending_model(&mut pending_model_text, &mut blocks);
                        blocks.push(RenderedTranscriptBlock::ModelStatus {
                            provider: model_event.provider,
                            status: "started".to_owned(),
                        });
                    }
                    NormalizedModelStreamEventKind::Completed => {
                        flush_pending_model(&mut pending_model_text, &mut blocks);
                        blocks.push(RenderedTranscriptBlock::ModelStatus {
                            provider: model_event.provider,
                            status: "completed".to_owned(),
                        });
                    }
                    NormalizedModelStreamEventKind::Usage { .. } => {}
                    NormalizedModelStreamEventKind::Error { message } => {
                        flush_pending_model(&mut pending_model_text, &mut blocks);
                        blocks.push(RenderedTranscriptBlock::ModelStatus {
                            provider: model_event.provider,
                            status: format!("error: {message}"),
                        });
                    }
                    NormalizedModelStreamEventKind::Raw { event, .. } => {
                        flush_pending_model(&mut pending_model_text, &mut blocks);
                        blocks.push(RenderedTranscriptBlock::ModelStatus {
                            provider: model_event.provider,
                            status: format!("raw: {event}"),
                        });
                    }
                }
            }
            TranscriptRenderEvent::CommandOutput {
                command_id,
                stream,
                bytes,
            } => {
                flush_pending_model(&mut pending_model_text, &mut blocks);
                if pending_command
                    .as_ref()
                    .is_some_and(|pending| pending.command_id != command_id)
                {
                    flush_pending_command(&mut pending_command, &mut blocks);
                }
                let pending = pending_command.get_or_insert_with(|| PendingCommandOutput {
                    command_id,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    truncated: false,
                });
                pending.append(stream, &bytes, config.max_command_capture_bytes);
            }
            TranscriptRenderEvent::CommandExited {
                command_id,
                exit_code,
            } => {
                flush_pending_model(&mut pending_model_text, &mut blocks);
                flush_pending_command(&mut pending_command, &mut blocks);
                blocks.push(RenderedTranscriptBlock::CommandExit {
                    command_id,
                    exit_code,
                });
            }
            TranscriptRenderEvent::FileChanged {
                path,
                action,
                diff_summary,
            } => {
                flush_pending_model(&mut pending_model_text, &mut blocks);
                flush_pending_command(&mut pending_command, &mut blocks);
                blocks.push(RenderedTranscriptBlock::FileChange {
                    path,
                    action,
                    diff_summary,
                });
            }
        }
    }

    flush_pending_model(&mut pending_model_text, &mut blocks);
    flush_pending_command(&mut pending_command, &mut blocks);
    blocks
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingCommandOutput {
    command_id: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
}

impl PendingCommandOutput {
    fn append(&mut self, stream: CommandOutputStream, bytes: &[u8], max_bytes: usize) {
        let destination = match stream {
            CommandOutputStream::Stdout => &mut self.stdout,
            CommandOutputStream::Stderr => &mut self.stderr,
        };
        if destination.len() >= max_bytes {
            self.truncated = true;
            return;
        }
        let available = max_bytes - destination.len();
        let to_take = available.min(bytes.len());
        destination.extend_from_slice(&bytes[..to_take]);
        if to_take < bytes.len() {
            self.truncated = true;
        }
    }
}

fn flush_pending_model(pending_model_text: &mut String, blocks: &mut Vec<RenderedTranscriptBlock>) {
    if !pending_model_text.is_empty() {
        blocks.push(RenderedTranscriptBlock::AssistantText {
            text: std::mem::take(pending_model_text),
        });
    }
}

fn flush_pending_command(
    pending_command: &mut Option<PendingCommandOutput>,
    blocks: &mut Vec<RenderedTranscriptBlock>,
) {
    if let Some(pending) = pending_command.take() {
        blocks.push(RenderedTranscriptBlock::CommandOutput {
            command_id: pending.command_id,
            stdout: pending.stdout,
            stderr: pending.stderr,
            truncated: pending.truncated,
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledConversation {
    pub conversation_id: ConversationId,
    pub title: String,
    pub status: ConversationStatus,
    pub hidden_progress: Option<HiddenConversationProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenConversationProgress {
    pub summary: String,
    pub unread_events: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConversationScheduler {
    conversations: BTreeMap<ConversationId, ScheduledConversation>,
    visible_conversation_id: Option<ConversationId>,
}

impl ConversationScheduler {
    pub fn add_conversation(&mut self, conversation_id: ConversationId, title: impl Into<String>) {
        let conversation = ScheduledConversation {
            conversation_id: conversation_id.clone(),
            title: title.into(),
            status: ConversationStatus::Idle,
            hidden_progress: None,
        };
        self.conversations
            .insert(conversation_id.clone(), conversation);
        self.visible_conversation_id.get_or_insert(conversation_id);
    }

    pub fn switch_visible(&mut self, conversation_id: &ConversationId) -> bool {
        if !self.conversations.contains_key(conversation_id) {
            return false;
        }
        self.visible_conversation_id = Some(conversation_id.clone());
        if let Some(conversation) = self.conversations.get_mut(conversation_id) {
            conversation.hidden_progress = None;
        }
        true
    }

    pub fn set_status(&mut self, conversation_id: &ConversationId, status: ConversationStatus) {
        if let Some(conversation) = self.conversations.get_mut(conversation_id) {
            conversation.status = status;
        }
    }

    pub fn record_progress(
        &mut self,
        conversation_id: &ConversationId,
        summary: impl Into<String>,
    ) {
        if self.visible_conversation_id.as_ref() == Some(conversation_id) {
            return;
        }
        if let Some(conversation) = self.conversations.get_mut(conversation_id) {
            let progress = conversation
                .hidden_progress
                .get_or_insert(HiddenConversationProgress {
                    summary: String::new(),
                    unread_events: 0,
                });
            progress.summary = summary.into();
            progress.unread_events += 1;
        }
    }

    pub fn visible_conversation_id(&self) -> Option<&ConversationId> {
        self.visible_conversation_id.as_ref()
    }

    pub fn conversation(&self, conversation_id: &ConversationId) -> Option<&ScheduledConversation> {
        self.conversations.get(conversation_id)
    }

    pub fn conversations(&self) -> Vec<&ScheduledConversation> {
        self.conversations.values().collect()
    }

    pub fn hidden_progress(&self) -> Vec<HiddenConversationProgress> {
        self.conversations
            .values()
            .filter(|conversation| {
                self.visible_conversation_id.as_ref() != Some(&conversation.conversation_id)
            })
            .filter_map(|conversation| conversation.hidden_progress.clone())
            .collect()
    }

    pub fn conversation_count(&self) -> usize {
        self.conversations.len()
    }
}

pub trait TranscriptEventStore: Send + Sync + 'static {
    fn append(
        &self,
        event: TranscriptEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), TranscriptStoreError>> + Send + '_>>;

    fn load(
        &self,
        conversation_id: ConversationId,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<TranscriptEvent>, TranscriptStoreError>> + Send + '_>>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TranscriptStoreError {
    #[error("transcript store unavailable: {0}")]
    Unavailable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationActorConfig {
    pub command_channel_capacity: usize,
    pub event_channel_capacity: usize,
}

impl Default for ConversationActorConfig {
    fn default() -> Self {
        Self {
            command_channel_capacity: DEFAULT_CONVERSATION_CHANNEL_CAPACITY,
            event_channel_capacity: DEFAULT_EVENT_CHANNEL_CAPACITY,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConversationActorConfigError {
    #[error("conversation command channel capacity must be greater than zero")]
    ZeroCommandCapacity,
    #[error("conversation event channel capacity must be greater than zero")]
    ZeroEventCapacity,
}

impl ConversationActorConfig {
    fn validate(self) -> Result<Self, ConversationActorConfigError> {
        if self.command_channel_capacity == 0 {
            return Err(ConversationActorConfigError::ZeroCommandCapacity);
        }
        if self.event_channel_capacity == 0 {
            return Err(ConversationActorConfigError::ZeroEventCapacity);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone)]
pub struct ConversationActorHandle {
    conversation_id: ConversationId,
    command_tx: mpsc::Sender<ConversationCommand>,
    shutdown: CancellationToken,
}

impl ConversationActorHandle {
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }

    pub async fn send(
        &self,
        command: ConversationCommand,
    ) -> Result<(), mpsc::error::SendError<ConversationCommand>> {
        self.command_tx.send(command).await
    }

    pub fn request_shutdown(&self) {
        self.shutdown.cancel();
    }
}

#[derive(Debug)]
pub struct ConversationActorEventReceiver {
    event_rx: mpsc::Receiver<ConversationActorEvent>,
}

impl ConversationActorEventReceiver {
    pub async fn recv(&mut self) -> Option<ConversationActorEvent> {
        self.event_rx.recv().await
    }
}

pub struct ConversationActor {
    conversation_id: ConversationId,
    command_rx: mpsc::Receiver<ConversationCommand>,
    event_tx: mpsc::Sender<ConversationActorEvent>,
    shutdown: CancellationToken,
    store: Arc<dyn TranscriptEventStore>,
    status: ConversationStatus,
    plans: BTreeMap<String, PlanArtifact>,
    next_sequence: u64,
}

impl ConversationActor {
    pub fn spawn_on(
        runtime: &tokio::runtime::Handle,
        conversation_id: ConversationId,
        store: Arc<dyn TranscriptEventStore>,
        config: ConversationActorConfig,
    ) -> Result<
        (ConversationActorHandle, ConversationActorEventReceiver),
        ConversationActorConfigError,
    > {
        let config = config.validate()?;
        let (command_tx, command_rx) = mpsc::channel(config.command_channel_capacity);
        let (event_tx, event_rx) = mpsc::channel(config.event_channel_capacity);
        let shutdown = CancellationToken::new();
        let actor = Self {
            conversation_id: conversation_id.clone(),
            command_rx,
            event_tx,
            shutdown: shutdown.clone(),
            store,
            status: ConversationStatus::Idle,
            plans: BTreeMap::new(),
            next_sequence: 1,
        };
        runtime.spawn(actor.run());
        Ok((
            ConversationActorHandle {
                conversation_id,
                command_tx,
                shutdown,
            },
            ConversationActorEventReceiver { event_rx },
        ))
    }

    async fn run(mut self) {
        self.publish(ConversationActorEvent::Started {
            conversation_id: self.conversation_id.clone(),
        })
        .await;

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                command = self.command_rx.recv() => {
                    match command {
                        Some(ConversationCommand::Stop) | None => break,
                        Some(command) => self.handle_command(command).await,
                    }
                }
            }
        }

        self.publish(ConversationActorEvent::Stopped {
            conversation_id: self.conversation_id.clone(),
        })
        .await;
    }

    async fn handle_command(&mut self, command: ConversationCommand) {
        match command {
            ConversationCommand::AppendUserMessage { text } => {
                self.append(TranscriptEventKind::UserMessage { text }).await;
            }
            ConversationCommand::AppendAssistantTextDelta { delta } => {
                self.append(TranscriptEventKind::AssistantTextDelta { delta })
                    .await;
            }
            ConversationCommand::AppendModelRequest { metadata } => {
                self.append(TranscriptEventKind::ModelRequest {
                    metadata: Box::new(metadata),
                })
                .await;
            }
            ConversationCommand::AppendModelEvent { event } => {
                self.append(TranscriptEventKind::ModelStreamEvent { event })
                    .await;
            }
            ConversationCommand::StartPlan { prompt } => {
                let artifact = PlanArtifact {
                    id: format!("plan-{}", self.next_sequence),
                    prompt,
                    status: PlanArtifactStatus::Draft,
                    created_sequence: self.next_sequence,
                    executed_sequence: None,
                };
                self.plans.insert(artifact.id.clone(), artifact.clone());
                self.append(TranscriptEventKind::PlanCreated {
                    artifact: Box::new(artifact),
                })
                .await;
            }
            ConversationCommand::ExecutePlan { plan_id } => {
                let Some(plan) = self.plans.get_mut(&plan_id) else {
                    self.append(TranscriptEventKind::Error {
                        error: Box::new(
                            FastrockError::new(
                                FastrockErrorKind::PlanNotFound,
                                format!("plan_id={plan_id}"),
                            )
                            .with_source(FastrockSubsystem::Core)
                            .with_conversation_id(self.conversation_id.0.clone()),
                        ),
                    })
                    .await;
                    return;
                };
                plan.status = PlanArtifactStatus::Executed;
                plan.executed_sequence = Some(self.next_sequence);
                self.status = ConversationStatus::Running;
                self.append(TranscriptEventKind::PlanExecuted { plan_id })
                    .await;
                self.publish(ConversationActorEvent::StatusChanged {
                    conversation_id: self.conversation_id.clone(),
                    status: ConversationStatus::Running,
                })
                .await;
            }
            ConversationCommand::SetStatus { status } => {
                self.status = status.clone();
                self.append(TranscriptEventKind::StatusChanged {
                    status: status.clone(),
                })
                .await;
                self.publish(ConversationActorEvent::StatusChanged {
                    conversation_id: self.conversation_id.clone(),
                    status,
                })
                .await;
            }
            ConversationCommand::Stop => unreachable!("stop is handled in actor loop"),
        }
    }

    async fn append(&mut self, kind: TranscriptEventKind) {
        let event = TranscriptEvent {
            conversation_id: self.conversation_id.clone(),
            sequence: self.next_sequence,
            kind,
        };
        self.next_sequence += 1;
        if self.store.append(event.clone()).await.is_ok() {
            self.publish(ConversationActorEvent::TranscriptAppended {
                conversation_id: self.conversation_id.clone(),
                event,
            })
            .await;
        }
    }

    async fn publish(&self, event: ConversationActorEvent) {
        let _ = self.event_tx.send(event).await;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    CreateConversation {
        conversation_id: ConversationId,
        title: String,
        project_folder_id: Option<ProjectFolderId>,
    },
    SelectConversation {
        conversation_id: ConversationId,
    },
    SetConversationStatus {
        conversation_id: ConversationId,
        status: ConversationStatus,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    SupervisorStarted,
    ConversationCreated {
        conversation_id: ConversationId,
        title: String,
        project_folder_id: Option<ProjectFolderId>,
        status: ConversationStatus,
    },
    ActiveConversationChanged {
        conversation_id: ConversationId,
    },
    ConversationStatusChanged {
        conversation_id: ConversationId,
        status: ConversationStatus,
    },
    CommandRejected {
        command: &'static str,
        reason: String,
    },
    SupervisorStopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppSupervisorConfig {
    pub command_channel_capacity: usize,
    pub event_channel_capacity: usize,
}

impl Default for AppSupervisorConfig {
    fn default() -> Self {
        Self {
            command_channel_capacity: DEFAULT_COMMAND_CHANNEL_CAPACITY,
            event_channel_capacity: DEFAULT_EVENT_CHANNEL_CAPACITY,
        }
    }
}

impl AppSupervisorConfig {
    pub fn validate(self) -> Result<Self, AppSupervisorConfigError> {
        if self.command_channel_capacity == 0 {
            return Err(AppSupervisorConfigError::ZeroCommandCapacity);
        }
        if self.event_channel_capacity == 0 {
            return Err(AppSupervisorConfigError::ZeroEventCapacity);
        }
        Ok(self)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AppSupervisorConfigError {
    #[error("command channel capacity must be greater than zero")]
    ZeroCommandCapacity,
    #[error("event channel capacity must be greater than zero")]
    ZeroEventCapacity,
}

#[derive(Debug, Clone)]
pub struct AppSupervisorHandle {
    command_tx: mpsc::Sender<AppCommand>,
    shutdown: CancellationToken,
}

impl AppSupervisorHandle {
    pub async fn send(
        &self,
        command: AppCommand,
    ) -> Result<(), mpsc::error::SendError<AppCommand>> {
        self.command_tx.send(command).await
    }

    pub fn try_send(
        &self,
        command: AppCommand,
    ) -> Result<(), mpsc::error::TrySendError<AppCommand>> {
        self.command_tx.try_send(command)
    }

    pub fn command_sender(&self) -> mpsc::Sender<AppCommand> {
        self.command_tx.clone()
    }

    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    pub fn request_shutdown(&self) {
        self.shutdown.cancel();
    }
}

#[derive(Debug)]
pub struct AppEventReceiver {
    event_rx: mpsc::Receiver<AppEvent>,
}

impl AppEventReceiver {
    pub async fn recv(&mut self) -> Option<AppEvent> {
        self.event_rx.recv().await
    }

    pub fn try_recv(&mut self) -> Result<AppEvent, mpsc::error::TryRecvError> {
        self.event_rx.try_recv()
    }
}

#[derive(Debug)]
pub struct AppSupervisor {
    command_rx: mpsc::Receiver<AppCommand>,
    event_tx: mpsc::Sender<AppEvent>,
    shutdown: CancellationToken,
    conversations: BTreeMap<ConversationId, ConversationRecord>,
    active_conversation_id: Option<ConversationId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConversationRecord {
    title: String,
    project_folder_id: Option<ProjectFolderId>,
    status: ConversationStatus,
}

impl AppSupervisor {
    pub fn spawn_on(
        runtime: &tokio::runtime::Handle,
        config: AppSupervisorConfig,
    ) -> Result<(AppSupervisorHandle, AppEventReceiver), AppSupervisorConfigError> {
        let config = config.validate()?;
        let (command_tx, command_rx) = mpsc::channel(config.command_channel_capacity);
        let (event_tx, event_rx) = mpsc::channel(config.event_channel_capacity);
        let shutdown = CancellationToken::new();

        let supervisor = Self {
            command_rx,
            event_tx,
            shutdown: shutdown.clone(),
            conversations: BTreeMap::new(),
            active_conversation_id: None,
        };

        runtime.spawn(supervisor.run());

        Ok((
            AppSupervisorHandle {
                command_tx,
                shutdown,
            },
            AppEventReceiver { event_rx },
        ))
    }

    async fn run(mut self) {
        self.publish(AppEvent::SupervisorStarted).await;

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => {
                    break;
                }
                command = self.command_rx.recv() => {
                    match command {
                        Some(AppCommand::Shutdown) | None => {
                            self.shutdown.cancel();
                            break;
                        }
                        Some(command) => self.handle_command(command).await,
                    }
                }
            }
        }

        self.publish(AppEvent::SupervisorStopped).await;
    }

    async fn handle_command(&mut self, command: AppCommand) {
        match command {
            AppCommand::CreateConversation {
                conversation_id,
                title,
                project_folder_id,
            } => {
                let record = ConversationRecord {
                    title: title.clone(),
                    project_folder_id: project_folder_id.clone(),
                    status: ConversationStatus::Idle,
                };
                self.conversations
                    .insert(conversation_id.clone(), record.clone());
                self.publish(AppEvent::ConversationCreated {
                    conversation_id,
                    title,
                    project_folder_id,
                    status: record.status,
                })
                .await;
            }
            AppCommand::SelectConversation { conversation_id } => {
                if self.conversations.contains_key(&conversation_id) {
                    self.active_conversation_id = Some(conversation_id.clone());
                    self.publish(AppEvent::ActiveConversationChanged { conversation_id })
                        .await;
                } else {
                    self.reject("SelectConversation", "conversation does not exist")
                        .await;
                }
            }
            AppCommand::SetConversationStatus {
                conversation_id,
                status,
            } => {
                if let Some(record) = self.conversations.get_mut(&conversation_id) {
                    record.status = status.clone();
                    self.publish(AppEvent::ConversationStatusChanged {
                        conversation_id,
                        status,
                    })
                    .await;
                } else {
                    self.reject("SetConversationStatus", "conversation does not exist")
                        .await;
                }
            }
            AppCommand::Shutdown => unreachable!("shutdown is handled in the supervisor loop"),
        }
    }

    async fn reject(&self, command: &'static str, reason: &'static str) {
        self.publish(AppEvent::CommandRejected {
            command,
            reason: reason.to_owned(),
        })
        .await;
    }

    async fn publish(&self, event: AppEvent) {
        let _ = self.event_tx.send(event).await;
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::error::TrySendError;

    use super::*;

    #[test]
    fn app_supervisor_config_rejects_unbounded_zero_capacity() {
        assert_eq!(
            AppSupervisorConfig {
                command_channel_capacity: 0,
                event_channel_capacity: 1,
            }
            .validate(),
            Err(AppSupervisorConfigError::ZeroCommandCapacity)
        );
        assert_eq!(
            AppSupervisorConfig {
                command_channel_capacity: 1,
                event_channel_capacity: 0,
            }
            .validate(),
            Err(AppSupervisorConfigError::ZeroEventCapacity)
        );
    }

    #[test]
    fn app_command_channel_is_bounded() {
        let (command_tx, mut command_rx) = mpsc::channel(1);
        command_tx.try_send(AppCommand::Shutdown).unwrap();

        let err = command_tx.try_send(AppCommand::Shutdown).unwrap_err();
        assert!(matches!(err, TrySendError::Full(AppCommand::Shutdown)));

        command_rx.try_recv().unwrap();
    }

    #[test]
    fn t14_command_policy_blocks_denied_commands_and_allowlist_misses() {
        let policy = CommandPolicy {
            deny_exact: vec![argv(vec!["rm", "-rf", "/"])],
            deny_prefixes: vec![argv(vec!["git", "push"])],
            allow_prefixes: vec![argv(vec!["cargo"]), argv(vec!["git", "status"])],
            ..CommandPolicy::default()
        };

        assert!(matches!(
            evaluate_command_policy(
                &policy,
                &command_policy_request(argv(vec!["rm", "-rf", "/"]))
            ),
            CommandPolicyDecision::Deny {
                reason: CommandPolicyDenialReason::DenyExact { .. }
            }
        ));
        assert!(matches!(
            evaluate_command_policy(
                &policy,
                &command_policy_request(argv(vec!["git", "push", "origin"]))
            ),
            CommandPolicyDecision::Deny {
                reason: CommandPolicyDenialReason::DenyPrefix { .. }
            }
        ));
        assert!(matches!(
            evaluate_command_policy(&policy, &command_policy_request(argv(vec!["npm", "test"]))),
            CommandPolicyDecision::Deny {
                reason: CommandPolicyDenialReason::NotAllowed { .. }
            }
        ));
        assert!(matches!(
            evaluate_command_policy(
                &policy,
                &command_policy_request(argv(vec!["cargo", "test"]))
            ),
            CommandPolicyDecision::Allow { .. }
        ));
    }

    #[test]
    fn t14_command_policy_enforces_concurrency_and_labels_outside_project_mutation() {
        let policy = CommandPolicy {
            max_concurrent_commands: Some(2),
            ..CommandPolicy::default()
        };

        assert!(matches!(
            evaluate_command_policy(
                &policy,
                &CommandPolicyRequest {
                    command: argv(vec!["cargo", "test"]),
                    cwd: "/repo".to_owned(),
                    project_root: Some("/repo".to_owned()),
                    active_commands: 2,
                },
            ),
            CommandPolicyDecision::Deny {
                reason: CommandPolicyDenialReason::MaxConcurrentCommands {
                    limit: 2,
                    active: 2
                }
            }
        ));

        assert_eq!(
            evaluate_command_policy(
                &CommandPolicy::default(),
                &CommandPolicyRequest {
                    command: argv(vec!["rm", "../outside.txt"]),
                    cwd: "/repo".to_owned(),
                    project_root: Some("/repo".to_owned()),
                    active_commands: 0,
                },
            ),
            CommandPolicyDecision::Allow {
                labels: vec![CommandPolicyLabel::OutsideProjectMutation {
                    argument: "../outside.txt".to_owned(),
                }],
            }
        );
    }

    #[test]
    fn t25_fastrock_error_carries_code_retryability_source_action_and_context() {
        let error = FastrockError::new(
            FastrockErrorKind::ProviderQuotaThrottle,
            "Mantle returned 429 for model anthropic.claude",
        )
        .with_conversation_id("conversation-1")
        .with_project_folder_id("project-1")
        .with_profile_id("profile-1");

        assert_eq!(error.code, "FR_PROVIDER_QUOTA_THROTTLE");
        assert_eq!(
            error.summary,
            "Provider quota or throttle limit reached".to_owned()
        );
        assert!(error.retryable);
        assert_eq!(error.source, FastrockSubsystem::Bedrock);
        assert!(error.suggested_action.contains("Back off"));
        assert_eq!(
            error.context,
            FastrockErrorContext {
                conversation_id: Some("conversation-1".to_owned()),
                project_folder_id: Some("project-1".to_owned()),
                profile_id: Some("profile-1".to_owned()),
            }
        );
    }

    #[test]
    fn v9_fastrock_error_redacts_secret_like_values_before_serialization() {
        let error = FastrockError::new(
            FastrockErrorKind::ProviderAuth,
            "api_key=sk-test token=abc123 aws_secret_access_key=wJalrXUtnFEMI/K7MDENG/bPxRfiCY",
        );
        let json = serde_json::to_string(&error).unwrap();

        assert_eq!(
            error.detail,
            "api_key=[REDACTED] token=[REDACTED] aws_secret_access_key=[REDACTED]"
        );
        assert!(!json.contains("sk-test"));
        assert!(!json.contains("abc123"));
        assert!(!json.contains("wJalr"));
    }

    #[test]
    fn t25_transcript_error_event_survives_prompt_and_json_roundtrip() {
        let error = FastrockError::new(FastrockErrorKind::RtkMissing, "rtk not found")
            .with_conversation_id("conversation-1");
        let event = TranscriptEvent {
            conversation_id: ConversationId::new("conversation-1"),
            sequence: 1,
            kind: TranscriptEventKind::Error {
                error: Box::new(error.clone()),
            },
        };
        let decoded: TranscriptEvent =
            serde_json::from_str(&serde_json::to_string(&event).unwrap())
                .expect("transcript event round-trips");

        assert_eq!(decoded, event);

        let context = assemble_prompt_context(PromptAssemblyInput {
            mode: AgentMode::Code,
            project_folder: Some("/repo".to_owned()),
            transcript_events: vec![event],
        });

        assert_eq!(
            context.messages,
            vec![PromptMessage {
                role: PromptRole::Assistant,
                content: format!("[error {}] {}", error.code, error.summary),
            }]
        );
    }

    #[tokio::test]
    async fn supervisor_emits_started_and_stopped_events() {
        let (handle, mut events) = AppSupervisor::spawn_on(
            &tokio::runtime::Handle::current(),
            AppSupervisorConfig::default(),
        )
        .unwrap();

        assert_eq!(events.recv().await, Some(AppEvent::SupervisorStarted));

        handle.send(AppCommand::Shutdown).await.unwrap();

        assert_eq!(events.recv().await, Some(AppEvent::SupervisorStopped));
    }

    #[tokio::test]
    async fn supervisor_processes_conversation_commands() {
        let (handle, mut events) = AppSupervisor::spawn_on(
            &tokio::runtime::Handle::current(),
            AppSupervisorConfig::default(),
        )
        .unwrap();
        assert_eq!(events.recv().await, Some(AppEvent::SupervisorStarted));

        let conversation_id = ConversationId::new("conversation-1");
        let project_folder_id = Some(ProjectFolderId::new("project-1"));
        handle
            .send(AppCommand::CreateConversation {
                conversation_id: conversation_id.clone(),
                title: "Implement runtime".to_owned(),
                project_folder_id: project_folder_id.clone(),
            })
            .await
            .unwrap();
        assert_eq!(
            events.recv().await,
            Some(AppEvent::ConversationCreated {
                conversation_id: conversation_id.clone(),
                title: "Implement runtime".to_owned(),
                project_folder_id,
                status: ConversationStatus::Idle,
            })
        );

        handle
            .send(AppCommand::SelectConversation {
                conversation_id: conversation_id.clone(),
            })
            .await
            .unwrap();
        assert_eq!(
            events.recv().await,
            Some(AppEvent::ActiveConversationChanged {
                conversation_id: conversation_id.clone()
            })
        );

        handle
            .send(AppCommand::SetConversationStatus {
                conversation_id: conversation_id.clone(),
                status: ConversationStatus::Running,
            })
            .await
            .unwrap();
        assert_eq!(
            events.recv().await,
            Some(AppEvent::ConversationStatusChanged {
                conversation_id,
                status: ConversationStatus::Running,
            })
        );
    }

    #[tokio::test]
    async fn supervisor_rejects_unknown_conversation_commands() {
        let (handle, mut events) = AppSupervisor::spawn_on(
            &tokio::runtime::Handle::current(),
            AppSupervisorConfig::default(),
        )
        .unwrap();
        assert_eq!(events.recv().await, Some(AppEvent::SupervisorStarted));

        handle
            .send(AppCommand::SelectConversation {
                conversation_id: ConversationId::new("missing"),
            })
            .await
            .unwrap();

        assert_eq!(
            events.recv().await,
            Some(AppEvent::CommandRejected {
                command: "SelectConversation",
                reason: "conversation does not exist".to_owned(),
            })
        );
    }

    #[tokio::test]
    async fn t11_conversation_actor_appends_transcript_events_to_store() {
        let store = Arc::new(MemoryTranscriptStore::default());
        let conversation_id = ConversationId::new("conversation-1");
        let (handle, mut events) = ConversationActor::spawn_on(
            &tokio::runtime::Handle::current(),
            conversation_id.clone(),
            store.clone(),
            ConversationActorConfig::default(),
        )
        .unwrap();
        assert_eq!(
            events.recv().await,
            Some(ConversationActorEvent::Started {
                conversation_id: conversation_id.clone()
            })
        );

        handle
            .send(ConversationCommand::AppendUserMessage {
                text: "build".to_owned(),
            })
            .await
            .unwrap();
        let appended = events.recv().await.unwrap();

        assert_eq!(
            appended,
            ConversationActorEvent::TranscriptAppended {
                conversation_id: conversation_id.clone(),
                event: TranscriptEvent {
                    conversation_id: conversation_id.clone(),
                    sequence: 1,
                    kind: TranscriptEventKind::UserMessage {
                        text: "build".to_owned()
                    },
                },
            }
        );
        assert_eq!(store.load(conversation_id).await.unwrap()[0].sequence, 1);
    }

    #[tokio::test]
    async fn t11_conversation_actor_emits_status_and_persists_it() {
        let store = Arc::new(MemoryTranscriptStore::default());
        let conversation_id = ConversationId::new("conversation-status");
        let (handle, mut events) = ConversationActor::spawn_on(
            &tokio::runtime::Handle::current(),
            conversation_id.clone(),
            store.clone(),
            ConversationActorConfig::default(),
        )
        .unwrap();
        assert!(matches!(
            events.recv().await,
            Some(ConversationActorEvent::Started { .. })
        ));

        handle
            .send(ConversationCommand::SetStatus {
                status: ConversationStatus::Running,
            })
            .await
            .unwrap();

        assert!(matches!(
            events.recv().await,
            Some(ConversationActorEvent::TranscriptAppended { .. })
        ));
        assert_eq!(
            events.recv().await,
            Some(ConversationActorEvent::StatusChanged {
                conversation_id: conversation_id.clone(),
                status: ConversationStatus::Running,
            })
        );
        assert_eq!(
            store.load(conversation_id).await.unwrap()[0].kind,
            TranscriptEventKind::StatusChanged {
                status: ConversationStatus::Running,
            }
        );
    }

    #[tokio::test]
    async fn v7_parallel_conversation_actors_do_not_cancel_each_other() {
        let store = Arc::new(MemoryTranscriptStore::default());
        let first_id = ConversationId::new("first");
        let second_id = ConversationId::new("second");
        let (first, mut first_events) = ConversationActor::spawn_on(
            &tokio::runtime::Handle::current(),
            first_id.clone(),
            store.clone(),
            ConversationActorConfig::default(),
        )
        .unwrap();
        let (second, mut second_events) = ConversationActor::spawn_on(
            &tokio::runtime::Handle::current(),
            second_id.clone(),
            store.clone(),
            ConversationActorConfig::default(),
        )
        .unwrap();
        assert!(matches!(
            first_events.recv().await,
            Some(ConversationActorEvent::Started { .. })
        ));
        assert!(matches!(
            second_events.recv().await,
            Some(ConversationActorEvent::Started { .. })
        ));

        first
            .send(ConversationCommand::AppendAssistantTextDelta {
                delta: "a".to_owned(),
            })
            .await
            .unwrap();
        second.send(ConversationCommand::Stop).await.unwrap();
        assert!(matches!(
            first_events.recv().await,
            Some(ConversationActorEvent::TranscriptAppended { .. })
        ));
        assert_eq!(
            second_events.recv().await,
            Some(ConversationActorEvent::Stopped {
                conversation_id: second_id.clone()
            })
        );

        first
            .send(ConversationCommand::AppendAssistantTextDelta {
                delta: "b".to_owned(),
            })
            .await
            .unwrap();
        assert!(matches!(
            first_events.recv().await,
            Some(ConversationActorEvent::TranscriptAppended { .. })
        ));
        assert_eq!(store.load(first_id).await.unwrap().len(), 2);
        assert_eq!(store.load(second_id).await.unwrap().len(), 0);
    }

    #[test]
    fn t12_prompt_context_assembles_transcript_messages_in_order() {
        let conversation_id = ConversationId::new("conversation-prompt");
        let context = assemble_prompt_context(PromptAssemblyInput {
            mode: AgentMode::Code,
            project_folder: Some("/repo".to_owned()),
            transcript_events: vec![
                TranscriptEvent {
                    conversation_id: conversation_id.clone(),
                    sequence: 1,
                    kind: TranscriptEventKind::UserMessage {
                        text: "hello".to_owned(),
                    },
                },
                TranscriptEvent {
                    conversation_id: conversation_id.clone(),
                    sequence: 2,
                    kind: TranscriptEventKind::AssistantTextDelta {
                        delta: "hel".to_owned(),
                    },
                },
                TranscriptEvent {
                    conversation_id,
                    sequence: 3,
                    kind: TranscriptEventKind::AssistantTextDelta {
                        delta: "lo".to_owned(),
                    },
                },
            ],
        });

        assert!(context.system_instruction.contains("Code mode"));
        assert!(context.system_instruction.contains("/repo"));
        assert_eq!(
            context.messages,
            vec![
                PromptMessage {
                    role: PromptRole::User,
                    content: "hello".to_owned(),
                },
                PromptMessage {
                    role: PromptRole::Assistant,
                    content: "hello".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn t12_prompt_context_coalesces_normalized_model_text_delta() {
        let conversation_id = ConversationId::new("conversation-model-event");
        let context = assemble_prompt_context(PromptAssemblyInput {
            mode: AgentMode::Debug,
            project_folder: None,
            transcript_events: vec![
                TranscriptEvent {
                    conversation_id: conversation_id.clone(),
                    sequence: 1,
                    kind: TranscriptEventKind::ModelStreamEvent {
                        event: NormalizedModelStreamEvent::text_delta(
                            ModelProviderKind::BedrockMantle,
                            "a",
                        ),
                    },
                },
                TranscriptEvent {
                    conversation_id,
                    sequence: 2,
                    kind: TranscriptEventKind::ModelStreamEvent {
                        event: NormalizedModelStreamEvent::text_delta(
                            ModelProviderKind::BedrockMantle,
                            "b",
                        ),
                    },
                },
            ],
        });

        assert_eq!(
            context.messages,
            vec![PromptMessage {
                role: PromptRole::Assistant,
                content: "ab".to_owned(),
            }]
        );
    }

    #[test]
    fn t12_builtin_modes_have_distinct_operational_instructions() {
        let modes = [
            AgentMode::Code,
            AgentMode::Architect,
            AgentMode::Ask,
            AgentMode::Debug,
            AgentMode::Plan,
        ];

        for mode in modes {
            let instruction = builtin_mode_instruction(&mode, Some("/repo"));
            assert!(instruction.contains("/repo"));
            assert!(!instruction.trim().is_empty());
        }
        assert!(
            builtin_mode_instruction(&AgentMode::Plan, Some("/repo")).contains("do not mutate")
        );
    }

    #[test]
    fn t12_custom_modes_validate_import_export_and_resolve_instruction() {
        let mode = CustomModeDefinition {
            slug: "review".to_owned(),
            display_name: "Review".to_owned(),
            role_definition: "Find correctness risks before edits.".to_owned(),
            instructions: "Prioritize bugs, regressions, and missing tests.".to_owned(),
            tool_groups: BTreeSet::from(["read".to_owned(), "test".to_owned()]),
            file_restrictions: vec![ModeFileRestriction {
                pattern: "**/*.rs".to_owned(),
                access: ModeFileAccess::ReadOnly,
            }],
            default_profile_id: Some("profile-review".to_owned()),
            scope: CustomModeScope::Project(ProjectFolderId::new("folder-1")),
        };
        let mut registry = CustomModeRegistry::new(vec![mode.clone()]).unwrap();
        let instruction = registry
            .instruction_for_mode(&AgentMode::Custom("review".to_owned()), Some("/repo"))
            .unwrap();

        assert!(instruction.contains("Review mode"));
        assert!(instruction.contains("/repo"));
        assert!(instruction.contains("Find correctness risks"));

        let exported = registry.export_json_pretty().unwrap();
        let imported = CustomModeRegistry::import_json(&exported).unwrap();
        assert_eq!(imported.list(), vec![mode.clone()]);

        let mut edited = mode;
        edited.instructions = "Review code and cite exact lines.".to_owned();
        registry.edit(edited.clone()).unwrap();
        assert_eq!(
            registry.get("review").unwrap().instructions,
            edited.instructions
        );
        assert_eq!(registry.delete("review").unwrap(), edited);
        assert!(matches!(
            registry.instruction_for_mode(&AgentMode::Custom("review".to_owned()), None),
            Err(CustomModeError::NotFound(slug)) if slug == "review"
        ));
    }

    #[test]
    fn t12_custom_modes_reject_invalid_slug_and_duplicate_slug() {
        let mut mode = CustomModeDefinition {
            slug: "Review".to_owned(),
            display_name: "Review".to_owned(),
            role_definition: "Find risks.".to_owned(),
            instructions: "Inspect only.".to_owned(),
            tool_groups: BTreeSet::new(),
            file_restrictions: Vec::new(),
            default_profile_id: None,
            scope: CustomModeScope::Global,
        };

        assert_eq!(
            mode.validate(),
            Err(CustomModeError::InvalidSlug("Review".to_owned()))
        );

        mode.slug = "review".to_owned();
        let error = CustomModeRegistry::new(vec![mode.clone(), mode]).unwrap_err();
        assert_eq!(error, CustomModeError::DuplicateSlug("review".to_owned()));
    }

    #[test]
    fn v10_plan_mode_allows_read_only_tools() {
        let decision = route_tool_request(&ToolRouteRequest {
            conversation_id: ConversationId::new("plan-conversation"),
            mode: AgentMode::Plan,
            tool_name: "read_file".to_owned(),
            mutation: ToolMutationClass::ReadOnly,
        });

        assert_eq!(decision, ToolRouteDecision::Allow);
    }

    #[test]
    fn v10_plan_mode_blocks_mutation_classes_at_tool_router() {
        for mutation in [
            ToolMutationClass::WritesProjectFiles,
            ToolMutationClass::RunsCommand,
            ToolMutationClass::NetworkMutation,
            ToolMutationClass::StartsRemoteSession,
            ToolMutationClass::UpdatesSettings,
        ] {
            let decision = route_tool_request(&ToolRouteRequest {
                conversation_id: ConversationId::new("plan-conversation"),
                mode: AgentMode::Plan,
                tool_name: "mutate".to_owned(),
                mutation,
            });

            assert!(matches!(
                decision,
                ToolRouteDecision::Reject {
                    reason: ToolRouteRejectionReason::PlanModeMutation,
                    ..
                }
            ));
        }
    }

    #[test]
    fn v10_plan_mode_classifies_read_only_rtk_commands() {
        for command in [
            vec!["rtk", "rg", "Plan mode", "SPEC.md"],
            vec!["rtk", "git", "status", "--short"],
            vec!["rtk", "git", "diff", "--", "src/lib.rs"],
            vec!["cargo", "test", "--workspace"],
            vec!["cargo", "build", "-p", "fastrock-app"],
            vec!["sed", "-n", "1,20p", "SPEC.md"],
            vec!["find", ".", "-maxdepth", "2", "-type", "f"],
        ] {
            assert_eq!(
                classify_plan_mode_rtk_command(&argv(command)),
                PlanModeCommandClassification::ReadOnly
            );
        }
    }

    #[test]
    fn v10_plan_mode_blocks_mutating_rtk_commands_by_purpose() {
        for (command, reason) in [
            (
                vec!["rtk", "cargo", "fmt", "--check"],
                PlanModeCommandBlockReason::Formatter,
            ),
            (
                vec!["rtk", "cargo", "generate"],
                PlanModeCommandBlockReason::Codegen,
            ),
            (
                vec!["rtk", "git", "commit", "-m", "plan"],
                PlanModeCommandBlockReason::GitMutation,
            ),
            (
                vec!["rtk", "npm", "install"],
                PlanModeCommandBlockReason::PackageInstall,
            ),
            (
                vec!["rtk", "sqlx", "migrate", "run"],
                PlanModeCommandBlockReason::Migration,
            ),
            (
                vec!["rtk", "rm", "-rf", "target"],
                PlanModeCommandBlockReason::FileMutation,
            ),
            (
                vec!["rtk", "bash", "-lc", "rg fastrock"],
                PlanModeCommandBlockReason::ShellCommand,
            ),
            (
                vec!["rtk", "make", "test"],
                PlanModeCommandBlockReason::UnknownCommand,
            ),
        ] {
            assert_eq!(
                classify_plan_mode_rtk_command(&argv(command)),
                PlanModeCommandClassification::Mutation { reason }
            );
        }
    }

    #[test]
    fn v10_plan_mode_routes_rtk_command_purpose() {
        assert_eq!(
            route_plan_mode_rtk_command(
                ConversationId::new("plan-conversation"),
                &argv(vec!["rtk", "cargo", "test", "--workspace"]),
            ),
            ToolRouteDecision::Allow
        );

        assert!(matches!(
            route_plan_mode_rtk_command(
                ConversationId::new("plan-conversation"),
                &argv(vec!["rtk", "cargo", "fmt", "--check"]),
            ),
            ToolRouteDecision::Reject {
                reason: ToolRouteRejectionReason::PlanModeMutation,
                ..
            }
        ));
    }

    #[test]
    fn t13_non_plan_modes_allow_mutation_routing() {
        let decision = route_tool_request(&ToolRouteRequest {
            conversation_id: ConversationId::new("code-conversation"),
            mode: AgentMode::Code,
            tool_name: "rtk".to_owned(),
            mutation: ToolMutationClass::RunsCommand,
        });

        assert_eq!(decision, ToolRouteDecision::Allow);
    }

    #[test]
    fn t13_plan_transcript_events_become_prompt_context_artifacts() {
        let conversation_id = ConversationId::new("plan-conversation");
        let context = assemble_prompt_context(PromptAssemblyInput {
            mode: AgentMode::Plan,
            project_folder: Some("/repo".to_owned()),
            transcript_events: vec![
                TranscriptEvent {
                    conversation_id: conversation_id.clone(),
                    sequence: 1,
                    kind: TranscriptEventKind::PlanCreated {
                        artifact: Box::new(PlanArtifact {
                            id: "plan-1".to_owned(),
                            prompt: "inspect and propose edits".to_owned(),
                            status: PlanArtifactStatus::Draft,
                            created_sequence: 1,
                            executed_sequence: None,
                        }),
                    },
                },
                TranscriptEvent {
                    conversation_id,
                    sequence: 2,
                    kind: TranscriptEventKind::PlanExecuted {
                        plan_id: "plan-1".to_owned(),
                    },
                },
            ],
        });

        assert_eq!(
            context.messages,
            vec![
                PromptMessage {
                    role: PromptRole::Assistant,
                    content: "[plan plan-1] inspect and propose edits".to_owned(),
                },
                PromptMessage {
                    role: PromptRole::Assistant,
                    content: "[plan executed] plan-1".to_owned(),
                },
            ]
        );
    }

    fn argv(parts: Vec<&str>) -> Vec<String> {
        parts.into_iter().map(str::to_owned).collect()
    }

    fn command_policy_request(command: Vec<String>) -> CommandPolicyRequest {
        CommandPolicyRequest {
            command,
            cwd: "/repo".to_owned(),
            project_root: Some("/repo".to_owned()),
            active_commands: 0,
        }
    }

    #[tokio::test]
    async fn t13_conversation_actor_records_and_executes_plan_artifacts() {
        let store = Arc::new(MemoryTranscriptStore::default());
        let conversation_id = ConversationId::new("plan-conversation");
        let (handle, mut events) = ConversationActor::spawn_on(
            &tokio::runtime::Handle::current(),
            conversation_id.clone(),
            store,
            ConversationActorConfig::default(),
        )
        .unwrap();

        assert_eq!(
            events.recv().await,
            Some(ConversationActorEvent::Started {
                conversation_id: conversation_id.clone(),
            })
        );

        handle
            .send(ConversationCommand::StartPlan {
                prompt: "inspect before editing".to_owned(),
            })
            .await
            .unwrap();

        let Some(ConversationActorEvent::TranscriptAppended { event, .. }) = events.recv().await
        else {
            panic!("expected plan-created transcript event");
        };
        let TranscriptEventKind::PlanCreated { artifact } = event.kind else {
            panic!("expected plan-created event");
        };
        assert_eq!(artifact.id, "plan-1");
        assert_eq!(artifact.status, PlanArtifactStatus::Draft);
        assert_eq!(artifact.created_sequence, 1);

        handle
            .send(ConversationCommand::ExecutePlan {
                plan_id: "plan-1".to_owned(),
            })
            .await
            .unwrap();

        assert!(matches!(
            events.recv().await,
            Some(ConversationActorEvent::TranscriptAppended {
                event: TranscriptEvent {
                    kind: TranscriptEventKind::PlanExecuted { plan_id },
                    ..
                },
                ..
            }) if plan_id == "plan-1"
        ));
        assert_eq!(
            events.recv().await,
            Some(ConversationActorEvent::StatusChanged {
                conversation_id: conversation_id.clone(),
                status: ConversationStatus::Running,
            })
        );
    }

    #[tokio::test]
    async fn t13_executing_unknown_plan_records_structured_error() {
        let store = Arc::new(MemoryTranscriptStore::default());
        let conversation_id = ConversationId::new("plan-conversation");
        let (handle, mut events) = ConversationActor::spawn_on(
            &tokio::runtime::Handle::current(),
            conversation_id.clone(),
            store,
            ConversationActorConfig::default(),
        )
        .unwrap();
        assert_eq!(
            events.recv().await,
            Some(ConversationActorEvent::Started {
                conversation_id: conversation_id.clone(),
            })
        );

        handle
            .send(ConversationCommand::ExecutePlan {
                plan_id: "missing".to_owned(),
            })
            .await
            .unwrap();

        let Some(ConversationActorEvent::TranscriptAppended { event, .. }) = events.recv().await
        else {
            panic!("expected error transcript event");
        };
        let TranscriptEventKind::Error { error } = event.kind else {
            panic!("expected structured error event");
        };

        assert_eq!(error.code, "FR_PLAN_NOT_FOUND");
        assert_eq!(error.source, FastrockSubsystem::Core);
        assert_eq!(
            error.context.conversation_id,
            Some("plan-conversation".to_owned())
        );
    }

    #[test]
    fn t21_goal_loop_creates_persistable_snapshot_and_updates_status() {
        let mut state = GoalLoopState::new_goal("ship feature", Some(100), 10);

        assert_eq!(
            state.snapshot(),
            Some(ThreadGoalSnapshot {
                objective: "ship feature".to_owned(),
                status: ThreadGoalStatus::Active,
                token_budget: Some(100),
                tokens_used: 0,
                elapsed_ms: 0,
            })
        );

        state.update_status(ThreadGoalStatus::Paused, 20);

        assert_eq!(state.goal().unwrap().status, ThreadGoalStatus::Paused);
        assert_eq!(state.goal().unwrap().updated_at_ms, 20);
    }

    #[test]
    fn t21_goal_loop_resumes_recoverable_stop_states_and_clears_goal() {
        for status in [
            ThreadGoalStatus::Paused,
            ThreadGoalStatus::Blocked,
            ThreadGoalStatus::UsageLimited,
            ThreadGoalStatus::BudgetLimited,
        ] {
            let mut state = GoalLoopState::new_goal("ship feature", Some(100), 10);
            state.update_status(status, 20);

            assert!(state.resume(30));
            assert_eq!(state.goal().unwrap().status, ThreadGoalStatus::Active);
            assert_eq!(state.goal().unwrap().updated_at_ms, 30);
            assert!(state.clear().is_some());
            assert_eq!(state.snapshot(), None);
        }
    }

    #[test]
    fn t21_goal_loop_does_not_resume_completed_goal() {
        let mut state = GoalLoopState::new_goal("finish", None, 10);
        state.update_status(ThreadGoalStatus::Complete, 20);

        assert!(!state.resume(30));
        assert!(!state.mark_usage_limited(40));
        assert!(!state.record_blocking_error("same-error", 50, 1));
        assert_eq!(state.goal().unwrap().status, ThreadGoalStatus::Complete);
        assert_eq!(state.goal().unwrap().updated_at_ms, 20);
    }

    #[test]
    fn v11_goal_usage_charges_each_turn_delta_once() {
        let mut state = GoalLoopState::new_goal("finish", Some(100), 10);

        assert!(state.charge_turn_usage("turn-1", 30, 1000, 20));
        assert!(!state.charge_turn_usage("turn-1", 30, 1000, 30));
        assert!(state.charge_turn_usage("turn-2", 70, 2000, 40));

        let goal = state.goal().unwrap();
        assert_eq!(goal.tokens_used, 100);
        assert_eq!(goal.elapsed_ms, 3000);
        assert_eq!(goal.status, ThreadGoalStatus::BudgetLimited);

        let mut hydrated = GoalLoopState::from_persisted_goal_with_charged_turn_ids(
            goal.clone(),
            ["turn-1", "turn-2"],
        );
        assert!(!hydrated.charge_turn_usage("turn-1", 10, 100, 50));
        assert!(hydrated.charge_turn_usage("turn-3", 10, 100, 60));
        assert_eq!(hydrated.goal().unwrap().tokens_used, 110);
    }

    #[test]
    fn t21_goal_loop_usage_limit_and_repeated_blocking_error_stop_goal() {
        let mut state = GoalLoopState::new_goal("finish", Some(100), 10);

        assert!(state.mark_usage_limited(20));
        assert_eq!(state.goal().unwrap().status, ThreadGoalStatus::UsageLimited);
        assert!(state.resume(30));
        assert!(!state.record_blocking_error("model-disconnect", 40, 3));
        assert!(!state.record_blocking_error("model-disconnect", 50, 3));
        assert!(state.record_blocking_error("model-disconnect", 60, 3));

        let goal = state.goal().unwrap();
        assert_eq!(goal.status, ThreadGoalStatus::Blocked);
        assert_eq!(goal.updated_at_ms, 60);
    }

    #[test]
    fn t21_goal_loop_resume_resets_blocking_error_audit() {
        let mut state = GoalLoopState::new_goal("finish", None, 10);

        assert!(state.record_blocking_error("tool-error", 20, 1));
        assert_eq!(state.goal().unwrap().status, ThreadGoalStatus::Blocked);
        assert!(state.resume(30));
        assert!(!state.record_blocking_error("tool-error", 40, 2));
        assert_eq!(state.goal().unwrap().status, ThreadGoalStatus::Active);
    }

    #[test]
    fn t23_model_request_metadata_is_visible_but_not_replayed_into_prompt() {
        let metadata = ModelRequestMetadata {
            provider: ModelProviderKind::BedrockMantle,
            profile_id: Some("profile-mantle".to_owned()),
            model_id: "anthropic.claude-test".to_owned(),
            endpoint: "https://bedrock-mantle.us-east-1.api.aws/v1/responses".to_owned(),
            region: Some("us-east-1".to_owned()),
            project_id: Some("project-1".to_owned()),
            api_shape: Some("responses".to_owned()),
            store: Some(true),
            streaming: true,
            request_kind: "responses.create".to_owned(),
        };
        let rendered = coalesce_transcript_events(
            [
                TranscriptRenderEvent::ModelRequest(metadata.clone()),
                TranscriptRenderEvent::Model(NormalizedModelStreamEvent::text_delta(
                    ModelProviderKind::BedrockMantle,
                    "done",
                )),
            ],
            TranscriptRenderConfig::default(),
        );

        assert_eq!(
            rendered,
            vec![
                RenderedTranscriptBlock::ModelRequest {
                    provider: ModelProviderKind::BedrockMantle,
                    details: BTreeMap::from([
                        ("api_shape".to_owned(), "responses".to_owned()),
                        (
                            "endpoint".to_owned(),
                            "https://bedrock-mantle.us-east-1.api.aws/v1/responses".to_owned(),
                        ),
                        ("model".to_owned(), "anthropic.claude-test".to_owned()),
                        ("profile_id".to_owned(), "profile-mantle".to_owned()),
                        ("project_id".to_owned(), "project-1".to_owned()),
                        ("region".to_owned(), "us-east-1".to_owned()),
                        ("request_kind".to_owned(), "responses.create".to_owned()),
                        ("store".to_owned(), "true".to_owned()),
                        ("streaming".to_owned(), "true".to_owned()),
                    ]),
                },
                RenderedTranscriptBlock::AssistantText {
                    text: "done".to_owned(),
                },
            ]
        );

        let serialized = serde_json::to_string(&metadata).unwrap();
        let serialized_lower = serialized.to_ascii_lowercase();
        assert!(!serialized_lower.contains("authorization"));
        assert!(!serialized_lower.contains("api_key"));
        assert!(!serialized_lower.contains("bearer"));

        let context = assemble_prompt_context(PromptAssemblyInput {
            mode: AgentMode::Code,
            project_folder: Some("/repo".to_owned()),
            transcript_events: vec![TranscriptEvent {
                conversation_id: ConversationId::new("conversation-1"),
                sequence: 1,
                kind: TranscriptEventKind::ModelRequest {
                    metadata: Box::new(metadata),
                },
            }],
        });
        assert!(context.messages.is_empty());
    }

    #[test]
    fn t23_model_text_deltas_coalesce_before_rendering() {
        let blocks = coalesce_transcript_events(
            [
                TranscriptRenderEvent::Model(NormalizedModelStreamEvent::text_delta(
                    ModelProviderKind::BedrockMantle,
                    "hel",
                )),
                TranscriptRenderEvent::Model(NormalizedModelStreamEvent::text_delta(
                    ModelProviderKind::BedrockMantle,
                    "lo",
                )),
                TranscriptRenderEvent::Model(NormalizedModelStreamEvent::completed(
                    ModelProviderKind::BedrockMantle,
                )),
            ],
            TranscriptRenderConfig::default(),
        );

        assert_eq!(
            blocks,
            vec![
                RenderedTranscriptBlock::AssistantText {
                    text: "hello".to_owned()
                },
                RenderedTranscriptBlock::ModelStatus {
                    provider: ModelProviderKind::BedrockMantle,
                    status: "completed".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn v12_command_output_coalesces_and_truncates_before_ui_rendering() {
        let blocks = coalesce_transcript_events(
            [
                TranscriptRenderEvent::CommandOutput {
                    command_id: "cmd-1".to_owned(),
                    stream: CommandOutputStream::Stdout,
                    bytes: b"abc".to_vec(),
                },
                TranscriptRenderEvent::CommandOutput {
                    command_id: "cmd-1".to_owned(),
                    stream: CommandOutputStream::Stdout,
                    bytes: b"def".to_vec(),
                },
                TranscriptRenderEvent::CommandOutput {
                    command_id: "cmd-1".to_owned(),
                    stream: CommandOutputStream::Stderr,
                    bytes: b"err".to_vec(),
                },
            ],
            TranscriptRenderConfig {
                max_command_capture_bytes: 4,
            },
        );

        assert_eq!(
            blocks,
            vec![RenderedTranscriptBlock::CommandOutput {
                command_id: "cmd-1".to_owned(),
                stdout: b"abcd".to_vec(),
                stderr: b"err".to_vec(),
                truncated: true,
            }]
        );
    }

    #[test]
    fn v12_active_transcript_ui_updates_are_rate_limited_to_thirty_hz() {
        let mut coalescer = TranscriptUiCoalescer::new(
            TranscriptUiCoalescerConfig::default(),
            ConversationVisibility::Active,
            0,
        )
        .unwrap();
        let mut updates = Vec::new();

        for now_ms in 1..=1_000 {
            if let Some(blocks) = coalescer.push(
                TranscriptRenderEvent::Model(NormalizedModelStreamEvent::text_delta(
                    ModelProviderKind::BedrockMantle,
                    "x",
                )),
                now_ms,
            ) {
                updates.push(blocks);
            }
        }
        if let Some(blocks) = coalescer.force_flush(1_000) {
            updates.push(blocks);
        }

        assert!(
            updates.len() <= 30,
            "got {} UI updates in one second",
            updates.len()
        );
        assert_eq!(rendered_assistant_text_len(&updates), 1_000);
    }

    #[test]
    fn v12_hidden_transcript_ui_updates_are_more_aggressively_coalesced() {
        let mut coalescer = TranscriptUiCoalescer::new(
            TranscriptUiCoalescerConfig::default(),
            ConversationVisibility::Hidden,
            0,
        )
        .unwrap();

        for now_ms in 1..250 {
            assert!(
                coalescer
                    .push(
                        TranscriptRenderEvent::Model(NormalizedModelStreamEvent::text_delta(
                            ModelProviderKind::BedrockRuntime,
                            "h",
                        )),
                        now_ms,
                    )
                    .is_none()
            );
        }

        let blocks = coalescer
            .push(
                TranscriptRenderEvent::Model(NormalizedModelStreamEvent::text_delta(
                    ModelProviderKind::BedrockRuntime,
                    "h",
                )),
                250,
            )
            .unwrap();

        assert_eq!(assistant_text_from_blocks(&blocks), "h".repeat(250));
        assert_eq!(coalescer.pending_event_count(), 0);
    }

    #[test]
    fn t23_file_events_flush_pending_blocks() {
        let blocks = coalesce_transcript_events(
            [
                TranscriptRenderEvent::Model(NormalizedModelStreamEvent::text_delta(
                    ModelProviderKind::BedrockRuntime,
                    "done",
                )),
                TranscriptRenderEvent::FileChanged {
                    path: "src/main.rs".to_owned(),
                    action: FileTranscriptAction::Modified,
                    diff_summary: DiffSummary {
                        added_lines: 2,
                        removed_lines: 1,
                        modified_lines: 3,
                    },
                },
            ],
            TranscriptRenderConfig::default(),
        );

        assert_eq!(
            blocks,
            vec![
                RenderedTranscriptBlock::AssistantText {
                    text: "done".to_owned()
                },
                RenderedTranscriptBlock::FileChange {
                    path: "src/main.rs".to_owned(),
                    action: FileTranscriptAction::Modified,
                    diff_summary: DiffSummary {
                        added_lines: 2,
                        removed_lines: 1,
                        modified_lines: 3,
                    },
                },
            ]
        );
        let RenderedTranscriptBlock::FileChange { diff_summary, .. } = &blocks[1] else {
            panic!("expected file change block");
        };
        assert_eq!(diff_summary.changed_lines(), 6);
    }

    #[test]
    fn v7_scheduler_switching_visible_conversation_does_not_cancel_hidden_running() {
        let first = ConversationId::new("first");
        let second = ConversationId::new("second");
        let mut scheduler = ConversationScheduler::default();
        scheduler.add_conversation(first.clone(), "First");
        scheduler.add_conversation(second.clone(), "Second");
        scheduler.set_status(&first, ConversationStatus::Running);
        scheduler.set_status(&second, ConversationStatus::Running);

        assert!(scheduler.switch_visible(&second));

        assert_eq!(scheduler.visible_conversation_id(), Some(&second));
        assert_eq!(
            scheduler.conversation(&first).unwrap().status,
            ConversationStatus::Running
        );
        assert_eq!(
            scheduler.conversation(&second).unwrap().status,
            ConversationStatus::Running
        );
    }

    #[test]
    fn t24_hidden_conversation_progress_is_tracked_and_cleared_when_visible() {
        let first = ConversationId::new("first");
        let second = ConversationId::new("second");
        let mut scheduler = ConversationScheduler::default();
        scheduler.add_conversation(first.clone(), "First");
        scheduler.add_conversation(second.clone(), "Second");

        scheduler.record_progress(&second, "streaming tokens");
        scheduler.record_progress(&second, "running rtk");

        assert_eq!(
            scheduler.hidden_progress(),
            vec![HiddenConversationProgress {
                summary: "running rtk".to_owned(),
                unread_events: 2,
            }]
        );

        assert!(scheduler.switch_visible(&second));
        assert!(scheduler.hidden_progress().is_empty());
    }

    fn rendered_assistant_text_len(updates: &[Vec<RenderedTranscriptBlock>]) -> usize {
        updates
            .iter()
            .flat_map(|blocks| blocks.iter())
            .map(|block| match block {
                RenderedTranscriptBlock::AssistantText { text } => text.len(),
                _ => 0,
            })
            .sum()
    }

    fn assistant_text_from_blocks(blocks: &[RenderedTranscriptBlock]) -> String {
        blocks
            .iter()
            .filter_map(|block| match block {
                RenderedTranscriptBlock::AssistantText { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[derive(Default)]
    struct MemoryTranscriptStore {
        events: tokio::sync::Mutex<Vec<TranscriptEvent>>,
    }

    impl TranscriptEventStore for MemoryTranscriptStore {
        fn append(
            &self,
            event: TranscriptEvent,
        ) -> Pin<Box<dyn Future<Output = Result<(), TranscriptStoreError>> + Send + '_>> {
            Box::pin(async move {
                self.events.lock().await.push(event);
                Ok(())
            })
        }

        fn load(
            &self,
            conversation_id: ConversationId,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<Vec<TranscriptEvent>, TranscriptStoreError>> + Send + '_,
            >,
        > {
            Box::pin(async move {
                Ok(self
                    .events
                    .lock()
                    .await
                    .iter()
                    .filter(|event| event.conversation_id == conversation_id)
                    .cloned()
                    .collect())
            })
        }
    }
}
