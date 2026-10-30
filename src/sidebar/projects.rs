use super::*;
use fastrock_workspaces::projects::Project;

pub(super) fn bind(thread: &mut ThreadSummary, projects: &[Project]) {
    thread.project_root = thread
        .project_id
        .as_ref()
        .and_then(|id| projects.iter().find(|p| &p.id == id))
        .map(|p| p.root.clone());
}

pub(super) fn rows(
    threads: &[ThreadSummary],
    projects: &[Project],
    collapsed: &HashSet<PathBuf>,
    more: bool,
    include_empty: bool,
) -> Vec<RowSpec> {
    let mut rows = build_rows(threads, collapsed, false);
    if include_empty {
        let mut seen: HashSet<PathBuf> = rows
            .iter()
            .filter_map(|row| match row {
                RowSpec::Folder { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        for project in projects {
            let root = dunce::simplified(&project.root).to_path_buf();
            if seen.insert(root.clone()) {
                rows.push(RowSpec::Folder {
                    path: root.clone(),
                    count: 0,
                    collapsed: collapsed.contains(&root),
                });
            }
        }
    }
    if more {
        rows.push(RowSpec::More);
    }
    rows
}

impl AppController {
    pub(super) fn sidebar_refresh_projects(&mut self) {
        if self.sidebar.projects_loading {
            return;
        }
        self.sidebar.projects_loading = true;
        let backend = self.backend.clone();
        self.backend.spawn(async move {
            let result = backend
                .goose_request("fastrock/project/list", serde_json::json!({}))
                .await
                .and_then(|v| serde_json::from_value::<Vec<Project>>(v).map_err(|e| e.to_string()));
            crate::ui_thread::post(move |app| {
                app.sidebar.projects_loading = false;
                if let Ok(projects) = result {
                    app.sidebar.projects = projects;
                    for thread in app
                        .sidebar
                        .threads
                        .iter_mut()
                        .chain(app.sidebar.recent.iter_mut())
                        .chain(app.sidebar.history_hits.iter_mut())
                    {
                        bind(thread, &app.sidebar.projects);
                    }
                    app.sidebar_render();
                    app.newtab_on_threads_changed();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v51_workspaces_group_under_repository_and_empty_projects_remain() {
        let projects = vec![
            Project {
                id: "one".into(),
                name: "repo".into(),
                home: "/store".into(),
                repo: "repo".into(),
                root: "/repo".into(),
            },
            Project {
                id: "empty".into(),
                name: "empty".into(),
                home: "/store".into(),
                repo: "empty".into(),
                root: "/empty".into(),
            },
        ];
        let mut threads: Vec<_> = ["parent", "child"]
            .into_iter()
            .map(|id| ThreadSummary {
                id: id.into(),
                title: id.into(),
                cwd: PathBuf::from(format!("/scratch/{id}")),
                project_id: Some("one".into()),
                project_root: None,
                updated_at: 0,
            })
            .collect();
        for thread in &mut threads {
            bind(thread, &projects);
        }
        let groups = group_by_folder(&threads);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].cwd, Path::new("/repo"));
        let rows = rows(&threads, &projects, &HashSet::new(), false, true);
        assert!(matches!(&rows[0],RowSpec::Folder {path,count:2,..} if path == Path::new("/repo")));
        assert!(matches!(&rows[1],RowSpec::Thread {cwd,..} if cwd == Path::new("/scratch/parent")));
        assert!(
            matches!(rows.last(),Some(RowSpec::Folder {path,count:0,..}) if path == Path::new("/empty"))
        );
    }
}
