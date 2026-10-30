use super::recipe_storage as storage;
use super::*;
use goose_runtime::recipe::Recipe;
use std::path::PathBuf;

impl Adapter {
    pub(super) async fn recipe_request(self: &Arc<Self>, params: &Value, action: &str) -> Reply {
        if !self.embedded_runtime {
            return Err(unsupported("Recipes"));
        }
        static EDITS: Mutex<()> = Mutex::const_new(());
        let _edit = EDITS.lock().await;
        let catalog = self
            .rpc
            .call("_goose/unstable/recipes/list", json!({}))
            .await?;
        let rows = catalog["recipes"]
            .as_array()
            .ok_or_else(|| error("Invalid Goose recipe catalog"))?;
        if rows.len() > 512 {
            return Err(error("Recipe library exceeds 512 entries"));
        }
        if action == "list" {
            let entries:Vec<_> = rows.iter().map(|r|json!({"id":r["id"],"title":r["recipe"]["title"],"description":r["recipe"]["description"],"path":r["file_path"]})).collect();
            return Ok(json!({"recipes":entries,"projects":self.project_catalog().await?}));
        }
        let id = text(params, "id");
        let path = if id.is_empty() {
            None
        } else {
            Some(PathBuf::from(
                rows.iter()
                    .find(|r| r["id"] == id)
                    .and_then(|r| r["file_path"].as_str())
                    .ok_or_else(|| error("Recipe no longer exists; reload"))?,
            ))
        };
        let p = params.clone();
        let action_owned = action.to_owned();
        let doc=tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
            let current=path.as_ref().map(|p|storage::read(p)).transpose()?;
            if action_owned=="read" {
                let mut doc=current.context("Choose a saved recipe")?;
                let recipe=storage::parse(doc["content"].as_str().unwrap(),path.as_deref().and_then(|p|p.parent()))?;
                doc["parameters"]=serde_json::to_value(recipe.parameters.unwrap_or_default())?;
                doc["id"]=p["id"].clone();return Ok(doc);
            }
            if let Some(current)=&current {
                anyhow::ensure!(current["revision"]==p["revision"],"Recipe changed; reload before saving or running");
            }
            match action_owned.as_str() {
                "save"=>{
                    let recipe=storage::parse(text(&p,"content"),path.as_deref().and_then(|p|p.parent()))?;
                    if let Some(path)=&path { storage::backup(path,text(&p,"revision"))?; }
                    let path=goose_runtime::recipe::local_recipes::save_recipe_to_file(recipe,path)?;
                    Ok(json!({"id":goose_runtime::recipe::manifest::short_id_from_path(&path.to_string_lossy())}))
                },
                "delete"=>{
                    let path=path.context("Choose a saved recipe")?;
                    anyhow::ensure!(p["confirmed"]==true,"Confirm recipe deletion first");
                    storage::backup(&path,text(&p,"revision"))?;
                    Ok(json!({"id":p["id"]}))
                },
                "prepare"=>{
                    let path=path.context("Save the recipe before preparing a conversation")?;
                    let recipe=storage::render(current.as_ref().unwrap()["content"].as_str().unwrap(),path.parent().unwrap(),&p["values"])?;
                    Ok(serde_json::to_value(recipe)?)
                },
                _=>anyhow::bail!("Unknown recipe action"),
            }
        }).await.map_err(|_|error("Recipe worker stopped"))?.map_err(|e|error(e.to_string()))?;
        if action == "delete" {
            return self
                .rpc
                .call("_goose/unstable/recipes/delete", json!({"id":id}))
                .await;
        }
        if action != "prepare" {
            return Ok(doc);
        }
        let recipe: Recipe =
            serde_json::from_value(doc.clone()).map_err(|e| error(e.to_string()))?;
        validate_provider(&recipe).await?;
        let project = self.project_catalog().await?;
        let project = project
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["id"] == params["project"])
            .ok_or_else(|| error("Choose a registered repository first"))?;
        let workspace = self
            .open_project(&json!({"path":project["root"],"projectId":project["id"]}))
            .await?;
        let response=self.new_session(&json!({"cwd":workspace["cwd"],"config":{"fastrock.workspace":workspace["selection"],"fastrock.recipe":doc},"dynamicTools":params["dynamicTools"]})).await?;
        Ok(
            json!({"response":response,"prompt":recipe.prompt.unwrap_or_default(),"workspace":workspace["selection"]}),
        )
    }
}
use anyhow::Context;
async fn validate_provider(recipe: &Recipe) -> Result<(), JSONRPCErrorError> {
    super::provider_policy::selected(
        recipe
            .settings
            .as_ref()
            .and_then(|s| s.goose_provider.clone()),
    )
    .await
    .map(|_| ())
    .map_err(|e| error(e.to_string()))
}
/// Attach mandatory in-process tools even when Goose's recipe extension list
/// replaces the ordinary MCP selection. Never launch a legacy vwork command.
pub(super) async fn session_recipe(
    params: &Value,
    servers: &[Value],
) -> Result<Option<String>, JSONRPCErrorError> {
    let value = &params["config"]["fastrock.recipe"];
    if value.is_null() {
        return Ok(None);
    }
    let mut recipe: Recipe =
        serde_json::from_value(value.clone()).map_err(|_| error("Invalid prepared recipe"))?;
    validate_provider(&recipe).await?;
    if let Some(extensions) = &mut recipe.extensions {
        extensions.retain(|extension| {
            let value = serde_json::to_value(extension).unwrap_or_default();
            !super::capability_config::managed_extension(
                value["name"].as_str().unwrap_or(""),
                &value,
            ) && !servers.iter().any(|s| {
                goose_runtime::config::extensions::name_to_key(text(s, "name"))
                    == goose_runtime::config::extensions::name_to_key(text(&value, "name"))
            })
        });
        for server in servers {
            let extension=serde_json::from_value(json!({"type":"streamable_http","name":server["name"],"uri":server["url"],"timeout":300})).map_err(|_|error("Invalid embedded recipe tool binding"))?;
            extensions.push(extension);
        }
    }
    goose_runtime::recipe_deeplink::encode(&recipe)
        .map(Some)
        .map_err(|e| error(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn v66_recipe_extensions_retain_embedded_bindings_and_provider_scope() {
        let recipe = json!({"title":"Fixture","description":"Fixture","instructions":"Read source", "settings":{"goose_provider":"aws_bedrock"},"extensions":[
            {"type":"stdio","name":"old-source","cmd":"vwork","args":["mcp"]},
            {"type":"platform","name":"code_execution"},
            {"type":"streamable_http","name":"Source","uri":"https://example.invalid/mcp"}
        ]});
        let params = json!({"config":{"fastrock.recipe":recipe}});
        let servers = vec![
            json!({"type":"http","name":"source","url":"http://127.0.0.1:1/owned","headers":[]}),
        ];
        let link = session_recipe(&params, &servers).await.unwrap().unwrap();
        let stored = goose_runtime::recipe_deeplink::decode(&link).unwrap();
        let extensions = serde_json::to_value(stored.extensions.unwrap()).unwrap();
        let rows = extensions.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|e| e["name"] == "code_execution"));
        assert!(
            rows.iter()
                .any(|e| e["name"] == "source" && e["uri"] == "http://127.0.0.1:1/owned")
        );
        let mut rejected = params;
        rejected["config"]["fastrock.recipe"]["settings"]["goose_provider"] = json!("ollama");
        assert!(session_recipe(&rejected, &servers).await.is_err());
    }
}
