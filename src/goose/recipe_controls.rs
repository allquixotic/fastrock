use crate::{
    app::AppController,
    ui::{RecipeParameter, RecipeRow, RecipeState},
};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
#[derive(Default)]
pub(crate) struct Controller {
    catalog: Value,
    document: Value,
}
impl AppController {
    pub(super) fn recipes_install(&mut self) {
        let state = self.window.global::<RecipeState>();
        state.on_action(crate::ui_thread::scoped_callback!(|action| {
            let action = action.to_string();
            crate::ui_thread::with_app(move |app| app.recipes_action(&action));
        }));
        state.on_select(crate::ui_thread::scoped_callback!(|id| {
            let id = id.to_string();
            crate::ui_thread::with_app(move |app| {
                let state = app.window.global::<RecipeState>();
                if !state.get_dirty() && !state.get_busy() {
                    app.recipes_request("read", json!({"id":id}));
                }
            });
        }));
    }
    pub(crate) fn recipes_load(&mut self) {
        self.recipes_action("list");
    }
    fn recipes_action(&mut self, action: &str) {
        let state = self.window.global::<RecipeState>();
        if state.get_busy() {
            return;
        }
        if state.get_dirty() && !matches!(action, "save" | "discard") {
            state.set_status("Save or discard recipe edits first.".into());
            return;
        }
        let doc = &self.settings.recipes.document;
        match action {
            "new"=>{
                self.settings.recipes.document=json!({});state.set_selected("".into());state.set_editing(true);state.set_deleting(false);state.set_dirty(true);
                state.set_content("title: New recipe\ndescription: Describe this reusable task\ninstructions: |\n  Follow the repository instructions.\nprompt: |\n  Describe the task to perform.\n".into());
                state.set_parameters(ModelRc::default());state.set_status("".into());
            },
            "discard"=>{state.set_dirty(false);let id=doc["id"].as_str().unwrap_or("").to_owned();if id.is_empty(){state.set_editing(false);state.set_content("".into());}else{self.recipes_request("read",json!({"id":id}));}},
            "delete"=>{
                if state.get_deleting(){self.recipes_request("delete",json!({"id":doc["id"],"revision":doc["revision"],"confirmed":true}));}
            },
            "list"=>self.recipes_request("list",json!({})),
            "save"=>self.recipes_request("save",json!({"id":doc["id"],"revision":doc["revision"],"content":state.get_content().as_str()})),
            "prepare"=>{
                let values:serde_json::Map<String,Value>=state.get_parameters().iter().map(|p|(p.key.to_string(),json!(p.value.as_str()))).collect();
                let project=self.settings.recipes.catalog["projects"].as_array().and_then(|p|p.get(state.get_project_index() as usize)).map(|p|p["id"].clone()).unwrap_or_default();
                self.recipes_request("prepare",json!({"id":doc["id"],"revision":doc["revision"],"values":values,"project":project,"dynamicTools":if self.prefs.cross_tab_tools{self.xtab_dynamic_tools()}else{None}}));
            },
            _=>{}
        }
    }
    fn recipes_request(&mut self, action: &str, params: Value) {
        let state = self.window.global::<RecipeState>();
        state.set_busy(true);
        state.set_status("".into());
        let backend = self.backend.clone();
        let action = action.to_owned();
        self.backend.spawn(async move {
            let result = backend
                .goose_request(&format!("fastrock/recipe/{action}"), params)
                .await;
            crate::ui_thread::post(move |app| {
                let state = app.window.global::<RecipeState>();
                state.set_busy(false);
                match result {
                    Err(e) => state.set_status(e.into()),
                    Ok(value) => match action.as_str() {
                        "list" => {
                            let rows = value["recipes"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .map(|r| RecipeRow {
                                    id: r["id"].as_str().unwrap_or("").into(),
                                    title: r["title"].as_str().unwrap_or("").into(),
                                    description: r["description"].as_str().unwrap_or("").into(),
                                })
                                .collect::<Vec<_>>();
                            let projects = value["projects"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .map(|p| {
                                    SharedString::from(format!(
                                        "{} · {}",
                                        p["name"].as_str().unwrap_or(""),
                                        dunce::simplified(std::path::Path::new(
                                            p["root"].as_str().unwrap_or("")
                                        ))
                                        .display()
                                    ))
                                })
                                .collect::<Vec<_>>();
                            if state.get_project_index() < 0
                                || state.get_project_index() as usize >= projects.len()
                            {
                                state.set_project_index(if projects.is_empty() { -1 } else { 0 });
                            }
                            state.set_rows(ModelRc::new(VecModel::from(rows)));
                            state.set_projects(ModelRc::new(VecModel::from(projects)));
                            app.settings.recipes.catalog = value;
                        }
                        "read" => {
                            state.set_selected(value["id"].as_str().unwrap_or("").into());
                            state.set_content(value["content"].as_str().unwrap_or("").into());
                            let rows = value["parameters"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .map(|p| RecipeParameter {
                                    key: p["key"].as_str().unwrap_or("").into(),
                                    description: p["description"].as_str().unwrap_or("").into(),
                                    kind: p["input_type"].as_str().unwrap_or("string").into(),
                                    required: p["requirement"] != "optional",
                                    value: p["default"].as_str().unwrap_or("").into(),
                                    options: ModelRc::new(VecModel::from(
                                        p["options"]
                                            .as_array()
                                            .into_iter()
                                            .flatten()
                                            .filter_map(Value::as_str)
                                            .map(SharedString::from)
                                            .collect::<Vec<_>>(),
                                    )),
                                })
                                .collect::<Vec<_>>();
                            state.set_parameters(ModelRc::new(VecModel::from(rows)));
                            state.set_editing(true);
                            state.set_deleting(false);
                            state.set_dirty(false);
                            app.settings.recipes.document = value;
                            app.recipes_load();
                        }
                        "save" => {
                            state.set_dirty(false);
                            app.recipes_request("read", json!({"id":value["id"]}));
                        }
                        "delete" => {
                            state.set_deleting(false);
                            state.set_editing(false);
                            state.set_selected("".into());
                            state.set_content("".into());
                            app.settings.recipes.document = json!({});
                            app.recipes_load();
                        }
                        "prepare" => match serde_json::from_value(value["response"].clone()) {
                            Ok(response) => app.attach_recipe_conversation(
                                response,
                                value["workspace"].clone(),
                                value["prompt"].as_str().unwrap_or("").to_owned(),
                            ),
                            Err(e) => state.set_status(
                                format!("Cannot open prepared conversation: {e}").into(),
                            ),
                        },
                        _ => {}
                    },
                }
            });
        });
    }
    pub(crate) fn recipes_automation(&mut self, args: &[String]) {
        let arg = |n| args.get(n).map(String::as_str).unwrap_or("");
        let state = self.window.global::<RecipeState>();
        match arg(0) {
            "select" => state.invoke_select(arg(1).into()),
            "content" => {
                state.set_content(arg(1).into());
                state.set_dirty(true);
            }
            "parameter" => {
                let model = state.get_parameters();
                for i in 0..model.row_count() {
                    if let Some(mut p) = model.row_data(i) {
                        if p.key == arg(1) {
                            p.value = arg(2).into();
                            model.set_row_data(i, p);
                            break;
                        }
                    }
                }
            }
            "project" => state.set_project_index(arg(1).parse().unwrap_or(-1)),
            "dump" => {
                let value = json!({"catalog":self.settings.recipes.catalog,"document":self.settings.recipes.document,"status":state.get_status().as_str(),"dirty":state.get_dirty(),"busy":state.get_busy(),"deleting":state.get_deleting(),"selected":state.get_selected().as_str()});
                let path = std::path::PathBuf::from(arg(1));
                self.backend.spawn(async move {
                    let _ = tokio::fs::write(path, value.to_string()).await;
                });
            }
            _ => state.invoke_action(arg(0).into()),
        }
    }
}
