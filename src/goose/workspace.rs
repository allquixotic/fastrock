use super::*;
use fastrock_workspaces::session::{Selection, WorkspaceSession};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct BindingRecord {
    pub workspace: Option<WorkspaceSession>,
    pub forked_from: Option<String>,
    #[serde(default)]
    pub delegation: Option<super::delegation::Delegation>,
    #[serde(default)]
    pub side: bool,
}

pub(super) fn instructions(workspace: &WorkspaceSession) -> String {
    format!(
        "Repository {} is held in compressed vwork workspace {}. Use the bound vwork tools for repository files, Git operations, builds and temporary checkouts. Read repository instructions through vwork before editing. Your ordinary working directory is separate from the repository source. Changes to other workspaces require explicit integration.",
        workspace.repo, workspace.selection.workspace
    )
}

impl Adapter {
    pub(super) async fn selected_workspace(
        &self,
        params: &Value,
    ) -> Result<Option<WorkspaceSession>, JSONRPCErrorError> {
        let value = &params["config"]["fastrock.workspace"];
        if value.is_null() {
            return Ok(None);
        }
        if params["ephemeral"] == true
            || params["sandbox"] == "read-only"
            || params["config"]["features.shell_tool"] == false
        {
            return Err(error(
                "Temporary or restricted conversations cannot bind a writable workspace",
            ));
        }
        let selection: Selection = serde_json::from_value(value.clone())
            .map_err(|_| error("Invalid workspace selection"))?;
        let resolved = selection
            .resolve()
            .await
            .map_err(|e| error(format!("Resolve workspace: {e:#}")))?;
        let home = &resolved.selection.home;
        let configured = self
            .configured_vwork(&resolved.source_root.to_string_lossy())
            .await?;
        let mut found = false;
        for binding in configured {
            if tokio::fs::canonicalize(&binding.home).await.ok().as_ref() == Some(home) {
                found = true;
                break;
            }
        }
        if !found {
            return Err(error(
                "Selected workspace store is not enabled in Goose configuration",
            ));
        }
        Ok(Some(resolved))
    }

    pub(super) fn save_binding_record(
        &self,
        id: &str,
        record: &BindingRecord,
    ) -> Result<(), JSONRPCErrorError> {
        std::fs::create_dir_all(&self.queue_root).map_err(|e| error(e.to_string()))?;
        let bytes = serde_json::to_vec(record).map_err(|e| error(e.to_string()))?;
        crate::rally::store::atomic_write(
            &self.queue_path(id).with_extension("binding.json"),
            &bytes,
        )
        .map_err(|e| error(format!("Save conversation workspace: {e}")))
    }

    pub(super) async fn binding_record(
        &self,
        id: &str,
    ) -> Result<BindingRecord, JSONRPCErrorError> {
        let path = self.queue_path(id).with_extension("binding.json");
        match tokio::fs::metadata(&path).await {
            Ok(meta) if meta.len() <= 64 * 1024 => serde_json::from_slice(
                &tokio::fs::read(path)
                    .await
                    .map_err(|e| error(e.to_string()))?,
            )
            .map_err(|_| {
                error("Invalid saved conversation workspace; preserve the file for recovery")
            }),
            Ok(_) => Err(error("Saved conversation workspace exceeds 64 KiB")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BindingRecord::default()),
            Err(e) => Err(error(e.to_string())),
        }
    }
}
