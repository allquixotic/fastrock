//! Registered repositories are projects; each new chat receives a virtual workspace.
use crate::{
    embedded::Connection,
    session::{Selection, WorkspaceSession},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

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

pub async fn open(
    connection: &Connection,
    root: PathBuf,
    workspace: String,
) -> Result<WorkspaceSession> {
    let home = connection
        .run(move |store| {
            store.with_exclusive_engine(|engine| {
                ensure!(root.is_absolute(), "Project root must be absolute");
                let rows = engine.call("repo_list", &json!({}))?;
                let rows = rows
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("Invalid repository catalog"))?;
                let repo = if let Some(id) = rows
                    .iter()
                    .find(|r| r["path"].as_str().is_some_and(|p| Path::new(p) == root))
                    .and_then(|r| r["id"].as_str())
                {
                    id.to_string()
                } else {
                    let digest = Sha256::digest(root.to_string_lossy().as_bytes());
                    let id = format!("project-{:x}", digest);
                    engine.call("repo_register", &json!({"repo":id,"path":root}))?;
                    id
                };
                engine.call(
                    "workspace_create",
                    &json!({"repo":repo,"workspace":workspace,"reference":"HEAD"}),
                )?;
                Ok(Selection {
                    home: engine.home.clone(),
                    workspace,
                })
            })
        })
        .await?;
    home.resolve().await
}
