//! Native recipe documents use Goose's public schema and file writer.
use anyhow::{Context, Result, ensure};
use goose_runtime::recipe::{Recipe, RecipeParameterInputType as Kind};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

pub(super) const LIMIT: usize = 1024 * 1024;
pub(super) fn read(path: &Path) -> Result<Value> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= LIMIT, "Recipe exceeds 1 MiB");
    let revision = format!("{:x}", Sha256::digest(&bytes));
    let content = String::from_utf8(bytes).context("Recipe must be UTF-8")?;
    Ok(json!({"content":content,"revision":revision}))
}
pub(super) fn parse(content: &str, dir: Option<&Path>) -> Result<Recipe> {
    ensure!(content.len() <= LIMIT, "Recipe exceeds 1 MiB");
    let recipe = goose_runtime::recipe::validate_recipe::validate_recipe_template_from_content(
        content,
        dir.map(|p| p.to_string_lossy().into_owned()),
    )?;
    ensure!(
        !recipe.check_for_security_warnings(),
        "Remove hidden control characters from the recipe"
    );
    Ok(recipe)
}
pub(super) fn backup(path: &Path, expected: &str) -> Result<()> {
    let before = read(path)?;
    ensure!(
        before["revision"] == expected,
        "Recipe changed; reload before saving or running"
    );
    let name = path
        .file_name()
        .context("Recipe path has no name")?
        .to_string_lossy();
    let backup = path.with_file_name(format!("{name}.fastrock-backup-{expected}"));
    if !backup.exists() {
        // Public Goose writer guards replacement symlinks; backup retains exact original bytes.
        crate::rally::store::atomic_write(&backup, before["content"].as_str().unwrap().as_bytes())?;
    }
    Ok(())
}
pub(super) fn render(content: &str, dir: &Path, values: &Value) -> Result<Recipe> {
    let recipe = parse(content, Some(dir))?;
    let values = values
        .as_object()
        .context("Recipe values must be a mapping")?;
    ensure!(values.len() <= 32, "At most 32 recipe parameters");
    let parameters = recipe.parameters.as_deref().unwrap_or_default();
    ensure!(
        values
            .keys()
            .all(|k| parameters.iter().any(|p| &p.key == k)),
        "Unknown recipe parameter"
    );
    let mut args = Vec::new();
    for parameter in parameters {
        let value = values
            .get(&parameter.key)
            .and_then(Value::as_str)
            .or(parameter.default.as_deref())
            .unwrap_or("");
        ensure!(
            value.len() <= 128 * 1024 && !value.contains('\0'),
            "Invalid or oversized recipe parameter"
        );
        ensure!(
            !value.is_empty()
                || matches!(
                    parameter.requirement,
                    goose_runtime::recipe::RecipeParameterRequirement::Optional
                ),
            "Enter a value for {}",
            parameter.key
        );
        if !value.is_empty() {
            match parameter.input_type {
                Kind::Number => {
                    ensure!(
                        value.parse::<f64>().is_ok_and(f64::is_finite),
                        "{} must be a number",
                        parameter.key
                    );
                }
                Kind::Boolean => {
                    ensure!(
                        matches!(value, "true" | "false"),
                        "{} must be true or false",
                        parameter.key
                    );
                }
                Kind::Date => {
                    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                        .context("Use YYYY-MM-DD for dates")?;
                }
                Kind::Select => {
                    ensure!(
                        parameter
                            .options
                            .as_ref()
                            .is_some_and(|o| o.iter().any(|v| v == value)),
                        "Choose a listed option for {}",
                        parameter.key
                    );
                }
                Kind::File => {
                    let path = dir.join(value);
                    ensure!(
                        std::fs::metadata(&path)?.len() <= LIMIT as u64,
                        "Recipe parameter file exceeds 1 MiB"
                    );
                    args.push((parameter.key.clone(), path.to_string_lossy().into_owned()));
                    continue;
                }
                Kind::String => {}
            }
        }
        args.push((parameter.key.clone(), value.to_owned()));
    }
    let mut rendered = goose_runtime::recipe::build_recipe::build_recipe_from_template(
        content.to_owned(),
        dir,
        args,
        None::<fn(&str, &str) -> Result<String>>,
    )?;
    rendered.parameters = None;
    // ACP renders even an already supplied recipe. Prove this second pass is an
    // identity before activating providers/extensions; never silently expand a
    // template expression introduced by a parameter or file's contents.
    let yaml = rendered.to_yaml()?;
    ensure!(yaml.len() <= LIMIT, "Rendered recipe exceeds 1 MiB");
    let second = goose_runtime::recipe::build_recipe::build_recipe_from_template(
        yaml,
        dir,
        vec![],
        None::<fn(&str, &str) -> Result<String>>,
    )
    .context("Goose cannot safely reopen template syntax introduced by these parameter values")?;
    ensure!(
        serde_json::to_value(&rendered)? == serde_json::to_value(second)?,
        "Goose would reinterpret template syntax in these parameter values"
    );
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    const RECIPE: &str = "title: Fixture\ndescription: Recipe test\ninstructions: |\n  Read {{ topic }}\nprompt: Explain {{ topic }}\nparameters:\n- key: topic\n  input_type: string\n  requirement: required\n  description: Subject\n";
    #[test]
    fn v66_recipe_render_validates_values_and_preserves_template() {
        let dir = tempfile::tempdir().unwrap();
        let r = render(RECIPE, dir.path(), &json!({"topic":"fjord"})).unwrap();
        assert_eq!(r.prompt.as_deref(), Some("Explain fjord"));
        assert!(r.parameters.is_none());
        assert!(render(RECIPE, dir.path(), &json!({})).is_err());
        assert!(render(RECIPE, dir.path(), &json!({"topic":"{{ missing }}"})).is_err());
        assert!(render(RECIPE, dir.path(), &json!({"topic":"ok","extra":"x"})).is_err());
        let number = RECIPE.replace("input_type: string", "input_type: number");
        assert!(render(&number, dir.path(), &json!({"topic":"NaN"})).is_err());
    }
    #[test]
    fn v66_recipe_stale_guard_and_backup_keep_original_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recipe.yaml");
        std::fs::write(&path, RECIPE).unwrap();
        let old = read(&path).unwrap();
        backup(&path, old["revision"].as_str().unwrap()).unwrap();
        std::fs::write(&path, "changed").unwrap();
        assert!(backup(&path, old["revision"].as_str().unwrap()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "changed");
        assert!(std::fs::read_dir(dir.path()).unwrap().flatten().any(|e| {
            e.file_name()
                .to_string_lossy()
                .contains(".fastrock-backup-")
                && std::fs::read_to_string(e.path()).unwrap() == RECIPE
        }));
    }
}
