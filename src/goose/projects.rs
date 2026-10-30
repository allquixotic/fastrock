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
        let home = if let Some(project) = existing.iter().find(|p| p.root == root) {
            project.home.clone()
        } else {
            self.configured_vwork(&cwd)
                .await?
                .first()
                .ok_or_else(|| error("No embedded workspace store is configured"))?
                .home
                .clone()
        };
        let workspace = format!("chat-{}", uuid::Uuid::new_v4().simple());
        let session = projects::open(&Connection::new(home), root, workspace)
            .await
            .map_err(|e| error(format!("Open repository workspace: {e:#}")))?;
        serde_json::to_value(session).map_err(|e| error(e.to_string()))
    }
}
