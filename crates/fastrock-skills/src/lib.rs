#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    pub path: String,
    pub mode_slugs: Vec<String>,
    pub enabled: bool,
    pub scope: SkillScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillScope {
    Global,
    Project(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDocument {
    pub manifest: SkillManifest,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillPromptInjection {
    pub skill_name: String,
    pub content: String,
    pub full_body: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillWriteRequest {
    pub name: String,
    pub description: String,
    pub mode_slugs: Vec<String>,
    pub enabled: bool,
    pub body: String,
}

impl SkillWriteRequest {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            mode_slugs: Vec::new(),
            enabled: true,
            body: body.into(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SkillError {
    #[error("skill file missing frontmatter delimiter")]
    MissingFrontmatter,
    #[error("skill manifest missing field: {0}")]
    MissingField(&'static str),
    #[error("skill manifest has invalid field {field}: {reason}")]
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    #[error("skill already exists: {0}")]
    AlreadyExists(String),
    #[error("skill not found: {0}")]
    NotFound(String),
    #[error("skill I/O error: {0}")]
    Io(String),
}

pub fn discover_skills(
    root: impl AsRef<Path>,
    scope: SkillScope,
) -> Result<Vec<SkillDocument>, SkillError> {
    let mut skills = Vec::new();
    discover_skills_inner(root.as_ref(), &scope, &mut skills)?;
    skills.sort_by(|left, right| left.manifest.name.cmp(&right.manifest.name));
    Ok(skills)
}

pub fn skill_prompt_injections<'a>(
    skills: impl IntoIterator<Item = &'a SkillDocument>,
    mode_slug: &str,
) -> Vec<SkillPromptInjection> {
    skill_prompt_injections_for_project(skills, mode_slug, None)
}

pub fn skill_prompt_injections_for_project<'a>(
    skills: impl IntoIterator<Item = &'a SkillDocument>,
    mode_slug: &str,
    project_id: Option<&str>,
) -> Vec<SkillPromptInjection> {
    resolve_available_skills(skills, mode_slug, project_id)
        .map(|skill| SkillPromptInjection {
            skill_name: skill.manifest.name.clone(),
            content: format!(
                "Skill: {}\nDescription: {}\nModes: {}\nSource: {}",
                skill.manifest.name,
                skill.manifest.description,
                if skill.manifest.mode_slugs.is_empty() {
                    "*".to_owned()
                } else {
                    skill.manifest.mode_slugs.join(",")
                },
                skill.manifest.path
            ),
            full_body: false,
        })
        .collect()
}

pub fn selected_skill_prompt_injections_for_project<'a>(
    skills: impl IntoIterator<Item = &'a SkillDocument>,
    mode_slug: &str,
    project_id: Option<&str>,
    selected_names: impl IntoIterator<Item = &'a str>,
) -> Vec<SkillPromptInjection> {
    let selected_names = selected_names.into_iter().collect::<BTreeSet<_>>();
    resolve_available_skills(skills, mode_slug, project_id)
        .filter(|skill| selected_names.contains(skill.manifest.name.as_str()))
        .map(|skill| SkillPromptInjection {
            skill_name: skill.manifest.name.clone(),
            content: format!(
                "Selected Skill: {}\nDescription: {}\n\n{}",
                skill.manifest.name, skill.manifest.description, skill.body
            ),
            full_body: true,
        })
        .collect()
}

pub fn create_skill(
    skills_root: impl AsRef<Path>,
    scope: SkillScope,
    request: SkillWriteRequest,
) -> Result<SkillDocument, SkillError> {
    validate_write_request(&request)?;
    let skill_dir = skill_directory(skills_root.as_ref(), &request.name)?;
    if skill_dir.exists() {
        return Err(SkillError::AlreadyExists(request.name));
    }
    write_skill_file(&skill_dir, &request)?;
    load_skill_from_dir(skill_dir, scope)
}

pub fn edit_skill(
    skills_root: impl AsRef<Path>,
    scope: SkillScope,
    request: SkillWriteRequest,
) -> Result<SkillDocument, SkillError> {
    validate_write_request(&request)?;
    let skill_dir = skill_directory(skills_root.as_ref(), &request.name)?;
    if !skill_dir.join("SKILL.md").exists() {
        return Err(SkillError::NotFound(request.name));
    }
    write_skill_file(&skill_dir, &request)?;
    load_skill_from_dir(skill_dir, scope)
}

pub fn delete_skill(skills_root: impl AsRef<Path>, name: &str) -> Result<bool, SkillError> {
    validate_skill_name(name)?;
    let skill_dir = skill_directory(skills_root.as_ref(), name)?;
    if !skill_dir.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&skill_dir).map_err(|error| SkillError::Io(error.to_string()))?;
    Ok(true)
}

pub fn move_skill(
    source_root: impl AsRef<Path>,
    destination_root: impl AsRef<Path>,
    destination_scope: SkillScope,
    name: &str,
) -> Result<SkillDocument, SkillError> {
    validate_skill_name(name)?;
    let source_dir = skill_directory(source_root.as_ref(), name)?;
    let source_path = source_dir.join("SKILL.md");
    if !source_path.exists() {
        return Err(SkillError::NotFound(name.to_owned()));
    }
    let destination_dir = skill_directory(destination_root.as_ref(), name)?;
    if destination_dir.exists() {
        return Err(SkillError::AlreadyExists(name.to_owned()));
    }

    let contents =
        std::fs::read_to_string(&source_path).map_err(|error| SkillError::Io(error.to_string()))?;
    let source_document = parse_skill_document(&source_path, &contents, SkillScope::Global)?;
    let request = SkillWriteRequest {
        name: source_document.manifest.name,
        description: source_document.manifest.description,
        mode_slugs: source_document.manifest.mode_slugs,
        enabled: source_document.manifest.enabled,
        body: source_document.body,
    };
    let destination_document = create_skill(destination_root, destination_scope, request)?;
    std::fs::remove_dir_all(source_dir).map_err(|error| SkillError::Io(error.to_string()))?;
    Ok(destination_document)
}

pub fn resolve_available_skills<'a>(
    skills: impl IntoIterator<Item = &'a SkillDocument>,
    mode_slug: &str,
    project_id: Option<&str>,
) -> impl Iterator<Item = &'a SkillDocument> {
    let mut selected = BTreeMap::<String, (SkillResolutionRank, &'a SkillDocument)>::new();
    for skill in skills {
        let Some(rank) = resolution_rank(skill, mode_slug, project_id) else {
            continue;
        };
        selected
            .entry(skill.manifest.name.clone())
            .and_modify(|(existing_rank, existing_skill)| {
                if rank > *existing_rank
                    || (rank == *existing_rank
                        && skill.manifest.path < existing_skill.manifest.path)
                {
                    *existing_rank = rank;
                    *existing_skill = skill;
                }
            })
            .or_insert((rank, skill));
    }
    selected
        .into_values()
        .map(|(_, skill)| skill)
        .collect::<Vec<_>>()
        .into_iter()
}

pub fn parse_skill_document(
    path: impl Into<PathBuf>,
    contents: &str,
    scope: SkillScope,
) -> Result<SkillDocument, SkillError> {
    let path = path.into();
    let (frontmatter, body) = split_frontmatter(contents)?;
    let mut name = None;
    let mut description = None;
    let mut modes = Vec::new();
    let mut enabled = true;

    for line in frontmatter.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "name" => name = Some(unquote(value).to_owned()),
            "description" => description = Some(unquote(value).to_owned()),
            "modes" | "mode_slugs" => modes = parse_string_list(value),
            "enabled" => enabled = !matches!(value, "false" | "False" | "0"),
            _ => {}
        }
    }

    let manifest = SkillManifest {
        name: required_non_empty(name, "name")?,
        description: required_non_empty(description, "description")?,
        path: path.to_string_lossy().into_owned(),
        mode_slugs: modes,
        enabled,
        scope,
    };
    validate_manifest(&manifest)?;

    Ok(SkillDocument {
        manifest,
        body: body.trim().to_owned(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SkillResolutionRank {
    scope: u8,
    mode: u8,
}

fn resolution_rank(
    skill: &SkillDocument,
    mode_slug: &str,
    project_id: Option<&str>,
) -> Option<SkillResolutionRank> {
    if !skill.manifest.enabled {
        return None;
    }
    let scope = match (&skill.manifest.scope, project_id) {
        (SkillScope::Project(skill_project), Some(project_id)) if skill_project == project_id => 1,
        (SkillScope::Project(_), _) => return None,
        (SkillScope::Global, _) => 0,
    };
    let mode = if skill.manifest.mode_slugs.is_empty() {
        0
    } else if skill
        .manifest
        .mode_slugs
        .iter()
        .any(|slug| slug == mode_slug)
    {
        1
    } else if skill.manifest.mode_slugs.iter().any(|slug| slug == "*") {
        0
    } else {
        return None;
    };
    Some(SkillResolutionRank { scope, mode })
}

fn discover_skills_inner(
    root: &Path,
    scope: &SkillScope,
    skills: &mut Vec<SkillDocument>,
) -> Result<(), SkillError> {
    for entry in std::fs::read_dir(root).map_err(|error| SkillError::Io(error.to_string()))? {
        let entry = entry.map_err(|error| SkillError::Io(error.to_string()))?;
        let path = entry.path();
        if path.is_dir() {
            discover_skills_inner(&path, scope, skills)?;
        } else if path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
            let contents = std::fs::read_to_string(&path)
                .map_err(|error| SkillError::Io(error.to_string()))?;
            skills.push(parse_skill_document(path, &contents, scope.clone())?);
        }
    }
    Ok(())
}

fn validate_write_request(request: &SkillWriteRequest) -> Result<(), SkillError> {
    validate_skill_name(&request.name)?;
    required_non_empty(Some(request.description.clone()), "description")?;
    if request.description.chars().count() > 1024 {
        return Err(SkillError::InvalidField {
            field: "description",
            reason: "description must be at most 1024 characters",
        });
    }
    if request.description.contains('\n') {
        return Err(SkillError::InvalidField {
            field: "description",
            reason: "description cannot contain newlines",
        });
    }
    if request
        .mode_slugs
        .iter()
        .any(|slug| slug.trim().is_empty() || slug.contains('\n') || slug.contains(','))
    {
        return Err(SkillError::InvalidField {
            field: "mode_slugs",
            reason: "mode slug cannot be empty or contain newline/comma",
        });
    }
    Ok(())
}

fn validate_skill_name(name: &str) -> Result<(), SkillError> {
    required_non_empty(Some(name.to_owned()), "name")?;
    if matches!(name, "." | "..")
        || name.contains('/')
        || name.contains('\\')
        || name.contains(std::path::MAIN_SEPARATOR)
    {
        return Err(SkillError::InvalidField {
            field: "name",
            reason: "name must be a single folder name",
        });
    }
    Ok(())
}

fn skill_directory(root: &Path, name: &str) -> Result<PathBuf, SkillError> {
    validate_skill_name(name)?;
    Ok(root.join(name))
}

fn load_skill_from_dir(skill_dir: PathBuf, scope: SkillScope) -> Result<SkillDocument, SkillError> {
    let skill_path = skill_dir.join("SKILL.md");
    let contents =
        std::fs::read_to_string(&skill_path).map_err(|error| SkillError::Io(error.to_string()))?;
    parse_skill_document(skill_path, &contents, scope)
}

fn write_skill_file(skill_dir: &Path, request: &SkillWriteRequest) -> Result<(), SkillError> {
    std::fs::create_dir_all(skill_dir).map_err(|error| SkillError::Io(error.to_string()))?;
    let skill_path = skill_dir.join("SKILL.md");
    let tmp_path = skill_dir.join(".SKILL.md.tmp");
    std::fs::write(&tmp_path, render_skill_document(request))
        .map_err(|error| SkillError::Io(error.to_string()))?;
    std::fs::rename(&tmp_path, &skill_path).map_err(|error| SkillError::Io(error.to_string()))?;
    Ok(())
}

fn render_skill_document(request: &SkillWriteRequest) -> String {
    let mut document = String::new();
    document.push_str("---\n");
    document.push_str(&format!("name: {}\n", request.name));
    document.push_str(&format!("description: {}\n", request.description));
    if !request.mode_slugs.is_empty() {
        document.push_str(&format!("modes: [{}]\n", request.mode_slugs.join(", ")));
    }
    if !request.enabled {
        document.push_str("enabled: false\n");
    }
    document.push_str("---\n");
    document.push_str(request.body.trim());
    document.push('\n');
    document
}

fn validate_manifest(manifest: &SkillManifest) -> Result<(), SkillError> {
    if manifest.description.chars().count() > 1024 {
        return Err(SkillError::InvalidField {
            field: "description",
            reason: "description must be at most 1024 characters",
        });
    }
    if let Some(parent_name) = Path::new(&manifest.path)
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        && parent_name != manifest.name
    {
        return Err(SkillError::InvalidField {
            field: "name",
            reason: "name must match parent directory",
        });
    }
    if manifest
        .mode_slugs
        .iter()
        .any(|slug| slug.trim().is_empty())
    {
        return Err(SkillError::InvalidField {
            field: "mode_slugs",
            reason: "mode slug cannot be empty",
        });
    }
    Ok(())
}

fn split_frontmatter(contents: &str) -> Result<(&str, &str), SkillError> {
    let contents = contents
        .strip_prefix("---")
        .ok_or(SkillError::MissingFrontmatter)?;
    let Some((frontmatter, body)) = contents.split_once("---") else {
        return Err(SkillError::MissingFrontmatter);
    };
    Ok((frontmatter, body))
}

fn required_non_empty(value: Option<String>, field: &'static str) -> Result<String, SkillError> {
    let value = value.ok_or(SkillError::MissingField(field))?;
    if value.trim().is_empty() {
        return Err(SkillError::InvalidField {
            field,
            reason: "value cannot be empty",
        });
    }
    Ok(value)
}

fn parse_string_list(value: &str) -> Vec<String> {
    let value = value.trim();
    let value = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(value);
    value
        .split(',')
        .map(unquote)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn unquote(value: &str) -> &str {
    value.trim().trim_matches('"').trim_matches('\'')
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn t19_parses_and_validates_skill_manifest() {
        let document = parse_skill_document(
            "/skills/rust/SKILL.md",
            r#"---
name: rust
description: Rust project guidance
modes: [code, debug]
enabled: true
---
Use cargo fmt and cargo test.
"#,
            SkillScope::Global,
        )
        .unwrap();

        assert_eq!(document.manifest.name, "rust");
        assert_eq!(document.manifest.mode_slugs, vec!["code", "debug"]);
        assert!(document.body.contains("cargo test"));
    }

    #[test]
    fn t19_rejects_missing_required_manifest_fields() {
        assert_eq!(
            parse_skill_document(
                "SKILL.md",
                "---\ndescription: missing name\n---\nBody",
                SkillScope::Global,
            ),
            Err(SkillError::MissingField("name"))
        );
    }

    #[test]
    fn t19_discovers_skill_files_recursively() {
        let temp_dir = tempdir().unwrap();
        let skill_dir = temp_dir.path().join("rust");
        std::fs::create_dir(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: rust\ndescription: Rust\n---\nBody",
        )
        .unwrap();

        let skills =
            discover_skills(temp_dir.path(), SkillScope::Project("p1".to_owned())).unwrap();

        assert_eq!(skills.len(), 1);
        assert_eq!(
            skills[0].manifest.scope,
            SkillScope::Project("p1".to_owned())
        );
    }

    #[test]
    fn t19_prompt_injection_filters_by_mode_and_enabled() {
        let code = parse_skill_document(
            "code/SKILL.md",
            "---\nname: code\ndescription: Code\nmodes: [code]\n---\nCode body",
            SkillScope::Global,
        )
        .unwrap();
        let plan = parse_skill_document(
            "plan/SKILL.md",
            "---\nname: plan\ndescription: Plan\nmodes: [plan]\n---\nPlan body",
            SkillScope::Global,
        )
        .unwrap();

        let injections = skill_prompt_injections([&code, &plan], "code");

        assert_eq!(injections.len(), 1);
        assert_eq!(injections[0].skill_name, "code");
        assert!(injections[0].content.contains("Description: Code"));
        assert!(!injections[0].content.contains("Code body"));
        assert!(!injections[0].full_body);
    }

    #[test]
    fn t19_selected_skill_injection_loads_full_body_only_for_selected_skill() {
        let code = parse_skill_document(
            "code/SKILL.md",
            "---\nname: code\ndescription: Code\nmodes: [code]\n---\nCode body",
            SkillScope::Global,
        )
        .unwrap();
        let debug = parse_skill_document(
            "debug/SKILL.md",
            "---\nname: debug\ndescription: Debug\nmodes: [debug]\n---\nDebug body",
            SkillScope::Global,
        )
        .unwrap();

        let injections =
            selected_skill_prompt_injections_for_project([&code, &debug], "code", None, ["code"]);

        assert_eq!(injections.len(), 1);
        assert_eq!(injections[0].skill_name, "code");
        assert!(injections[0].content.contains("Code body"));
        assert!(injections[0].full_body);
    }

    #[test]
    fn t19_project_and_mode_specific_skills_override_generic_global_skills() {
        let global_generic = parse_skill_document(
            "rust/SKILL.md",
            "---\nname: rust\ndescription: Global Rust\n---\nGlobal body",
            SkillScope::Global,
        )
        .unwrap();
        let project_generic = parse_skill_document(
            "rust/SKILL.md",
            "---\nname: rust\ndescription: Project Rust\n---\nProject generic body",
            SkillScope::Project("project-1".to_owned()),
        )
        .unwrap();
        let project_code = parse_skill_document(
            "rust/SKILL.md",
            "---\nname: rust\ndescription: Project Rust Code\nmodes: [code]\n---\nProject code body",
            SkillScope::Project("project-1".to_owned()),
        )
        .unwrap();

        let resolved = resolve_available_skills(
            [&global_generic, &project_generic, &project_code],
            "code",
            Some("project-1"),
        )
        .collect::<Vec<_>>();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].manifest.description, "Project Rust Code");
    }

    #[test]
    fn t19_skill_name_must_match_parent_directory() {
        assert_eq!(
            parse_skill_document(
                "wrong/SKILL.md",
                "---\nname: rust\ndescription: Rust\n---\nBody",
                SkillScope::Global,
            ),
            Err(SkillError::InvalidField {
                field: "name",
                reason: "name must match parent directory",
            })
        );
    }

    #[test]
    fn t19_create_edit_delete_skill_files_for_gui_management() {
        let temp_dir = tempdir().unwrap();
        let mut request = SkillWriteRequest::new("rust", "Rust guidance", "Use cargo test.");
        request.mode_slugs = vec!["code".to_owned()];

        let created = create_skill(temp_dir.path(), SkillScope::Global, request.clone()).unwrap();

        assert_eq!(created.manifest.name, "rust");
        assert_eq!(created.manifest.mode_slugs, vec!["code"]);
        assert!(temp_dir.path().join("rust").join("SKILL.md").exists());
        assert_eq!(
            create_skill(temp_dir.path(), SkillScope::Global, request.clone()),
            Err(SkillError::AlreadyExists("rust".to_owned()))
        );

        request.description = "Rust debug guidance".to_owned();
        request.mode_slugs = vec!["debug".to_owned()];
        let edited = edit_skill(temp_dir.path(), SkillScope::Global, request).unwrap();

        assert_eq!(edited.manifest.description, "Rust debug guidance");
        assert_eq!(edited.manifest.mode_slugs, vec!["debug"]);
        assert!(delete_skill(temp_dir.path(), "rust").unwrap());
        assert!(!delete_skill(temp_dir.path(), "rust").unwrap());
        assert!(!temp_dir.path().join("rust").exists());
    }

    #[test]
    fn t19_move_skill_between_global_and_project_roots_preserves_metadata() {
        let temp_dir = tempdir().unwrap();
        let global_root = temp_dir.path().join("global");
        let project_root = temp_dir.path().join("project");
        let mut request = SkillWriteRequest::new("deploy", "Deploy guidance", "Use rtk.");
        request.mode_slugs = vec!["code".to_owned(), "debug".to_owned()];

        create_skill(&global_root, SkillScope::Global, request).unwrap();
        let moved = move_skill(
            &global_root,
            &project_root,
            SkillScope::Project("project-1".to_owned()),
            "deploy",
        )
        .unwrap();

        assert!(!global_root.join("deploy").exists());
        assert!(project_root.join("deploy").join("SKILL.md").exists());
        assert_eq!(
            moved.manifest.scope,
            SkillScope::Project("project-1".to_owned())
        );
        assert_eq!(moved.manifest.mode_slugs, vec!["code", "debug"]);
        assert!(moved.body.contains("rtk"));
    }

    #[test]
    fn t19_skill_management_rejects_path_traversal_names() {
        let temp_dir = tempdir().unwrap();
        let request = SkillWriteRequest::new("../escape", "Escape", "Nope.");

        assert_eq!(
            create_skill(temp_dir.path(), SkillScope::Global, request),
            Err(SkillError::InvalidField {
                field: "name",
                reason: "name must be a single folder name",
            })
        );
    }
}
