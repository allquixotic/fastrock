use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
pub type Object = Map<String, Value>;
pub trait ObjectExt {
    fn text(&self, key: &str) -> String;
    fn reference(&self, key: &str) -> String;
    fn number(&self, key: &str) -> f64;
    fn flag(&self, key: &str) -> bool;
    fn id(&self) -> String;
    fn kind(&self) -> String;
    fn count(&self, key: &str) -> usize;
}
impl ObjectExt for Object {
    fn text(&self, key: &str) -> String {
        match self.get(key) {
            Some(Value::String(v)) => v.clone(),
            Some(Value::Bool(v)) => v.to_string(),
            Some(Value::Number(v)) => v.to_string(),
            Some(Value::Object(v)) => v
                .text("_refObjectName")
                .or_else_string(|| v.text("Name"))
                .or_else_string(|| v.text("_ref")),
            _ => String::new(),
        }
    }
    fn reference(&self, key: &str) -> String {
        match self.get(key) {
            Some(Value::Object(v)) => v.text("_ref"),
            _ => self.text(key),
        }
    }
    fn number(&self, key: &str) -> f64 {
        self.get(key)
            .and_then(Value::as_f64)
            .unwrap_or_else(|| self.text(key).parse().unwrap_or(0.0))
    }
    fn flag(&self, key: &str) -> bool {
        self.get(key).and_then(Value::as_bool).unwrap_or(false)
    }
    fn id(&self) -> String {
        self.text("FormattedID")
            .or_else_string(|| self.text("ObjectID"))
    }
    fn kind(&self) -> String {
        canonical_kind(&self.text("_type"))
            .unwrap_or("HierarchicalRequirement")
            .into()
    }
    fn count(&self, key: &str) -> usize {
        match self.get(key) {
            Some(Value::Array(v)) => v.len(),
            Some(Value::Object(v)) => v.number("Count") as usize,
            _ => 0,
        }
    }
}
trait StringFallback {
    fn or_else_string(self, f: impl FnOnce() -> String) -> String;
}
impl StringFallback for String {
    fn or_else_string(self, f: impl FnOnce() -> String) -> String {
        if self.is_empty() { f() } else { self }
    }
}
pub const ARTIFACT_KINDS: &[&str] = &[
    "HierarchicalRequirement",
    "Defect",
    "Task",
    "PortfolioItem/Feature",
    "PortfolioItem/Epic",
    "TestCase",
    "TestSet",
    "DefectSuite",
];
pub fn canonical_kind(value: &str) -> Option<&'static str> {
    let alias = match value.to_ascii_lowercase().as_str() {
        "story" | "userstory" => "HierarchicalRequirement",
        "feature" => "PortfolioItem/Feature",
        "epic" | "initiative" => "PortfolioItem/Epic",
        _ => value,
    };
    ARTIFACT_KINDS
        .iter()
        .copied()
        .chain([
            "Workspace",
            "Project",
            "Iteration",
            "Release",
            "User",
            "ConversationPost",
            "Attachment",
            "AttachmentContent",
            "Revision",
            "RevisionHistory",
            "State",
            "Tag",
            "TypeDefinition",
            "AllowedAttributeValue",
            "TestFolder",
            "TestCaseResult",
            "PreliminaryEstimate",
            "Milestone",
            "Artifact",
        ])
        .find(|v| v.eq_ignore_ascii_case(alias))
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Query {
    pub expression: String,
    pub fetch: String,
    pub order: String,
    pub workspace: String,
    pub project: String,
    pub parents: bool,
    pub children: bool,
    pub start: usize,
    pub page_size: usize,
    pub artifact_types: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Page {
    pub results: Vec<Object>,
    pub total: usize,
    pub start: usize,
    pub page_size: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub display_name: String,
    pub attribute_type: String,
    pub reference_type: String,
    pub required: bool,
    pub read_only: bool,
    pub allowed_values: Vec<String>,
}
impl Field {
    pub fn label(&self) -> &str {
        if self.display_name.is_empty() {
            &self.name
        } else {
            &self.display_name
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PageSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub section: &'static str,
    pub kind: &'static str,
    pub mode: &'static str,
}
pub const PAGES: &[PageSpec] = &[
    PageSpec {
        id: "myrally",
        title: "My Rally",
        section: "Home",
        kind: "HierarchicalRequirement",
        mode: "dashboard",
    },
    PageSpec {
        id: "backlog",
        title: "Backlog",
        section: "Plan",
        kind: "HierarchicalRequirement",
        mode: "list",
    },
    PageSpec {
        id: "userstories",
        title: "User Stories",
        section: "Plan",
        kind: "HierarchicalRequirement",
        mode: "list",
    },
    PageSpec {
        id: "timeboxes",
        title: "Timeboxes",
        section: "Plan",
        kind: "Iteration",
        mode: "list",
    },
    PageSpec {
        id: "teamplan",
        title: "Team Planning",
        section: "Plan",
        kind: "HierarchicalRequirement",
        mode: "planning",
    },
    PageSpec {
        id: "workviews",
        title: "Work Views",
        section: "Plan",
        kind: "HierarchicalRequirement",
        mode: "list",
    },
    PageSpec {
        id: "iterationstatus",
        title: "Iteration Status",
        section: "Track",
        kind: "HierarchicalRequirement",
        mode: "board",
    },
    PageSpec {
        id: "teamboard",
        title: "Team Board",
        section: "Track",
        kind: "HierarchicalRequirement",
        mode: "board",
    },
    PageSpec {
        id: "teamstatus",
        title: "Team Status",
        section: "Track",
        kind: "HierarchicalRequirement",
        mode: "dashboard",
    },
    PageSpec {
        id: "tasks",
        title: "Tasks",
        section: "Track",
        kind: "Task",
        mode: "list",
    },
    PageSpec {
        id: "defects",
        title: "Defects",
        section: "Quality",
        kind: "Defect",
        mode: "list",
    },
    PageSpec {
        id: "defectsuites",
        title: "Defect Suites",
        section: "Quality",
        kind: "DefectSuite",
        mode: "list",
    },
    PageSpec {
        id: "testcases",
        title: "Test Cases",
        section: "Quality",
        kind: "TestCase",
        mode: "list",
    },
    PageSpec {
        id: "testfolders",
        title: "Test Plan",
        section: "Quality",
        kind: "TestFolder",
        mode: "list",
    },
    PageSpec {
        id: "qualitymanagement",
        title: "Quality Management",
        section: "Quality",
        kind: "TestCase",
        mode: "dashboard",
    },
    PageSpec {
        id: "portfolioitemstreegrid",
        title: "Portfolio Items",
        section: "Portfolio",
        kind: "PortfolioItem/Feature",
        mode: "list",
    },
    PageSpec {
        id: "capacityplanning",
        title: "Capacity Planning",
        section: "Portfolio",
        kind: "HierarchicalRequirement",
        mode: "planning",
    },
    PageSpec {
        id: "timeline",
        title: "Timeline",
        section: "Portfolio",
        kind: "PortfolioItem/Feature",
        mode: "timeline",
    },
    PageSpec {
        id: "releasetracking",
        title: "FY Quarter Tracking",
        section: "Portfolio",
        kind: "Release",
        mode: "list",
    },
    PageSpec {
        id: "portfoliokanban",
        title: "Portfolio Kanban",
        section: "Portfolio",
        kind: "PortfolioItem/Feature",
        mode: "board",
    },
    PageSpec {
        id: "reports",
        title: "Reports",
        section: "Reports",
        kind: "HierarchicalRequirement",
        mode: "dashboard",
    },
    PageSpec {
        id: "customreports",
        title: "Custom Reports",
        section: "Reports",
        kind: "HierarchicalRequirement",
        mode: "dashboard",
    },
    PageSpec {
        id: "insights",
        title: "Insights",
        section: "Reports",
        kind: "HierarchicalRequirement",
        mode: "dashboard",
    },
    PageSpec {
        id: "customviews",
        title: "Custom Views",
        section: "Reports",
        kind: "HierarchicalRequirement",
        mode: "list",
    },
    PageSpec {
        id: "projects",
        title: "Teams",
        section: "Home",
        kind: "Project",
        mode: "list",
    },
    PageSpec {
        id: "users",
        title: "Users",
        section: "Home",
        kind: "User",
        mode: "list",
    },
    PageSpec {
        id: "mywork",
        title: "My Work Items",
        section: "Home",
        kind: "HierarchicalRequirement",
        mode: "list",
    },
];
pub fn page(id: &str) -> PageSpec {
    PAGES
        .iter()
        .copied()
        .find(|p| p.id == id)
        .unwrap_or(PAGES[7])
}
pub fn state_field(kind: &str) -> &'static str {
    match kind {
        "TestCase" => "LastVerdict",
        "Task"
        | "Defect"
        | "PortfolioItem/Feature"
        | "PortfolioItem/Epic"
        | "Iteration"
        | "Release" => "State",
        _ => "ScheduleState",
    }
}
pub fn quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}
pub fn eq(field: &str, value: &str) -> String {
    format!("({field} = {})", quote(value))
}
pub fn and(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.into(),
        (_, true) => a.into(),
        _ => format!("({a} AND {b})"),
    }
}
