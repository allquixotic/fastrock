//! Typed drafts and three-way reload. UI owns presentation; writes stay explicit.
use super::{client::Client, types::*};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conflict {
    pub before: Value,
    pub local: Value,
    pub remote: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Editor {
    pub original: Object,
    pub draft: Object,
    pub fields: Vec<Field>,
    pub conflicts: BTreeMap<String, Conflict>,
    pub new: bool,
    pub kind: String,
    pub connection: String,
    pub tab: String,
    pub comments: String,
    pub relation_rows: Vec<Object>,
    pub relation_total: usize,
    pub relation_start: usize,
    pub relation_error: String,
    pub history: Vec<Object>,
    pub redo: Vec<Object>,
}
impl Editor {
    pub fn new(
        original: Object,
        kind: String,
        new: bool,
        fields: Vec<Field>,
        connection: String,
    ) -> Self {
        Self {
            draft: original.clone(),
            original,
            fields,
            conflicts: BTreeMap::new(),
            new,
            kind,
            connection,
            tab: "Details".into(),
            comments: String::new(),
            relation_rows: vec![],
            relation_total: 0,
            relation_start: 1,
            relation_error: String::new(),
            history: vec![],
            redo: vec![],
        }
    }
    pub fn dirty(&self) -> bool {
        self.new
            || self.draft != self.original
            || !self.conflicts.is_empty()
            || !self.comments.trim().is_empty()
    }
    pub fn edit(&mut self, field: &str, value: Value) {
        if self.draft.get(field) == Some(&value) {
            return;
        }
        self.history.push(self.draft.clone());
        if self.history.len() > 64 {
            self.history.remove(0);
        }
        while self
            .history
            .iter()
            .map(|o| serde_json::to_vec(o).map(|v| v.len()).unwrap_or(0))
            .sum::<usize>()
            > 4 * 1024 * 1024
            && !self.history.is_empty()
        {
            self.history.remove(0);
        }
        self.redo.clear();
        self.draft.insert(field.into(), value);
    }
    pub fn undo(&mut self) {
        if let Some(previous) = self.history.pop() {
            self.redo.push(std::mem::replace(&mut self.draft, previous));
        }
    }
    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.history.push(std::mem::replace(&mut self.draft, next));
        }
    }
    /// Accept acknowledged fields without losing omitted data or later edits.
    pub fn accept_save(&mut self, submitted: &Object, changes: &Object, returned: Object) {
        let mut acknowledged = acknowledged_object(&self.original, changes, returned);
        for (key, value) in submitted {
            if let (Some(reference), Some(remote)) = (
                value.get("_ref").and_then(Value::as_str),
                acknowledged.get(key).and_then(Value::as_object),
            ) {
                if remote.get("_ref").and_then(Value::as_str) == Some(reference) {
                    let mut labeled = value.as_object().unwrap().clone();
                    labeled.extend(remote.clone());
                    acknowledged.insert(key.clone(), Value::Object(labeled));
                }
            }
        }
        let latest = std::mem::replace(&mut self.draft, acknowledged.clone());
        let keys = latest
            .keys()
            .chain(submitted.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        for key in keys {
            if latest.get(&key) != submitted.get(&key) {
                if let Some(value) = latest.get(&key) {
                    self.draft.insert(key, value.clone());
                } else {
                    self.draft.remove(&key);
                }
            }
        }
        self.conflicts.clear();
        for key in ["Tags", "Milestones"] {
            if acknowledged
                .get(key)
                .is_some_and(|v| v.is_object() && v.get("Count").is_some())
            {
                let known = changes.get(key).or_else(|| self.original.get(key));
                if let Some(known) = known.filter(|v| v.is_array()) {
                    if latest.get(key) == submitted.get(key) {
                        self.draft.insert(key.into(), known.clone());
                    }
                    self.conflicts.insert(
                        key.into(),
                        Conflict {
                            before: self.original.get(key).cloned().unwrap_or(Value::Null),
                            local: self.draft.get(key).cloned().unwrap_or(Value::Null),
                            remote: acknowledged.get(key).cloned().unwrap_or(Value::Null),
                        },
                    );
                }
            }
        }
        self.original = acknowledged;
        self.new = false;
        self.history.clear();
        self.redo.clear();
    }
    pub fn reload(&mut self, remote: Object) {
        let names = self
            .original
            .keys()
            .chain(self.draft.keys())
            .chain(remote.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        for key in names {
            let before = self.original.get(&key).cloned().unwrap_or(Value::Null);
            let local = self.draft.get(&key).cloned().unwrap_or(Value::Null);
            let new = remote.get(&key).cloned().unwrap_or(Value::Null);
            if local == before {
                if new.is_null() {
                    self.draft.remove(&key);
                } else {
                    self.draft.insert(key.clone(), new);
                }
            } else if new != before && new != local && !key.starts_with('_') {
                self.conflicts.insert(
                    key.clone(),
                    Conflict {
                        before,
                        local,
                        remote: new,
                    },
                );
            }
        }
        // Read-only identity/revision always comes from server; never becomes editable.
        for key in ["VersionId", "LastUpdateDate", "ObjectID", "FormattedID"] {
            if let Some(value) = remote.get(key) {
                self.draft.insert(key.into(), value.clone());
                self.conflicts.remove(key);
            }
        }
        self.original = remote;
    }
    pub fn resolve(&mut self, field: &str, keep_local: bool) {
        if let Some(conflict) = self.conflicts.remove(field) {
            if !keep_local {
                self.edit(field, conflict.remote);
            }
        }
    }
    pub fn relation_tabs(&self) -> Vec<&'static str> {
        let mut tabs = vec!["Details", "More fields"];
        if self.new {
            return tabs;
        }
        tabs.push("Discussions");
        for (label, field) in [
            ("Attachments", "Attachments"),
            ("Revisions", "RevisionHistory"),
            ("Tasks", "Tasks"),
            (
                "Children",
                if self.original.contains_key("UserStories") {
                    "UserStories"
                } else {
                    "Children"
                },
            ),
            ("Defects", "Defects"),
            ("Test Cases", "TestCases"),
            ("Results", "Results"),
        ] {
            if self.original.contains_key(field) {
                tabs.push(label);
            }
        }
        tabs
    }
    pub fn changes(&self, client: &Client) -> Result<Object> {
        ensure!(
            self.conflicts.is_empty(),
            "Resolve field conflicts before saving"
        );
        let mut changes = Object::new();
        for field in &self.fields {
            if field.read_only || identity_field(&field.name) {
                continue;
            }
            let value = self.draft.get(&field.name).cloned().unwrap_or(Value::Null);
            if field.required {
                ensure!(
                    !value.is_null() && value.as_str().is_none_or(|s| !s.trim().is_empty()),
                    "{} is required",
                    field.label()
                );
            }
            if value
                == self
                    .original
                    .get(&field.name)
                    .cloned()
                    .unwrap_or(Value::Null)
                && !self.new
            {
                continue;
            }
            if value.is_null() {
                if !self.new {
                    let value = if field.attribute_type == "OBJECT" && !value.is_null() {
                        json!({"_ref":value.as_str().map(str::to_owned).unwrap_or_else(||value.as_object().unwrap().text("_ref"))})
                    } else {
                        value
                    };
                    changes.insert(field.name.clone(), value);
                }
                continue;
            }
            validate_type(field, &value)?;
            if field.name == "DisplayColor" {
                let color = value.as_str().unwrap_or("");
                ensure!(
                    color.is_empty()
                        || (color.len() == 7
                            && color.starts_with('#')
                            && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())),
                    "Choose a valid display color"
                );
            }
            if field.attribute_type == "COLLECTION" {
                ensure!(
                    ["Tags", "Milestones"].contains(&field.name.as_str()),
                    "{} is not an editable collection",
                    field.label()
                );
                let values = value.as_array().ok_or_else(|| {
                    anyhow::anyhow!("Load every selected {} before editing", field.label())
                })?;
                ensure!(
                    values.len() <= 10000,
                    "Collection exceeds 10,000 selections"
                );
                for value in values {
                    let reference = value["_ref"].as_str().unwrap_or("");
                    let kind = client.reference_kind(reference)?;
                    ensure!(
                        kind == if field.name == "Tags" {
                            "Tag"
                        } else {
                            "Milestone"
                        },
                        "Choose valid {} references",
                        field.label()
                    );
                }
            }
            if field.attribute_type == "OBJECT" && !value.is_null() {
                let reference = value
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| value.as_object().map(|v| v.text("_ref")))
                    .unwrap_or_default();
                ensure!(
                    !reference.is_empty(),
                    "Choose a valid {} reference",
                    field.label()
                );
                if !reference.is_empty() {
                    let kind = client.reference_kind(&reference)?;
                    if !field.reference_type.is_empty() {
                        ensure!(
                            reference_type_matches(&field.reference_type, kind),
                            "Choose valid {} reference",
                            field.label()
                        );
                    }
                }
            }
            if !field.allowed_values.is_empty() && !value.is_null() {
                ensure!(
                    field
                        .allowed_values
                        .contains(&value.as_str().unwrap_or("").to_string()),
                    "Choose a valid {} value",
                    field.label()
                );
            }
            let value = if field.attribute_type == "OBJECT" && !value.is_null() {
                json!({"_ref":value.as_str().map(str::to_owned).unwrap_or_else(||value.as_object().unwrap().text("_ref"))})
            } else {
                value
            };
            changes.insert(field.name.clone(), value);
        }
        if self.new {
            for key in ["Workspace", "Project"] {
                if let Some(value) = self.draft.get(key) {
                    changes.insert(key.into(), value.clone());
                }
            }
        }
        Ok(changes)
    }
}

