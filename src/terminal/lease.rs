//! Terminal copies use vwork's unchanged lease, revision and recovery protocol.
use anyhow::{Context, Result, ensure};
use fastrock_workspaces::{embedded::Connection, session::WorkspaceSession};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct ContextInfo {
    pub cwd: PathBuf,
    pub workspace: Option<WorkspaceSession>,
}

pub(super) struct Lease {
    pub cwd: PathBuf,
    pub renew_after: Duration,
    connection: Option<Connection>,
    workspace: String,
    revision: String,
    id: String,
}

impl Lease {
    pub async fn open(context: ContextInfo) -> Result<Self> {
        let Some(workspace) = context.workspace else {
            ensure!(
                context.cwd.is_absolute(),
                "Terminal directory must be absolute"
            );
            ensure!(
                tokio::fs::metadata(&context.cwd).await?.is_dir(),
                "Terminal directory is unavailable"
            );
            return Ok(Self {
                cwd: context.cwd,
                renew_after: Duration::from_secs(86400),
                connection: None,
                workspace: String::new(),
                revision: String::new(),
                id: String::new(),
            });
        };
        let connection = Connection::new(workspace.selection.home);
        let name = workspace.selection.workspace;
        let name_for_read = name.clone();
        let (revision, ttl) = connection
            .run(move |store| {
                store.with_engine(|engine| {
                    Ok((
                        engine.workspace(&name_for_read)?.head,
                        engine.config.temporary_ttl_seconds,
                    ))
                })
            })
            .await?;
        let value = connection
            .call(
                "workspace_temporary".into(),
                json!({"workspace":name,
            "expected_revision":revision,"purpose":"User-requested Fastrock interactive terminal"}),
            )
            .await?;
        Ok(Self {
            cwd: PathBuf::from(
                value["path"]
                    .as_str()
                    .context("Vwork returned no terminal copy path")?,
            ),
            id: value["id"]
                .as_str()
                .context("Vwork returned no terminal lease")?
                .into(),
            renew_after: Duration::from_millis((ttl.saturating_mul(1000) / 3).clamp(100, 60000)),
            connection: Some(connection),
            workspace: name,
            revision,
        })
    }

    pub fn is_virtual(&self) -> bool {
        self.connection.is_some()
    }

    pub async fn renew(&self) -> Result<()> {
        if let Some(connection) = &self.connection {
            connection
                .call("temporary_renew".into(), json!({"lease":self.id}))
                .await?;
        }
        Ok(())
    }

    /// Called only after the PTY process tree stops. Import is one-shot: the
    /// public vwork API pins each export marker to its original revision.
    pub async fn finish(self, paths: Vec<String>) -> String {
        let Some(connection) = self.connection else {
            return "Terminal closed".into();
        };
        let imported = if paths.is_empty() {
            None
        } else {
            Some(
                connection
                    .call(
                        "workspace_import".into(),
                        json!({"workspace":self.workspace,
                "expected_revision":self.revision,"source":self.cwd,"paths":paths}),
                    )
                    .await,
            )
        };
        let release = connection
            .call("temporary_release".into(), json!({"lease":self.id}))
            .await;
        let mut status = match imported {
            Some(Ok(_)) => "Selected source files imported. ".to_owned(),
            Some(Err(error)) => format!("Import failed: {error}. "),
            None => String::new(),
        };
        match release {
            Ok(value) => {
                if let Some(recovery) = value["recovery_workspace"].as_str() {
                    status.push_str(&format!("Source edits preserved in workspace {recovery}."));
                } else {
                    status.push_str("Terminal copy released.");
                }
            }
            Err(error) => status.push_str(&format!(
                "Terminal copy retained at {}: {error}",
                self.cwd.display()
            )),
        }
        status
    }
}

