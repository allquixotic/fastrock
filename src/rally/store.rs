//! Preferences contain no token. Vault keys remain compatible with Go Fastrock.
use super::view::{Display, View};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub rally_endpoint: String,
    pub rally_workspace: String,
    pub rally_project: String,
    pub project_parents: bool,
    pub project_children: bool,
    pub rally_hidden_rows: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    pub rally_display: Display,
    pub views: Vec<View>,
    pub open_rally_tabs: Vec<View>,
    pub documents: Vec<Document>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            rally_endpoint: "https://rally1.rallydev.com".into(),
            rally_workspace: String::new(),
            rally_project: String::new(),
            project_parents: false,
            project_children: true,
            rally_hidden_rows: vec![],
            rally_display: Display::default(),
            views: vec![],
            open_rally_tabs: vec![],
            documents: vec![],
        }
    }
}
#[derive(Clone, Debug)]
pub struct Store {
    pub directory: PathBuf,
}
impl Store {
    pub fn new() -> Self {
        let directory = std::env::var_os("FASTROCK_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::config_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("fastrock")
            });
        Self { directory }
    }
    pub fn load(&self) -> Result<Preferences> {
        let current = self.directory.join("rally.json");
        let legacy = self.directory.join("settings.json");
        let migrating = !current.exists();
        let path = if !migrating { current } else { legacy };
        if !path.exists() {
            let mut prefs = Preferences::default();
            prefs.documents = self.legacy_documents()?;
            return Ok(prefs);
        }
        let bytes = std::fs::read(path)?;
        anyhow::ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "Rally preferences exceed 16 MiB"
        );
        let mut prefs: Preferences = serde_json::from_slice(&bytes)
            .context("Could not read Rally preferences; original file retained")?;
        if migrating
            && serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .is_some_and(|value| value["rallyNavHidden"] == true)
        {
            for key in ["sections", "pages"] {
                if !prefs.rally_hidden_rows.iter().any(|row| row == key) {
                    prefs.rally_hidden_rows.push(key.into());
                }
            }
        }
        if migrating && prefs.documents.is_empty() {
            prefs.documents = self.legacy_documents()?;
        }
        Ok(prefs)
    }
    pub fn save(&self, prefs: &Preferences) -> Result<()> {
        std::fs::create_dir_all(&self.directory)?;
        let value = serde_json::to_vec_pretty(prefs)?;
        anyhow::ensure!(
            value.len() <= 16 * 1024 * 1024,
            "Rally session exceeds 16 MiB; close unused documents"
        );
        atomic_write(&self.directory.join("rally.json"), &value)
    }
    pub fn token(&self, endpoint: &str) -> Result<String> {
        if let Ok(token) = std::env::var("FASTROCK_RALLY_TOKEN") {
            if !token.is_empty() {
                return Ok(token);
            }
        }
        vault_entry(endpoint)?
            .get_password()
            .context("Configure Rally API token in Settings")
    }
    pub fn set_token(&self, endpoint: &str, token: &str) -> Result<()> {
        let entry = vault_entry(endpoint)?;
        if token.is_empty() {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(e.into()),
            }
        } else {
            entry
                .set_password(token)
                .context("Could not save token in OS credential vault")
        }
    }
}
pub fn vault_target(endpoint: &str) -> String {
    format!("fastrock:{endpoint}")
}
fn vault_entry(endpoint: &str) -> Result<keyring::Entry> {
    #[cfg(windows)]
    {
        Ok(keyring::Entry::new_with_target(
            &vault_target(endpoint),
            "fastrock",
            endpoint,
        )?)
    }
    #[cfg(not(windows))]
    {
        Ok(keyring::Entry::new("fastrock", endpoint)?)
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let directory = path.parent().context("File has no parent directory")?;
    let temporary = directory.join(format!(".fastrock-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let from = temporary
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let to = path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            if unsafe {
                windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                    from.as_ptr(),
                    to.as_ptr(),
                    windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                        | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(not(windows))]
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_preferences_keep_scope_and_views_without_token() {
        let p:Preferences=serde_json::from_str(r#"{"rallyEndpoint":"https://rally.example","rallyWorkspace":"/workspace/1","rallyProject":"/project/2","views":[{"name":"Mine","page":"teamboard","columns":["Name"],"blocked":true}],"projectChildren":false}"#).unwrap();
        assert_eq!(p.rally_project, "/project/2");
        assert!(p.views[0].blocked);
        let output = serde_json::to_string(&p).unwrap();
        assert!(!output.to_ascii_lowercase().contains("token"));
        assert_eq!(
            vault_target("https://rally.example"),
            "fastrock:https://rally.example"
        );
    }
    #[test]
    fn saved_views_are_independent_snapshots() {
        let mut original = View::new("teamboard");
        original.columns = vec!["Name".into()];
        let mut copy = original.clone();
        copy.columns.push("Owner".into());
        assert_eq!(original.columns.len(), 1);
    }
}

pub fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
/// Native control state belongs to the document, including unapplied drafts.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DocumentUi {
    pub filter_field: String,
    pub filter_operator: String,
    pub filter_value: String,
    pub filters_open: bool,
    pub fields_open: bool,
    pub display_open: bool,
    pub wip: String,
    pub age: String,
    pub color_by: String,
    pub view_name: String,
    pub view_search: String,
    pub model_index: i32,
    pub table_x: f32,
    pub table_y: f32,
    pub board_x: f32,
    pub board_y: f32,
    pub detail_y: f32,
    pub relations_y: f32,
    pub assistant_y: f32,
    pub lane_scroll: std::collections::BTreeMap<String, f32>,
}
impl Default for DocumentUi {
    fn default() -> Self {
        Self {
            filter_field: "Iteration".into(),
            filter_operator: "is".into(),
            filter_value: String::new(),
            filters_open: false,
            fields_open: false,
            display_open: false,
            wip: String::new(),
            age: String::new(),
            color_by: String::new(),
            view_name: String::new(),
            view_search: String::new(),
            model_index: 0,
            table_x: 0.,
            table_y: 0.,
            board_x: 0.,
            board_y: 0.,
            detail_y: 0.,
            relations_y: 0.,
            assistant_y: 0.,
            lane_scroll: Default::default(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Document {
    pub ui: DocumentUi,
    pub identity: String,
    pub view: View,
    pub editor: Option<super::editor::Editor>,
    pub selected: std::collections::BTreeMap<String, super::types::Object>,
    pub collapsed: std::collections::BTreeSet<String>,
    pub inline: Option<String>,
    pub start: usize,
    pub resident_count: usize,
    pub assistant_visible: bool,
    pub assistant_history: String,
    pub assistant_draft: String,
    pub proposal: super::assistant::Plan,
}
impl Store {
    fn legacy_documents(&self) -> Result<Vec<Document>> {
        use super::{
            editor::{Editor, typed_value},
            types::*,
        };
        use serde_json::{Value, json};
        let path = self.directory.join("session.json");
        if !path.exists() {
            return Ok(vec![]);
        }
        let bytes = std::fs::read(path)?;
        anyhow::ensure!(
            bytes.len() <= 64 * 1024 * 1024,
            "Legacy session exceeds 64 MiB; original retained"
        );
        let root: Value = serde_json::from_slice(&bytes)
            .context("Legacy session is unreadable; original retained")?;
        let mut docs = vec![];
        for document in root["Documents"].as_array().into_iter().flatten() {
            if document["Tab"]["Kind"].as_str() != Some("rally") {
                continue;
            }
            let data = &document["Rally"];
            let page = document["Tab"]["Page"].as_str().unwrap_or("teamboard");
            let mut view = View::new(page);
            for (key, target) in [
                ("Mode", &mut view.mode),
                ("Group", &mut view.group),
                ("Timebox", &mut view.timebox),
                ("TimeboxName", &mut view.timebox_name),
                ("ReleaseTimebox", &mut view.release_timebox),
                ("ReleaseName", &mut view.release_name),
                ("Search", &mut view.search),
                ("Owner", &mut view.owner),
                ("State", &mut view.state),
                ("Sort", &mut view.sort),
            ] {
                if let Some(value) = data[key].as_str() {
                    *target = value.into();
                }
            }
            view.query = data["QueryApplied"]
                .as_str()
                .or_else(|| data["Query"].as_str())
                .unwrap_or("")
                .into();
            view.query_draft = data["Query"].as_str().unwrap_or(&view.query).into();
            view.blocked = data["OnlyBlocked"].as_bool().unwrap_or(false);
            view.ready = data["OnlyReady"].as_bool().unwrap_or(false);
            view.current_iteration = data["CurrentIteration"].as_bool().unwrap_or(false);
            view.descending = data["Descending"].as_bool().unwrap_or(false);
            if let Some(values) = data["Columns"].as_array() {
                view.columns = values
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
            }
            if let Some(values) = data["CardFields"].as_array() {
                view.card_fields = values
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
            }
            if !data["Display"].is_null() {
                view.display = serde_json::from_value(data["Display"].clone())?;
            }
            if !data["StructuredFilters"].is_null() {
                view.filters = serde_json::from_value(data["StructuredFilters"].clone())?;
            }
            view.widgets = data["Widgets"].as_bool().unwrap_or(false);
            view.exit_agreements = data["ExitAgreements"].as_bool().unwrap_or(false);
            view.rules = data["Rules"].as_bool().unwrap_or(false);
            let mut doc = Document {
                view,
                start: data["ResidentStart"].as_u64().unwrap_or(1) as usize,
                ..Default::default()
            };
            doc.collapsed = data["CollapsedLanes"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(_, v)| v.as_bool() == Some(true))
                .map(|(k, _)| k.clone())
                .collect();
            doc.resident_count =
                data["ResidentCount"].as_u64().unwrap_or_default().min(2048) as usize;
            doc.ui.filters_open = data["Filters"].as_bool().unwrap_or(false);
            doc.ui.fields_open = data["ShowFields"].as_bool().unwrap_or(false);
            doc.ui.view_name = data["ViewName"].as_str().unwrap_or_default().into();
            if let Some(draft) = data.get("FilterDraft") {
                doc.ui.filter_field = draft["Field"].as_str().unwrap_or("Iteration").into();
                doc.ui.filter_operator = draft["Operator"].as_str().unwrap_or("is").into();
                doc.ui.filter_value = draft["Value"].as_str().unwrap_or_default().into();
            }
            if let Some(scroll) = data["LaneScroll"].as_object() {
                for (lane, y) in scroll {
                    let key = lane
                        .strip_prefix("board--")
                        .map(|s| format!("|{s}"))
                        .unwrap_or_else(|| lane.clone());
                    doc.ui
                        .lane_scroll
                        .insert(key, -(y.as_f64().unwrap_or_default() as f32));
                }
            }
            doc.selected = serde_json::from_value(data["Selected"].clone()).unwrap_or_default();
            let detail = &data["Detail"];
            if let Some(original) = detail["Original"].as_object() {
                let mut fields = vec![];
                for f in detail["Fields"].as_array().into_iter().flatten() {
                    fields.push(Field {
                        name: f["Name"].as_str().unwrap_or("").into(),
                        display_name: f["DisplayName"].as_str().unwrap_or("").into(),
                        attribute_type: f["AttributeType"].as_str().unwrap_or("STRING").into(),
                        reference_type: f["ReferenceType"].as_str().unwrap_or("").into(),
                        required: f["Required"].as_bool().unwrap_or(false),
                        read_only: f["ReadOnly"].as_bool().unwrap_or(false),
                        allowed_values: serde_json::from_value(f["AllowedValues"].clone())
                            .unwrap_or_default(),
                    });
                }
                let mut editor = Editor::new(
                    original.clone(),
                    detail["Kind"]
                        .as_str()
                        .unwrap_or("HierarchicalRequirement")
                        .into(),
                    detail["New"].as_bool().unwrap_or(false),
                    fields,
                    String::new(),
                );
                editor.tab = detail["Tab"].as_str().unwrap_or("Details").into();
                editor.comments = detail["Comment"].as_str().unwrap_or("").into();
                for (key, value) in detail["Values"].as_object().into_iter().flatten() {
                    if let Some(text) = value.as_str() {
                        let field = editor.fields.iter().find(|f| &f.name == key);
                        let value = field
                            .map(|f| typed_value(f, text))
                            .transpose()?
                            .unwrap_or_else(|| json!(text));
                        editor.draft.insert(key.clone(), value);
                    }
                }
                for (key, value) in detail["Rich"].as_object().into_iter().flatten() {
                    editor
                        .draft
                        .insert(key.clone(), json!(super::rich::RichDoc::legacy(value)?));
                }
                doc.inline = detail["InlineField"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                doc.editor = Some(editor);
            }
            docs.push(doc);
        }
        // Older sessions did not have document snapshots. Keep their page tabs.
        if docs.is_empty() {
            for tab in root["Tabs"].as_array().into_iter().flatten() {
                if tab["Kind"].as_str() == Some("rally") {
                    docs.push(Document {
                        view: View::new(tab["Page"].as_str().unwrap_or("teamboard")),
                        start: 1,
                        ..Default::default()
                    });
                }
            }
        }
        Ok(docs)
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    #[test]
    fn v21_null_display_and_closed_legacy_tabs_do_not_reappear() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().into(),
        };
        std::fs::write(
            temp.path().join("settings.json"),
            r#"{"rallyDisplay":null,"views":[{"display":null,"name":"Original"}]}"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("session.json"),
            r#"{"Tabs":[{"Kind":"rally","Page":"tasks"}]}"#,
        )
        .unwrap();
        let mut p = store.load().unwrap();
        assert_eq!(p.documents.len(), 1);
        assert_eq!(p.documents[0].view.page, "tasks");
        p.documents.clear();
        store.save(&p).unwrap();
        assert!(store.load().unwrap().documents.is_empty());
        assert!(temp.path().join("settings.json").exists());
    }
    #[test]
    fn v21_document_roundtrip_retains_drafts_and_review_outcomes() {
        use serde_json::json;
        let temp = tempfile::tempdir().unwrap();
        let store = Store {
            directory: temp.path().into(),
        };
        let mut e = super::super::editor::Editor::new(
            json!({"Name":"old","Description":"<b>x</b>"})
                .as_object()
                .cloned()
                .unwrap(),
            "Task".into(),
            false,
            vec![],
            "https://rally.example".into(),
        );
        e.edit("Name", json!("unsent"));
        let mut prefs = Preferences::default();
        prefs.documents.push(Document {
            view: View::new("tasks"),
            editor: Some(e),
            ui: DocumentUi {
                filter_value: "unapplied draft".into(),
                table_y: -180.,
                lane_scroll: std::collections::BTreeMap::from([("|Defined".into(), -500.)]),
                ..Default::default()
            },
            assistant_draft: "unsent question".into(),
            ..Default::default()
        });
        store.save(&prefs).unwrap();
        let restored = store.load().unwrap();
        assert_eq!(
            restored.documents[0].editor.as_ref().unwrap().draft["Name"],
            "unsent"
        );
        assert_eq!(restored.documents[0].assistant_draft, "unsent question");
        assert_eq!(restored.documents[0].ui.filter_value, "unapplied draft");
        assert_eq!(restored.documents[0].ui.table_y, -180.);
        assert_eq!(restored.documents[0].ui.lane_scroll["|Defined"], -500.);
    }
}