pub fn acknowledged_object(before: &Object, changes: &Object, returned: Object) -> Object {
    let mut result = before.clone();
    result.extend(changes.clone());
    for (key, value) in returned {
        if ["Tags", "Milestones"].contains(&key.as_str()) {
            if let (Some(known), Some(count)) = (
                result.get(&key).and_then(Value::as_array),
                value.get("Count").and_then(Value::as_u64),
            ) {
                if count == known.len() as u64 {
                    continue;
                }
            }
        }
        result.insert(key, value);
    }
    result
}
pub fn identity_field(name: &str) -> bool {
    name.starts_with('_')
        || [
            "ObjectID",
            "FormattedID",
            "VersionId",
            "LastUpdateDate",
            "CreationDate",
            "Rank",
            "DragAndDropRank",
        ]
        .contains(&name)
}
pub fn typed_value(field: &Field, text: &str) -> Result<Value> {
    if text.is_empty() && !field.allowed_values.is_empty() && !field.required {
        return Ok(Value::Null);
    }
    if text.is_empty() && field.attribute_type != "STRING" && field.attribute_type != "TEXT" {
        return Ok(Value::Null);
    }
    Ok(match field.attribute_type.as_str() {
        "BOOLEAN" => json!(text.parse::<bool>()?),
        "INTEGER" => json!(text.trim().parse::<i64>()?),
        "QUANTITY" | "DECIMAL" | "REAL" => {
            let n = text.trim().parse::<f64>()?;
            ensure!(n.is_finite(), "Value must be finite");
            json!(n)
        }
        "DATE" => {
            chrono::DateTime::parse_from_rfc3339(text)
                .or_else(|_| chrono::DateTime::parse_from_rfc3339(&format!("{text}T00:00:00Z")))?;
            json!(text)
        }
        "COLLECTION" => serde_json::from_str(text)?,
        _ => json!(text),
    })
}
pub fn shared_fields<'a>(schemas: impl IntoIterator<Item = &'a [Field]>) -> Vec<Field> {
    let schemas = schemas.into_iter().collect::<Vec<_>>();
    let Some(first) = schemas.first() else {
        return vec![];
    };
    first
        .iter()
        .filter(|f| {
            !f.read_only
                && !identity_field(&f.name)
                && schemas.iter().all(|fields| {
                    fields.iter().any(|other| {
                        other.name == f.name
                            && !other.read_only
                            && other.attribute_type == f.attribute_type
                            && other.reference_type == f.reference_type
                    })
                })
        })
        .map(|field| {
            let mut f = field.clone();
            f.allowed_values.retain(|value| {
                schemas.iter().all(|fields| {
                    fields
                        .iter()
                        .find(|other| other.name == f.name)
                        .is_some_and(|other| other.allowed_values.contains(value))
                })
            });
            f
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v19_sparse_save_preserves_late_edits_html_and_complete_collections() {
        let before = json!({"Name":"before","Description":"<DIV data-x='1'>Exact HTML</DIV>","Notes":"old","VersionId":"1","Tags":[{"_ref":"tag/5"}]}).as_object().unwrap().clone();
        let mut editor = Editor::new(before, "Task".into(), false, vec![], String::new());
        editor.edit("Name", json!("submitted"));
        let submitted = editor.draft.clone();
        editor.edit("Name", json!("typed while saving"));
        editor.comments = "later comment".into();
        editor.accept_save(
            &submitted,
            json!({"Name":"submitted"}).as_object().unwrap(),
            json!({"VersionId":"2","Notes":null,"Tags":{"Count":1,"_ref":"task/1/Tags"}})
                .as_object()
                .unwrap()
                .clone(),
        );
        assert_eq!(editor.original.text("Name"), "submitted");
        assert_eq!(editor.draft.text("Name"), "typed while saving");
        assert_eq!(
            editor.draft.text("Description"),
            "<DIV data-x='1'>Exact HTML</DIV>"
        );
        assert_eq!(editor.draft.get("Notes"), Some(&Value::Null));
        assert!(editor.draft["Tags"].is_array());
        assert_eq!(editor.draft.text("VersionId"), "2");
        assert_eq!(editor.comments, "later comment");
        assert!(editor.dirty());
    }
    #[test]
    fn v19_changed_collection_summary_requires_fresh_review() {
        let before = json!({"Name":"before","Tags":[{"_ref":"tag/5"}]})
            .as_object()
            .unwrap()
            .clone();
        let mut editor = Editor::new(before, "Task".into(), false, vec![], String::new());
        let submitted = editor.draft.clone();
        editor.accept_save(
            &submitted,
            &Object::new(),
            json!({"Tags":{"Count":2,"_ref":"task/1/Tags"}})
                .as_object()
                .unwrap()
                .clone(),
        );
        assert!(editor.conflicts.contains_key("Tags"));
        assert!(editor.draft["Tags"].is_array());
        assert!(editor.dirty());
        let cleared = acknowledged_object(
            &editor.draft,
            &Object::new(),
            json!({"Tags":null}).as_object().unwrap().clone(),
        );
        assert_eq!(cleared.get("Tags"), Some(&Value::Null));
    }
    #[test]
    fn reload_keeps_local_edits_and_flags_overlap() {
        let mut e = Editor::new(
            json!({"Name":"before","Description":"old","VersionId":"1"})
                .as_object()
                .cloned()
                .unwrap(),
            "Task".into(),
            false,
            vec![],
            "scope".into(),
        );
        e.edit("Name", json!("mine"));
        e.comments = "unsent".into();
        e.reload(
            json!({"Name":"theirs","Description":"remote","VersionId":"2"})
                .as_object()
                .cloned()
                .unwrap(),
        );
        assert_eq!(e.draft.text("Name"), "mine");
        assert_eq!(e.draft.text("Description"), "remote");
        assert_eq!(e.draft.text("VersionId"), "2");
        assert_eq!(e.comments, "unsent");
        assert_eq!(e.conflicts.len(), 1);
        e.resolve("Name", true);
        assert!(e.conflicts.is_empty());
    }
    #[test]
    fn untouched_html_survives_undo_exactly() {
        let html = "<DIV class='x'><b>a &amp; b</b></DIV>";
        let mut e = Editor::new(
            json!({"Description":html}).as_object().cloned().unwrap(),
            "Task".into(),
            false,
            vec![],
            String::new(),
        );
        e.edit("Description", json!("<p>new</p>"));
        e.undo();
        assert_eq!(e.draft.text("Description"), html);
    }
}

pub fn reference_type_matches(expected: &str, kind: &str) -> bool {
    if expected.eq_ignore_ascii_case("Artifact") {
        ARTIFACT_KINDS.contains(&kind)
    } else if expected.eq_ignore_ascii_case("PortfolioItem") {
        kind.starts_with("PortfolioItem/")
    } else {
        canonical_kind(expected) == Some(kind)
    }
}
pub fn validate_type(field: &Field, value: &Value) -> Result<()> {
    let valid = match field.attribute_type.as_str() {
        "BOOLEAN" => value.is_boolean(),
        "INTEGER" => value.as_i64().is_some(),
        "QUANTITY" | "DECIMAL" | "REAL" => value.as_f64().is_some_and(f64::is_finite),
        "STRING" | "TEXT" => value.is_string(),
        "DATE" => value.as_str().is_some_and(|s| {
            chrono::DateTime::parse_from_rfc3339(s).is_ok()
                || chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
        }),
        "OBJECT" => value.is_string() || value.is_object(),
        "COLLECTION" => value.is_array(),
        _ => true,
    };
    ensure!(
        valid,
        "{} has an invalid {} value",
        field.label(),
        field.attribute_type
    );
    Ok(())
}

#[cfg(test)]
mod acceptance_tests {
    use super::*;
    #[test]
    fn v19_conflict_resolution_keeps_edits_made_after_reload() {
        let original = json!({"Name":"old","VersionId":"1"})
            .as_object()
            .unwrap()
            .clone();
        let mut editor = Editor::new(
            original,
            "HierarchicalRequirement".into(),
            false,
            vec![],
            String::new(),
        );
        editor.edit("Name", json!("first local"));
        editor.reload(
            json!({"Name":"remote","VersionId":"2"})
                .as_object()
                .unwrap()
                .clone(),
        );
        editor.edit("Name", json!("latest local"));
        editor.resolve("Name", true);
        assert_eq!(editor.draft["Name"], "latest local");
        assert!(editor.conflicts.is_empty());
    }
    #[test]
    fn v20_related_tabs_follow_supported_fields() {
        let original = json!({"Tasks":{},"UserStories":{},"RevisionHistory":{}})
            .as_object()
            .unwrap()
            .clone();
        let mut editor = Editor::new(
            original,
            "PortfolioItem/Feature".into(),
            false,
            vec![],
            String::new(),
        );
        assert_eq!(
            editor.relation_tabs(),
            vec![
                "Details",
                "More fields",
                "Discussions",
                "Revisions",
                "Tasks",
                "Children"
            ]
        );
        editor.new = true;
        assert_eq!(editor.relation_tabs(), vec!["Details", "More fields"]);
    }
    #[test]
    fn v19_typed_proposals_reject_wrong_json_shapes() {
        let mut field = Field {
            attribute_type: "BOOLEAN".into(),
            ..Default::default()
        };
        assert!(validate_type(&field, &json!("true")).is_err());
        assert!(validate_type(&field, &json!(true)).is_ok());
        field.attribute_type = "DECIMAL".into();
        assert!(validate_type(&field, &json!("12")).is_err());
        field.attribute_type = "COLLECTION".into();
        assert!(validate_type(&field, &json!({"Count":2})).is_err());
    }
}
