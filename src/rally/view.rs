use super::{client::Client, types::*};
use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Filter {
    pub field: String,
    pub operator: String,
    pub value: String,
    pub label: String,
}
impl Default for Filter {
    fn default() -> Self {
        Self {
            field: "Iteration".into(),
            operator: "is".into(),
            value: String::new(),
            label: String::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Display {
    pub density: String,
    pub color_by: String,
    pub wip_limit: usize,
    pub age_days: usize,
}
impl Default for Display {
    fn default() -> Self {
        Self {
            density: "Comfortable".into(),
            color_by: "Work Item".into(),
            wip_limit: 0,
            age_days: 3,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct View {
    pub widgets: bool,
    pub exit_agreements: bool,
    pub rules: bool,
    pub name: String,
    pub page: String,
    pub query: String,
    pub query_draft: String,
    pub search: String,
    pub group: String,
    pub mode: String,
    pub timebox: String,
    pub timebox_name: String,
    pub release_timebox: String,
    pub release_name: String,
    pub current_iteration: bool,
    pub owner: String,
    pub state: String,
    pub blocked: bool,
    pub ready: bool,
    pub columns: Vec<String>,
    pub card_fields: Vec<String>,
    pub sort: String,
    pub descending: bool,
    pub filters: Vec<Filter>,
    #[serde(deserialize_with = "super::store::null_default")]
    pub display: Display,
}
impl Default for View {
    fn default() -> Self {
        Self {
            widgets: false,
            exit_agreements: false,
            rules: false,
            name: String::new(),
            page: "teamboard".into(),
            query: String::new(),
            query_draft: String::new(),
            search: String::new(),
            group: "None".into(),
            mode: "board".into(),
            timebox: String::new(),
            timebox_name: String::new(),
            release_timebox: String::new(),
            release_name: String::new(),
            current_iteration: false,
            owner: String::new(),
            state: String::new(),
            blocked: false,
            ready: false,
            columns: vec![
                "Rank",
                "FormattedID",
                "Name",
                "ScheduleState",
                "PlanEstimate",
                "Owner",
                "Iteration",
                "Blocked",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            card_fields: vec!["Owner", "Iteration", "Tasks", "PlanEstimate"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            sort: "Rank".into(),
            descending: false,
            filters: vec![],
            display: Display::default(),
        }
    }
}
impl View {
    pub fn new(id: &str) -> Self {
        let spec = page(id);
        let mut v = Self {
            page: spec.id.into(),
            mode: spec.mode.into(),
            current_iteration: id == "iterationstatus",
            ..Default::default()
        };
        v.columns = match spec.kind {
            "Defect" => vec![
                "Rank",
                "FormattedID",
                "Name",
                "State",
                "Severity",
                "Priority",
                "Owner",
                "Blocked",
            ],
            "Task" => vec![
                "Rank",
                "FormattedID",
                "Name",
                "State",
                "Estimate",
                "ToDo",
                "Actuals",
                "Owner",
                "Blocked",
            ],
            "TestCase" => vec![
                "Rank",
                "FormattedID",
                "Name",
                "LastVerdict",
                "Method",
                "Owner",
            ],
            "Iteration" | "Release" => {
                v.sort = "StartDate".into();
                vec!["ObjectID", "Name", "StartDate", "EndDate", "Project"]
            }
            "Project" => {
                v.sort = "Name".into();
                vec!["ObjectID", "Name", "State", "Owner"]
            }
            "User" => {
                v.sort = "DisplayName".into();
                vec!["ObjectID", "DisplayName", "UserName"]
            }
            "PortfolioItem/Feature" | "PortfolioItem/Epic" => vec![
                "Rank",
                "FormattedID",
                "Name",
                "State",
                "Owner",
                "PlannedStartDate",
                "PlannedEndDate",
                "Blocked",
            ],
            _ => v.columns.iter().map(String::as_str).collect(),
        }
        .into_iter()
        .map(str::to_owned)
        .collect();
        v
    }
    pub fn query(
        &self,
        client: &Client,
        scope: &super::store::Preferences,
        user: Option<&Object>,
    ) -> Result<Query> {
        let spec = page(&self.page);
        let mut q = Query {
            expression: self.query.clone(),
            workspace: scope.rally_workspace.clone(),
            project: scope.rally_project.clone(),
            parents: scope.project_parents,
            children: scope.project_children,
            start: 1,
            page_size: 128,
            ..Default::default()
        };
        if self.page == "teamboard" {
            q.artifact_types = vec![
                "HierarchicalRequirement",
                "Defect",
                "TestSet",
                "DefectSuite",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect();
        }
        ensure!(self.filters.len() <= 64, "At most 64 structured filters");
        for filter in &self.filters {
            ensure!(
                ["is", "is not", "contains"].contains(&filter.operator.as_str())
                    && !filter.value.trim().is_empty(),
                "Invalid saved filter; remove it before loading"
            );
            if filter.field == "Type" {
                let matches = |kind: &str| {
                    if filter.operator == "contains" {
                        kind.to_lowercase().contains(&filter.value.to_lowercase())
                    } else {
                        (canonical_kind(&filter.value) == Some(kind)) == (filter.operator == "is")
                    }
                };
                if self.page == "teamboard" {
                    q.artifact_types.retain(|kind| matches(kind));
                    if q.artifact_types.is_empty() {
                        q.artifact_types.push("HierarchicalRequirement".into());
                        q.expression = and(&q.expression, "(ObjectID = 0)");
                    }
                } else if !matches(spec.kind) {
                    q.expression = and(&q.expression, "(ObjectID = 0)");
                }
                continue;
            }
            let kind = match filter.field.as_str() {
                "Iteration" => "Iteration",
                "Release" => "Release",
                "Project" => "Project",
                "Tags" => "Tag",
                _ => anyhow::bail!("Unsupported restored filter field"),
            };
            let clause = if filter.operator == "contains" {
                format!("({}.Name contains {})", filter.field, quote(&filter.value))
            } else {
                ensure!(
                    client.reference_kind(&filter.value)? == kind,
                    "Choose a reference from this Rally connection"
                );
                let operator = if filter.field == "Tags" {
                    if filter.operator == "is" {
                        "contains"
                    } else {
                        "!contains"
                    }
                } else if filter.operator == "is" {
                    "="
                } else {
                    "!="
                };
                format!("({} {operator} {})", filter.field, quote(&filter.value))
            };
            q.expression = and(&q.expression, &clause);
        }
        if self.page == "backlog" {
            q.expression = and(&q.expression, "(Iteration = null)");
        }
        if self.page == "mywork" {
            let reference = user.map(|u| u.text("_ref")).unwrap_or_default();
            q.expression = and(
                &q.expression,
                &if reference.is_empty() {
                    "(ObjectID = 0)".into()
                } else {
                    eq("Owner", &reference)
                },
            );
        }
        if self.current_iteration && self.timebox.is_empty() {
            q.expression = and(&q.expression, "(ObjectID = 0)");
        }
        for (field, reference, name) in [
            ("Iteration", &self.timebox, &self.timebox_name),
            ("Release", &self.release_timebox, &self.release_name),
        ] {
            if !reference.is_empty() {
                let clause = if scope.project_parents || scope.project_children {
                    if name.is_empty() {
                        eq(field, reference)
                    } else {
                        eq(&format!("{field}.Name"), name)
                    }
                } else {
                    eq(field, reference)
                };
                q.expression = and(&q.expression, &clause);
            }
        }
        if !self.search.trim().is_empty() {
            let fields = match spec.kind {
                "User" => vec!["DisplayName", "UserName"],
                "Project" | "Workspace" | "Iteration" | "Release" | "TestFolder" => vec!["Name"],
                _ => vec!["Name", "Description", "FormattedID", "Owner.Name"],
            };
            let clauses = fields
                .into_iter()
                .map(|f| format!("({f} contains {})", quote(self.search.trim())))
                .collect::<Vec<_>>();
            q.expression = and(&q.expression, &format!("({})", clauses.join(" OR ")));
        }
        if self.blocked {
            q.expression = and(&q.expression, "(Blocked = true)");
        }
        if self.ready {
            q.expression = and(&q.expression, "(Ready = true)");
        }
        if !self.owner.is_empty() {
            let clause = if self.owner == "Unassigned" {
                "(Owner = null)".into()
            } else {
                eq(
                    if self.owner.contains('/') {
                        "Owner"
                    } else {
                        "Owner.Name"
                    },
                    &self.owner,
                )
            };
            q.expression = and(&q.expression, &clause);
        }
        if !self.state.is_empty() {
            q.expression = and(
                &q.expression,
                &eq(
                    if self.page == "teamboard" {
                        "ScheduleState"
                    } else {
                        state_field(spec.kind)
                    },
                    &self.state,
                ),
            );
        }
        let sort = if self.sort == "Rank" {
            "DragAndDropRank"
        } else {
            &self.sort
        };
        let direction = if self.descending { "DESC" } else { "ASC" };
        q.order = format!("{sort} {direction}");
        if !self.group.is_empty() && self.group != "None" && self.group != self.sort {
            q.order = format!("{} ASC,{}", self.group, q.order);
        }
        if ["Workspace", "User", "Project"].contains(&spec.kind) {
            q.project.clear();
        }
        Ok(q)
    }
}
pub fn current_iteration(rows: &[Object], project: &str, now: DateTime<Utc>) -> Option<Object> {
    let mut rows = rows
        .iter()
        .filter_map(|row| {
            let start = DateTime::parse_from_rfc3339(&row.text("StartDate"))
                .ok()?
                .with_timezone(&Utc);
            let end = DateTime::parse_from_rfc3339(&row.text("EndDate"))
                .ok()?
                .with_timezone(&Utc);
            (start <= now && now < end && !row.text("_ref").is_empty()).then_some((
                row.reference("Project") == project,
                start,
                row,
            ))
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then(a.2.text("_ref").cmp(&b.2.text("_ref")))
    });
    rows.first().map(|row| row.2.clone())
}
pub fn csv_export(columns: &[String], items: &[Object]) -> Result<Vec<u8>> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer.write_record(columns)?;
    for item in items {
        writer.write_record(columns.iter().map(|key| {
            let value = item.text(key);
            let risky = value
                .trim_start_matches(|c: char| c.is_whitespace() || c.is_control())
                .starts_with(['=', '+', '-', '@']);
            if risky { format!("'{value}") } else { value }
        }))?;
    }
    Ok(writer.into_inner()?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_search_and_backlog_compose() {
        let c = Client::new("https://rally.example", "s".into()).unwrap();
        let mut v = View::new("backlog");
        v.search = "x\" OR true".into();
        v.blocked = true;
        let prefs = super::super::store::Preferences {
            rally_project: "/slm/webservice/v2.0/project/2".into(),
            ..Default::default()
        };
        let q = v.query(&c, &prefs, None).unwrap();
        assert_eq!(q.project, prefs.rally_project);
        assert!(
            q.expression.contains("Iteration = null")
                && q.expression.contains("Blocked = true")
                && q.expression.contains("x\\\"")
        );
        assert_eq!(q.order, "DragAndDropRank ASC");
    }
    #[test]
    fn invalid_saved_filters_do_not_broaden_scope() {
        let c = Client::new("https://rally.example", "s".into()).unwrap();
        let mut v = View::new("teamboard");
        v.filters.push(Filter {
            field: "Workspace".into(),
            value: "bad".into(),
            ..Default::default()
        });
        assert!(v.query(&c, &Default::default(), None).is_err());
    }
    #[test]
    fn export_neutralizes_spreadsheet_formulas() {
        let mut o = Object::new();
        o.insert("Name".into(), " =1+1".into());
        let bytes = csv_export(&["Name".into()], &[o]).unwrap();
        assert!(String::from_utf8(bytes).unwrap().contains("' =1+1"));
    }
}
