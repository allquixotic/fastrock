//! AWS model-card policy, and public per-session model configuration integration.
//! No endpoint rewrite, environment mutation or upstream source patch.
use goose_provider_types::model::ModelConfig;
use serde_json::{Value, json};

#[derive(Clone, Default)]
pub(super) struct Route {
    pub url: String,
    pub path: String,
}
impl Route {
    pub fn ultrafast(&self, model: &str) -> bool {
        let Ok(url) = url::Url::parse(&self.url) else {
            return false;
        };
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some_and(|p| p != 443)
            || url.query().is_some()
            || url.fragment().is_some()
            || self.path.trim_matches('/') != "openai/v1/responses"
        {
            return false;
        }
        let host = url.host_str().unwrap_or_default();
        let supported = |m: &str| matches!(m, "openai.gpt-6.1-sol" | "openai.gpt-6-astra");
        if host == "bedrock-mantle.us-east-1.api.aws" {
            return supported(model);
        }
        let Some(region) = host
            .strip_prefix("bedrock-runtime.")
            .and_then(|s| s.strip_suffix(".amazonaws.com"))
        else {
            return false;
        };
        let us = matches!(
            region,
            "us-east-1" | "us-east-2" | "us-west-1" | "us-west-2" | "ca-central-1" | "ca-west-1"
        );
        if let Some(model) = model.strip_prefix("us.") {
            return us && supported(model);
        }
        // Published global source regions for Astra; Sol 6.1 lists US/Canada
        // source regions. Unverified region/model combinations fail closed.
        let global = us
            || matches!(
                region,
                "eu-central-1"
                    | "eu-north-1"
                    | "eu-west-1"
                    | "eu-west-2"
                    | "eu-west-3"
                    | "ap-northeast-1"
                    | "ap-northeast-2"
                    | "ap-northeast-3"
                    | "ap-south-1"
                    | "ap-southeast-1"
                    | "ap-southeast-2"
                    | "sa-east-1"
            );
        global
            && (us || model == "global.openai.gpt-6-astra")
            && model.strip_prefix("global.").is_some_and(supported)
    }
}
pub(super) fn tier(model: &ModelConfig) -> Option<String> {
    model
        .request_params
        .as_ref()?
        .get("service_tier")?
        .as_str()
        .map(str::to_owned)
}
pub(super) fn option(provider: &str, model: &str, current: Option<&str>) -> Value {
    let mut choices = vec![json!({"value":"default", "name":"Standard"})];
    if super::provider_policy::ultrafast(provider, model) {
        choices.push(json!({"value":"ultrafast", "name":"Ultrafast"}));
    }
    json!({"id":"speed", "name":"Speed", "type":"select", "currentValue":current.unwrap_or("default"), "options":choices})
}

pub(super) fn change(model: &mut ModelConfig, value: &str, supported: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        value == "default" || (value == "ultrafast" && supported),
        "This speed is not supported by the selected model and endpoint"
    );
    if supported {
        model
            .request_params
            .get_or_insert_with(Default::default)
            .insert("service_tier".into(), json!(value));
    } else if let Some(params) = &mut model.request_params {
        params.remove("service_tier");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn route(host: &str) -> Route {
        Route {
            url: format!("https://{host}/openai/v1"),
            path: "openai/v1/responses".into(),
        }
    }
    #[test]
    fn v90_aws_model_route_and_region_matrix() {
        let runtime = route("bedrock-runtime.us-east-1.amazonaws.com");
        for model in [
            "us.openai.gpt-6.1-sol",
            "global.openai.gpt-6.1-sol",
            "us.openai.gpt-6-astra",
        ] {
            assert!(runtime.ultrafast(model));
        }
        for model in [
            "openai.gpt-6.1-sol",
            "us.openai.gpt-6-luna",
            "us.openai.gpt-6-sol",
            "gpt-6.1-sol",
            "us.openai.gpt-99",
        ] {
            assert!(!runtime.ultrafast(model));
        }
        assert!(route("bedrock-mantle.us-east-1.api.aws").ultrafast("openai.gpt-6.1-sol"));
        assert!(!route("bedrock-mantle.us-west-2.api.aws").ultrafast("openai.gpt-6-astra"));
        assert!(
            !route("bedrock-runtime.eu-west-1.amazonaws.com").ultrafast("us.openai.gpt-6.1-sol")
        );
        assert!(
            route("bedrock-runtime.eu-west-1.amazonaws.com").ultrafast("global.openai.gpt-6-astra")
        );
        assert!(
            !route("bedrock-runtime.us-east-1.amazonaws.com.evil.test")
                .ultrafast("us.openai.gpt-6.1-sol")
        );
        assert!(
            !Route {
                path: "openai/v1/chat/completions".into(),
                ..runtime
            }
            .ultrafast("us.openai.gpt-6.1-sol")
        );
    }
    #[test]
    fn v90_speed_keeps_effort_and_rejects_unsupported_values() {
        let mut model = ModelConfig::new("us.openai.gpt-6.1-sol");
        model.request_params = Some(std::collections::HashMap::from([
            ("reasoning_effort".into(), json!("high")),
            ("custom".into(), json!(42)),
        ]));
        let original = model.clone();
        assert!(change(&mut model, "fast", true).is_err());
        assert!(change(&mut model, "ultrafast", false).is_err());
        assert_eq!(
            serde_json::to_value(&model).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        change(&mut model, "ultrafast", true).unwrap();
        assert_eq!(tier(&model).as_deref(), Some("ultrafast"));
        assert_eq!(
            model.request_params.as_ref().unwrap()["reasoning_effort"],
            "high"
        );
        assert_eq!(model.request_params.as_ref().unwrap()["custom"], 42);
        change(&mut model, "default", true).unwrap();
        assert_eq!(tier(&model).as_deref(), Some("default"));
        change(&mut model, "default", false).unwrap();
        assert!(tier(&model).is_none());
    }
}
