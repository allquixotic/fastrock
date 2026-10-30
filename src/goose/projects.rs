use super::*;
use fastrock_workspaces::{embedded::Connection, projects};

impl Adapter {
    async fn registered_projects(
        &self,
        cwd: &str,
    ) -> Result<Vec<projects::Project>, JSONRPCErrorError> {
        let mut homes: Vec<_> = self
            .configured_vwork(cwd)
            .await?
            .into_iter()
            .map(|b| b.home)
            .collect();
        homes.extend(self.sessions.lock().await.values().filter_map(|s| {
            s.binding
                .workspace
                .as_ref()
                .map(|w| w.selection.home.clone())
        }));
        homes.sort();
        homes.dedup();
        let mut seen = std::collections::HashSet::new();
        let mut projects = Vec::new();
        for home in homes {
            let rows = projects::list(&Connection::new(home))
                .await
                .map_err(|e| error(format!("Read registered projects: {e:#}")))?;
            projects.extend(rows.into_iter().filter(|p| seen.insert(p.id.clone())));
        }
        projects.sort_by(|a, b| a.root.cmp(&b.root).then(a.id.cmp(&b.id)));
        Ok(projects)
    }

    pub(super) async fn project_catalog(&self) -> Reply {
        serde_json::to_value(self.registered_projects(&self.cwd).await?)
            .map_err(|e| error(e.to_string()))
    }

    pub(super) async fn project_settings(&self, params: &Value, save: bool) -> Reply {
        let path = std::path::PathBuf::from(text(params, "path"));
        let existing = self.registered_projects(&path.to_string_lossy()).await?;
        let project_id = text(params, "projectId");
        let project = if let Some(project) = existing.into_iter().find(|project| {
            if project_id.is_empty() {
                project.root == path
            } else {
                project.id == project_id
            }
        }) {
            project
        } else {
            if save || !project_id.is_empty() {
                return Err(error("Project is no longer registered"));
            }
            if !path.is_absolute() {
                return Err(error("Choose a repository project first"));
            }
            let root = projects::root(path)
                .await
                .map_err(|e| error(e.to_string()))?
                .ok_or_else(|| error("This folder is not a Git repository"))?;
            let bindings = self.configured_vwork(&root.to_string_lossy()).await?;
            let home = bindings
                .first()
                .ok_or_else(|| error("No embedded workspace store is configured"))?
                .home
                .clone();
            let connection = Connection::new(home);
            let repo = projects::register_project(&connection, root)
                .await
                .map_err(|e| error(e.to_string()))?;
            projects::details(&connection, repo)
                .await
                .map_err(|e| error(e.to_string()))?
                .project
        };
        let connection = Connection::new(project.home.clone());
        let result = if save {
            let expected = params["expectedRevision"]
                .as_u64()
                .ok_or_else(|| error("Project settings revision missing"))?;
            projects::save_settings(
                &connection,
                project.repo,
                expected,
                text(params, "reference").trim().into(),
            )
            .await
        } else {
            projects::details(&connection, project.repo).await
        };
        serde_json::to_value(result.map_err(|e| error(e.to_string()))?)
            .map_err(|e| error(e.to_string()))
    }

    pub(super) async fn open_project(&self, params: &Value) -> Reply {
        let path = std::path::PathBuf::from(text(params, "path"));
        if !path.is_absolute() {
            return Err(error("Choose an absolute repository directory"));
        }
        let root = projects::root(path)
            .await
            .map_err(|e| error(format!("Inspect project: {e:#}")))?;
        let Some(root) = root else {
            return Ok(Value::Null);
        };
        let cwd = root.to_string_lossy();
        let existing = self.registered_projects(&cwd).await?;
        let project_id = text(params, "projectId");
        let home = if let Some(project) = existing
            .iter()
            .find(|p| p.root == root && (project_id.is_empty() || p.id == project_id))
        {
            project.home.clone()
        } else {
            if !project_id.is_empty() {
                return Err(error("Project is no longer registered"));
            }
            self.configured_vwork(&cwd)
                .await?
                .first()
                .ok_or_else(|| error("No embedded workspace store is configured"))?
                .home
                .clone()
        };
        let workspace = format!("chat-{}", uuid::Uuid::new_v4().simple());
        let session = projects::open_at(
            &Connection::new(home),
            root,
            workspace,
            params["reference"].as_str().map(str::to_owned),
        )
        .await
        .map_err(|e| error(format!("Open repository workspace: {e:#}")))?;
        serde_json::to_value(session).map_err(|e| error(e.to_string()))
    }

    pub(super) async fn thread_location(&self, params: &Value) -> Reply {
        let id = session_id(text(params, "threadId"))?;
        self.deletion_ready(id).await?;
        let binding = self.binding_record(id).await?;
        let workspace = binding
            .workspace
            .ok_or_else(|| error("This conversation has no virtual repository workspace"))?;
        let reference = projects::reference(
            &Connection::new(workspace.selection.home.clone()),
            workspace.selection.workspace.clone(),
        )
        .await
        .map_err(|e| error(e.to_string()))?;
        Ok(
            json!({"path":workspace.source_root,"reference":reference,"workspace":workspace.selection.workspace}),
        )
    }
}