pub(super) fn import_paths(text: &str) -> Result<Vec<String>> {
    ensure!(text.len() <= 64 * 1024, "Import path list exceeds 64 KiB");
    let mut paths = Vec::new();
    for path in text.lines().map(str::trim).filter(|p| !p.is_empty()) {
        ensure!(
            paths.len() < 1000,
            "Import at most 1000 named paths at a time"
        );
        ensure!(
            !path.chars().any(char::is_control),
            "Import paths cannot contain control characters"
        );
        let normalized = path.replace('\\', "/");
        ensure!(
            !normalized.starts_with('/')
                && !normalized.contains(':')
                && normalized
                    .split('/')
                    .all(|p| !matches!(p, "" | "." | ".." | ".git" | ".vwork-managed.json")),
            "Import paths must name source files inside the terminal copy"
        );
        if !paths.contains(&normalized) {
            paths.push(normalized);
        }
    }
    ensure!(
        !paths.is_empty(),
        "Name at least one source file, one path per line"
    );
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v61_terminal_import_names_only_relative_source_paths() {
        assert_eq!(
            import_paths("src/main.rs\nnew file.txt\nsrc\\main.rs").unwrap(),
            ["src/main.rs", "new file.txt"]
        );
        for path in [
            "../outside",
            "C:\\outside",
            "/outside",
            ".git/config",
            ".vwork-managed.json",
            "a//b",
            "",
        ] {
            assert!(import_paths(path).is_err(), "{path}");
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use fastrock_workspaces::session::Selection;
    async fn fixture() -> (tempfile::TempDir, Connection, ContextInfo) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let repo = git2::Repository::init(&root).unwrap();
        std::fs::write(root.join("file.txt"), "original\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("file.txt")).unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "Fixture", &tree, &[])
            .unwrap();
        let home = temp.path().join("state");
        let connection = Connection::new(home.clone());
        connection
            .call("repo_register".into(), json!({"repo":"repo","path":root}))
            .await
            .unwrap();
        connection
            .call(
                "workspace_create".into(),
                json!({"repo":"repo","workspace":"parent"}),
            )
            .await
            .unwrap();
        let workspace = Selection {
            home,
            workspace: "parent".into(),
        }
        .resolve()
        .await
        .unwrap();
        let context = ContextInfo {
            cwd: workspace.cwd.clone(),
            workspace: Some(workspace),
        };
        (temp, connection, context)
    }
    #[tokio::test]
    async fn v61_terminal_lease_renew_import_recovery_and_stale_revision() {
        let (_temp, connection, context) = fixture().await;
        let root = context.workspace.as_ref().unwrap().source_root.clone();
        let lease = Lease::open(context.clone()).await.unwrap();
        lease.renew().await.unwrap();
        let cwd = lease.cwd.clone();
        std::fs::write(cwd.join("file.txt"), "imported\n").unwrap();
        std::fs::write(cwd.join("unimported.txt"), "recover this\n").unwrap();
        let status = lease.finish(vec!["file.txt".into()]).await;
        assert!(
            status.contains("Selected source files imported"),
            "{status}"
        );
        assert!(status.contains("preserved in workspace"), "{status}");
        assert!(!cwd.exists());
        let value = connection
            .call(
                "files_read".into(),
                json!({"workspace":"parent","path":"file.txt"}),
            )
            .await
            .unwrap();
        assert!(value.to_string().contains("imported"));
        let lease = Lease::open(context).await.unwrap();
        let revision = lease.revision.clone();
        let cwd = lease.cwd.clone();
        std::fs::write(cwd.join("file.txt"), "stale terminal\n").unwrap();
        connection.call("files_write".into(),json!({"workspace":"parent","path":"file.txt","content":"concurrent edit\n","expected_revision":revision})).await.unwrap();
        let status = lease.finish(vec!["file.txt".into()]).await;
        assert!(status.contains("Import failed"), "{status}");
        assert!(status.contains("preserved in workspace"), "{status}");
        let value = connection
            .call(
                "files_read".into(),
                json!({"workspace":"parent","path":"file.txt"}),
            )
            .await
            .unwrap();
        assert!(value.to_string().contains("concurrent edit"));
        assert_eq!(
            std::fs::read_to_string(root.join("file.txt")).unwrap(),
            "original\n"
        );
    }
}
