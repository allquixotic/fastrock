//! Storage-neutral workspace files. Adapters own transport and storage details.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin};
use thiserror::Error;

pub mod vwork;

pub const PAGE_SIZE: usize = 200;
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Reply, WorkspaceError>> + Send + 'a>>;

pub trait WorkspaceProvider: Send + Sync {
    fn execute(&self, request: Request) -> ProviderFuture<'_>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    pub id: String,
    pub label: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub repo: String,
    pub revision: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub mode: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePage {
    pub revision: String,
    pub entries: Vec<FileEntry>,
    pub total: usize,
    pub next: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub path: String,
    pub text: String,
    pub revision: String,
    pub mode: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Catalog,
    Files {
        workspace: String,
        prefix: String,
        offset: usize,
    },
    Read {
        workspace: String,
        file: FileEntry,
    },
    Write {
        workspace: String,
        document: Document,
    },
    Create {
        repo: String,
        name: String,
    },
    Fork {
        workspace: String,
        name: String,
    },
    Diff {
        workspace: String,
    },
    Size {
        workspace: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Catalog {
        repositories: Vec<Repository>,
        workspaces: Vec<Workspace>,
    },
    Files(FilePage),
    Document(Document),
    Saved {
        revision: String,
    },
    Created(Workspace),
    Report(String),
}
#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("{0}")]
    Message(String),
    #[error("Workspace transport failed; check the configured daemon")]
    Transport,
    #[error("Invalid workspace response")]
    Protocol,
    #[error("File is binary, a symlink, or exceeds the 2 MiB editor limit")]
    NotEditable,
}

pub fn validate_path(path: &str, allow_empty: bool) -> Result<(), WorkspaceError> {
    if (path.is_empty() && allow_empty)
        || (!path.is_empty()
            && !path.starts_with('/')
            && !path.contains(['\\', '\0', ':'])
            && path
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != ".." && !p.eq_ignore_ascii_case(".git")))
    {
        Ok(())
    } else {
        Err(WorkspaceError::Message(
            "Use a repository-relative path with / separators".into(),
        ))
    }
}
