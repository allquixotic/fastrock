//! Bounded webhook authentication, template evaluation and durable receipts.
use anyhow::{Result, bail, ensure};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const MAX_BODY: usize = 64 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hook {
    pub header: String,
    pub prefix: String,
    pub base64: bool,
    pub replay_key: String,
    pub coalesce_key: String,
    pub coalesce_seconds: u32,
    pub max_age_minutes: u32,
    pub filters: Vec<Filter>,
}
impl Default for Hook {
    fn default() -> Self {
        Self {
            header: "x-hub-signature-256".into(),
            prefix: "sha256=".into(),
            base64: false,
            replay_key: String::new(),
            coalesce_key: String::new(),
            coalesce_seconds: 120,
            max_age_minutes: 1440,
            filters: Vec::new(),
        }
    }
}
impl Hook {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.header.is_empty()
                && self.header.len() <= 128
                && self
                    .header
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "Enter a valid signature header"
        );
        ensure!(
            self.prefix.len() <= 64 && !self.prefix.chars().any(char::is_control),
            "Invalid signature prefix"
        );
        ensure!(
            (1..=1440).contains(&self.max_age_minutes)
                && (10..=3600).contains(&self.coalesce_seconds),
            "Delivery age must be 1–1440 minutes; coalescing window 10–3600 seconds"
        );
        ensure!(
            self.filters.len() <= 16,
            "At most 16 webhook filters are supported"
        );
        for template in [&self.replay_key, &self.coalesce_key] {
            ensure!(template.len() <= 1024, "Webhook key template is too long");
            if !template.is_empty() {
                template_parts(template)?;
            }
        }
        for f in &self.filters {
            valid_path(&f.path, false)?;
            ensure!(
                matches!(
                    f.op.as_str(),
                    "equals" | "notEquals" | "startsWith" | "in" | "exists"
                ),
                "Invalid webhook filter operator"
            );
            ensure!(f.value.len() <= 1024, "Filter value is too long");
        }
        Ok(())
    }
    pub fn verify(&self, key: &[u8], signature: &str, body: &[u8]) -> Result<()> {
        ensure!(body.len() <= MAX_BODY, "Webhook body too large");
        let encoded = signature
            .strip_prefix(&self.prefix)
            .ok_or_else(|| anyhow::anyhow!("Invalid signature"))?;
        let bytes = if self.base64 {
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| anyhow::anyhow!("Invalid signature"))?
        } else {
            ensure!(encoded.len() == 64, "Invalid signature");
            (0..64)
                .step_by(2)
                .map(|i| {
                    encoded
                        .get(i..i + 2)
                        .and_then(|v| u8::from_str_radix(v, 16).ok())
                        .ok_or_else(|| anyhow::anyhow!("Invalid signature"))
                })
                .collect::<Result<Vec<_>>>()?
        };
        let mut mac = Hmac::<Sha256>::new_from_slice(key)?;
        mac.update(body);
        mac.verify_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid signature"))
    }
    pub fn matches(&self, context: &Value) -> Result<bool> {
        for f in &self.filters {
            let values = path_values(context, &f.path)?;
            let strings: Vec<_> = values.iter().filter_map(|v| scalar(v)).collect();
            let matched = match f.op.as_str() {
                "exists" => !values.is_empty(),
                "equals" => strings.iter().any(|s| s == &f.value),
                "notEquals" => strings.iter().all(|s| s != &f.value),
                "startsWith" => strings.iter().any(|s| s.starts_with(&f.value)),
                "in" => strings
                    .iter()
                    .any(|s| f.value.split(',').any(|v| v.trim() == s)),
                _ => false,
            };
            if !matched {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub path: String,
    pub op: String,
    pub value: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub id: String,
    pub received: i64,
    pub ready: i64,
    pub replay: String,
    pub coalesce: Option<String>,
    pub status: String,
    pub prompt: String,
    pub run: Option<String>,
    pub child: Option<String>,
}
impl Delivery {
    pub fn expired(&self, hook: &Hook, now: i64) -> bool {
        self.received
            .saturating_add(i64::from(hook.max_age_minutes) * 60)
            < now
    }
}
fn hash(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
pub fn accept(
    history: &mut Vec<Delivery>,
    hook: &Hook,
    context: &Value,
    raw: &[u8],
    prompt: &str,
    now: i64,
) -> Result<Value> {
    // Retain every pending/uncertain delivery and a full day of terminal replay
    // identities. At capacity reject new work rather than discard a guard.
    history.retain(|d| {
        matches!(d.status.as_str(), "pending" | "starting" | "needs_review")
            || d.received >= now - 86400
    });
    let replay = if hook.replay_key.is_empty() {
        hash(raw)
    } else {
        hash(render(&hook.replay_key, context)?.as_bytes())
    };
    if let Some(old) = history.iter().find(|d| d.replay == replay) {
        return Ok(json!({"id":old.id,"outcome":"duplicate"}));
    }
    ensure!(
        history.len() < 4096,
        "Webhook receipt capacity reached; retry after retained receipts expire"
    );
    let matched = hook.matches(context)?;
    let content = if matched {
        render(prompt, context)?
    } else {
        String::new()
    };
    let coalesce = if !matched || hook.coalesce_key.is_empty() {
        None
    } else {
        Some(hash(render(&hook.coalesce_key, context)?.as_bytes()))
    };
    let mut ready = now;
    if matched && coalesce.is_some() {
        ready += i64::from(hook.coalesce_seconds);
        for old in history
            .iter_mut()
            .filter(|d| d.status == "pending" && d.coalesce == coalesce && d.ready > now)
        {
            ready = ready.min(old.ready);
            old.status = "coalesced".into();
            old.prompt.clear();
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let status = if matched { "pending" } else { "filtered" };
    history.push(Delivery {
        id: id.clone(),
        received: now,
        ready,
        replay,
        coalesce,
        status: status.into(),
        prompt: content,
        run: None,
        child: None,
    });
    Ok(json!({"id":id,"outcome":status}))
}
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
}
fn valid_path(path: &str, root_allowed: bool) -> Result<()> {
    let parts: Vec<_> = path.split('.').collect();
    ensure!(
        path.len() <= 256
            && parts.len() <= 32
            && parts
                .iter()
                .all(|p| !p.is_empty() && !p.chars().any(char::is_whitespace)),
        "Invalid webhook field path"
    );
    ensure!(
        matches!(parts[0], "body" | "headers" | "query") || (root_allowed && path == "request"),
        "Webhook fields must start with body, headers or query"
    );
    Ok(())
}
fn path_values<'a>(context: &'a Value, path: &str) -> Result<Vec<&'a Value>> {
    valid_path(path, true)?;
    if path == "request" {
        return Ok(vec![context]);
    }
    let mut values = vec![context];
    for part in path.split('.') {
        let mut next = Vec::new();
        for value in values {
            if part == "[]" {
                if let Some(array) = value.as_array() {
                    next.extend(array.iter());
                }
            } else if let Some(value) = value.get(part) {
                next.push(value);
            }
            ensure!(next.len() <= 4096, "Webhook field expansion is too large");
        }
        values = next;
    }
    Ok(values)
}
pub(super) fn template_parts(template: &str) -> Result<Vec<(&str, bool)>> {
    let mut rest = template;
    let mut parts = Vec::new();
    while let Some(start) = rest.find("{{") {
        ensure!(
            !rest[..start].contains("}}"),
            "Unmatched webhook template delimiter"
        );
        parts.push((&rest[..start], false));
        rest = &rest[start + 2..];
        let end = rest
            .find("}}")
            .ok_or_else(|| anyhow::anyhow!("Unclosed webhook field"))?;
        let path = rest[..end].trim();
        valid_path(path, true)?;
        parts.push((path, true));
        ensure!(parts.len() <= 256, "Too many webhook template fields");
        rest = &rest[end + 2..];
    }
    ensure!(!rest.contains("}}"), "Unmatched webhook template delimiter");
    parts.push((rest, false));
    Ok(parts)
}
pub fn render(template: &str, context: &Value) -> Result<String> {
    let mut out = String::new();
    for (part, field) in template_parts(template)? {
        if field {
            let values = path_values(context, part)?;
            ensure!(
                values.len() == 1,
                "Webhook template field is missing or ambiguous"
            );
            out.push_str(&scalar(values[0]).unwrap_or_else(|| values[0].to_string()));
        } else {
            out.push_str(part);
        }
        ensure!(out.len() <= 32768, "Expanded webhook prompt exceeds 32 KiB");
    }
    if out.trim().is_empty() {
        bail!("Webhook template is empty");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v58_signature_covers_exact_body_and_rejects_malformed_digest() {
        let hook = Hook::default();
        let mut mac = Hmac::<Sha256>::new_from_slice(b"fixture-key").unwrap();
        mac.update(b"{\"id\":1}");
        let signature = format!("sha256={:x}", mac.finalize().into_bytes());
        hook.verify(b"fixture-key", &signature, b"{\"id\":1}")
            .unwrap();
        assert!(
            hook.verify(b"fixture-key", &signature, b"{\"id\":2}")
                .is_err()
        );
        assert!(hook.verify(b"fixture-key", "sha256=λ", b"{}").is_err());
        let mut b64 = hook;
        b64.base64 = true;
        b64.prefix.clear();
        assert!(b64.verify(b"fixture-key", "AA==", b"{}").is_err());
    }
    #[test]
    fn v58_templates_filters_replay_and_coalescing_are_bounded() {
        let mut hook = Hook {
            replay_key: "{{body.id}}".into(),
            coalesce_key: "{{body.group}}".into(),
            ..Hook::default()
        };
        hook.filters.push(Filter {
            path: "body.labels.[].name".into(),
            op: "equals".into(),
            value: "build".into(),
        });
        hook.validate().unwrap();
        let mut context = json!({"body":{"id":"a","group":"g","labels":[{"name":"build"}]}});
        let mut history = Vec::new();
        let first = accept(&mut history, &hook, &context, b"a", "Run {{body.id}}", 100).unwrap();
        assert_eq!(
            accept(&mut history, &hook, &context, b"b", "Run", 101).unwrap()["outcome"],
            "duplicate"
        );
        context["body"]["id"] = json!("b");
        accept(&mut history, &hook, &context, b"b", "Run {{body.id}}", 102).unwrap();
        assert_eq!(history[0].id, first["id"]);
        assert_eq!(history[0].status, "coalesced");
        assert!(history[0].prompt.is_empty());
        assert_eq!(history[1].ready, 220);
        assert_eq!(history[1].prompt, "Run b");
        assert!(render("{{headers.authorization}}", &context).is_err());
        assert!(render(&"x".repeat(32769), &context).is_err());
        context["body"]["labels"] = json!([]);
        assert!(!hook.matches(&context).unwrap());
    }
    #[test]
    fn v58_receipt_expiry_capacity_and_filtered_missing_fields() {
        let mut hook = Hook {
            max_age_minutes: 1,
            coalesce_key: "{{body.group}}".into(),
            ..Hook::default()
        };
        hook.filters.push(Filter {
            path: "body.action".into(),
            op: "equals".into(),
            value: "run".into(),
        });
        let mut history = Vec::new();
        accept(
            &mut history,
            &hook,
            &json!({"body":{"action":"ignore"}}),
            b"ignored",
            "{{body.missing}}",
            100,
        )
        .unwrap();
        assert_eq!(history[0].status, "filtered");
        assert!(!history[0].expired(&hook, 160));
        assert!(history[0].expired(&hook, 161));
        history[0].status = "pending".into();
        history[0].prompt = "Retained".into();
        history = vec![history[0].clone(); 4096];
        assert!(accept(&mut history, &hook, &json!({}), b"new", "work", 100000).is_err());
        assert_eq!(history.len(), 4096); // Unresolved identities cannot be evicted.
    }
}
