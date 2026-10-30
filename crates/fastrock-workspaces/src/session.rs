//! Nonsecret conversation binding to a compressed virtual workspace.
use crate::embedded::Connection;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub home: PathBuf,
    pub workspace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSession {
    pub selection: Selection,
    pub repo: String,
    pub source_root: PathBuf,
    pub cwd: PathBuf,
}

impl Selection {
    /// Resolve against the owning Engine, never against client-supplied paths.
    /// Ordinary shell commands start in an independent directory; source files
    /// remain compressed and are accessed using the bound vwork tools.
    pub async fn resolve(self) -> Result<WorkspaceSession> {
        let connection = Connection::new(self.home.clone());
        connection
            .run(move |store| {
                store.with_engine(|engine| {
                    let workspace = engine.workspace(&self.workspace)?;
                    anyhow::ensure!(
                        std::path::Path::new(&workspace.id).components().count() == 1
                            && matches!(
                                std::path::Path::new(&workspace.id).components().next(),
                                Some(std::path::Component::Normal(_))
                            ),
                        "Invalid workspace directory name"
                    );
                    let repo = engine.repo(&workspace.repo)?;
                    let root = engine.home.join("fastrock-agent-workdirs");
                    if let Err(e) = std::fs::create_dir(&root) {
                        if e.kind() != std::io::ErrorKind::AlreadyExists {
                            return Err(e.into());
                        }
                    }
                    let root = root.canonicalize()?;
                    anyhow::ensure!(
                        root.starts_with(&engine.home),
                        "Workspace working directory escapes its store"
                    );
                    let cwd = root.join(&workspace.id);
                    if let Err(e) = std::fs::create_dir(&cwd) {
                        if e.kind() != std::io::ErrorKind::AlreadyExists {
                            return Err(e.into());
                        }
                    }
                    let cwd = cwd.canonicalize()?;
                    anyhow::ensure!(
                        cwd.is_dir(),
                        "Workspace working directory is not a directory"
                    );
                    anyhow::ensure!(
                        cwd.starts_with(&engine.home),
                        "Workspace working directory escapes its store"
                    );
                    Ok(WorkspaceSession {
                        selection: Selection {
                            home: engine.home.clone(),
                            workspace: workspace.id,
                        },
                        repo: repo.id,
                        source_root: repo.path,
                        cwd,
                    })
                })
            })
            .await
    }
}
