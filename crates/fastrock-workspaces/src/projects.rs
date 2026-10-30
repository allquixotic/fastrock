//! Registered repositories are projects; each new chat receives a virtual workspace.
use crate::{
    embedded::Connection,
    session::{Selection, WorkspaceSession},
};
use anyhow::{Context, Result, ensure};
use redb::{ReadableDatabase, TableDefinition};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use vwork_engine::engine::Engine;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub home: PathBuf,
    pub repo: String,
    pub root: PathBuf,
}

pub fn id(home: &Path, repo: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(home.as_os_str().as_encoded_bytes());
    digest.update([0]);
    digest.update(repo.as_bytes());
    format!("vwork:{:x}", digest.finalize())
}

pub async fn root(path: PathBuf) -> Result<Option<PathBuf>> {
    tokio::task::spawn_blocking(move || match git2::Repository::discover(path) {
        Ok(repo) => repo
            .workdir()
            .unwrap_or(repo.path())
            .canonicalize()
            .map(Some)
            .map_err(Into::into),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    })
    .await?
}

pub async fn list(connection: &Connection) -> Result<Vec<Project>> {
    connection
        .run(|store| {
            store.with_engine(|engine| {
                let rows = engine.call("repo_list", &json!({}))?;
                let rows = rows
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("Invalid repository catalog"))?;
                Ok(rows
                    .iter()
                    .filter_map(|row| {
                        let repo = row["id"].as_str()?;
                        let root = PathBuf::from(row["path"].as_str()?);
                        if !root.is_absolute() {
                            return None;
                        }
                        Some(Project {
                            id: id(&engine.home, repo),
                            name: if repo.strip_prefix("project-").is_some_and(|v| {
                                v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit())
                            }) {
                                root.file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| repo.to_owned())
                            } else {
                                repo.to_owned()
                            },
                            home: engine.home.clone(),
                            repo: repo.into(),
                            root,
                        })
                    })
                    .collect())
            })
        })
        .await
}

const SETTINGS: TableDefinition<&str, &[u8]> = TableDefinition::new("fastrock_project_settings_v1");
const REFERENCES: TableDefinition<&str, &str> =
    TableDefinition::new("fastrock_workspace_references_v1");
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub revision: u64,
    pub default_reference: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            revision: 0,
            default_reference: "HEAD".into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Details {
    pub project: Project,
    pub settings: Settings,
    pub branches: Vec<String>,
}

