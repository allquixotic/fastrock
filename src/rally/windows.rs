//! Durable, token-free document handoff between native Slint app windows.
//! Files are private local IPC; the target acknowledges only after atomic persistence.
use super::store::{Document, Preferences, Store, atomic_write};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone)]
pub struct WindowHub {
    pub root: PathBuf,
    pub id: String,
    pub directory: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Owner {
    pub id: String,
    pub pid: u32,
    pub home: PathBuf,
    pub label: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Handoff {
    #[serde(skip)]
    pub phase: Option<String>,
    #[serde(skip)]
    pub source_alive: bool,
    pub document: Document,
    pub scope: Scope,
    pub source: String,
    pub receipt: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scope {
    endpoint: String,
    workspace: String,
    project: String,
    parents: bool,
    children: bool,
}
impl Scope {
    pub fn of(prefs: &Preferences) -> Self {
        Self {
            endpoint: prefs.rally_endpoint.clone(),
            workspace: prefs.rally_workspace.clone(),
            project: prefs.rally_project.clone(),
            parents: prefs.project_parents,
            children: prefs.project_children,
        }
    }
}
impl WindowHub {
    pub fn new(store: &Store) -> Result<Self> {
        Self::with_identity(
            store,
            store.directory.clone(),
            uuid::Uuid::new_v4().to_string(),
        )
    }
    /// A recovered window must never steal a registry from another live process.
    pub fn restore_identity(store: &Store, root: PathBuf, id: String) -> Result<Self> {
        ensure!(
            uuid::Uuid::parse_str(&id).is_ok(),
            "Invalid window identity"
        );
        let path = root.join("windows").join(&id).join("owner.json");
        match std::fs::read(&path) {
            Ok(bytes) => {
                let owner: Owner = serde_json::from_slice(&bytes)
                    .context("Invalid saved window owner; preserve its drafts")?;
                ensure!(
                    owner.pid == std::process::id() || !process_alive(owner.pid),
                    "Saved window is still owned by another running Fastrock process"
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Self::with_identity(store, root, id)
    }
    pub fn with_identity(store: &Store, root: PathBuf, id: String) -> Result<Self> {
        ensure!(
            uuid::Uuid::parse_str(&id).is_ok(),
            "Invalid window identity"
        );
        let directory = root.join("windows").join(&id);
        std::fs::create_dir_all(directory.join("inbox"))?;
        let hub = Self {
            root,
            id: id.clone(),
            directory,
        };
        let owner = Owner {
            id,
            pid: std::process::id(),
            home: store.directory.clone(),
            label: format!("Window {}", &hub.id[..8]),
        };
        atomic_write(
            &hub.directory.join("owner.json"),
            &serde_json::to_vec(&owner)?,
        )?;
        let _ = std::fs::remove_file(hub.directory.join("launching"));
        Ok(hub)
    }
    pub fn owners(&self) -> Result<Vec<Owner>> {
        let mut owners = vec![];
        for entry in std::fs::read_dir(self.root.join("windows"))? {
            let path = entry?.path().join("owner.json");
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(owner) = serde_json::from_slice::<Owner>(&bytes) {
                    if process_alive(owner.pid) {
                        owners.push(owner);
                    }
                }
            }
        }
        owners.sort_by(|a, b| a.label.cmp(&b.label));
        Ok(owners)
    }
    pub fn incoming(&self) -> Result<Vec<(PathBuf, Handoff)>> {
        let mut incoming = vec![];
        for entry in std::fs::read_dir(self.directory.join("inbox"))?.take(100) {
            let path = entry?.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let metadata = std::fs::metadata(&path)?;
            ensure!(
                metadata.len() <= 16 * 1024 * 1024,
                "Window transfer exceeds 16 MiB"
            );
            let mut handoff: Handoff = serde_json::from_slice(&std::fs::read(&path)?)?;
            handoff.phase = std::fs::read(&handoff.receipt)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|value| value["phase"].as_str().map(str::to_owned));
            // Ack paths must belong to this registry; never write a supplied arbitrary path.
            ensure!(
                handoff.receipt.parent() == Some(self.root.join("receipts").as_path()),
                "Invalid window receipt"
            );
            handoff.source_alive = std::fs::read(
                self.root
                    .join("windows")
                    .join(&handoff.source)
                    .join("owner.json"),
            )
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Owner>(&bytes).ok())
            .is_some_and(|owner| process_alive(owner.pid));
            incoming.push((path, handoff));
        }
        Ok(incoming)
    }
    pub fn send(&self, target: &str, document: Document, prefs: &Preferences) -> Result<PathBuf> {
        ensure!(
            uuid::Uuid::parse_str(target).is_ok(),
            "Invalid target window"
        );
        let owner = self
            .owners()?
            .into_iter()
            .find(|owner| owner.id == target)
            .context("Target window is closed")?;
        ensure!(owner.id != self.id, "Document is already in this window");
        let key = uuid::Uuid::new_v4().to_string();
        let receipt = self.root.join("receipts").join(format!("{key}.json"));
        std::fs::create_dir_all(receipt.parent().unwrap())?;
        let handoff = Handoff {
            phase: None,
            source_alive: true,
            document,
            scope: Scope::of(prefs),
            source: self.id.clone(),
            receipt: receipt.clone(),
        };
        let bytes = serde_json::to_vec(&handoff)?;
        ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "Document transfer exceeds 16 MiB"
        );
        atomic_write(
            &self
                .root
                .join("windows")
                .join(target)
                .join("inbox")
                .join(format!("{key}.json")),
            &bytes,
        )?;
        Ok(receipt)
    }
    pub fn prepare_child(&self, prefs: &Preferences) -> Result<(String, Store)> {
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store {
            directory: self.root.join("windows").join(&id),
        };
        store.save(prefs)?;
        atomic_write(
            &store.directory.join("launching"),
            std::process::id().to_string().as_bytes(),
        )?;
        Ok((id, store))
    }
    pub fn retired_documents(&self) -> Result<Vec<(PathBuf, Vec<Document>)>> {
        let mut documents = vec![];
        for entry in std::fs::read_dir(self.root.join("windows"))? {
            let directory = entry?.path();
            let launching = directory.join("launching");
            if let Ok(pid) = std::fs::read_to_string(&launching) {
                if pid.trim().parse::<u32>().is_ok_and(process_alive) {
                    continue;
                }
                // A crash before the new window registered must not strand drafts.
                let _ = std::fs::remove_file(launching);
            }
            if directory == self.directory {
                continue;
            }
            let owner_path = directory.join("owner.json");
            let owner = std::fs::read(&owner_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Owner>(&bytes).ok());
            if owner.is_some_and(|owner| process_alive(owner.pid)) {
                continue;
            }
            let store = Store {
                directory: directory.clone(),
            };
            if let Ok(prefs) = store.load() {
                if !prefs.documents.is_empty() {
                    documents.push((directory, prefs.documents));
                }
            }
        }
        Ok(documents)
    }
    pub fn retire(&self) {
        let _ = std::fs::remove_file(self.directory.join("owner.json"));
    }
}
pub fn acknowledge(path: &Path, handoff: &Handoff, result: Result<()>) -> Result<()> {
    let result = match result {
        Ok(()) => serde_json::json!({"accepted":true}),
        Err(error) => serde_json::json!({"accepted":false,"error":error.to_string()}),
    };
    atomic_write(&handoff.receipt, &serde_json::to_vec(&result)?)?;
    std::fs::remove_file(path)?;
    Ok(())
}
#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        ok && code == 259
    }
}
#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v48_explicit_window_identity_and_abandoned_launch_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().into(),
        };
        let parent = WindowHub::new(&store).unwrap();
        let prefs = Preferences {
            documents: vec![Document {
                identity: "recover-me".into(),
                assistant_draft: "draft".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let (id, child) = parent.prepare_child(&prefs).unwrap();
        assert!(parent.retired_documents().unwrap().is_empty());
        atomic_write(
            &child.directory.join("launching"),
            b"invalid-abandoned-owner",
        )
        .unwrap();
        assert_eq!(
            parent.retired_documents().unwrap()[0].1[0].identity,
            "recover-me"
        );
        let child_hub = WindowHub::with_identity(&child, parent.root.clone(), id.clone()).unwrap();
        assert_eq!(child_hub.id, id);
        let owners = parent.owners().unwrap();
        assert_eq!(owners.len(), 2);
        assert!(owners.iter().all(|owner| owner.pid == std::process::id()));
        assert!(parent.retired_documents().unwrap().is_empty());
        child_hub.retire();
        assert_eq!(
            parent.retired_documents().unwrap()[0].1[0].assistant_draft,
            "draft"
        );
    }

    #[test]
    fn v21_window_handoff_is_durable_and_token_free() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let source = uuid::Uuid::new_v4().to_string();
        let target = uuid::Uuid::new_v4().to_string();
        let source_hub = WindowHub {
            root: root.clone(),
            id: source.clone(),
            directory: root.join("windows").join(source),
        };
        let target_hub = WindowHub {
            root: root.clone(),
            id: target.clone(),
            directory: root.join("windows").join(&target),
        };
        std::fs::create_dir_all(target_hub.directory.join("inbox")).unwrap();
        atomic_write(
            &target_hub.directory.join("owner.json"),
            &serde_json::to_vec(&Owner {
                id: target.clone(),
                pid: std::process::id(),
                home: temp.path().into(),
                label: "Target".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let doc = Document {
            identity: "stable-doc".into(),
            assistant_draft: "Unsent question".into(),
            ..Default::default()
        };
        let receipt = source_hub
            .send(&target, doc, &Preferences::default())
            .unwrap();
        assert!(!receipt.exists());
        let mut incoming = target_hub.incoming().unwrap();
        let (path, handoff) = incoming.pop().unwrap();
        assert_eq!(handoff.document.identity, "stable-doc");
        assert_eq!(handoff.document.assistant_draft, "Unsent question");
        ready(&handoff, Ok(())).unwrap();
        assert!(path.exists());
        assert_eq!(
            target_hub.incoming().unwrap()[0].1.phase.as_deref(),
            Some("ready")
        );
        commit(&receipt).unwrap();
        assert!(path.exists());
        assert_eq!(
            target_hub.incoming().unwrap()[0].1.phase.as_deref(),
            Some("committed")
        );
        acknowledge(&path, &handoff, Ok(())).unwrap();
        assert!(!path.exists());
        assert!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(receipt).unwrap()).unwrap()
                ["accepted"]
                == true
        );
    }
}

pub fn ready(handoff: &Handoff, result: Result<()>) -> Result<()> {
    let value = match result {
        Ok(()) => serde_json::json!({"phase":"ready"}),
        Err(error) => serde_json::json!({"phase":"rejected","error":error.to_string()}),
    };
    atomic_write(&handoff.receipt, &serde_json::to_vec(&value)?)
}
pub fn commit(receipt: &Path) -> Result<()> {
    atomic_write(receipt, b"{\"phase\":\"committed\"}")
}
