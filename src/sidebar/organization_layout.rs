//! Project-first grouping, without filesystem probes or duplicated folder rows.
use super::{
    organization::{attention, working},
    organization_store::State,
    *,
};
use fastrock_workspaces::projects::Project;
use sha2::{Digest, Sha256};

const SECTIONS: [&str; 5] = ["pinned", "active", "working", "snoozed", "settled"];

pub(super) fn section_name(id: &str) -> Option<&str> {
    if SECTIONS.contains(&id) {
        return Some(id);
    }
    let rest = id.strip_prefix("project:")?;
    let (hash, section) = rest.split_once(':')?;
    (hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()) && SECTIONS.contains(&section))
        .then_some(section)
}

fn section_id(root: Option<&Path>, section: &str) -> String {
    match root {
        Some(root) => format!(
            "project:{}:{section}",
            hex::encode(Sha256::digest(root.as_os_str().as_encoded_bytes()))
        ),
        None => section.into(),
    }
}

pub(super) struct Layout<'a> {
    pub state: &'a State,
    pub marks: &'a HashMap<String, ThreadMark>,
    pub collapsed: &'a HashSet<PathBuf>,
    pub now: i64,
    pub searching: bool,
}
impl Layout<'_> {
    pub fn rows(
        &self,
        threads: &[ThreadSummary],
        projects: &[Project],
        include_empty: bool,
        more: bool,
    ) -> Vec<RowSpec> {
        let mut rows = vec![];
        let order: HashMap<_, _> = self
            .state
            .order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        if self.state.policy.group_projects {
            let groups = group_by_folder(threads);
            let mut seen = HashSet::new();
            for group in groups {
                seen.insert(group.cwd);
                let collapsed = self.collapsed.contains(group.cwd);
                rows.push(RowSpec::Folder {
                    path: group.cwd.into(),
                    count: group.threads.len(),
                    collapsed,
                });
                if !collapsed {
                    self.sections(&mut rows, &group.threads, Some(group.cwd), &order);
                }
            }
            if include_empty {
                for project in projects {
                    let root = dunce::simplified(&project.root);
                    if seen.insert(root) {
                        rows.push(RowSpec::Folder {
                            path: root.into(),
                            count: 0,
                            collapsed: self.collapsed.contains(root),
                        });
                    }
                }
            }
        } else {
            self.sections(&mut rows, &threads.iter().collect::<Vec<_>>(), None, &order);
        }
        if more {
            rows.push(RowSpec::More);
        }
        rows
    }

    fn sections(
        &self,
        rows: &mut Vec<RowSpec>,
        threads: &[&ThreadSummary],
        root: Option<&Path>,
        order: &HashMap<&str, usize>,
    ) {
        for (section, title) in SECTIONS
            .into_iter()
            .zip(["Pinned", "Active", "Working", "Snoozed", "Settled"])
        {
            let mut subset: Vec<_> = threads
                .iter()
                .copied()
                .filter(|t| {
                    self.state.section_at(
                        &t.id,
                        self.marks.get(&t.id).is_some_and(|m| working(m.status)),
                        self.marks.get(&t.id).is_some_and(|m| attention(m.status)),
                        t.updated_at,
                        self.now,
                    ) == section
                })
                .collect();
            if subset.is_empty() {
                continue;
            }
            subset.sort_by_key(|t| match section {
                "snoozed" => (
                    0,
                    self.state
                        .entries
                        .get(&t.id)
                        .and_then(|e| e.snoozed_until)
                        .unwrap_or(i64::MAX),
                ),
                "settled" => (
                    0,
                    self.state
                        .entries
                        .get(&t.id)
                        .and_then(|e| e.settled_at)
                        .unwrap_or(t.updated_at)
                        .saturating_neg(),
                ),
                "working" => (0, t.updated_at.saturating_neg()),
                _ => (
                    order.get(t.id.as_str()).copied().unwrap_or(usize::MAX),
                    t.updated_at.saturating_neg(),
                ),
            });
            let id = section_id(root, section);
            // Settled is hidden until explicitly expanded, including during search.
            let collapsed =
                self.state.is_collapsed(&id) && (section == "settled" || !self.searching);
            rows.push(RowSpec::Section {
                id,
                title: title.into(),
                count: subset.len(),
                collapsed,
                project: root.map(Path::to_path_buf),
            });
            if !collapsed {
                rows.extend(subset.into_iter().map(|t| RowSpec::Thread {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    cwd: t.cwd.clone(),
                    updated_at: t.updated_at,
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::organization_store::Entry;
    use super::*;
    #[test]
    fn v88_projects_appear_once_and_settled_requires_expansion() {
        let mut state = State::default();
        let marks = HashMap::new();
        let collapsed = HashSet::new();
        let threads: Vec<_> = ["active", "old", "other"]
            .into_iter()
            .map(|id| ThreadSummary {
                id: id.into(),
                title: id.into(),
                cwd: if id == "other" { "/other" } else { "/home" }.into(),
                project_id: None,
                project_root: None,
                updated_at: 1,
            })
            .collect();
        state.entries.insert(
            "old".into(),
            Entry {
                settled_at: Some(1),
                ..Default::default()
            },
        );
        let render = |state: &State, searching| {
            Layout {
                state,
                marks: &marks,
                collapsed: &collapsed,
                now: 2,
                searching,
            }
            .rows(&threads, &[], false, true)
        };
        for searching in [false, true] {
            let rows = render(&state, searching);
            assert_eq!(
                rows.iter()
                    .filter(
                        |r| matches!(r, RowSpec::Folder {path,..} if path == Path::new("/home"))
                    )
                    .count(),
                1
            );
            assert!(
                !rows
                    .iter()
                    .any(|r| matches!(r, RowSpec::Thread {id,..} if id == "old"))
            );
            assert!(matches!(rows.last(), Some(RowSpec::More)));
        }
        state
            .expanded
            .insert(section_id(Some(Path::new("/home")), "settled"));
        assert_eq!(
            render(&state, false)
                .iter()
                .filter(|r| matches!(r, RowSpec::Thread { .. }))
                .count(),
            3
        );
        assert!(!state.is_collapsed(&section_id(Some(Path::new("/home")), "settled")));
        assert!(state.is_collapsed(&section_id(Some(Path::new("/other")), "settled")));
        state.policy.group_projects = false;
        let rows = render(&state, false);
        assert!(!rows.iter().any(|r| matches!(r, RowSpec::Folder { .. })));
        assert!(
            rows.iter()
                .any(|r| matches!(r, RowSpec::Section {id, collapsed:true,..} if id == "settled"))
        );
    }
}