/// The starting ref is display metadata; the actual workspace revision remains
/// authoritative after edits and integration. Old workspaces fall back to their
/// base commit instead of inventing a branch name.
pub async fn reference(connection: &Connection, workspace: String) -> Result<String> {
    connection
        .run(move |store| {
            store.with_engine(|engine| {
                let workspace = engine.workspace(&workspace)?;
                let revision = engine.revision(&workspace.head)?;
                let transaction = engine.db.begin_read()?;
                match transaction.open_table(REFERENCES) {
                    Ok(table) => {
                        if let Some(value) = table.get(workspace.id.as_str())? {
                            return Ok(value.value().to_owned());
                        }
                    }
                    Err(redb::TableError::TableDoesNotExist(_)) => {}
                    Err(error) => return Err(error.into()),
                }
                Ok(revision.base)
            })
        })
        .await
}
fn settings(engine: &Engine, repo: &str) -> Result<Settings> {
    let transaction = engine.db.begin_read()?;
    let table = match transaction.open_table(SETTINGS) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Settings::default()),
        Err(error) => return Err(error.into()),
    };
    table
        .get(repo)?
        .map(|value| serde_json::from_slice(value.value()).map_err(Into::into))
        .unwrap_or_else(|| Ok(Settings::default()))
}
fn register(engine: &Engine, root: &Path) -> Result<String> {
    ensure!(root.is_absolute(), "Project root must be absolute");
    let rows = engine.call("repo_list", &json!({}))?;
    let rows = rows.as_array().context("Invalid repository catalog")?;
    if let Some(id) = rows
        .iter()
        .find(|row| {
            row["path"]
                .as_str()
                .is_some_and(|path| Path::new(path) == root)
        })
        .and_then(|row| row["id"].as_str())
    {
        return Ok(id.into());
    }
    let id = format!(
        "project-{:x}",
        Sha256::digest(root.to_string_lossy().as_bytes())
    );
    engine.call("repo_register", &json!({"repo":id,"path":root}))?;
    Ok(id)
}
pub async fn register_project(connection: &Connection, root: PathBuf) -> Result<String> {
    connection
        .run(move |store| store.with_exclusive_engine(|engine| register(engine, &root)))
        .await
}
pub async fn details(connection: &Connection, repo: String) -> Result<Details> {
    let project = list(connection)
        .await?
        .into_iter()
        .find(|project| project.repo == repo)
        .context("Project is no longer registered")?;
    connection
        .run(move |store| {
            store.with_engine(|engine| {
                let record = engine.repo(&repo)?;
                let branches: Vec<_> = record
                    .refs
                    .keys()
                    .filter_map(|reference| {
                        reference.strip_prefix("refs/heads/").map(str::to_owned)
                    })
                    .take(10_001)
                    .collect();
                ensure!(
                    branches.len() <= 10_000,
                    "Project branch catalog exceeds limit"
                );
                Ok(Details {
                    project,
                    settings: settings(engine, &repo)?,
                    branches,
                })
            })
        })
        .await
}
pub async fn save_settings(
    connection: &Connection,
    repo: String,
    expected: u64,
    reference: String,
) -> Result<Details> {
    let saved_repo = repo.clone();
    connection
        .run(move |store| {
            store.with_exclusive_engine(|engine| {
                ensure!(
                    !reference.is_empty()
                        && reference.len() <= 1024
                        && !reference.chars().any(char::is_control),
                    "Choose a valid Git reference"
                );
                let current = settings(engine, &saved_repo)?;
                ensure!(
                    current.revision == expected,
                    "Project settings changed in another window; reload before saving"
                );
                let record = engine.repo(&saved_repo)?;
                vwork_engine::git::resolve(&engine.db, &record, &reference)?;
                let value = Settings {
                    revision: expected
                        .checked_add(1)
                        .context("Project settings revision exhausted")?,
                    default_reference: reference,
                };
                let bytes = serde_json::to_vec(&value)?;
                let transaction = engine.db.begin_write()?;
                transaction
                    .open_table(SETTINGS)?
                    .insert(saved_repo.as_str(), bytes.as_slice())?;
                transaction.commit()?;
                Ok(())
            })
        })
        .await?;
    details(connection, repo).await
}
pub async fn open(
    connection: &Connection,
    root: PathBuf,
    workspace: String,
) -> Result<WorkspaceSession> {
    open_at(connection, root, workspace, None).await
}
pub async fn open_at(
    connection: &Connection,
    root: PathBuf,
    workspace: String,
    reference: Option<String>,
) -> Result<WorkspaceSession> {
    let selection = connection
        .run(move |store| {
            store.with_exclusive_engine(|engine| {
                let repo = register(engine, &root)?;
                let reference = reference.unwrap_or(settings(engine, &repo)?.default_reference);
                ensure!(
                    !reference.is_empty()
                        && reference.len() <= 1024
                        && !reference.chars().any(char::is_control),
                    "Choose a valid Git reference"
                );
                engine.call(
                    "workspace_create",
                    &json!({"repo":repo,"workspace":workspace,"reference":reference}),
                )?;
                let transaction = engine.db.begin_write()?;
                transaction
                    .open_table(REFERENCES)?
                    .insert(workspace.as_str(), reference.as_str())?;
                transaction.commit()?;
                Ok(Selection {
                    home: engine.home.clone(),
                    workspace,
                })
            })
        })
        .await?;
    selection.resolve().await
}
