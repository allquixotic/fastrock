#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const UI_CRATE_NAME: &str = "fastrock-ui";
const CROCKFORD_BASE32: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
static GENERATED_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum UiProjectTargetKind {
    Local,
    Ssh,
    AwsSessionManager,
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationListRow {
    pub conversation_id: String,
    pub title: String,
    pub project_folder_id: Option<String>,
    pub folder_label: String,
    pub folder_path: String,
    pub target_kind: UiProjectTargetKind,
    pub target_label: String,
    pub profile_name: String,
    pub status: String,
    pub goal_status: Option<String>,
    pub last_updated_ms: i64,
    pub selected: bool,
    pub hidden_unread_events: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationListFilter {
    pub query: String,
    pub project_folder_id: Option<String>,
    pub target_kind: Option<UiProjectTargetKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationListGroup {
    pub key: String,
    pub label: String,
    pub target_label: String,
    pub target_kind: UiProjectTargetKind,
    pub rows: Vec<ConversationListRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationSidebarModel {
    rows: Vec<ConversationListRow>,
}

impl ConversationSidebarModel {
    pub fn new(rows: Vec<ConversationListRow>) -> Self {
        let mut model = Self { rows };
        model.sort_rows();
        model
    }

    pub fn rows(&self) -> &[ConversationListRow] {
        &self.rows
    }

    pub fn filtered_rows(&self, filter: &ConversationListFilter) -> Vec<ConversationListRow> {
        self.rows
            .iter()
            .filter(|row| row_matches_filter(row, filter))
            .cloned()
            .collect()
    }

    pub fn grouped(&self, filter: &ConversationListFilter) -> Vec<ConversationListGroup> {
        let mut groups = Vec::<ConversationListGroup>::new();
        for row in self.filtered_rows(filter) {
            let key = conversation_group_key(&row);
            if let Some(group) = groups.iter_mut().find(|group| group.key == key) {
                group.rows.push(row);
            } else {
                groups.push(ConversationListGroup {
                    key,
                    label: row.folder_label.clone(),
                    target_label: row.target_label.clone(),
                    target_kind: row.target_kind,
                    rows: vec![row],
                });
            }
        }
        groups.sort_by(|left, right| {
            left.target_label
                .cmp(&right.target_label)
                .then_with(|| left.label.cmp(&right.label))
        });
        groups
    }

    fn sort_rows(&mut self) {
        self.rows.sort_by(|left, right| {
            right
                .last_updated_ms
                .cmp(&left.last_updated_ms)
                .then_with(|| left.title.cmp(&right.title))
                .then_with(|| left.conversation_id.cmp(&right.conversation_id))
        });
    }
}

fn row_matches_filter(row: &ConversationListRow, filter: &ConversationListFilter) -> bool {
    if let Some(project_folder_id) = &filter.project_folder_id
        && row.project_folder_id.as_ref() != Some(project_folder_id)
    {
        return false;
    }
    if let Some(target_kind) = filter.target_kind
        && row.target_kind != target_kind
    {
        return false;
    }
    let query = filter.query.trim().to_ascii_lowercase();
    query.is_empty()
        || row.title.to_ascii_lowercase().contains(&query)
        || row.folder_label.to_ascii_lowercase().contains(&query)
        || row.folder_path.to_ascii_lowercase().contains(&query)
        || row.target_label.to_ascii_lowercase().contains(&query)
        || row.profile_name.to_ascii_lowercase().contains(&query)
        || row.status.to_ascii_lowercase().contains(&query)
        || row
            .goal_status
            .as_ref()
            .is_some_and(|goal| goal.to_ascii_lowercase().contains(&query))
}

fn conversation_group_key(row: &ConversationListRow) -> String {
    format!(
        "{}:{}:{}",
        project_target_kind_label(row.target_kind),
        row.target_label,
        row.project_folder_id
            .as_deref()
            .unwrap_or(row.folder_path.as_str())
    )
}

fn project_target_kind_label(kind: UiProjectTargetKind) -> &'static str {
    match kind {
        UiProjectTargetKind::Local => "local",
        UiProjectTargetKind::Ssh => "ssh",
        UiProjectTargetKind::AwsSessionManager => "ssm",
        UiProjectTargetKind::Remote => "remote",
    }
}

fn unique_duplicate_profile_name(profiles: &[LlmProfileForm], source_name: &str) -> String {
    let base = format!("{} Copy", source_name.trim());
    if profile_name_is_available(profiles, &base) {
        return base;
    }

    (2..)
        .map(|index| format!("{base} {index}"))
        .find(|candidate| profile_name_is_available(profiles, candidate))
        .expect("unbounded iterator returns a profile name")
}

fn profile_name_is_available(profiles: &[LlmProfileForm], candidate: &str) -> bool {
    !profiles
        .iter()
        .any(|profile| profile.name.eq_ignore_ascii_case(candidate))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmProfileForm {
    pub id: String,
    pub name: String,
    pub provider: UiLlmProvider,
    pub model_id: String,
    #[serde(default)]
    pub request_tuning: UiRequestTuning,
    pub region: String,
    pub endpoint_override: Option<String>,
    pub credential_source: UiCredentialSource,
    pub credential_preview: Option<UiAwsCredentialPreview>,
    pub mantle_settings: UiMantleProfileSettings,
    pub runtime_settings: UiRuntimeProfileSettings,
    pub enabled: bool,
    pub default_for_new_conversations: bool,
}

impl LlmProfileForm {
    pub fn new_bedrock_mantle(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: "New Bedrock Mantle Profile".to_owned(),
            provider: UiLlmProvider::BedrockMantle,
            model_id: String::new(),
            request_tuning: UiRequestTuning::default(),
            region: "us-east-1".to_owned(),
            endpoint_override: None,
            credential_source: UiCredentialSource::AwsCliProfile {
                profile_name: "default".to_owned(),
            },
            credential_preview: None,
            mantle_settings: UiMantleProfileSettings::default(),
            runtime_settings: UiRuntimeProfileSettings::default(),
            enabled: true,
            default_for_new_conversations: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiRequestTuning {
    pub max_output_tokens: Option<u32>,
    pub temperature_milli: Option<u16>,
    pub top_p_milli: Option<u16>,
    pub timeout_ms: Option<u64>,
    pub retry_max_attempts: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UiLlmProvider {
    BedrockMantle,
    BedrockRuntime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UiCredentialSource {
    DefaultChain,
    AwsCliProfile { profile_name: String },
    Environment,
    SecretRef { secret_ref: String },
}

impl UiCredentialSource {
    fn validate_as_aws_source(&self, errors: &mut Vec<ProfileValidationError>) {
        match self {
            Self::AwsCliProfile { profile_name } if profile_name.trim().is_empty() => {
                errors.push(ProfileValidationError {
                    field: "credential_source.profile_name",
                    message: "AWS CLI profile name is required",
                });
            }
            Self::SecretRef { secret_ref } if secret_ref.trim().is_empty() => {
                errors.push(ProfileValidationError {
                    field: "credential_source.secret_ref",
                    message: "secret reference is required",
                });
            }
            Self::DefaultChain
            | Self::Environment
            | Self::AwsCliProfile { .. }
            | Self::SecretRef { .. } => {}
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiAwsCredentialPreview {
    pub source_type: String,
    pub profile_name: Option<String>,
    pub region_source: Option<String>,
    pub account_identity: Option<String>,
    pub expiration: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiMantleProfileSettings {
    pub auth_mode: UiMantleAuthMode,
    pub project_id: Option<String>,
    pub api_shape: UiMantleApiShape,
    pub store_default: bool,
    pub use_background: bool,
}

impl Default for UiMantleProfileSettings {
    fn default() -> Self {
        Self {
            auth_mode: UiMantleAuthMode::AwsSigV4,
            project_id: None,
            api_shape: UiMantleApiShape::Auto,
            store_default: true,
            use_background: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UiMantleAuthMode {
    BearerApiKey { secret_ref: String },
    AwsSigV4,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UiMantleApiShape {
    Auto,
    Responses,
    ChatCompletions,
    AnthropicMessages,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiRuntimeProfileSettings {
    pub target_type: UiRuntimeTargetType,
    pub target: String,
    pub service_tier: Option<String>,
    pub prompt_cache_ttl_seconds: Option<u64>,
    pub reasoning_budget_tokens: Option<u32>,
}

impl Default for UiRuntimeProfileSettings {
    fn default() -> Self {
        Self {
            target_type: UiRuntimeTargetType::FoundationModel,
            target: String::new(),
            service_tier: None,
            prompt_cache_ttl_seconds: None,
            reasoning_budget_tokens: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UiRuntimeTargetType {
    FoundationModel,
    InferenceProfileArn,
    ApplicationInferenceProfileArn,
    PromptRouterArn,
    CustomArn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileValidationError {
    pub field: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LlmProfileSettingsState {
    profiles: Vec<LlmProfileForm>,
    selected_profile_id: Option<String>,
    draft: Option<LlmProfileForm>,
}

impl LlmProfileSettingsState {
    pub fn profiles(&self) -> &[LlmProfileForm] {
        &self.profiles
    }

    pub fn selected_profile_id(&self) -> Option<&str> {
        self.selected_profile_id.as_deref()
    }

    pub fn draft(&self) -> Option<&LlmProfileForm> {
        self.draft.as_ref()
    }

    pub fn profile(&self, profile_id: &str) -> Option<&LlmProfileForm> {
        self.profiles
            .iter()
            .find(|profile| profile.id == profile_id)
    }

    pub fn add_profile(&mut self) -> &LlmProfileForm {
        let id = self.allocate_profile_id();
        self.draft = Some(LlmProfileForm::new_bedrock_mantle(id));
        self.draft.as_ref().expect("draft was just created")
    }

    pub fn edit_profile(&mut self, profile_id: &str) -> bool {
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
        else {
            return false;
        };
        self.selected_profile_id = Some(profile.id.clone());
        self.draft = Some(profile);
        true
    }

    pub fn update_draft(&mut self, draft: LlmProfileForm) {
        self.draft = Some(draft);
    }

    pub fn replace_profiles(
        &mut self,
        profiles: Vec<LlmProfileForm>,
    ) -> Result<(), Vec<ProfileValidationError>> {
        let mut validated = Self::default();
        for profile in profiles {
            validated.update_draft(profile);
            validated.save_draft()?;
        }
        *self = validated;
        Ok(())
    }

    pub fn save_draft(&mut self) -> Result<(), Vec<ProfileValidationError>> {
        let Some(draft) = self.draft.clone() else {
            return Err(vec![ProfileValidationError {
                field: "draft",
                message: "no profile is being edited",
            }]);
        };
        let errors = self.validate_profile(&draft);
        if !errors.is_empty() {
            return Err(errors);
        }

        if draft.default_for_new_conversations {
            for profile in &mut self.profiles {
                if profile.id != draft.id {
                    profile.default_for_new_conversations = false;
                }
            }
        }

        match self
            .profiles
            .iter_mut()
            .find(|profile| profile.id == draft.id)
        {
            Some(existing) => *existing = draft.clone(),
            None => self.profiles.push(draft.clone()),
        }
        self.selected_profile_id = Some(draft.id);
        self.draft = None;
        Ok(())
    }

    pub fn remove_profile(&mut self, profile_id: &str) -> bool {
        let len_before = self.profiles.len();
        self.profiles.retain(|profile| profile.id != profile_id);
        if self.selected_profile_id.as_deref() == Some(profile_id) {
            self.selected_profile_id = None;
        }
        if self
            .draft
            .as_ref()
            .is_some_and(|draft| draft.id == profile_id)
        {
            self.draft = None;
        }
        self.profiles.len() != len_before
    }

    pub fn duplicate_profile(&mut self, profile_id: &str) -> Option<&LlmProfileForm> {
        let mut duplicate = self
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()?;
        duplicate.id = self.allocate_profile_id();
        duplicate.name = unique_duplicate_profile_name(&self.profiles, &duplicate.name);
        duplicate.default_for_new_conversations = false;
        self.draft = Some(duplicate);
        self.draft.as_ref()
    }

    pub fn set_credential_preview(
        &mut self,
        profile_id: &str,
        preview: Option<UiAwsCredentialPreview>,
    ) -> bool {
        let mut changed = false;
        if let Some(profile) = self
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
        {
            profile.credential_preview = preview.clone();
            changed = true;
        }
        if let Some(draft) = self.draft.as_mut().filter(|draft| draft.id == profile_id) {
            draft.credential_preview = preview;
            changed = true;
        }
        changed
    }

    fn allocate_profile_id(&mut self) -> String {
        loop {
            let id = generated_persisted_id();
            let used_by_saved = self.profiles.iter().any(|profile| profile.id == id);
            let used_by_draft = self.draft.as_ref().is_some_and(|draft| draft.id == id);
            if !used_by_saved && !used_by_draft {
                return id;
            }
        }
    }

    pub fn filter_profiles(
        &self,
        query: &str,
        provider: Option<UiLlmProvider>,
        region: Option<&str>,
    ) -> Vec<&LlmProfileForm> {
        let query = query.trim().to_ascii_lowercase();
        let region = region.map(str::trim).filter(|value| !value.is_empty());
        self.profiles
            .iter()
            .filter(|profile| provider.is_none_or(|provider| profile.provider == provider))
            .filter(|profile| region.is_none_or(|region| profile.region == region))
            .filter(|profile| {
                query.is_empty()
                    || profile.name.to_ascii_lowercase().contains(&query)
                    || profile.model_id.to_ascii_lowercase().contains(&query)
                    || profile.region.to_ascii_lowercase().contains(&query)
            })
            .collect()
    }

    fn validate_profile(&self, draft: &LlmProfileForm) -> Vec<ProfileValidationError> {
        let mut errors = Vec::new();
        if draft.name.trim().is_empty() {
            errors.push(ProfileValidationError {
                field: "name",
                message: "name is required",
            });
        }
        if self.profiles.iter().any(|profile| {
            profile.id != draft.id && profile.name.eq_ignore_ascii_case(draft.name.trim())
        }) {
            errors.push(ProfileValidationError {
                field: "name",
                message: "name must be unique",
            });
        }
        if draft.model_id.trim().is_empty() {
            errors.push(ProfileValidationError {
                field: "model_id",
                message: "model id is required",
            });
        }
        if draft.region.trim().is_empty() {
            errors.push(ProfileValidationError {
                field: "region",
                message: "AWS region is required",
            });
        }
        if let Some(endpoint_override) = &draft.endpoint_override {
            let endpoint_override = endpoint_override.trim();
            if endpoint_override.is_empty() {
                errors.push(ProfileValidationError {
                    field: "endpoint_override",
                    message: "endpoint override cannot be empty",
                });
            } else if !endpoint_override.starts_with("https://")
                && !endpoint_override.starts_with("http://")
            {
                errors.push(ProfileValidationError {
                    field: "endpoint_override",
                    message: "endpoint override must be an HTTP URL",
                });
            }
        }
        if draft
            .request_tuning
            .max_output_tokens
            .is_some_and(|tokens| tokens == 0)
        {
            errors.push(ProfileValidationError {
                field: "request.max_output_tokens",
                message: "max output tokens must be greater than zero",
            });
        }
        if draft
            .request_tuning
            .temperature_milli
            .is_some_and(|temperature| temperature > 1000)
        {
            errors.push(ProfileValidationError {
                field: "request.temperature_milli",
                message: "temperature must be between 0 and 1",
            });
        }
        if draft
            .request_tuning
            .top_p_milli
            .is_some_and(|top_p| top_p > 1000)
        {
            errors.push(ProfileValidationError {
                field: "request.top_p_milli",
                message: "top-p must be between 0 and 1",
            });
        }
        if draft
            .request_tuning
            .timeout_ms
            .is_some_and(|timeout_ms| timeout_ms == 0)
        {
            errors.push(ProfileValidationError {
                field: "request.timeout_ms",
                message: "timeout must be greater than zero",
            });
        }
        if draft
            .request_tuning
            .retry_max_attempts
            .is_some_and(|attempts| attempts == 0)
        {
            errors.push(ProfileValidationError {
                field: "request.retry_max_attempts",
                message: "retry attempts must be greater than zero",
            });
        }

        match draft.provider {
            UiLlmProvider::BedrockMantle => {
                if matches!(&draft.mantle_settings.auth_mode, UiMantleAuthMode::AwsSigV4) {
                    draft.credential_source.validate_as_aws_source(&mut errors);
                }
                if let UiMantleAuthMode::BearerApiKey { secret_ref } =
                    &draft.mantle_settings.auth_mode
                    && secret_ref.trim().is_empty()
                {
                    errors.push(ProfileValidationError {
                        field: "mantle.auth.secret_ref",
                        message: "Mantle API key secret reference is required",
                    });
                }
            }
            UiLlmProvider::BedrockRuntime => {
                draft.credential_source.validate_as_aws_source(&mut errors);
                if draft
                    .runtime_settings
                    .prompt_cache_ttl_seconds
                    .is_some_and(|ttl| ttl == 0)
                {
                    errors.push(ProfileValidationError {
                        field: "runtime.prompt_cache_ttl_seconds",
                        message: "prompt cache TTL must be greater than zero",
                    });
                }
            }
        }
        errors
    }
}

fn generated_persisted_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
        & 0x0000_FFFF_FFFF_FFFF;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn is_ulid(value: &str) -> bool {
        value.len() == 26
            && value.bytes().all(|byte| {
                matches!(
                    byte,
                    b'0'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'T' | b'V'..=b'Z'
                )
            })
    }

    fn valid_profile(id: &str, name: &str) -> LlmProfileForm {
        let mut profile = LlmProfileForm::new_bedrock_mantle(id);
        profile.name = name.to_owned();
        profile.model_id = "anthropic.claude-3-5-sonnet-20241022-v2:0".to_owned();
        profile
    }

    #[test]
    fn t6_add_edit_remove_duplicate_profiles() {
        let mut state = LlmProfileSettingsState::default();
        let draft_id = state.add_profile().id.clone();
        assert!(is_ulid(&draft_id));
        let mut draft = valid_profile(&draft_id, "Mantle");
        state.update_draft(draft.clone());
        state.save_draft().unwrap();

        assert_eq!(state.profiles().len(), 1);
        assert!(state.edit_profile(&draft_id));

        draft.name = "Mantle Updated".to_owned();
        state.update_draft(draft.clone());
        state.save_draft().unwrap();
        assert_eq!(state.profiles()[0].name, "Mantle Updated");

        let duplicate_id = state.duplicate_profile(&draft_id).unwrap().id.clone();
        let mut duplicate = state.draft().unwrap().clone();
        duplicate.name = "Mantle Copy".to_owned();
        state.update_draft(duplicate);
        state.save_draft().unwrap();
        assert_ne!(duplicate_id, draft_id);
        assert!(is_ulid(&duplicate_id));
        assert_eq!(state.profiles().len(), 2);

        assert!(state.remove_profile(&draft_id));
        assert_eq!(state.profiles().len(), 1);
    }

    #[test]
    fn t6_save_validation_rejects_missing_required_fields() {
        let mut state = LlmProfileSettingsState::default();
        let draft_id = state.add_profile().id.clone();
        let mut draft = LlmProfileForm::new_bedrock_mantle(draft_id);
        draft.name = " ".to_owned();
        draft.provider = UiLlmProvider::BedrockRuntime;
        draft.model_id = String::new();
        draft.region = String::new();
        draft.credential_source = UiCredentialSource::AwsCliProfile {
            profile_name: String::new(),
        };
        state.update_draft(draft);

        let errors = state.save_draft().unwrap_err();

        assert!(errors.iter().any(|error| error.field == "name"));
        assert!(errors.iter().any(|error| error.field == "model_id"));
        assert!(errors.iter().any(|error| error.field == "region"));
        assert!(
            errors
                .iter()
                .any(|error| error.field == "credential_source.profile_name")
        );
    }

    #[test]
    fn t6_request_tuning_validation_rejects_out_of_range_values() {
        let mut state = LlmProfileSettingsState::default();
        let mut draft = valid_profile("profile-1", "Mantle");
        draft.request_tuning.max_output_tokens = Some(0);
        draft.request_tuning.temperature_milli = Some(1001);
        draft.request_tuning.top_p_milli = Some(1001);
        draft.request_tuning.timeout_ms = Some(0);
        draft.request_tuning.retry_max_attempts = Some(0);
        state.update_draft(draft);

        let errors = state.save_draft().unwrap_err();

        assert!(
            errors
                .iter()
                .any(|error| error.field == "request.max_output_tokens")
        );
        assert!(
            errors
                .iter()
                .any(|error| error.field == "request.temperature_milli")
        );
        assert!(
            errors
                .iter()
                .any(|error| error.field == "request.top_p_milli")
        );
        assert!(
            errors
                .iter()
                .any(|error| error.field == "request.timeout_ms")
        );
        assert!(
            errors
                .iter()
                .any(|error| error.field == "request.retry_max_attempts")
        );
    }

    #[test]
    fn t6_default_profile_is_unique_after_save() {
        let mut state = LlmProfileSettingsState::default();
        state.update_draft(LlmProfileForm {
            default_for_new_conversations: true,
            ..valid_profile("profile-1", "Mantle")
        });
        state.save_draft().unwrap();
        state.update_draft(LlmProfileForm {
            default_for_new_conversations: true,
            ..valid_profile("profile-2", "Runtime")
        });
        state.save_draft().unwrap();

        let defaults = state
            .profiles()
            .iter()
            .filter(|profile| profile.default_for_new_conversations)
            .count();

        assert_eq!(defaults, 1);
        assert!(
            state
                .profiles()
                .iter()
                .find(|profile| profile.name == "Runtime")
                .unwrap()
                .default_for_new_conversations
        );
    }

    #[test]
    fn t6_duplicate_profile_requires_unique_name_on_save() {
        let mut state = LlmProfileSettingsState::default();
        state.update_draft(valid_profile("profile-1", "Mantle"));
        state.save_draft().unwrap();
        state.update_draft(valid_profile("profile-2", "mantle"));

        let errors = state.save_draft().unwrap_err();

        assert_eq!(
            errors,
            vec![ProfileValidationError {
                field: "name",
                message: "name must be unique",
            }]
        );
    }

    #[test]
    fn t6_duplicate_profile_allocates_unique_id_and_name_after_hydration() {
        let mut state = LlmProfileSettingsState::default();
        state.update_draft(valid_profile("profile-1", "Mantle"));
        state.save_draft().unwrap();
        state.update_draft(valid_profile("profile-2", "Mantle Copy"));
        state.save_draft().unwrap();

        let duplicate = state.duplicate_profile("profile-1").unwrap().clone();
        assert!(is_ulid(&duplicate.id));
        assert_ne!(duplicate.id, "profile-1");
        assert_ne!(duplicate.id, "profile-2");
        assert_eq!(duplicate.name, "Mantle Copy 2");
        let duplicate_id = duplicate.id.clone();
        state.save_draft().unwrap();

        assert_eq!(state.profiles().len(), 3);
        assert!(state.profile(&duplicate_id).is_some());
    }

    #[test]
    fn v16_settings_support_cli_profile_for_mantle_sigv4_and_runtime() {
        let mut state = LlmProfileSettingsState::default();
        let mut mantle = valid_profile("mantle", "Mantle");
        mantle.credential_source = UiCredentialSource::AwsCliProfile {
            profile_name: "mantle-admin".to_owned(),
        };
        mantle.mantle_settings.project_id = Some("project-1".to_owned());
        mantle.mantle_settings.api_shape = UiMantleApiShape::Auto;
        mantle.mantle_settings.store_default = true;
        state.update_draft(mantle);
        state.save_draft().unwrap();

        let mut runtime = valid_profile("runtime", "Runtime");
        runtime.provider = UiLlmProvider::BedrockRuntime;
        runtime.credential_source = UiCredentialSource::AwsCliProfile {
            profile_name: "runtime-admin".to_owned(),
        };
        runtime.runtime_settings.target_type = UiRuntimeTargetType::InferenceProfileArn;
        runtime.runtime_settings.target =
            "arn:aws:bedrock:us-east-1:123456789012:inference-profile/test".to_owned();
        runtime.runtime_settings.prompt_cache_ttl_seconds = Some(300);
        state.update_draft(runtime);
        state.save_draft().unwrap();

        assert!(matches!(
            &state.profiles()[0].credential_source,
            UiCredentialSource::AwsCliProfile { profile_name } if profile_name == "mantle-admin"
        ));
        assert!(matches!(
            &state.profiles()[1].credential_source,
            UiCredentialSource::AwsCliProfile { profile_name } if profile_name == "runtime-admin"
        ));
        assert_eq!(
            state.profiles()[0].mantle_settings.api_shape,
            UiMantleApiShape::Auto
        );
        assert_eq!(
            state.profiles()[1].runtime_settings.target_type,
            UiRuntimeTargetType::InferenceProfileArn
        );
    }

    #[test]
    fn t6_provider_specific_settings_validate_secret_and_ttl() {
        let mut state = LlmProfileSettingsState::default();
        let mut mantle = valid_profile("mantle", "Mantle");
        mantle.mantle_settings.auth_mode = UiMantleAuthMode::BearerApiKey {
            secret_ref: " ".to_owned(),
        };
        state.update_draft(mantle);

        let errors = state.save_draft().unwrap_err();

        assert!(
            errors
                .iter()
                .any(|error| error.field == "mantle.auth.secret_ref")
        );

        let mut runtime = valid_profile("runtime", "Runtime");
        runtime.provider = UiLlmProvider::BedrockRuntime;
        runtime.runtime_settings.prompt_cache_ttl_seconds = Some(0);
        state.update_draft(runtime);

        let errors = state.save_draft().unwrap_err();

        assert!(
            errors
                .iter()
                .any(|error| error.field == "runtime.prompt_cache_ttl_seconds")
        );
    }

    #[test]
    fn t6_profile_filter_matches_provider_model_and_region() {
        let mut state = LlmProfileSettingsState::default();
        state.update_draft(valid_profile("mantle", "Claude Mantle"));
        state.save_draft().unwrap();

        let mut runtime = valid_profile("runtime", "Runtime Nova");
        runtime.provider = UiLlmProvider::BedrockRuntime;
        runtime.model_id = "amazon.nova-pro-v1:0".to_owned();
        runtime.region = "us-west-2".to_owned();
        state.update_draft(runtime);
        state.save_draft().unwrap();

        let matches = state.filter_profiles(
            "nova",
            Some(UiLlmProvider::BedrockRuntime),
            Some("us-west-2"),
        );

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "Runtime Nova");
    }

    #[test]
    fn t2_conversation_sidebar_groups_and_filters_by_project_and_remote_target() {
        let model = ConversationSidebarModel::new(vec![
            conversation_row(
                "local-new",
                "Local New",
                "folder-local",
                "Local Repo",
                UiProjectTargetKind::Local,
                "local",
                30,
            ),
            conversation_row(
                "ssh-old",
                "SSH Old",
                "folder-ssh",
                "SSH Repo",
                UiProjectTargetKind::Ssh,
                "ssh:prod",
                10,
            ),
            conversation_row(
                "ssh-new",
                "SSH New",
                "folder-ssh",
                "SSH Repo",
                UiProjectTargetKind::Ssh,
                "ssh:prod",
                20,
            ),
        ]);

        assert_eq!(
            model
                .rows()
                .iter()
                .map(|row| row.conversation_id.as_str())
                .collect::<Vec<_>>(),
            vec!["local-new", "ssh-new", "ssh-old"]
        );

        let ssh_rows = model.filtered_rows(&ConversationListFilter {
            target_kind: Some(UiProjectTargetKind::Ssh),
            ..ConversationListFilter::default()
        });
        assert_eq!(ssh_rows.len(), 2);
        assert!(ssh_rows.iter().all(|row| row.target_label == "ssh:prod"));

        let local_groups = model.grouped(&ConversationListFilter {
            project_folder_id: Some("folder-local".to_owned()),
            ..ConversationListFilter::default()
        });
        assert_eq!(local_groups.len(), 1);
        assert_eq!(local_groups[0].label, "Local Repo");
        assert_eq!(local_groups[0].rows.len(), 1);
    }

    #[test]
    fn t2_conversation_sidebar_query_matches_goal_status_profile_and_folder() {
        let mut row = conversation_row(
            "goal",
            "Ship",
            "folder-local",
            "Local Repo",
            UiProjectTargetKind::Local,
            "local",
            10,
        );
        row.profile_name = "Bedrock Mantle".to_owned();
        row.goal_status = Some("Blocked".to_owned());
        let model = ConversationSidebarModel::new(vec![row]);

        assert_eq!(
            model
                .filtered_rows(&ConversationListFilter {
                    query: "blocked".to_owned(),
                    ..ConversationListFilter::default()
                })
                .len(),
            1
        );
        assert_eq!(
            model
                .filtered_rows(&ConversationListFilter {
                    query: "mantle".to_owned(),
                    ..ConversationListFilter::default()
                })
                .len(),
            1
        );
        assert!(
            model
                .filtered_rows(&ConversationListFilter {
                    query: "runtime".to_owned(),
                    ..ConversationListFilter::default()
                })
                .is_empty()
        );
    }

    fn conversation_row(
        id: &str,
        title: &str,
        project_folder_id: &str,
        folder_label: &str,
        target_kind: UiProjectTargetKind,
        target_label: &str,
        last_updated_ms: i64,
    ) -> ConversationListRow {
        ConversationListRow {
            conversation_id: id.to_owned(),
            title: title.to_owned(),
            project_folder_id: Some(project_folder_id.to_owned()),
            folder_label: folder_label.to_owned(),
            folder_path: format!("/repo/{project_folder_id}"),
            target_kind,
            target_label: target_label.to_owned(),
            profile_name: "Bedrock Runtime".to_owned(),
            status: "Running".to_owned(),
            goal_status: Some("Active".to_owned()),
            last_updated_ms,
            selected: false,
            hidden_unread_events: 0,
        }
    }
}
