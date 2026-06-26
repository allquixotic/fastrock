#![forbid(unsafe_code)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use fastrock_cache::{MemoryKind, MemoryRecord, MemoryScope};
use keyring::Entry;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::oneshot;

pub type PersistenceResult<T> = Result<T, PersistenceError>;
pub const DATABASE_FILE_NAME: &str = "fastrock.db";
pub const USER_SETTINGS_FILE_NAME: &str = "settings.toml";
pub const PROJECT_SETTINGS_DIR_NAME: &str = ".fastrock";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SecretRef(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConfigScope {
    Global,
    Project(String),
}

impl ConfigScope {
    fn storage_key(&self) -> String {
        match self {
            Self::Global => "global".to_owned(),
            Self::Project(project_id) => format!("project:{project_id}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistenceConfig {
    pub database_path: PathBuf,
}

impl PersistenceConfig {
    pub fn new(database_path: impl Into<PathBuf>) -> Self {
        Self {
            database_path: database_path.into(),
        }
    }

    pub fn default_for_current_platform() -> PersistenceResult<Self> {
        Ok(Self::new(default_database_path()?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastrockPlatform {
    Macos,
    Windows,
    Linux,
}

pub fn default_database_path() -> PersistenceResult<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    let local_appdata = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let xdg_data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    default_database_path_for(
        current_platform(),
        home.as_deref(),
        appdata.as_deref(),
        local_appdata.as_deref(),
        xdg_data_home.as_deref(),
    )
}

pub fn default_database_path_for(
    platform: FastrockPlatform,
    home: Option<&Path>,
    appdata: Option<&Path>,
    local_appdata: Option<&Path>,
    xdg_data_home: Option<&Path>,
) -> PersistenceResult<PathBuf> {
    Ok(
        default_app_data_dir_for(platform, home, appdata, local_appdata, xdg_data_home)?
            .join(DATABASE_FILE_NAME),
    )
}

pub fn default_user_settings_path_for(
    platform: FastrockPlatform,
    home: Option<&Path>,
    appdata: Option<&Path>,
    local_appdata: Option<&Path>,
    xdg_data_home: Option<&Path>,
) -> PersistenceResult<PathBuf> {
    Ok(
        default_app_data_dir_for(platform, home, appdata, local_appdata, xdg_data_home)?
            .join(USER_SETTINGS_FILE_NAME),
    )
}

pub fn default_app_data_dir_for(
    platform: FastrockPlatform,
    home: Option<&Path>,
    appdata: Option<&Path>,
    local_appdata: Option<&Path>,
    xdg_data_home: Option<&Path>,
) -> PersistenceResult<PathBuf> {
    Ok(match platform {
        FastrockPlatform::Macos => home
            .map(|home| {
                home.join("Library")
                    .join("Application Support")
                    .join("Fastrock")
            })
            .ok_or_else(|| PersistenceError::Startup("HOME is required on macOS".to_owned()))?,
        FastrockPlatform::Windows => local_appdata
            .or(appdata)
            .map(|base| base.join("Fastrock"))
            .ok_or_else(|| {
                PersistenceError::Startup(
                    "LOCALAPPDATA or APPDATA is required on Windows".to_owned(),
                )
            })?,
        FastrockPlatform::Linux => xdg_data_home
            .map(|base| base.join("fastrock"))
            .or_else(|| home.map(|home| home.join(".local").join("share").join("fastrock")))
            .ok_or_else(|| {
                PersistenceError::Startup("XDG_DATA_HOME or HOME is required on Linux".to_owned())
            })?,
    })
}

pub fn project_settings_path(project_root: impl AsRef<Path>) -> PathBuf {
    project_root
        .as_ref()
        .join(PROJECT_SETTINGS_DIR_NAME)
        .join(USER_SETTINGS_FILE_NAME)
}

pub fn read_settings_file(path: impl AsRef<Path>) -> PersistenceResult<Option<String>> {
    match fs::read_to_string(path.as_ref()) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(PersistenceError::Startup(error.to_string())),
    }
}

pub fn write_settings_file_atomic(path: impl AsRef<Path>, contents: &str) -> PersistenceResult<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|error| PersistenceError::Startup(error.to_string()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("settings.toml");
    let tmp_path = path.with_file_name(format!(".{file_name}.tmp-{}", now_ms()));
    fs::write(&tmp_path, contents).map_err(|error| PersistenceError::Startup(error.to_string()))?;
    match fs::rename(&tmp_path, path) {
        Ok(()) => Ok(()),
        Err(_) if path.exists() => {
            fs::remove_file(path).map_err(|error| PersistenceError::Startup(error.to_string()))?;
            fs::rename(&tmp_path, path)
                .map_err(|error| PersistenceError::Startup(error.to_string()))
        }
        Err(error) => {
            let _ = fs::remove_file(&tmp_path);
            Err(PersistenceError::Startup(error.to_string()))
        }
    }
}

fn current_platform() -> FastrockPlatform {
    if cfg!(target_os = "macos") {
        FastrockPlatform::Macos
    } else if cfg!(target_os = "windows") {
        FastrockPlatform::Windows
    } else {
        FastrockPlatform::Linux
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationMetadata {
    pub id: String,
    pub title: String,
    pub project_folder_id: Option<String>,
    pub status: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationEvent {
    pub conversation_id: String,
    pub event_type: String,
    pub payload_json: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredConversationEvent {
    pub id: i64,
    pub conversation_id: String,
    pub event_type: String,
    pub payload_json: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretRefRecord {
    pub id: SecretRef,
    pub service: String,
    pub account: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingsDocument {
    pub scope: ConfigScope,
    pub key: String,
    pub document_json: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmProfileRecord {
    pub id: String,
    pub provider: String,
    pub enabled: bool,
    pub document_json: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectFolderRecord {
    pub id: String,
    pub label: String,
    pub target_kind: String,
    pub path: String,
    pub remote_target_id: Option<String>,
    pub document_json: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalRecord {
    pub conversation_id: String,
    pub status: String,
    pub document_json: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditorTabRecord {
    pub path: String,
    pub conversation_id: Option<String>,
    pub active: bool,
    pub document_json: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpStatusSnapshot {
    pub server_id: String,
    pub status: String,
    pub document_json: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheMetadataRecord {
    pub cache_key: String,
    pub provider_plane: String,
    pub document_json: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistenceExportSnapshot {
    pub conversations: Vec<ConversationMetadata>,
    pub conversation_events: Vec<StoredConversationEvent>,
    pub settings_documents: Vec<SettingsDocument>,
    pub secret_refs: Vec<SecretRefRecord>,
    pub llm_profiles: Vec<LlmProfileRecord>,
    pub project_folders: Vec<ProjectFolderRecord>,
    pub goals: Vec<GoalRecord>,
    pub editor_tabs: Vec<EditorTabRecord>,
    pub mcp_status_snapshots: Vec<McpStatusSnapshot>,
    pub cache_metadata: Vec<CacheMetadataRecord>,
    pub memory_records: Vec<MemoryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistenceIntegrityReport {
    pub ok: bool,
    pub messages: Vec<String>,
}

impl PersistenceIntegrityReport {
    fn from_messages(messages: Vec<String>) -> Self {
        let ok = messages.len() == 1 && messages[0].eq_ignore_ascii_case("ok");
        Self { ok, messages }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistenceBackupReport {
    pub path: PathBuf,
    pub bytes: u64,
    pub created_at_ms: i64,
    pub integrity: PersistenceIntegrityReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistenceRepairReport {
    pub before: PersistenceIntegrityReport,
    pub after: PersistenceIntegrityReport,
    pub reindexed: bool,
    pub vacuumed: bool,
}

impl PersistenceExportSnapshot {
    pub fn from_json(contents: &str) -> PersistenceResult<Self> {
        serde_json::from_str(contents).map_err(|error| PersistenceError::Startup(error.to_string()))
    }

    pub fn to_json_pretty(&self) -> PersistenceResult<String> {
        serde_json::to_string_pretty(self)
            .map_err(|error| PersistenceError::Startup(error.to_string()))
    }
}

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("secret store error: {0}")]
    SecretStore(#[from] SecretStoreError),
    #[error("persistence actor is unavailable")]
    ActorUnavailable,
    #[error("persistence actor failed to start: {0}")]
    Startup(String),
    #[error("persistence actor join failed")]
    JoinFailed,
}

#[derive(Debug, Error)]
pub enum SecretStoreError {
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("secret not found")]
    NotFound,
}

pub trait SecretStore: Send {
    fn put_secret(&self, record: &SecretRefRecord, secret: &str) -> Result<(), SecretStoreError>;
    fn get_secret(&self, record: &SecretRefRecord) -> Result<String, SecretStoreError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeyringSecretStore;

impl SecretStore for OsKeyringSecretStore {
    fn put_secret(&self, record: &SecretRefRecord, secret: &str) -> Result<(), SecretStoreError> {
        Entry::new(&record.service, &record.account)?.set_password(secret)?;
        Ok(())
    }

    fn get_secret(&self, record: &SecretRefRecord) -> Result<String, SecretStoreError> {
        Ok(Entry::new(&record.service, &record.account)?.get_password()?)
    }
}

pub struct PersistenceActorHandle {
    command_tx: std_mpsc::Sender<PersistenceCommand>,
    join_handle: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct PersistenceActorClient {
    command_tx: std_mpsc::Sender<PersistenceCommand>,
}

impl PersistenceActorHandle {
    pub fn spawn(config: PersistenceConfig) -> PersistenceResult<Self> {
        Self::spawn_with_secret_store(config, OsKeyringSecretStore)
    }

    pub fn spawn_with_secret_store<S>(
        config: PersistenceConfig,
        secret_store: S,
    ) -> PersistenceResult<Self>
    where
        S: SecretStore + 'static,
    {
        let (command_tx, command_rx) = std_mpsc::channel();
        let (ready_tx, ready_rx) = std_mpsc::sync_channel(1);
        let join_handle = thread::Builder::new()
            .name("fastrock-persistence".to_owned())
            .spawn(move || {
                let startup = PersistenceConnection::open(config, Box::new(secret_store));
                match startup {
                    Ok(mut connection) => {
                        let _ = ready_tx.send(Ok(()));
                        connection.run(command_rx);
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                }
            })
            .map_err(|error| PersistenceError::Startup(error.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                command_tx,
                join_handle: Some(join_handle),
            }),
            Ok(Err(error)) => {
                let _ = join_handle.join();
                Err(error)
            }
            Err(error) => Err(PersistenceError::Startup(error.to_string())),
        }
    }

    pub async fn current_schema_version(&self) -> PersistenceResult<i64> {
        self.client()
            .request(|respond_to| PersistenceCommand::CurrentSchemaVersion { respond_to })
            .await
    }

    pub async fn journal_mode(&self) -> PersistenceResult<String> {
        self.client()
            .request(|respond_to| PersistenceCommand::JournalMode { respond_to })
            .await
    }

    pub async fn integrity_check(&self) -> PersistenceResult<PersistenceIntegrityReport> {
        self.client()
            .request(|respond_to| PersistenceCommand::IntegrityCheck { respond_to })
            .await
    }

    pub async fn backup_database(
        &self,
        path: impl Into<PathBuf>,
    ) -> PersistenceResult<PersistenceBackupReport> {
        self.client()
            .request(|respond_to| PersistenceCommand::BackupDatabase {
                path: path.into(),
                respond_to,
            })
            .await
    }

    pub async fn repair_database(&self) -> PersistenceResult<PersistenceRepairReport> {
        self.client()
            .request(|respond_to| PersistenceCommand::RepairDatabase { respond_to })
            .await
    }

    pub async fn upsert_conversation(
        &self,
        metadata: ConversationMetadata,
    ) -> PersistenceResult<()> {
        self.client()
            .request(|respond_to| PersistenceCommand::UpsertConversation {
                metadata,
                respond_to,
            })
            .await
    }

    pub async fn delete_conversation(&self, id: impl Into<String>) -> PersistenceResult<bool> {
        self.client()
            .request(|respond_to| PersistenceCommand::DeleteConversation {
                id: id.into(),
                respond_to,
            })
            .await
    }

    pub async fn list_conversations(&self) -> PersistenceResult<Vec<ConversationMetadata>> {
        self.client()
            .request(|respond_to| PersistenceCommand::ListConversations { respond_to })
            .await
    }

    pub async fn append_conversation_event(
        &self,
        event: ConversationEvent,
    ) -> PersistenceResult<i64> {
        self.client()
            .request(|respond_to| PersistenceCommand::AppendConversationEvent { event, respond_to })
            .await
    }

    pub async fn load_conversation_events(
        &self,
        conversation_id: impl Into<String>,
    ) -> PersistenceResult<Vec<StoredConversationEvent>> {
        self.client()
            .request(|respond_to| PersistenceCommand::LoadConversationEvents {
                conversation_id: conversation_id.into(),
                respond_to,
            })
            .await
    }

    pub async fn store_secret_ref(&self, record: SecretRefRecord) -> PersistenceResult<()> {
        self.client()
            .request(|respond_to| PersistenceCommand::StoreSecretRef { record, respond_to })
            .await
    }

    pub async fn store_secret(
        &self,
        record: SecretRefRecord,
        secret: impl Into<String>,
    ) -> PersistenceResult<()> {
        self.client()
            .request(|respond_to| PersistenceCommand::StoreSecret {
                record,
                secret: secret.into(),
                respond_to,
            })
            .await
    }

    pub async fn get_secret(&self, id: SecretRef) -> PersistenceResult<Option<String>> {
        self.client()
            .request(|respond_to| PersistenceCommand::GetSecret { id, respond_to })
            .await
    }

    pub async fn list_secret_refs(&self) -> PersistenceResult<Vec<SecretRefRecord>> {
        self.client()
            .request(|respond_to| PersistenceCommand::ListSecretRefs { respond_to })
            .await
    }

    pub async fn store_settings_document(
        &self,
        document: SettingsDocument,
    ) -> PersistenceResult<()> {
        self.client()
            .request(|respond_to| PersistenceCommand::StoreSettingsDocument {
                document,
                respond_to,
            })
            .await
    }

    pub async fn load_settings_document(
        &self,
        scope: ConfigScope,
        key: impl Into<String>,
    ) -> PersistenceResult<Option<SettingsDocument>> {
        self.client()
            .request(|respond_to| PersistenceCommand::LoadSettingsDocument {
                scope,
                key: key.into(),
                respond_to,
            })
            .await
    }

    pub async fn upsert_llm_profile(&self, record: LlmProfileRecord) -> PersistenceResult<()> {
        self.client()
            .request(|respond_to| PersistenceCommand::UpsertLlmProfile { record, respond_to })
            .await
    }

    pub async fn list_llm_profiles(&self) -> PersistenceResult<Vec<LlmProfileRecord>> {
        self.client()
            .request(|respond_to| PersistenceCommand::ListLlmProfiles { respond_to })
            .await
    }

    pub async fn delete_llm_profile(&self, id: impl Into<String>) -> PersistenceResult<bool> {
        self.client()
            .request(|respond_to| PersistenceCommand::DeleteLlmProfile {
                id: id.into(),
                respond_to,
            })
            .await
    }

    pub async fn upsert_project_folder(
        &self,
        record: ProjectFolderRecord,
    ) -> PersistenceResult<()> {
        self.client()
            .request(|respond_to| PersistenceCommand::UpsertProjectFolder { record, respond_to })
            .await
    }

    pub async fn list_project_folders(&self) -> PersistenceResult<Vec<ProjectFolderRecord>> {
        self.client()
            .request(|respond_to| PersistenceCommand::ListProjectFolders { respond_to })
            .await
    }

    pub async fn delete_project_folder(&self, id: impl Into<String>) -> PersistenceResult<bool> {
        self.client()
            .request(|respond_to| PersistenceCommand::DeleteProjectFolder {
                id: id.into(),
                respond_to,
            })
            .await
    }

    pub async fn upsert_goal(&self, record: GoalRecord) -> PersistenceResult<()> {
        self.client()
            .request(|respond_to| PersistenceCommand::UpsertGoal { record, respond_to })
            .await
    }

    pub async fn list_goals(&self) -> PersistenceResult<Vec<GoalRecord>> {
        self.client()
            .request(|respond_to| PersistenceCommand::ListGoals { respond_to })
            .await
    }

    pub async fn delete_goal(&self, conversation_id: impl Into<String>) -> PersistenceResult<bool> {
        self.client()
            .request(|respond_to| PersistenceCommand::DeleteGoal {
                conversation_id: conversation_id.into(),
                respond_to,
            })
            .await
    }

    pub async fn upsert_editor_tab(&self, record: EditorTabRecord) -> PersistenceResult<()> {
        self.client()
            .request(|respond_to| PersistenceCommand::UpsertEditorTab { record, respond_to })
            .await
    }

    pub async fn list_editor_tabs(&self) -> PersistenceResult<Vec<EditorTabRecord>> {
        self.client()
            .request(|respond_to| PersistenceCommand::ListEditorTabs { respond_to })
            .await
    }

    pub async fn delete_editor_tab(&self, path: impl Into<String>) -> PersistenceResult<bool> {
        self.client()
            .request(|respond_to| PersistenceCommand::DeleteEditorTab {
                path: path.into(),
                respond_to,
            })
            .await
    }

    pub async fn upsert_mcp_status(&self, snapshot: McpStatusSnapshot) -> PersistenceResult<()> {
        self.client()
            .request(|respond_to| PersistenceCommand::UpsertMcpStatus {
                snapshot,
                respond_to,
            })
            .await
    }

    pub async fn list_mcp_statuses(&self) -> PersistenceResult<Vec<McpStatusSnapshot>> {
        self.client()
            .request(|respond_to| PersistenceCommand::ListMcpStatuses { respond_to })
            .await
    }

    pub async fn delete_mcp_status(&self, server_id: impl Into<String>) -> PersistenceResult<bool> {
        self.client()
            .request(|respond_to| PersistenceCommand::DeleteMcpStatus {
                server_id: server_id.into(),
                respond_to,
            })
            .await
    }

    pub async fn upsert_cache_metadata(
        &self,
        record: CacheMetadataRecord,
    ) -> PersistenceResult<()> {
        self.client()
            .request(|respond_to| PersistenceCommand::UpsertCacheMetadata { record, respond_to })
            .await
    }

    pub async fn list_cache_metadata(&self) -> PersistenceResult<Vec<CacheMetadataRecord>> {
        self.client()
            .request(|respond_to| PersistenceCommand::ListCacheMetadata { respond_to })
            .await
    }

    pub async fn upsert_memory_record(&self, record: MemoryRecord) -> PersistenceResult<()> {
        self.client()
            .request(|respond_to| PersistenceCommand::UpsertMemoryRecord { record, respond_to })
            .await
    }

    pub async fn list_memory_records(&self) -> PersistenceResult<Vec<MemoryRecord>> {
        self.client()
            .request(|respond_to| PersistenceCommand::ListMemoryRecords { respond_to })
            .await
    }

    pub async fn set_memory_enabled(
        &self,
        id: impl Into<String>,
        enabled: bool,
        updated_at_ms: i64,
    ) -> PersistenceResult<bool> {
        self.client()
            .request(|respond_to| PersistenceCommand::SetMemoryEnabled {
                id: id.into(),
                enabled,
                updated_at_ms,
                respond_to,
            })
            .await
    }

    pub async fn delete_memory_record(&self, id: impl Into<String>) -> PersistenceResult<bool> {
        self.client()
            .request(|respond_to| PersistenceCommand::DeleteMemoryRecord {
                id: id.into(),
                respond_to,
            })
            .await
    }

    pub async fn export_redacted_snapshot(&self) -> PersistenceResult<PersistenceExportSnapshot> {
        self.client()
            .request(|respond_to| PersistenceCommand::ExportRedactedSnapshot { respond_to })
            .await
    }

    pub async fn import_redacted_snapshot(
        &self,
        snapshot: PersistenceExportSnapshot,
    ) -> PersistenceResult<()> {
        self.client()
            .request(|respond_to| PersistenceCommand::ImportRedactedSnapshot {
                snapshot,
                respond_to,
            })
            .await
    }

    pub fn client(&self) -> PersistenceActorClient {
        PersistenceActorClient {
            command_tx: self.command_tx.clone(),
        }
    }

    pub async fn shutdown(mut self) -> PersistenceResult<()> {
        let (respond_to, response_rx) = oneshot::channel();
        self.command_tx
            .send(PersistenceCommand::Shutdown { respond_to })
            .map_err(|_| PersistenceError::ActorUnavailable)?;
        response_rx
            .await
            .map_err(|_| PersistenceError::ActorUnavailable)?;

        if let Some(join_handle) = self.join_handle.take() {
            tokio::task::spawn_blocking(move || join_handle.join())
                .await
                .map_err(|_| PersistenceError::JoinFailed)?
                .map_err(|_| PersistenceError::JoinFailed)?;
        }

        Ok(())
    }
}

impl PersistenceActorClient {
    pub async fn upsert_conversation(
        &self,
        metadata: ConversationMetadata,
    ) -> PersistenceResult<()> {
        self.request(|respond_to| PersistenceCommand::UpsertConversation {
            metadata,
            respond_to,
        })
        .await
    }

    pub async fn delete_conversation(&self, id: impl Into<String>) -> PersistenceResult<bool> {
        self.request(|respond_to| PersistenceCommand::DeleteConversation {
            id: id.into(),
            respond_to,
        })
        .await
    }

    pub async fn list_conversations(&self) -> PersistenceResult<Vec<ConversationMetadata>> {
        self.request(|respond_to| PersistenceCommand::ListConversations { respond_to })
            .await
    }

    pub async fn integrity_check(&self) -> PersistenceResult<PersistenceIntegrityReport> {
        self.request(|respond_to| PersistenceCommand::IntegrityCheck { respond_to })
            .await
    }

    pub async fn backup_database(
        &self,
        path: impl Into<PathBuf>,
    ) -> PersistenceResult<PersistenceBackupReport> {
        self.request(|respond_to| PersistenceCommand::BackupDatabase {
            path: path.into(),
            respond_to,
        })
        .await
    }

    pub async fn repair_database(&self) -> PersistenceResult<PersistenceRepairReport> {
        self.request(|respond_to| PersistenceCommand::RepairDatabase { respond_to })
            .await
    }

    pub async fn append_conversation_event(
        &self,
        event: ConversationEvent,
    ) -> PersistenceResult<i64> {
        self.request(|respond_to| PersistenceCommand::AppendConversationEvent { event, respond_to })
            .await
    }

    pub async fn load_conversation_events(
        &self,
        conversation_id: impl Into<String>,
    ) -> PersistenceResult<Vec<StoredConversationEvent>> {
        self.request(|respond_to| PersistenceCommand::LoadConversationEvents {
            conversation_id: conversation_id.into(),
            respond_to,
        })
        .await
    }

    pub async fn load_settings_document(
        &self,
        scope: ConfigScope,
        key: impl Into<String>,
    ) -> PersistenceResult<Option<SettingsDocument>> {
        self.request(|respond_to| PersistenceCommand::LoadSettingsDocument {
            scope,
            key: key.into(),
            respond_to,
        })
        .await
    }

    pub async fn store_settings_document(
        &self,
        document: SettingsDocument,
    ) -> PersistenceResult<()> {
        self.request(|respond_to| PersistenceCommand::StoreSettingsDocument {
            document,
            respond_to,
        })
        .await
    }

    pub async fn store_secret_ref(&self, record: SecretRefRecord) -> PersistenceResult<()> {
        self.request(|respond_to| PersistenceCommand::StoreSecretRef { record, respond_to })
            .await
    }

    pub async fn store_secret(
        &self,
        record: SecretRefRecord,
        secret: impl Into<String>,
    ) -> PersistenceResult<()> {
        self.request(|respond_to| PersistenceCommand::StoreSecret {
            record,
            secret: secret.into(),
            respond_to,
        })
        .await
    }

    pub async fn get_secret(&self, id: SecretRef) -> PersistenceResult<Option<String>> {
        self.request(|respond_to| PersistenceCommand::GetSecret { id, respond_to })
            .await
    }

    pub async fn list_secret_refs(&self) -> PersistenceResult<Vec<SecretRefRecord>> {
        self.request(|respond_to| PersistenceCommand::ListSecretRefs { respond_to })
            .await
    }

    pub async fn list_llm_profiles(&self) -> PersistenceResult<Vec<LlmProfileRecord>> {
        self.request(|respond_to| PersistenceCommand::ListLlmProfiles { respond_to })
            .await
    }

    pub async fn upsert_llm_profile(&self, record: LlmProfileRecord) -> PersistenceResult<()> {
        self.request(|respond_to| PersistenceCommand::UpsertLlmProfile { record, respond_to })
            .await
    }

    pub async fn delete_llm_profile(&self, id: impl Into<String>) -> PersistenceResult<bool> {
        self.request(|respond_to| PersistenceCommand::DeleteLlmProfile {
            id: id.into(),
            respond_to,
        })
        .await
    }

    pub async fn list_project_folders(&self) -> PersistenceResult<Vec<ProjectFolderRecord>> {
        self.request(|respond_to| PersistenceCommand::ListProjectFolders { respond_to })
            .await
    }

    pub async fn upsert_project_folder(
        &self,
        record: ProjectFolderRecord,
    ) -> PersistenceResult<()> {
        self.request(|respond_to| PersistenceCommand::UpsertProjectFolder { record, respond_to })
            .await
    }

    pub async fn delete_project_folder(&self, id: impl Into<String>) -> PersistenceResult<bool> {
        self.request(|respond_to| PersistenceCommand::DeleteProjectFolder {
            id: id.into(),
            respond_to,
        })
        .await
    }

    pub async fn list_goals(&self) -> PersistenceResult<Vec<GoalRecord>> {
        self.request(|respond_to| PersistenceCommand::ListGoals { respond_to })
            .await
    }

    pub async fn upsert_goal(&self, record: GoalRecord) -> PersistenceResult<()> {
        self.request(|respond_to| PersistenceCommand::UpsertGoal { record, respond_to })
            .await
    }

    pub async fn delete_goal(&self, conversation_id: impl Into<String>) -> PersistenceResult<bool> {
        self.request(|respond_to| PersistenceCommand::DeleteGoal {
            conversation_id: conversation_id.into(),
            respond_to,
        })
        .await
    }

    pub async fn upsert_editor_tab(&self, record: EditorTabRecord) -> PersistenceResult<()> {
        self.request(|respond_to| PersistenceCommand::UpsertEditorTab { record, respond_to })
            .await
    }

    pub async fn list_editor_tabs(&self) -> PersistenceResult<Vec<EditorTabRecord>> {
        self.request(|respond_to| PersistenceCommand::ListEditorTabs { respond_to })
            .await
    }

    pub async fn delete_editor_tab(&self, path: impl Into<String>) -> PersistenceResult<bool> {
        self.request(|respond_to| PersistenceCommand::DeleteEditorTab {
            path: path.into(),
            respond_to,
        })
        .await
    }

    pub async fn list_mcp_statuses(&self) -> PersistenceResult<Vec<McpStatusSnapshot>> {
        self.request(|respond_to| PersistenceCommand::ListMcpStatuses { respond_to })
            .await
    }

    pub async fn upsert_mcp_status(&self, snapshot: McpStatusSnapshot) -> PersistenceResult<()> {
        self.request(|respond_to| PersistenceCommand::UpsertMcpStatus {
            snapshot,
            respond_to,
        })
        .await
    }

    pub async fn delete_mcp_status(&self, server_id: impl Into<String>) -> PersistenceResult<bool> {
        self.request(|respond_to| PersistenceCommand::DeleteMcpStatus {
            server_id: server_id.into(),
            respond_to,
        })
        .await
    }

    pub async fn list_cache_metadata(&self) -> PersistenceResult<Vec<CacheMetadataRecord>> {
        self.request(|respond_to| PersistenceCommand::ListCacheMetadata { respond_to })
            .await
    }

    pub async fn list_memory_records(&self) -> PersistenceResult<Vec<MemoryRecord>> {
        self.request(|respond_to| PersistenceCommand::ListMemoryRecords { respond_to })
            .await
    }

    pub async fn upsert_memory_record(&self, record: MemoryRecord) -> PersistenceResult<()> {
        self.request(|respond_to| PersistenceCommand::UpsertMemoryRecord { record, respond_to })
            .await
    }

    pub async fn set_memory_enabled(
        &self,
        id: impl Into<String>,
        enabled: bool,
        updated_at_ms: i64,
    ) -> PersistenceResult<bool> {
        self.request(|respond_to| PersistenceCommand::SetMemoryEnabled {
            id: id.into(),
            enabled,
            updated_at_ms,
            respond_to,
        })
        .await
    }

    pub async fn delete_memory_record(&self, id: impl Into<String>) -> PersistenceResult<bool> {
        self.request(|respond_to| PersistenceCommand::DeleteMemoryRecord {
            id: id.into(),
            respond_to,
        })
        .await
    }

    pub async fn export_redacted_snapshot(&self) -> PersistenceResult<PersistenceExportSnapshot> {
        self.request(|respond_to| PersistenceCommand::ExportRedactedSnapshot { respond_to })
            .await
    }

    pub async fn import_redacted_snapshot(
        &self,
        snapshot: PersistenceExportSnapshot,
    ) -> PersistenceResult<()> {
        self.request(|respond_to| PersistenceCommand::ImportRedactedSnapshot {
            snapshot,
            respond_to,
        })
        .await
    }

    async fn request<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<PersistenceResult<T>>) -> PersistenceCommand,
    ) -> PersistenceResult<T>
    where
        T: Send + 'static,
    {
        let (respond_to, response_rx) = oneshot::channel();
        self.command_tx
            .send(build(respond_to))
            .map_err(|_| PersistenceError::ActorUnavailable)?;
        response_rx
            .await
            .map_err(|_| PersistenceError::ActorUnavailable)?
    }
}

enum PersistenceCommand {
    CurrentSchemaVersion {
        respond_to: oneshot::Sender<PersistenceResult<i64>>,
    },
    JournalMode {
        respond_to: oneshot::Sender<PersistenceResult<String>>,
    },
    IntegrityCheck {
        respond_to: oneshot::Sender<PersistenceResult<PersistenceIntegrityReport>>,
    },
    BackupDatabase {
        path: PathBuf,
        respond_to: oneshot::Sender<PersistenceResult<PersistenceBackupReport>>,
    },
    RepairDatabase {
        respond_to: oneshot::Sender<PersistenceResult<PersistenceRepairReport>>,
    },
    UpsertConversation {
        metadata: ConversationMetadata,
        respond_to: oneshot::Sender<PersistenceResult<()>>,
    },
    DeleteConversation {
        id: String,
        respond_to: oneshot::Sender<PersistenceResult<bool>>,
    },
    ListConversations {
        respond_to: oneshot::Sender<PersistenceResult<Vec<ConversationMetadata>>>,
    },
    AppendConversationEvent {
        event: ConversationEvent,
        respond_to: oneshot::Sender<PersistenceResult<i64>>,
    },
    LoadConversationEvents {
        conversation_id: String,
        respond_to: oneshot::Sender<PersistenceResult<Vec<StoredConversationEvent>>>,
    },
    StoreSecretRef {
        record: SecretRefRecord,
        respond_to: oneshot::Sender<PersistenceResult<()>>,
    },
    StoreSecret {
        record: SecretRefRecord,
        secret: String,
        respond_to: oneshot::Sender<PersistenceResult<()>>,
    },
    GetSecret {
        id: SecretRef,
        respond_to: oneshot::Sender<PersistenceResult<Option<String>>>,
    },
    ListSecretRefs {
        respond_to: oneshot::Sender<PersistenceResult<Vec<SecretRefRecord>>>,
    },
    StoreSettingsDocument {
        document: SettingsDocument,
        respond_to: oneshot::Sender<PersistenceResult<()>>,
    },
    LoadSettingsDocument {
        scope: ConfigScope,
        key: String,
        respond_to: oneshot::Sender<PersistenceResult<Option<SettingsDocument>>>,
    },
    UpsertLlmProfile {
        record: LlmProfileRecord,
        respond_to: oneshot::Sender<PersistenceResult<()>>,
    },
    ListLlmProfiles {
        respond_to: oneshot::Sender<PersistenceResult<Vec<LlmProfileRecord>>>,
    },
    DeleteLlmProfile {
        id: String,
        respond_to: oneshot::Sender<PersistenceResult<bool>>,
    },
    UpsertProjectFolder {
        record: ProjectFolderRecord,
        respond_to: oneshot::Sender<PersistenceResult<()>>,
    },
    ListProjectFolders {
        respond_to: oneshot::Sender<PersistenceResult<Vec<ProjectFolderRecord>>>,
    },
    DeleteProjectFolder {
        id: String,
        respond_to: oneshot::Sender<PersistenceResult<bool>>,
    },
    UpsertGoal {
        record: GoalRecord,
        respond_to: oneshot::Sender<PersistenceResult<()>>,
    },
    ListGoals {
        respond_to: oneshot::Sender<PersistenceResult<Vec<GoalRecord>>>,
    },
    DeleteGoal {
        conversation_id: String,
        respond_to: oneshot::Sender<PersistenceResult<bool>>,
    },
    UpsertEditorTab {
        record: EditorTabRecord,
        respond_to: oneshot::Sender<PersistenceResult<()>>,
    },
    ListEditorTabs {
        respond_to: oneshot::Sender<PersistenceResult<Vec<EditorTabRecord>>>,
    },
    DeleteEditorTab {
        path: String,
        respond_to: oneshot::Sender<PersistenceResult<bool>>,
    },
    UpsertMcpStatus {
        snapshot: McpStatusSnapshot,
        respond_to: oneshot::Sender<PersistenceResult<()>>,
    },
    ListMcpStatuses {
        respond_to: oneshot::Sender<PersistenceResult<Vec<McpStatusSnapshot>>>,
    },
    DeleteMcpStatus {
        server_id: String,
        respond_to: oneshot::Sender<PersistenceResult<bool>>,
    },
    UpsertCacheMetadata {
        record: CacheMetadataRecord,
        respond_to: oneshot::Sender<PersistenceResult<()>>,
    },
    ListCacheMetadata {
        respond_to: oneshot::Sender<PersistenceResult<Vec<CacheMetadataRecord>>>,
    },
    UpsertMemoryRecord {
        record: MemoryRecord,
        respond_to: oneshot::Sender<PersistenceResult<()>>,
    },
    ListMemoryRecords {
        respond_to: oneshot::Sender<PersistenceResult<Vec<MemoryRecord>>>,
    },
    SetMemoryEnabled {
        id: String,
        enabled: bool,
        updated_at_ms: i64,
        respond_to: oneshot::Sender<PersistenceResult<bool>>,
    },
    DeleteMemoryRecord {
        id: String,
        respond_to: oneshot::Sender<PersistenceResult<bool>>,
    },
    ExportRedactedSnapshot {
        respond_to: oneshot::Sender<PersistenceResult<PersistenceExportSnapshot>>,
    },
    ImportRedactedSnapshot {
        snapshot: PersistenceExportSnapshot,
        respond_to: oneshot::Sender<PersistenceResult<()>>,
    },
    Shutdown {
        respond_to: oneshot::Sender<()>,
    },
}

struct PersistenceConnection {
    connection: Connection,
    secret_store: Box<dyn SecretStore>,
}

impl PersistenceConnection {
    fn open(
        config: PersistenceConfig,
        secret_store: Box<dyn SecretStore>,
    ) -> PersistenceResult<Self> {
        if let Some(parent) = config
            .database_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| PersistenceError::Startup(error.to_string()))?;
        }

        let mut connection = Connection::open(&config.database_path)?;
        configure_sqlite(&connection)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection,
            secret_store,
        })
    }

    fn run(&mut self, command_rx: std_mpsc::Receiver<PersistenceCommand>) {
        while let Ok(command) = command_rx.recv() {
            match command {
                PersistenceCommand::CurrentSchemaVersion { respond_to } => {
                    let _ = respond_to.send(self.current_schema_version());
                }
                PersistenceCommand::JournalMode { respond_to } => {
                    let _ = respond_to.send(self.journal_mode());
                }
                PersistenceCommand::IntegrityCheck { respond_to } => {
                    let _ = respond_to.send(self.integrity_check());
                }
                PersistenceCommand::BackupDatabase { path, respond_to } => {
                    let _ = respond_to.send(self.backup_database(path));
                }
                PersistenceCommand::RepairDatabase { respond_to } => {
                    let _ = respond_to.send(self.repair_database());
                }
                PersistenceCommand::UpsertConversation {
                    metadata,
                    respond_to,
                } => {
                    let _ = respond_to.send(self.upsert_conversation(metadata));
                }
                PersistenceCommand::DeleteConversation { id, respond_to } => {
                    let _ = respond_to.send(self.delete_conversation(&id));
                }
                PersistenceCommand::ListConversations { respond_to } => {
                    let _ = respond_to.send(self.list_conversations());
                }
                PersistenceCommand::AppendConversationEvent { event, respond_to } => {
                    let _ = respond_to.send(self.append_conversation_event(event));
                }
                PersistenceCommand::LoadConversationEvents {
                    conversation_id,
                    respond_to,
                } => {
                    let _ = respond_to.send(self.load_conversation_events(&conversation_id));
                }
                PersistenceCommand::StoreSecretRef { record, respond_to } => {
                    let _ = respond_to.send(self.store_secret_ref(record));
                }
                PersistenceCommand::StoreSecret {
                    record,
                    secret,
                    respond_to,
                } => {
                    let _ = respond_to.send(self.store_secret(record, &secret));
                }
                PersistenceCommand::GetSecret { id, respond_to } => {
                    let _ = respond_to.send(self.get_secret(&id));
                }
                PersistenceCommand::ListSecretRefs { respond_to } => {
                    let _ = respond_to.send(self.list_secret_refs());
                }
                PersistenceCommand::StoreSettingsDocument {
                    document,
                    respond_to,
                } => {
                    let _ = respond_to.send(self.store_settings_document(document));
                }
                PersistenceCommand::LoadSettingsDocument {
                    scope,
                    key,
                    respond_to,
                } => {
                    let _ = respond_to.send(self.load_settings_document(scope, &key));
                }
                PersistenceCommand::UpsertLlmProfile { record, respond_to } => {
                    let _ = respond_to.send(self.upsert_llm_profile(record));
                }
                PersistenceCommand::ListLlmProfiles { respond_to } => {
                    let _ = respond_to.send(self.list_llm_profiles());
                }
                PersistenceCommand::DeleteLlmProfile { id, respond_to } => {
                    let _ = respond_to.send(self.delete_llm_profile(&id));
                }
                PersistenceCommand::UpsertProjectFolder { record, respond_to } => {
                    let _ = respond_to.send(self.upsert_project_folder(record));
                }
                PersistenceCommand::ListProjectFolders { respond_to } => {
                    let _ = respond_to.send(self.list_project_folders());
                }
                PersistenceCommand::DeleteProjectFolder { id, respond_to } => {
                    let _ = respond_to.send(self.delete_project_folder(&id));
                }
                PersistenceCommand::UpsertGoal { record, respond_to } => {
                    let _ = respond_to.send(self.upsert_goal(record));
                }
                PersistenceCommand::ListGoals { respond_to } => {
                    let _ = respond_to.send(self.list_goals());
                }
                PersistenceCommand::DeleteGoal {
                    conversation_id,
                    respond_to,
                } => {
                    let _ = respond_to.send(self.delete_goal(&conversation_id));
                }
                PersistenceCommand::UpsertEditorTab { record, respond_to } => {
                    let _ = respond_to.send(self.upsert_editor_tab(record));
                }
                PersistenceCommand::ListEditorTabs { respond_to } => {
                    let _ = respond_to.send(self.list_editor_tabs());
                }
                PersistenceCommand::DeleteEditorTab { path, respond_to } => {
                    let _ = respond_to.send(self.delete_editor_tab(&path));
                }
                PersistenceCommand::UpsertMcpStatus {
                    snapshot,
                    respond_to,
                } => {
                    let _ = respond_to.send(self.upsert_mcp_status(snapshot));
                }
                PersistenceCommand::ListMcpStatuses { respond_to } => {
                    let _ = respond_to.send(self.list_mcp_statuses());
                }
                PersistenceCommand::DeleteMcpStatus {
                    server_id,
                    respond_to,
                } => {
                    let _ = respond_to.send(self.delete_mcp_status(&server_id));
                }
                PersistenceCommand::UpsertCacheMetadata { record, respond_to } => {
                    let _ = respond_to.send(self.upsert_cache_metadata(record));
                }
                PersistenceCommand::ListCacheMetadata { respond_to } => {
                    let _ = respond_to.send(self.list_cache_metadata());
                }
                PersistenceCommand::UpsertMemoryRecord { record, respond_to } => {
                    let _ = respond_to.send(self.upsert_memory_record(record));
                }
                PersistenceCommand::ListMemoryRecords { respond_to } => {
                    let _ = respond_to.send(self.list_memory_records());
                }
                PersistenceCommand::SetMemoryEnabled {
                    id,
                    enabled,
                    updated_at_ms,
                    respond_to,
                } => {
                    let _ = respond_to.send(self.set_memory_enabled(&id, enabled, updated_at_ms));
                }
                PersistenceCommand::DeleteMemoryRecord { id, respond_to } => {
                    let _ = respond_to.send(self.delete_memory_record(&id));
                }
                PersistenceCommand::ExportRedactedSnapshot { respond_to } => {
                    let _ = respond_to.send(self.export_redacted_snapshot());
                }
                PersistenceCommand::ImportRedactedSnapshot {
                    snapshot,
                    respond_to,
                } => {
                    let _ = respond_to.send(self.import_redacted_snapshot(snapshot));
                }
                PersistenceCommand::Shutdown { respond_to } => {
                    let _ = respond_to.send(());
                    break;
                }
            }
        }
    }

    fn current_schema_version(&self) -> PersistenceResult<i64> {
        current_schema_version(&self.connection)
    }

    fn journal_mode(&self) -> PersistenceResult<String> {
        self.connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(PersistenceError::from)
    }

    fn integrity_check(&self) -> PersistenceResult<PersistenceIntegrityReport> {
        integrity_check_connection(&self.connection)
    }

    fn backup_database(&self, path: PathBuf) -> PersistenceResult<PersistenceBackupReport> {
        if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
            fs::create_dir_all(parent)
                .map_err(|error| PersistenceError::Startup(error.to_string()))?;
        }
        let file_name = path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .unwrap_or("fastrock.db");
        let tmp_path = path.with_file_name(format!(".{file_name}.tmp-{}", now_ms()));
        if tmp_path.exists() {
            fs::remove_file(&tmp_path)
                .map_err(|error| PersistenceError::Startup(error.to_string()))?;
        }

        let tmp_path_sql = tmp_path.to_string_lossy().into_owned();
        self.connection
            .execute("VACUUM main INTO ?1", params![tmp_path_sql])?;

        let integrity = integrity_check_database_file(&tmp_path)?;
        if !integrity.ok {
            let _ = fs::remove_file(&tmp_path);
            return Err(PersistenceError::Startup(format!(
                "backup integrity check failed: {}",
                integrity.messages.join("; ")
            )));
        }

        match fs::rename(&tmp_path, &path) {
            Ok(()) => {}
            Err(_) if path.exists() => {
                fs::remove_file(&path)
                    .map_err(|error| PersistenceError::Startup(error.to_string()))?;
                fs::rename(&tmp_path, &path)
                    .map_err(|error| PersistenceError::Startup(error.to_string()))?;
            }
            Err(error) => {
                let _ = fs::remove_file(&tmp_path);
                return Err(PersistenceError::Startup(error.to_string()));
            }
        }

        let bytes = fs::metadata(&path)
            .map_err(|error| PersistenceError::Startup(error.to_string()))?
            .len();
        Ok(PersistenceBackupReport {
            path,
            bytes,
            created_at_ms: now_ms(),
            integrity,
        })
    }

    fn repair_database(&self) -> PersistenceResult<PersistenceRepairReport> {
        let before = self.integrity_check()?;
        self.connection.execute_batch("REINDEX; VACUUM;")?;
        let after = self.integrity_check()?;
        Ok(PersistenceRepairReport {
            before,
            after,
            reindexed: true,
            vacuumed: true,
        })
    }

    fn upsert_conversation(&self, metadata: ConversationMetadata) -> PersistenceResult<()> {
        self.connection.execute(
            "INSERT INTO conversations (
                id, title, project_folder_id, status, created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                project_folder_id = excluded.project_folder_id,
                status = excluded.status,
                updated_at_ms = excluded.updated_at_ms",
            params![
                metadata.id,
                metadata.title,
                metadata.project_folder_id,
                metadata.status,
                metadata.created_at_ms,
                metadata.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    fn list_conversations(&self) -> PersistenceResult<Vec<ConversationMetadata>> {
        let mut statement = self.connection.prepare(
            "SELECT id, title, project_folder_id, status, created_at_ms, updated_at_ms
            FROM conversations
            ORDER BY updated_at_ms DESC, id ASC",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(ConversationMetadata {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    project_folder_id: row.get(2)?,
                    status: row.get(3)?,
                    created_at_ms: row.get(4)?,
                    updated_at_ms: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn delete_conversation(&self, id: &str) -> PersistenceResult<bool> {
        let editor_count = self.connection.execute(
            "DELETE FROM editor_tabs WHERE conversation_id = ?1",
            params![id],
        )?;
        let goal_count = self
            .connection
            .execute("DELETE FROM goals WHERE conversation_id = ?1", params![id])?;
        let event_count = self.connection.execute(
            "DELETE FROM conversation_events WHERE conversation_id = ?1",
            params![id],
        )?;
        let conversation_count = self
            .connection
            .execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
        Ok(editor_count > 0 || goal_count > 0 || event_count > 0 || conversation_count > 0)
    }

    fn append_conversation_event(&self, event: ConversationEvent) -> PersistenceResult<i64> {
        self.connection.execute(
            "INSERT INTO conversation_events (
                conversation_id, event_type, payload_json, created_at_ms
            ) VALUES (?1, ?2, ?3, ?4)",
            params![
                event.conversation_id,
                event.event_type,
                event.payload_json,
                event.created_at_ms,
            ],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    fn load_conversation_events(
        &self,
        conversation_id: &str,
    ) -> PersistenceResult<Vec<StoredConversationEvent>> {
        let mut statement = self.connection.prepare(
            "SELECT id, conversation_id, event_type, payload_json, created_at_ms
            FROM conversation_events
            WHERE conversation_id = ?1
            ORDER BY id ASC",
        )?;
        let events = statement
            .query_map(params![conversation_id], |row| {
                Ok(StoredConversationEvent {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    event_type: row.get(2)?,
                    payload_json: row.get(3)?,
                    created_at_ms: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(events)
    }

    fn list_conversation_events(&self) -> PersistenceResult<Vec<StoredConversationEvent>> {
        let mut statement = self.connection.prepare(
            "SELECT id, conversation_id, event_type, payload_json, created_at_ms
            FROM conversation_events
            ORDER BY conversation_id ASC, id ASC",
        )?;
        let events = statement
            .query_map([], |row| {
                Ok(StoredConversationEvent {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    event_type: row.get(2)?,
                    payload_json: row.get(3)?,
                    created_at_ms: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(events)
    }

    fn store_secret_ref(&self, record: SecretRefRecord) -> PersistenceResult<()> {
        self.connection.execute(
            "INSERT INTO secret_refs (id, service, account, created_at_ms)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(id) DO UPDATE SET
                service = excluded.service,
                account = excluded.account",
            params![
                record.id.0,
                record.service,
                record.account,
                record.created_at_ms,
            ],
        )?;
        Ok(())
    }

    fn store_secret(&self, record: SecretRefRecord, secret: &str) -> PersistenceResult<()> {
        self.secret_store.put_secret(&record, secret)?;
        self.store_secret_ref(record)
    }

    fn get_secret(&self, id: &SecretRef) -> PersistenceResult<Option<String>> {
        let Some(record) = self.get_secret_ref(id)? else {
            return Ok(None);
        };
        Ok(Some(self.secret_store.get_secret(&record)?))
    }

    fn get_secret_ref(&self, id: &SecretRef) -> PersistenceResult<Option<SecretRefRecord>> {
        self.connection
            .query_row(
                "SELECT id, service, account, created_at_ms
                FROM secret_refs
                WHERE id = ?1",
                params![&id.0],
                |row| {
                    Ok(SecretRefRecord {
                        id: SecretRef(row.get(0)?),
                        service: row.get(1)?,
                        account: row.get(2)?,
                        created_at_ms: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(PersistenceError::Sqlite)
    }

    fn list_secret_refs(&self) -> PersistenceResult<Vec<SecretRefRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, service, account, created_at_ms
            FROM secret_refs
            ORDER BY id ASC",
        )?;
        let refs = statement
            .query_map([], |row| {
                Ok(SecretRefRecord {
                    id: SecretRef(row.get(0)?),
                    service: row.get(1)?,
                    account: row.get(2)?,
                    created_at_ms: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(refs)
    }

    fn store_settings_document(&self, document: SettingsDocument) -> PersistenceResult<()> {
        self.connection.execute(
            "INSERT INTO settings_documents (scope, key, document_json, updated_at_ms)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(scope, key) DO UPDATE SET
                document_json = excluded.document_json,
                updated_at_ms = excluded.updated_at_ms",
            params![
                document.scope.storage_key(),
                document.key,
                document.document_json,
                document.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    fn load_settings_document(
        &self,
        scope: ConfigScope,
        key: &str,
    ) -> PersistenceResult<Option<SettingsDocument>> {
        let scope_key = scope.storage_key();
        let mut statement = self.connection.prepare(
            "SELECT document_json, updated_at_ms
            FROM settings_documents
            WHERE scope = ?1 AND key = ?2",
        )?;
        let mut rows = statement.query(params![scope_key, key])?;

        if let Some(row) = rows.next()? {
            Ok(Some(SettingsDocument {
                scope,
                key: key.to_owned(),
                document_json: row.get(0)?,
                updated_at_ms: row.get(1)?,
            }))
        } else {
            Ok(None)
        }
    }

    fn list_settings_documents(&self) -> PersistenceResult<Vec<SettingsDocument>> {
        let mut statement = self.connection.prepare(
            "SELECT scope, key, document_json, updated_at_ms
            FROM settings_documents
            ORDER BY scope ASC, key ASC",
        )?;
        Ok(statement
            .query_map([], |row| {
                let scope_key: String = row.get(0)?;
                Ok(SettingsDocument {
                    scope: config_scope_from_storage_key(&scope_key),
                    key: row.get(1)?,
                    document_json: row.get(2)?,
                    updated_at_ms: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn upsert_llm_profile(&self, record: LlmProfileRecord) -> PersistenceResult<()> {
        self.connection.execute(
            "INSERT INTO llm_profiles (id, provider, enabled, document_json, updated_at_ms)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(id) DO UPDATE SET
                provider = excluded.provider,
                enabled = excluded.enabled,
                document_json = excluded.document_json,
                updated_at_ms = excluded.updated_at_ms",
            params![
                record.id,
                record.provider,
                bool_to_i64(record.enabled),
                record.document_json,
                record.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    fn list_llm_profiles(&self) -> PersistenceResult<Vec<LlmProfileRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, provider, enabled, document_json, updated_at_ms
            FROM llm_profiles
            ORDER BY id ASC",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(LlmProfileRecord {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    enabled: row.get::<_, i64>(2)? != 0,
                    document_json: row.get(3)?,
                    updated_at_ms: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn delete_llm_profile(&self, id: &str) -> PersistenceResult<bool> {
        let changed = self
            .connection
            .execute("DELETE FROM llm_profiles WHERE id = ?1", params![id])?;
        Ok(changed > 0)
    }

    fn upsert_project_folder(&self, record: ProjectFolderRecord) -> PersistenceResult<()> {
        self.connection.execute(
            "INSERT INTO project_folders (
                id, label, target_kind, path, remote_target_id, document_json, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(id) DO UPDATE SET
                label = excluded.label,
                target_kind = excluded.target_kind,
                path = excluded.path,
                remote_target_id = excluded.remote_target_id,
                document_json = excluded.document_json,
                updated_at_ms = excluded.updated_at_ms",
            params![
                record.id,
                record.label,
                record.target_kind,
                record.path,
                record.remote_target_id,
                record.document_json,
                record.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    fn list_project_folders(&self) -> PersistenceResult<Vec<ProjectFolderRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, label, target_kind, path, remote_target_id, document_json, updated_at_ms
            FROM project_folders
            ORDER BY id ASC",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(ProjectFolderRecord {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    target_kind: row.get(2)?,
                    path: row.get(3)?,
                    remote_target_id: row.get(4)?,
                    document_json: row.get(5)?,
                    updated_at_ms: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn delete_project_folder(&self, id: &str) -> PersistenceResult<bool> {
        let changed = self
            .connection
            .execute("DELETE FROM project_folders WHERE id = ?1", params![id])?;
        Ok(changed > 0)
    }

    fn upsert_goal(&self, record: GoalRecord) -> PersistenceResult<()> {
        self.connection.execute(
            "INSERT INTO goals (conversation_id, status, document_json, updated_at_ms)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(conversation_id) DO UPDATE SET
                status = excluded.status,
                document_json = excluded.document_json,
                updated_at_ms = excluded.updated_at_ms",
            params![
                record.conversation_id,
                record.status,
                record.document_json,
                record.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    fn list_goals(&self) -> PersistenceResult<Vec<GoalRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT conversation_id, status, document_json, updated_at_ms
            FROM goals
            ORDER BY conversation_id ASC",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(GoalRecord {
                    conversation_id: row.get(0)?,
                    status: row.get(1)?,
                    document_json: row.get(2)?,
                    updated_at_ms: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn delete_goal(&self, conversation_id: &str) -> PersistenceResult<bool> {
        let changed = self.connection.execute(
            "DELETE FROM goals WHERE conversation_id = ?1",
            params![conversation_id],
        )?;
        Ok(changed > 0)
    }

    fn upsert_editor_tab(&self, record: EditorTabRecord) -> PersistenceResult<()> {
        self.connection.execute(
            "INSERT INTO editor_tabs (
                path, conversation_id, active, document_json, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(path) DO UPDATE SET
                conversation_id = excluded.conversation_id,
                active = excluded.active,
                document_json = excluded.document_json,
                updated_at_ms = excluded.updated_at_ms",
            params![
                record.path,
                record.conversation_id,
                bool_to_i64(record.active),
                record.document_json,
                record.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    fn list_editor_tabs(&self) -> PersistenceResult<Vec<EditorTabRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT path, conversation_id, active, document_json, updated_at_ms
            FROM editor_tabs
            ORDER BY path ASC",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(EditorTabRecord {
                    path: row.get(0)?,
                    conversation_id: row.get(1)?,
                    active: row.get::<_, i64>(2)? != 0,
                    document_json: row.get(3)?,
                    updated_at_ms: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn delete_editor_tab(&self, path: &str) -> PersistenceResult<bool> {
        let changed = self
            .connection
            .execute("DELETE FROM editor_tabs WHERE path = ?1", params![path])?;
        Ok(changed > 0)
    }

    fn upsert_mcp_status(&self, snapshot: McpStatusSnapshot) -> PersistenceResult<()> {
        self.connection.execute(
            "INSERT INTO mcp_status_snapshots (server_id, status, document_json, updated_at_ms)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(server_id) DO UPDATE SET
                status = excluded.status,
                document_json = excluded.document_json,
                updated_at_ms = excluded.updated_at_ms",
            params![
                snapshot.server_id,
                snapshot.status,
                snapshot.document_json,
                snapshot.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    fn list_mcp_statuses(&self) -> PersistenceResult<Vec<McpStatusSnapshot>> {
        let mut statement = self.connection.prepare(
            "SELECT server_id, status, document_json, updated_at_ms
            FROM mcp_status_snapshots
            ORDER BY server_id ASC",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(McpStatusSnapshot {
                    server_id: row.get(0)?,
                    status: row.get(1)?,
                    document_json: row.get(2)?,
                    updated_at_ms: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn delete_mcp_status(&self, server_id: &str) -> PersistenceResult<bool> {
        let changed = self.connection.execute(
            "DELETE FROM mcp_status_snapshots WHERE server_id = ?1",
            params![server_id],
        )?;
        Ok(changed > 0)
    }

    fn upsert_cache_metadata(&self, record: CacheMetadataRecord) -> PersistenceResult<()> {
        self.connection.execute(
            "INSERT INTO cache_metadata (cache_key, provider_plane, document_json, updated_at_ms)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(cache_key) DO UPDATE SET
                provider_plane = excluded.provider_plane,
                document_json = excluded.document_json,
                updated_at_ms = excluded.updated_at_ms",
            params![
                record.cache_key,
                record.provider_plane,
                record.document_json,
                record.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    fn list_cache_metadata(&self) -> PersistenceResult<Vec<CacheMetadataRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT cache_key, provider_plane, document_json, updated_at_ms
            FROM cache_metadata
            ORDER BY cache_key ASC",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(CacheMetadataRecord {
                    cache_key: row.get(0)?,
                    provider_plane: row.get(1)?,
                    document_json: row.get(2)?,
                    updated_at_ms: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn upsert_memory_record(&self, record: MemoryRecord) -> PersistenceResult<()> {
        let (scope_kind, project_folder_id) = memory_scope_columns(&record.scope);
        self.connection.execute(
            "INSERT INTO memory_records (
                id, kind, scope_kind, project_folder_id, source, text,
                enabled, created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(id) DO UPDATE SET
                kind = excluded.kind,
                scope_kind = excluded.scope_kind,
                project_folder_id = excluded.project_folder_id,
                source = excluded.source,
                text = excluded.text,
                enabled = excluded.enabled,
                updated_at_ms = excluded.updated_at_ms",
            params![
                record.id,
                record.kind.label(),
                scope_kind,
                project_folder_id,
                record.source,
                record.text,
                bool_to_i64(record.enabled),
                record.created_at_ms,
                record.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    fn list_memory_records(&self) -> PersistenceResult<Vec<MemoryRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, kind, scope_kind, project_folder_id, source, text,
                enabled, created_at_ms, updated_at_ms
            FROM memory_records
            ORDER BY updated_at_ms DESC, id ASC",
        )?;
        Ok(statement
            .query_map([], memory_record_from_row)?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn set_memory_enabled(
        &self,
        id: &str,
        enabled: bool,
        updated_at_ms: i64,
    ) -> PersistenceResult<bool> {
        let changed = self.connection.execute(
            "UPDATE memory_records
            SET enabled = ?2, updated_at_ms = ?3
            WHERE id = ?1",
            params![id, bool_to_i64(enabled), updated_at_ms],
        )?;
        Ok(changed > 0)
    }

    fn delete_memory_record(&self, id: &str) -> PersistenceResult<bool> {
        let changed = self
            .connection
            .execute("DELETE FROM memory_records WHERE id = ?1", params![id])?;
        Ok(changed > 0)
    }

    fn import_redacted_snapshot(
        &mut self,
        snapshot: PersistenceExportSnapshot,
    ) -> PersistenceResult<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute_batch(
            "
            DELETE FROM memory_records;
            DELETE FROM cache_metadata;
            DELETE FROM mcp_status_snapshots;
            DELETE FROM editor_tabs;
            DELETE FROM goals;
            DELETE FROM project_folders;
            DELETE FROM llm_profiles;
            DELETE FROM secret_refs;
            DELETE FROM settings_documents;
            DELETE FROM conversation_events;
            DELETE FROM conversations;
            ",
        )?;

        for metadata in snapshot.conversations {
            transaction.execute(
                "INSERT INTO conversations (
                    id, title, project_folder_id, status, created_at_ms, updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    metadata.id,
                    metadata.title,
                    metadata.project_folder_id,
                    metadata.status,
                    metadata.created_at_ms,
                    metadata.updated_at_ms,
                ],
            )?;
        }
        for event in snapshot.conversation_events {
            transaction.execute(
                "INSERT INTO conversation_events (
                    id, conversation_id, event_type, payload_json, created_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    event.id,
                    event.conversation_id,
                    event.event_type,
                    event.payload_json,
                    event.created_at_ms,
                ],
            )?;
        }
        for document in snapshot.settings_documents {
            transaction.execute(
                "INSERT INTO settings_documents (scope, key, document_json, updated_at_ms)
                VALUES (?1, ?2, ?3, ?4)",
                params![
                    document.scope.storage_key(),
                    document.key,
                    document.document_json,
                    document.updated_at_ms,
                ],
            )?;
        }
        for record in snapshot.secret_refs {
            transaction.execute(
                "INSERT INTO secret_refs (id, service, account, created_at_ms)
                VALUES (?1, ?2, ?3, ?4)",
                params![
                    record.id.0,
                    record.service,
                    record.account,
                    record.created_at_ms
                ],
            )?;
        }
        for record in snapshot.llm_profiles {
            transaction.execute(
                "INSERT INTO llm_profiles (id, provider, enabled, document_json, updated_at_ms)
                VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    record.id,
                    record.provider,
                    bool_to_i64(record.enabled),
                    record.document_json,
                    record.updated_at_ms,
                ],
            )?;
        }
        for record in snapshot.project_folders {
            transaction.execute(
                "INSERT INTO project_folders (
                    id, label, target_kind, path, remote_target_id, document_json, updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    record.id,
                    record.label,
                    record.target_kind,
                    record.path,
                    record.remote_target_id,
                    record.document_json,
                    record.updated_at_ms,
                ],
            )?;
        }
        for record in snapshot.goals {
            transaction.execute(
                "INSERT INTO goals (conversation_id, status, document_json, updated_at_ms)
                VALUES (?1, ?2, ?3, ?4)",
                params![
                    record.conversation_id,
                    record.status,
                    record.document_json,
                    record.updated_at_ms,
                ],
            )?;
        }
        for record in snapshot.editor_tabs {
            transaction.execute(
                "INSERT INTO editor_tabs (
                    path, conversation_id, active, document_json, updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    record.path,
                    record.conversation_id,
                    bool_to_i64(record.active),
                    record.document_json,
                    record.updated_at_ms,
                ],
            )?;
        }
        for snapshot in snapshot.mcp_status_snapshots {
            transaction.execute(
                "INSERT INTO mcp_status_snapshots (
                    server_id, status, document_json, updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    snapshot.server_id,
                    snapshot.status,
                    snapshot.document_json,
                    snapshot.updated_at_ms,
                ],
            )?;
        }
        for record in snapshot.cache_metadata {
            transaction.execute(
                "INSERT INTO cache_metadata (
                    cache_key, provider_plane, document_json, updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    record.cache_key,
                    record.provider_plane,
                    record.document_json,
                    record.updated_at_ms,
                ],
            )?;
        }
        for record in snapshot.memory_records {
            let (scope_kind, project_folder_id) = memory_scope_columns(&record.scope);
            let scope_kind = scope_kind.to_owned();
            let project_folder_id = project_folder_id.map(str::to_owned);
            transaction.execute(
                "INSERT INTO memory_records (
                    id, kind, scope_kind, project_folder_id, source, text,
                    enabled, created_at_ms, updated_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.id,
                    record.kind.label(),
                    scope_kind,
                    project_folder_id,
                    record.source,
                    record.text,
                    bool_to_i64(record.enabled),
                    record.created_at_ms,
                    record.updated_at_ms,
                ],
            )?;
        }

        transaction.commit()?;
        Ok(())
    }

    fn export_redacted_snapshot(&self) -> PersistenceResult<PersistenceExportSnapshot> {
        Ok(PersistenceExportSnapshot {
            conversations: self.list_conversations()?,
            conversation_events: self.list_conversation_events()?,
            settings_documents: self.list_settings_documents()?,
            secret_refs: self.list_secret_refs()?,
            llm_profiles: self.list_llm_profiles()?,
            project_folders: self.list_project_folders()?,
            goals: self.list_goals()?,
            editor_tabs: self.list_editor_tabs()?,
            mcp_status_snapshots: self.list_mcp_statuses()?,
            cache_metadata: self.list_cache_metadata()?,
            memory_records: self.list_memory_records()?,
        })
    }
}

fn configure_sqlite(connection: &Connection) -> PersistenceResult<()> {
    connection.pragma_update(None, "foreign_keys", "ON")?;
    let _: String = connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

fn integrity_check_database_file(path: &Path) -> PersistenceResult<PersistenceIntegrityReport> {
    let connection = Connection::open(path)?;
    integrity_check_connection(&connection)
}

fn integrity_check_connection(
    connection: &Connection,
) -> PersistenceResult<PersistenceIntegrityReport> {
    let mut statement = connection.prepare("PRAGMA integrity_check")?;
    let messages = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PersistenceIntegrityReport::from_messages(messages))
}

fn bool_to_i64(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn migrate(connection: &mut Connection) -> PersistenceResult<()> {
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at_ms INTEGER NOT NULL
        );
        ",
    )?;

    if current_schema_version(connection)? < 1 {
        let applied_at_ms = now_ms();
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            "
            CREATE TABLE conversations (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                project_folder_id TEXT,
                status TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE TABLE conversation_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
            );

            CREATE INDEX conversation_events_conversation_id_idx
                ON conversation_events(conversation_id, id);

            CREATE TABLE settings_documents (
                scope TEXT NOT NULL,
                key TEXT NOT NULL,
                document_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                PRIMARY KEY(scope, key)
            );

            CREATE TABLE secret_refs (
                id TEXT PRIMARY KEY,
                service TEXT NOT NULL,
                account TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );
            ",
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (1, ?1)",
            params![applied_at_ms],
        )?;
        transaction.commit()?;
    }

    if current_schema_version(connection)? < 2 {
        let applied_at_ms = now_ms();
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS llm_profiles (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                document_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS llm_profiles_provider_idx
                ON llm_profiles(provider);

            CREATE TABLE IF NOT EXISTS project_folders (
                id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                target_kind TEXT NOT NULL,
                path TEXT NOT NULL,
                remote_target_id TEXT,
                document_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS project_folders_target_idx
                ON project_folders(target_kind, remote_target_id);

            CREATE TABLE IF NOT EXISTS goals (
                conversation_id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                document_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS goals_status_idx
                ON goals(status);

            CREATE TABLE IF NOT EXISTS editor_tabs (
                path TEXT PRIMARY KEY,
                conversation_id TEXT,
                active INTEGER NOT NULL,
                document_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS editor_tabs_conversation_idx
                ON editor_tabs(conversation_id, active);

            CREATE TABLE IF NOT EXISTS mcp_status_snapshots (
                server_id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                document_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS mcp_status_snapshots_status_idx
                ON mcp_status_snapshots(status);

            CREATE TABLE IF NOT EXISTS cache_metadata (
                cache_key TEXT PRIMARY KEY,
                provider_plane TEXT NOT NULL,
                document_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS cache_metadata_provider_idx
                ON cache_metadata(provider_plane);
            ",
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (2, ?1)",
            params![applied_at_ms],
        )?;
        transaction.commit()?;
    }

    if current_schema_version(connection)? < 3 {
        let applied_at_ms = now_ms();
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS memory_records (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                scope_kind TEXT NOT NULL,
                project_folder_id TEXT,
                source TEXT NOT NULL,
                text TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS memory_records_scope_idx
                ON memory_records(scope_kind, project_folder_id, enabled);

            CREATE INDEX IF NOT EXISTS memory_records_kind_idx
                ON memory_records(kind, enabled);
            ",
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (3, ?1)",
            params![applied_at_ms],
        )?;
        transaction.commit()?;
    }

    Ok(())
}

fn current_schema_version(connection: &Connection) -> PersistenceResult<i64> {
    connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .map_err(PersistenceError::from)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn memory_scope_columns(scope: &MemoryScope) -> (&'static str, Option<&str>) {
    match scope {
        MemoryScope::Global => ("global", None),
        MemoryScope::Project { project_folder_id } => ("project", Some(project_folder_id)),
    }
}

fn memory_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRecord> {
    let kind: String = row.get(1)?;
    let scope_kind: String = row.get(2)?;
    let project_folder_id: Option<String> = row.get(3)?;
    Ok(MemoryRecord {
        id: row.get(0)?,
        kind: memory_kind_from_label(&kind).unwrap_or(MemoryKind::Observation),
        scope: memory_scope_from_columns(&scope_kind, project_folder_id),
        source: row.get(4)?,
        text: row.get(5)?,
        enabled: row.get::<_, i64>(6)? != 0,
        created_at_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
    })
}

fn memory_kind_from_label(value: &str) -> Option<MemoryKind> {
    match value {
        "observation" => Some(MemoryKind::Observation),
        "project_fact" => Some(MemoryKind::ProjectFact),
        "user_preference" => Some(MemoryKind::UserPreference),
        "prior_fix" => Some(MemoryKind::PriorFix),
        _ => None,
    }
}

fn memory_scope_from_columns(scope_kind: &str, project_folder_id: Option<String>) -> MemoryScope {
    match (scope_kind, project_folder_id) {
        ("project", Some(project_folder_id)) => MemoryScope::Project { project_folder_id },
        _ => MemoryScope::Global,
    }
}

fn config_scope_from_storage_key(value: &str) -> ConfigScope {
    if let Some(project_id) = value.strip_prefix("project:") {
        ConfigScope::Project(project_id.to_owned())
    } else {
        ConfigScope::Global
    }
}

pub fn sqlite_file_contains(path: &Path, needle: &str) -> PersistenceResult<bool> {
    let bytes =
        std::fs::read(path).map_err(|error| PersistenceError::Startup(error.to_string()))?;
    Ok(String::from_utf8_lossy(&bytes).contains(needle))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::*;

    fn test_config() -> (tempfile::TempDir, PersistenceConfig) {
        let temp_dir = tempdir().unwrap();
        let database_path = temp_dir.path().join(DATABASE_FILE_NAME);
        (temp_dir, PersistenceConfig::new(database_path))
    }

    #[test]
    fn t4_default_database_path_uses_os_app_data_locations() {
        assert_eq!(
            default_database_path_for(
                FastrockPlatform::Macos,
                Some(Path::new("/Users/test")),
                None,
                None,
                None,
            )
            .unwrap(),
            PathBuf::from("/Users/test/Library/Application Support/Fastrock/fastrock.db")
        );
        assert_eq!(
            default_database_path_for(
                FastrockPlatform::Windows,
                None,
                Some(Path::new(r"C:\Users\test\AppData\Roaming")),
                Some(Path::new(r"C:\Users\test\AppData\Local")),
                None,
            )
            .unwrap(),
            PathBuf::from(r"C:\Users\test\AppData\Local")
                .join("Fastrock")
                .join("fastrock.db")
        );
        assert_eq!(
            default_database_path_for(
                FastrockPlatform::Linux,
                Some(Path::new("/home/test")),
                None,
                None,
                Some(Path::new("/home/test/.xdg-data")),
            )
            .unwrap(),
            PathBuf::from("/home/test/.xdg-data/fastrock/fastrock.db")
        );
        assert_eq!(
            default_database_path_for(
                FastrockPlatform::Linux,
                Some(Path::new("/home/test")),
                None,
                None,
                None,
            )
            .unwrap(),
            PathBuf::from("/home/test/.local/share/fastrock/fastrock.db")
        );
        assert_eq!(
            default_user_settings_path_for(
                FastrockPlatform::Macos,
                Some(Path::new("/Users/test")),
                None,
                None,
                None,
            )
            .unwrap(),
            PathBuf::from("/Users/test/Library/Application Support/Fastrock/settings.toml")
        );
        assert_eq!(
            project_settings_path("/repo"),
            PathBuf::from("/repo/.fastrock/settings.toml")
        );
    }

    #[test]
    fn t22_settings_files_round_trip_at_global_and_project_paths() {
        let temp_dir = tempdir().unwrap();
        let user_settings = default_user_settings_path_for(
            FastrockPlatform::Linux,
            Some(temp_dir.path()),
            None,
            None,
            None,
        )
        .unwrap();
        let project_settings = project_settings_path(temp_dir.path().join("repo"));

        assert_eq!(read_settings_file(&user_settings).unwrap(), None);
        write_settings_file_atomic(&user_settings, "theme = \"dark\"\n").unwrap();
        write_settings_file_atomic(&project_settings, "default_profile = \"mantle\"\n").unwrap();

        assert_eq!(
            read_settings_file(&user_settings).unwrap().as_deref(),
            Some("theme = \"dark\"\n")
        );
        assert_eq!(
            read_settings_file(&project_settings).unwrap().as_deref(),
            Some("default_profile = \"mantle\"\n")
        );
    }

    #[tokio::test]
    async fn t4_spawn_creates_parent_app_data_directory() {
        let temp_dir = tempdir().unwrap();
        let database_path = temp_dir
            .path()
            .join("Application Support")
            .join("Fastrock")
            .join(DATABASE_FILE_NAME);

        let actor = PersistenceActorHandle::spawn(PersistenceConfig::new(&database_path)).unwrap();

        assert!(database_path.exists());
        actor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn t4_sqlite_actor_applies_migrations_and_wal() {
        let (_temp_dir, config) = test_config();
        let actor = PersistenceActorHandle::spawn(config).unwrap();

        assert_eq!(actor.current_schema_version().await.unwrap(), 3);
        assert_eq!(
            actor.journal_mode().await.unwrap().to_ascii_lowercase(),
            "wal"
        );

        actor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn t4_migrations_are_idempotent_on_reopen() {
        let (_temp_dir, config) = test_config();
        let actor = PersistenceActorHandle::spawn(config.clone()).unwrap();
        actor.shutdown().await.unwrap();

        let actor = PersistenceActorHandle::spawn(config).unwrap();

        assert_eq!(actor.current_schema_version().await.unwrap(), 3);
        actor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn t4_conversation_events_round_trip_in_order() {
        let (_temp_dir, config) = test_config();
        let actor = PersistenceActorHandle::spawn(config).unwrap();
        let now = now_ms();

        actor
            .upsert_conversation(ConversationMetadata {
                id: "conversation-1".to_owned(),
                title: "Runtime".to_owned(),
                project_folder_id: Some("project-1".to_owned()),
                status: "running".to_owned(),
                created_at_ms: now,
                updated_at_ms: now,
            })
            .await
            .unwrap();
        let first_id = actor
            .append_conversation_event(ConversationEvent {
                conversation_id: "conversation-1".to_owned(),
                event_type: "user_message".to_owned(),
                payload_json: "{\"text\":\"go\"}".to_owned(),
                created_at_ms: now,
            })
            .await
            .unwrap();
        let second_id = actor
            .append_conversation_event(ConversationEvent {
                conversation_id: "conversation-1".to_owned(),
                event_type: "assistant_message".to_owned(),
                payload_json: "{\"text\":\"done\"}".to_owned(),
                created_at_ms: now + 1,
            })
            .await
            .unwrap();

        let events = actor
            .load_conversation_events("conversation-1")
            .await
            .unwrap();

        assert_eq!(events.len(), 2);
        assert!(first_id < second_id);
        assert_eq!(events[0].id, first_id);
        assert_eq!(events[1].id, second_id);

        actor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn v9_secret_ref_records_store_references_only() {
        let (_temp_dir, config) = test_config();
        let database_path = config.database_path.clone();
        let actor = PersistenceActorHandle::spawn(config).unwrap();

        actor
            .store_secret_ref(SecretRefRecord {
                id: SecretRef("secret:bedrock:mary".to_owned()),
                service: "aws-bedrock".to_owned(),
                account: "mary@example.test".to_owned(),
                created_at_ms: now_ms(),
            })
            .await
            .unwrap();
        let refs = actor.list_secret_refs().await.unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].id.0, "secret:bedrock:mary");

        actor.shutdown().await.unwrap();

        assert!(
            !sqlite_file_contains(&database_path, "not-a-real-secret-value").unwrap(),
            "database must not contain secret material"
        );
    }

    #[tokio::test]
    async fn v9_actor_secret_values_round_trip_through_secret_store_only() {
        let (_temp_dir, config) = test_config();
        let database_path = config.database_path.clone();
        let actor =
            PersistenceActorHandle::spawn_with_secret_store(config, MemorySecretStore::default())
                .unwrap();
        let record = SecretRefRecord {
            id: SecretRef("secret:bedrock:ci".to_owned()),
            service: "aws-bedrock".to_owned(),
            account: "ci@example.test".to_owned(),
            created_at_ms: now_ms(),
        };

        actor
            .store_secret(record.clone(), "not-a-real-secret-value")
            .await
            .unwrap();

        assert_eq!(
            actor
                .get_secret(record.id.clone())
                .await
                .unwrap()
                .as_deref(),
            Some("not-a-real-secret-value")
        );
        assert_eq!(
            actor.list_secret_refs().await.unwrap(),
            vec![record.clone()]
        );
        assert_eq!(
            actor.client().list_secret_refs().await.unwrap(),
            vec![record]
        );
        assert_eq!(
            actor
                .client()
                .get_secret(SecretRef("missing".to_owned()))
                .await
                .unwrap(),
            None
        );

        actor.shutdown().await.unwrap();
        assert!(
            !sqlite_file_contains(&database_path, "not-a-real-secret-value").unwrap(),
            "database must not contain secret material"
        );
    }

    #[tokio::test]
    async fn t4_integrity_check_and_repair_run_inside_persistence_actor() {
        let (_temp_dir, config) = test_config();
        let actor = PersistenceActorHandle::spawn(config).unwrap();

        let initial = actor.client().integrity_check().await.unwrap();
        assert!(initial.ok);
        assert_eq!(initial.messages, vec!["ok"]);

        let repair = actor.repair_database().await.unwrap();
        assert!(repair.before.ok);
        assert!(repair.after.ok);
        assert!(repair.reindexed);
        assert!(repair.vacuumed);

        actor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn t4_backup_database_creates_integrity_checked_redacted_copy() {
        let (temp_dir, config) = test_config();
        let backup_path = temp_dir.path().join("backups").join("fastrock.backup.db");
        let actor =
            PersistenceActorHandle::spawn_with_secret_store(config, MemorySecretStore::default())
                .unwrap();
        let now = now_ms();
        let secret_ref = SecretRefRecord {
            id: SecretRef("secret:bedrock:backup".to_owned()),
            service: "aws-bedrock".to_owned(),
            account: "backup".to_owned(),
            created_at_ms: now,
        };

        actor
            .upsert_conversation(ConversationMetadata {
                id: "conversation-backup".to_owned(),
                title: "Backup".to_owned(),
                project_folder_id: None,
                status: "idle".to_owned(),
                created_at_ms: now,
                updated_at_ms: now,
            })
            .await
            .unwrap();
        actor
            .store_secret(secret_ref.clone(), "not-a-real-secret-value")
            .await
            .unwrap();

        let report = actor.backup_database(&backup_path).await.unwrap();

        assert_eq!(report.path, backup_path);
        assert!(report.bytes > 0);
        assert!(report.integrity.ok);
        assert!(
            !sqlite_file_contains(&report.path, "not-a-real-secret-value").unwrap(),
            "backup database must not contain keyring secret material"
        );

        actor.shutdown().await.unwrap();

        let backup_actor = PersistenceActorHandle::spawn_with_secret_store(
            PersistenceConfig::new(report.path),
            MemorySecretStore::default(),
        )
        .unwrap();
        assert_eq!(
            backup_actor.list_conversations().await.unwrap()[0].id,
            "conversation-backup"
        );
        assert_eq!(
            backup_actor.list_secret_refs().await.unwrap(),
            vec![secret_ref]
        );
        backup_actor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn t5_settings_documents_round_trip_by_scope_and_key() {
        let (_temp_dir, config) = test_config();
        let actor = PersistenceActorHandle::spawn(config).unwrap();
        let document = SettingsDocument {
            scope: ConfigScope::Project("project-1".to_owned()),
            key: "llm-profiles".to_owned(),
            document_json: "{\"profiles\":[]}".to_owned(),
            updated_at_ms: now_ms(),
        };

        actor
            .store_settings_document(document.clone())
            .await
            .unwrap();

        assert_eq!(
            actor
                .load_settings_document(
                    ConfigScope::Project("project-1".to_owned()),
                    "llm-profiles"
                )
                .await
                .unwrap(),
            Some(document)
        );
        assert_eq!(
            actor
                .load_settings_document(ConfigScope::Global, "llm-profiles")
                .await
                .unwrap(),
            None
        );

        actor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn t22_materialized_state_tables_round_trip() {
        let (_temp_dir, config) = test_config();
        let actor = PersistenceActorHandle::spawn(config).unwrap();
        let now = now_ms();

        actor
            .upsert_llm_profile(LlmProfileRecord {
                id: "profile-mantle".to_owned(),
                provider: "bedrock_mantle".to_owned(),
                enabled: true,
                document_json: "{\"model\":\"anthropic.test\"}".to_owned(),
                updated_at_ms: now,
            })
            .await
            .unwrap();
        actor
            .upsert_project_folder(ProjectFolderRecord {
                id: "folder-local".to_owned(),
                label: "Fastrock".to_owned(),
                target_kind: "local".to_owned(),
                path: "/repo".to_owned(),
                remote_target_id: None,
                document_json: "{\"watch\":true}".to_owned(),
                updated_at_ms: now + 1,
            })
            .await
            .unwrap();
        actor
            .upsert_goal(GoalRecord {
                conversation_id: "conversation-1".to_owned(),
                status: "active".to_owned(),
                document_json: "{\"objective\":\"ship\"}".to_owned(),
                updated_at_ms: now + 2,
            })
            .await
            .unwrap();
        actor
            .upsert_editor_tab(EditorTabRecord {
                path: "/repo/src/main.rs".to_owned(),
                conversation_id: Some("conversation-1".to_owned()),
                active: true,
                document_json: "{\"dirty\":false}".to_owned(),
                updated_at_ms: now + 3,
            })
            .await
            .unwrap();
        actor
            .upsert_mcp_status(McpStatusSnapshot {
                server_id: "filesystem".to_owned(),
                status: "running".to_owned(),
                document_json: "{\"tools\":2}".to_owned(),
                updated_at_ms: now + 4,
            })
            .await
            .unwrap();
        actor
            .upsert_cache_metadata(CacheMetadataRecord {
                cache_key: "prompt:abc".to_owned(),
                provider_plane: "bedrock_runtime".to_owned(),
                document_json: "{\"tokens\":42}".to_owned(),
                updated_at_ms: now + 5,
            })
            .await
            .unwrap();
        actor
            .upsert_memory_record(MemoryRecord::new(
                "memory-1",
                MemoryKind::ProjectFact,
                MemoryScope::Project {
                    project_folder_id: "folder-local".to_owned(),
                },
                "Bedrock Runtime profile uses AWS CLI profile dev.",
                "conversation:1",
                now + 6,
            ))
            .await
            .unwrap();

        assert_eq!(
            actor.list_llm_profiles().await.unwrap()[0].id,
            "profile-mantle"
        );
        assert!(actor.list_llm_profiles().await.unwrap()[0].enabled);
        assert!(actor.delete_llm_profile("profile-mantle").await.unwrap());
        assert!(actor.list_llm_profiles().await.unwrap().is_empty());
        assert!(!actor.delete_llm_profile("profile-mantle").await.unwrap());
        assert_eq!(
            actor.list_project_folders().await.unwrap()[0].target_kind,
            "local"
        );
        assert!(actor.delete_project_folder("folder-local").await.unwrap());
        assert!(actor.list_project_folders().await.unwrap().is_empty());
        assert!(!actor.delete_project_folder("folder-local").await.unwrap());
        assert_eq!(actor.list_goals().await.unwrap()[0].status, "active");
        assert!(actor.delete_goal("conversation-1").await.unwrap());
        assert!(actor.list_goals().await.unwrap().is_empty());
        assert!(!actor.delete_goal("conversation-1").await.unwrap());
        assert!(actor.list_editor_tabs().await.unwrap()[0].active);
        assert!(actor.delete_editor_tab("/repo/src/main.rs").await.unwrap());
        assert!(actor.list_editor_tabs().await.unwrap().is_empty());
        assert!(!actor.delete_editor_tab("/repo/src/main.rs").await.unwrap());
        assert_eq!(
            actor.list_mcp_statuses().await.unwrap()[0].status,
            "running"
        );
        assert!(actor.delete_mcp_status("filesystem").await.unwrap());
        assert!(actor.list_mcp_statuses().await.unwrap().is_empty());
        assert!(!actor.delete_mcp_status("filesystem").await.unwrap());
        assert_eq!(
            actor.list_cache_metadata().await.unwrap()[0].provider_plane,
            "bedrock_runtime"
        );
        let memory = actor.list_memory_records().await.unwrap();
        assert_eq!(memory.len(), 1);
        assert_eq!(memory[0].id, "memory-1");
        assert_eq!(
            memory[0].scope,
            MemoryScope::Project {
                project_folder_id: "folder-local".to_owned()
            }
        );
        assert!(memory[0].enabled);
        assert!(
            actor
                .set_memory_enabled("memory-1", false, now + 7)
                .await
                .unwrap()
        );
        let memory = actor.list_memory_records().await.unwrap();
        assert!(!memory[0].enabled);
        assert_eq!(memory[0].updated_at_ms, now + 7);
        assert!(actor.delete_memory_record("memory-1").await.unwrap());
        assert!(actor.list_memory_records().await.unwrap().is_empty());
        assert!(!actor.delete_memory_record("memory-1").await.unwrap());

        actor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn v9_export_snapshot_omits_secret_values_by_default() {
        let (_temp_dir, config) = test_config();
        let actor = PersistenceActorHandle::spawn(config).unwrap();
        let now = now_ms();

        actor
            .upsert_conversation(ConversationMetadata {
                id: "conversation-1".to_owned(),
                title: "Export".to_owned(),
                project_folder_id: None,
                status: "running".to_owned(),
                created_at_ms: now,
                updated_at_ms: now,
            })
            .await
            .unwrap();
        actor
            .append_conversation_event(ConversationEvent {
                conversation_id: "conversation-1".to_owned(),
                event_type: "conversation.created".to_owned(),
                payload_json: "{\"status\":\"running\"}".to_owned(),
                created_at_ms: now,
            })
            .await
            .unwrap();
        actor
            .store_secret_ref(SecretRefRecord {
                id: SecretRef("secret:bedrock:dev".to_owned()),
                service: "aws-bedrock".to_owned(),
                account: "dev".to_owned(),
                created_at_ms: now,
            })
            .await
            .unwrap();
        actor
            .store_settings_document(SettingsDocument {
                scope: ConfigScope::Global,
                key: "settings".to_owned(),
                document_json: "{\"auth_ref\":\"secret:bedrock:dev\"}".to_owned(),
                updated_at_ms: now,
            })
            .await
            .unwrap();
        let secret_store = MemorySecretStore::default();
        secret_store
            .put_secret(
                &SecretRefRecord {
                    id: SecretRef("secret:bedrock:dev".to_owned()),
                    service: "aws-bedrock".to_owned(),
                    account: "dev".to_owned(),
                    created_at_ms: now,
                },
                "not-a-real-secret-value",
            )
            .unwrap();

        let export = actor.export_redacted_snapshot().await.unwrap();
        let json = export.to_json_pretty().unwrap();

        assert_eq!(export.conversations.len(), 1);
        assert_eq!(export.conversation_events.len(), 1);
        assert_eq!(export.settings_documents.len(), 1);
        assert_eq!(export.secret_refs.len(), 1);
        assert!(json.contains("secret:bedrock:dev"));
        assert!(!json.contains("not-a-real-secret-value"));

        actor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn t22_import_redacted_snapshot_round_trips_and_replaces_state() {
        let (_temp_dir, config) = test_config();
        let actor = PersistenceActorHandle::spawn(config).unwrap();
        let now = now_ms();
        let snapshot = PersistenceExportSnapshot {
            conversations: vec![ConversationMetadata {
                id: "conversation-1".to_owned(),
                title: "Imported".to_owned(),
                project_folder_id: Some("folder-1".to_owned()),
                status: "idle".to_owned(),
                created_at_ms: now,
                updated_at_ms: now + 1,
            }],
            conversation_events: vec![StoredConversationEvent {
                id: 42,
                conversation_id: "conversation-1".to_owned(),
                event_type: "conversation.created".to_owned(),
                payload_json: "{\"title\":\"Imported\"}".to_owned(),
                created_at_ms: now + 2,
            }],
            settings_documents: vec![SettingsDocument {
                scope: ConfigScope::Global,
                key: "settings".to_owned(),
                document_json: "{\"theme\":\"system\"}".to_owned(),
                updated_at_ms: now + 3,
            }],
            secret_refs: vec![SecretRefRecord {
                id: SecretRef("secret:bedrock:imported".to_owned()),
                service: "aws-bedrock".to_owned(),
                account: "imported".to_owned(),
                created_at_ms: now + 4,
            }],
            llm_profiles: vec![LlmProfileRecord {
                id: "profile-1".to_owned(),
                provider: "bedrock_mantle".to_owned(),
                enabled: true,
                document_json: "{\"model\":\"anthropic.test\"}".to_owned(),
                updated_at_ms: now + 5,
            }],
            project_folders: vec![ProjectFolderRecord {
                id: "folder-1".to_owned(),
                label: "Imported Folder".to_owned(),
                target_kind: "local".to_owned(),
                path: "/repo".to_owned(),
                remote_target_id: None,
                document_json: "{\"watch\":true}".to_owned(),
                updated_at_ms: now + 6,
            }],
            goals: vec![GoalRecord {
                conversation_id: "conversation-1".to_owned(),
                status: "active".to_owned(),
                document_json: "{\"objective\":\"finish\"}".to_owned(),
                updated_at_ms: now + 7,
            }],
            editor_tabs: vec![EditorTabRecord {
                path: "/repo/SPEC.md".to_owned(),
                conversation_id: Some("conversation-1".to_owned()),
                active: true,
                document_json: "{\"dirty\":false}".to_owned(),
                updated_at_ms: now + 8,
            }],
            mcp_status_snapshots: vec![McpStatusSnapshot {
                server_id: "filesystem".to_owned(),
                status: "running".to_owned(),
                document_json: "{\"tools\":1}".to_owned(),
                updated_at_ms: now + 9,
            }],
            cache_metadata: vec![CacheMetadataRecord {
                cache_key: "prompt:imported".to_owned(),
                provider_plane: "bedrock_mantle".to_owned(),
                document_json: "{\"hits\":1}".to_owned(),
                updated_at_ms: now + 10,
            }],
            memory_records: vec![MemoryRecord::new(
                "memory-imported",
                MemoryKind::Observation,
                MemoryScope::Global,
                "Imported fact.",
                "conversation:1",
                now + 11,
            )],
        };
        let json = snapshot.to_json_pretty().unwrap();
        let parsed = PersistenceExportSnapshot::from_json(&json).unwrap();

        actor
            .upsert_conversation(ConversationMetadata {
                id: "stale-conversation".to_owned(),
                title: "Stale".to_owned(),
                project_folder_id: None,
                status: "idle".to_owned(),
                created_at_ms: now - 1,
                updated_at_ms: now - 1,
            })
            .await
            .unwrap();
        actor
            .client()
            .import_redacted_snapshot(parsed)
            .await
            .unwrap();

        let imported = actor.export_redacted_snapshot().await.unwrap();
        assert_eq!(imported, snapshot);
        assert!(json.contains("secret:bedrock:imported"));
        assert!(!json.contains("not-a-real-secret-value"));

        actor.shutdown().await.unwrap();
    }

    #[test]
    fn v9_secret_store_keeps_secret_value_out_of_reference_record() {
        let store = MemorySecretStore::default();
        let record = SecretRefRecord {
            id: SecretRef("secret:aws:default".to_owned()),
            service: "aws-bedrock".to_owned(),
            account: "default".to_owned(),
            created_at_ms: now_ms(),
        };

        store
            .put_secret(&record, "not-a-real-secret-value")
            .unwrap();

        assert_eq!(
            store.get_secret(&record).unwrap(),
            "not-a-real-secret-value".to_owned()
        );
        assert!(!format!("{record:?}").contains("not-a-real-secret-value"));
    }

    #[derive(Default)]
    struct MemorySecretStore {
        values: Mutex<BTreeMap<(String, String), String>>,
    }

    impl SecretStore for MemorySecretStore {
        fn put_secret(
            &self,
            record: &SecretRefRecord,
            secret: &str,
        ) -> Result<(), SecretStoreError> {
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
}
