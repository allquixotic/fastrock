//! Speed choices come from the provider catalog; never infer Bedrock tiers
//! from model names or copy OpenAI-only capabilities into its catalog.

use std::collections::HashMap;

use codex_app_server_protocol::ConfigRequirements;
use codex_app_server_protocol::Model;
use codex_protocol::config_types::SERVICE_TIER_DEFAULT_REQUEST_VALUE;

use crate::ui::ComposerChoice;

pub(super) fn choices(
    model: Option<&Model>,
    current: Option<&str>,
    features: &HashMap<String, bool>,
    requirements: Option<&ConfigRequirements>,
    independent_speed_modes: Option<bool>,
) -> Vec<ComposerChoice> {
    let enabled = |name: &str| {
        features.get(name) != Some(&false)
            && requirements
                .and_then(|requirements| requirements.feature_requirements.as_ref())
                .and_then(|requirements| requirements.get(name))
                != Some(&false)
    };
    let tier_enabled = |tier: &str| match tier {
        "flex" => true,
        "ultrafast" => {
            enabled("ultrafast_mode")
                && (independent_speed_modes == Some(true) || enabled("fast_mode"))
        }
        _ => enabled("fast_mode"),
    };
    let mut result = vec![ComposerChoice {
        id: SERVICE_TIER_DEFAULT_REQUEST_VALUE.into(),
        label: "Standard".into(),
        detail: "Standard inference speed and pricing.".into(),
        selected: current.is_none() || current == Some(SERVICE_TIER_DEFAULT_REQUEST_VALUE),
    }];
    if let Some(model) = model {
        for tier in &model.service_tiers {
            if tier.id != SERVICE_TIER_DEFAULT_REQUEST_VALUE && tier_enabled(&tier.id) {
                result.push(ComposerChoice {
                    id: tier.id.as_str().into(),
                    label: tier.name.as_str().into(),
                    detail: tier.description.as_str().into(),
                    selected: current == Some(tier.id.as_str()),
                });
            }
        }
        // Compatibility with servers that predate serviceTiers.
        if model.service_tiers.is_empty() {
            for tier in &model.additional_speed_tiers {
                if !tier_enabled(tier) || tier == SERVICE_TIER_DEFAULT_REQUEST_VALUE {
                    continue;
                }
                result.push(ComposerChoice {
                    id: tier.as_str().into(),
                    label: match tier.as_str() {
                        "fast" | "priority" => "Fast",
                        "ultrafast" => "Ultrafast",
                        "flex" => "Flex",
                        _ => tier,
                    }
                    .into(),
                    detail: "Provider service tier; pricing may differ from Standard.".into(),
                    selected: current == Some(tier.as_str()),
                });
            }
        }
    }
    result
}

/// Preserve an explicitly selected tier only when the destination advertises it.
/// Explicit Standard also prevents adopting a premium catalog default.
pub(super) fn tier_after_model_change(
    model: Option<&Model>,
    current: Option<&str>,
) -> Option<Option<String>> {
    current
        .filter(|tier| *tier != SERVICE_TIER_DEFAULT_REQUEST_VALUE)
        .filter(|tier| {
            model.is_none_or(|model| {
                !model
                    .service_tiers
                    .iter()
                    .any(|candidate| candidate.id == *tier)
                    && !model
                        .additional_speed_tiers
                        .iter()
                        .any(|candidate| candidate == *tier)
            })
        })
        .map(|_| Some(SERVICE_TIER_DEFAULT_REQUEST_VALUE.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model(tiers: serde_json::Value) -> Model {
        serde_json::from_value(json!({
            "id": "openai.gpt-6-astra", "model": "openai.gpt-6-astra",
            "displayName": "Astra", "description": "", "hidden": false,
            "supportedReasoningEfforts": [], "defaultReasoningEffort": "medium",
            "serviceTiers": tiers, "isDefault": true
        }))
        .expect("model")
    }

    #[test]
    fn catalog_drives_bedrock_and_future_sol_tiers() {
        let mut model = model(json!([
            {"id": "ultrafast", "name": "Ultrafast", "description": "Premium speed"}
        ]));
        for slug in [
            "openai.gpt-6-astra",
            "us.openai.gpt-6-astra",
            "global.openai.gpt-6-astra",
            "openai.gpt-6.1-sol",
        ] {
            model.model = slug.into();
            let choices = choices(
                Some(&model),
                Some("ultrafast"),
                &HashMap::new(),
                None,
                Some(true),
            );
            assert_eq!(choices.len(), 2);
            assert_eq!(choices[1].id, "ultrafast");
            assert!(choices[1].selected);
        }
        let standard_only = self::model(json!([]));
        assert_eq!(
            choices(
                Some(&standard_only),
                None,
                &HashMap::new(),
                None,
                Some(true)
            )
            .len(),
            1
        );
        assert_eq!(
            tier_after_model_change(Some(&standard_only), Some("ultrafast")),
            Some(Some("default".into()))
        );
        assert_eq!(
            tier_after_model_change(Some(&model), Some("ultrafast")),
            None
        );
    }

    #[test]
    fn independent_ultrafast_and_managed_restrictions() {
        let model = model(json!([
            {"id": "priority", "name": "Fast", "description": ""},
            {"id": "ultrafast", "name": "Ultrafast", "description": ""}
        ]));
        let features = HashMap::from([("fast_mode".into(), false)]);
        assert_eq!(
            choices(Some(&model), None, &features, None, Some(true)).len(),
            2
        );
        assert_eq!(choices(Some(&model), None, &features, None, None).len(), 1);
        let requirements =
            serde_json::from_value(json!({"featureRequirements": {"ultrafast_mode": false}}))
                .expect("requirements");
        assert_eq!(
            choices(
                Some(&model),
                None,
                &features,
                Some(&requirements),
                Some(true)
            )
            .len(),
            1
        );
    }

    #[test]
    fn standard_remains_explicit_and_legacy_catalogs_work() {
        let mut model = model(json!([]));
        model.additional_speed_tiers = vec!["priority".into()];
        let choices = choices(Some(&model), Some("default"), &HashMap::new(), None, None);
        assert_eq!(choices[1].label, "Fast");
        assert!(choices[0].selected);
        assert_eq!(tier_after_model_change(Some(&model), Some("default")), None);
    }
}
