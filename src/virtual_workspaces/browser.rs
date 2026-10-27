//! Optional workspace UI state. The provider owns all storage/transport details.
use fastrock_workspaces::vwork::{self, Installation, VworkProvider};
use fastrock_workspaces::{
    Document, FileEntry, FilePage, MAX_TEXT_BYTES, PAGE_SIZE, Reply, Request, WorkspaceProvider,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogItem {
    pub provider: Installation,
    pub repo: String,
    pub workspace: String,
}
impl CatalogItem {
    pub fn label(&self) -> String {
        format!(
            "{} / {} / {}",
            self.provider.label,
            self.repo,
            if self.workspace.is_empty() {
                "(new workspace)"
            } else {
                &self.workspace
            }
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserRow {
    pub label: String,
    pub action: String,
    pub selected: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrowserSnapshot {
    pub visible: bool,
    pub busy: bool,
    pub status: String,
    pub catalog: Vec<BrowserRow>,
    pub files: Vec<BrowserRow>,
    pub title: String,
    pub text: String,
    pub dirty: bool,
    pub editable: bool,
    pub selected: bool,
    pub can_create: bool,
    pub has_next: bool,
    pub has_previous: bool,
    pub report: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    Discover,
    Call {
        epoch: u64,
        provider: Installation,
        request: Request,
    },
}
pub enum Event {
    Discovery {
        providers: Vec<Installation>,
        catalog: Vec<CatalogItem>,
        errors: Vec<String>,
    },
    Complete {
        epoch: u64,
        result: Result<Reply, String>,
    },
}
pub async fn execute(work: Work) -> Event {
    match work {
        Work::Discover => {
            let providers = vwork::discover(&vwork::DiscoveryPaths::current())
                .await
                .unwrap_or_default();
            let mut catalog = Vec::new();
            let mut errors = Vec::new();
            for provider in &providers {
                let result = match VworkProvider::new(provider) {
                    Ok(client) => client.execute(Request::Catalog).await,
                    Err(error) => Err(error),
                };
                match result {
                    Ok(Reply::Catalog {
                        repositories,
                        workspaces,
                    }) => {
                        let mut by_repo = std::collections::BTreeMap::<_, Vec<_>>::new();
                        for workspace in workspaces {
                            by_repo
                                .entry(workspace.repo.clone())
                                .or_default()
                                .push(workspace);
                        }
                        for repo in repositories {
                            catalog.push(CatalogItem {
                                provider: provider.clone(),
                                repo: repo.id.clone(),
                                workspace: String::new(),
                            });
                            for ws in by_repo.remove(&repo.id).unwrap_or_default() {
                                catalog.push(CatalogItem {
                                    provider: provider.clone(),
                                    repo: repo.id.clone(),
                                    workspace: ws.id.clone(),
                                });
                            }
                        }
                    }
                    Err(error) => errors.push(format!(
                        "{}: {error}. Start its vwork daemon and refresh.",
                        provider.label
                    )),
                    _ => errors.push("Invalid workspace catalog".into()),
                }
            }
            Event::Discovery {
                providers,
                catalog,
                errors,
            }
        }
        Work::Call {
            epoch,
            provider,
            request,
        } => {
            let result = match VworkProvider::new(&provider) {
                Ok(client) => client.execute(request).await.map_err(|e| e.to_string()),
                Err(error) => Err(error.to_string()),
            };
            Event::Complete { epoch, result }
        }
    }
}

#[derive(Debug, Default)]
pub struct Browser {
    providers: Vec<Installation>,
    catalog: Vec<CatalogItem>,
    catalog_filter: String,
    selected: Option<CatalogItem>,
    page: Option<FilePage>,
    prefix: String,
    offset: usize,
    document: Option<Document>,
    edited: String,
    busy: bool,
    epoch: u64,
    catalog_epoch: u64,
    page_epoch: u64,
    status: String,
    report: String,
}
impl Browser {
    pub fn snapshot(&self) -> BrowserSnapshot {
        let active = self
            .selected
            .as_ref()
            .is_some_and(|s| self.providers.contains(&s.provider));
        let dirty = self.dirty();
        BrowserSnapshot {
            visible: !self.providers.is_empty(),
            busy: self.busy,
            status: self.status.clone(),
            catalog: self
                .catalog
                .iter()
                .enumerate()
                .filter(|(_, c)| c.label().to_lowercase().contains(&self.catalog_filter))
                .take(PAGE_SIZE)
                .map(|(i, c)| BrowserRow {
                    label: c.label(),
                    action: format!("virtual-select:{}:{i}", self.catalog_epoch),
                    selected: self.selected.as_ref() == Some(c),
                })
                .collect(),
            files: self
                .page
                .as_ref()
                .map(|page| {
                    page.entries
                        .iter()
                        .enumerate()
                        .map(|(i, f)| BrowserRow {
                            label: format!("{}  ({} B)", f.path, f.size),
                            action: format!("virtual-open:{}:{i}", self.page_epoch),
                            selected: self.document.as_ref().is_some_and(|d| d.path == f.path),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            title: self
                .document
                .as_ref()
                .map(|d| format!("{} · revision {}", d.path, d.revision))
                .unwrap_or_else(|| "Select a file".into()),
            text: self.edited.clone(),
            dirty,
            editable: active && self.document.is_some() && !self.busy,
            selected: active
                && self
                    .selected
                    .as_ref()
                    .is_some_and(|s| !s.workspace.is_empty()),
            can_create: active,
            has_next: self.page.as_ref().is_some_and(|p| p.next.is_some()),
            has_previous: self.offset > 0,
            report: self.report.clone(),
        }
    }
    pub fn dirty(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|d| d.text != self.edited)
    }
    pub fn edit_status(&self) -> (bool, String) {
        (self.dirty(), self.status.clone())
    }
    /// Only used to restore the widget after a rejected oversized/binary paste.
    pub fn text(&self) -> &str {
        &self.edited
    }
    fn begin(&mut self, request: Request) -> Vec<Work> {
        let Some(selection) = &self.selected else {
            self.status = "Select a workspace first".into();
            return vec![];
        };
        if !self.providers.contains(&selection.provider) {
            self.status = "Workspace provider is no longer configured".into();
            return vec![];
        }
        self.epoch += 1;
        self.busy = true;
        self.status = "Working…".into();
        vec![Work::Call {
            epoch: self.epoch,
            provider: selection.provider.clone(),
            request,
        }]
    }
    fn list(&mut self) -> Vec<Work> {
        let Some(s) = &self.selected else {
            return vec![];
        };
        if s.workspace.is_empty() {
            return vec![];
        }
        self.begin(Request::Files {
            workspace: s.workspace.clone(),
            prefix: self.prefix.clone(),
            offset: self.offset,
        })
    }
    /// UI actions only mutate memory and enqueue work; they never perform I/O.
    pub fn action(&mut self, action: &str) -> Vec<Work> {
        if action == "virtual-refresh" {
            return vec![Work::Discover];
        }
        if let Some(filter) = action.strip_prefix("virtual-catalog-filter:") {
            self.catalog_filter = filter.to_lowercase();
            return vec![];
        }
        if self.providers.is_empty() {
            return vec![];
        }
        if self.busy {
            self.status = "Wait for the current workspace operation".into();
            return vec![];
        }
        if let Some(text) = action.strip_prefix("virtual-edit:") {
            if self.document.is_some() && text.len() <= MAX_TEXT_BYTES && !text.contains('\0') {
                self.edited = text.into();
            } else {
                self.status =
                    "Paste rejected: text must be UTF-8, without NUL, and at most 2 MiB".into();
            }
            return vec![];
        }
        if action == "virtual-discard" {
            if let Some(d) = &self.document {
                self.edited = d.text.clone();
            }
            self.status = "Unsaved edits discarded".into();
            return vec![];
        }
        if action == "virtual-save" {
            if let (Some(s), Some(d)) = (&self.selected, &self.document) {
                let mut d = d.clone();
                d.text = self.edited.clone();
                return self.begin(Request::Write {
                    workspace: s.workspace.clone(),
                    document: d,
                });
            }
            return vec![];
        }
        if action == "virtual-compact" {
            let work = self.begin(Request::Compact);
            if !work.is_empty() {
                self.status = "Checking and compacting the selected vwork store… This may take several minutes; unsaved text is preserved.".into();
            }
            return work;
        }
        if (action == "virtual-diff" || action == "virtual-size")
            && let Some(s) = &self.selected
        {
            let request = if action == "virtual-diff" {
                Request::Diff {
                    workspace: s.workspace.clone(),
                }
            } else {
                Request::Size {
                    workspace: s.workspace.clone(),
                }
            };
            return self.begin(request);
        }
        if self.dirty() {
            self.status = "Save or Discard unsaved edits before changing files/workspaces".into();
            return vec![];
        }
        if let Some(index) = row_index(action, "virtual-select:", self.catalog_epoch)
            && let Some(s) = self.catalog.get(index).cloned()
        {
            self.selected = Some(s);
            self.page = None;
            self.document = None;
            self.edited.clear();
            self.prefix.clear();
            self.offset = 0;
            self.report.clear();
            self.status = "Enter a name to create a workspace from this repository".into();
            return self.list();
        }
        if let Some(index) = row_index(action, "virtual-open:", self.page_epoch)
            && let (Some(s), Some(file)) = (
                &self.selected,
                self.page.as_ref().and_then(|p| p.entries.get(index)),
            )
        {
            return self.begin(Request::Read {
                workspace: s.workspace.clone(),
                file: file.clone(),
            });
        }
        if action == "virtual-reload"
            && let (Some(s), Some(d)) = (&self.selected, &self.document)
        {
            return self.begin(Request::Read {
                workspace: s.workspace.clone(),
                file: FileEntry {
                    path: d.path.clone(),
                    size: d.text.len() as u64,
                    mode: d.mode.clone(),
                },
            });
        }
        if let Some(prefix) = action.strip_prefix("virtual-filter:") {
            match fastrock_workspaces::validate_path(prefix, true) {
                Ok(()) => {
                    self.prefix = prefix.into();
                    self.offset = 0;
                    return self.list();
                }
                Err(error) => self.status = error.to_string(),
            }
        }
        if action == "virtual-next"
            && let Some(next) = self.page.as_ref().and_then(|p| p.next)
        {
            self.offset = next;
            return self.list();
        }
        if action == "virtual-previous" {
            self.offset = self.offset.saturating_sub(PAGE_SIZE);
            return self.list();
        }
        if let Some(name) = action.strip_prefix("virtual-create:")
            && let Some(s) = &self.selected
        {
            return self.begin(Request::Create {
                repo: s.repo.clone(),
                name: name.into(),
            });
        }
        if let Some(name) = action.strip_prefix("virtual-fork:")
            && let Some(s) = &self.selected
        {
            return self.begin(Request::Fork {
                workspace: s.workspace.clone(),
                name: name.into(),
            });
        }
        vec![]
    }
    pub fn apply(&mut self, event: Event) -> Vec<Work> {
        match event {
            Event::Discovery {
                providers,
                catalog,
                errors,
            } => {
                let changed = self.providers != providers || self.catalog != catalog;
                if let Some(selection) = &mut self.selected {
                    if let Some(provider) = providers.iter().find(|p| p.id == selection.provider.id)
                    {
                        selection.provider = provider.clone();
                    } else {
                        self.epoch += 1;
                        self.busy = false;
                    }
                }
                if self.catalog != catalog {
                    self.catalog_epoch += 1;
                }
                self.providers = providers;
                self.catalog = catalog;
                if !self.busy && !self.dirty() && (changed || self.status.is_empty()) {
                    self.status = if errors.is_empty() {
                        format!(
                            "{} repositories and workspaces. Filter to find more than 200 entries.",
                            self.catalog.len()
                        )
                    } else {
                        errors.join("\n")
                    };
                }
            }
            Event::Complete { epoch, result } => {
                if epoch != self.epoch {
                    return vec![];
                }
                self.busy = false;
                match result {
                    Err(error) => self.status = format!("{error}. Unsaved text is preserved."),
                    Ok(Reply::Files(page)) => {
                        self.page_epoch += 1;
                        self.status = format!(
                            "{} files · page starts at {} · revision {}",
                            page.total, self.offset, page.revision
                        );
                        self.page = Some(page);
                    }
                    Ok(Reply::Document(d)) => {
                        self.edited = d.text.clone();
                        self.document = Some(d);
                        self.report.clear();
                        self.status = "File opened from virtual workspace".into();
                    }
                    Ok(Reply::Saved { revision }) => {
                        if let Some(d) = &mut self.document {
                            d.text = self.edited.clone();
                            d.revision = revision.clone();
                        }
                        self.status = format!("Saved to virtual workspace · {revision}");
                    }
                    Ok(Reply::Report(report)) => {
                        self.report = report;
                        self.status = "Virtual workspace report".into();
                    }
                    Ok(Reply::Created(ws)) => {
                        if let Some(s) = &mut self.selected {
                            s.workspace = ws.id;
                            s.repo = ws.repo;
                            self.catalog.push(s.clone());
                        }
                        self.catalog_epoch += 1;
                        self.document = None;
                        self.edited.clear();
                        self.offset = 0;
                        self.prefix.clear();
                        self.report.clear();
                        return self.list();
                    }
                    _ => self.status = "Unexpected workspace response".into(),
                }
            }
        }
        vec![]
    }
}

fn row_index(action: &str, prefix: &str, epoch: u64) -> Option<usize> {
    let (generation, index) = action.strip_prefix(prefix)?.split_once(':')?;
    (generation.parse::<u64>().ok()? == epoch)
        .then(|| index.parse().ok())
        .flatten()
}
