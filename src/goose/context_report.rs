//! Read-only projection of Goose's public context-report response.
use super::{
    Adapter, Reply, error,
    model::{session_id, text},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_ROWS: usize = 512;
const MAX_PREVIEW: usize = 16 * 1024;
const MAX_TOTAL_PREVIEW: usize = 2 * 1024 * 1024;

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct Report {
    pub model: String,
    pub provider: String,
    pub total: u64,
    pub limit: u64,
    pub managed: bool,
    pub compaction_pending: bool,
    pub truncated: bool,
    pub rows: Vec<Row>,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Row {
    pub label: String,
    pub category: String,
    pub source: String,
    pub tokens: u64,
    pub preview: String,
    pub part: bool,
}

fn bounded(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (preview truncated)", &value[..end])
}

pub(super) fn project(value: &Value) -> anyhow::Result<Report> {
    let count = |v: &Value, key: &str| {
        v[key]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("Context report is missing {key}"))
    };
    let mut report = Report {
        model: bounded(text(&value["model"], "modelName"), 512),
        provider: bounded(text(&value["model"], "provider"), 256),
        total: count(value, "totalTokens")?,
        limit: count(&value["model"], "contextLimit")?,
        managed: value["providerManagesContext"] == true,
        compaction_pending: value["autoCompactionPending"] == true,
        ..Default::default()
    };
    let segments = value["segments"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Context report has no segments"))?;
    let mut budget = MAX_TOTAL_PREVIEW;
    for segment in segments {
        let parts = segment["parts"].as_array();
        for (index, item) in std::iter::once(segment)
            .chain(parts.into_iter().flatten())
            .enumerate()
        {
            if report.rows.len() == MAX_ROWS {
                report.truncated = true;
                break;
            }
            let preview = text(item, "contentPreview");
            let limit = budget.min(MAX_PREVIEW);
            report.truncated |= preview.len() > limit;
            let preview = bounded(preview, limit);
            budget = budget.saturating_sub(preview.len());
            report.rows.push(Row {
                label: bounded(text(item, "label"), 512),
                category: bounded(text(segment, "category"), 128).replace('_', " "),
                source: bounded(text(item, "source"), 2048),
                tokens: count(item, "tokenCount")?,
                preview,
                part: index > 0,
            });
        }
        if report.rows.len() == MAX_ROWS {
            report.truncated = true;
            break;
        }
    }
    Ok(report)
}

impl Adapter {
    pub(super) async fn context_report(&self, params: &Value) -> Reply {
        let id = session_id(text(params, "threadId"))?;
        let gate = self.gate(id).await;
        let _guard = gate.lock().await;
        {
            let sessions = self.sessions.lock().await;
            let session = sessions
                .get(id)
                .ok_or_else(|| error("Open a conversation first."))?;
            if !session.loaded
                || session.activation_pending
                || session.storage_pending
                || session.recovery_only
            {
                return Err(error(
                    "Resume this conversation before inspecting its current context.",
                ));
            }
            if session.active.is_some() || session.compacting || session.shell.is_some() {
                return Err(error(
                    "Wait for the running turn to finish, then refresh the context report.",
                ));
            }
        }
        let value = self
            .rpc
            .call("_goose/unstable/context/report", json!({"sessionId":id}))
            .await?;
        let report = project(&value)
            .map_err(|_| error("The runtime returned an invalid context report."))?;
        serde_json::to_value(report).map_err(|_| error("Could not display context report."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v104_context_counts_and_parts_are_not_double_counted() {
        let report = project(&json!({"model":{"modelName":"fixture","contextLimit":100},"totalTokens":12,"segments":[{"category":"messages","label":"Messages","tokenCount":12,"parts":[{"label":"User","tokenCount":5,"contentPreview":"hello"},{"label":"Assistant","tokenCount":7}]}],"autoCompactionPending":true})).unwrap();
        assert_eq!(report.total, 12);
        assert_eq!(report.rows.len(), 3);
        assert!(report.rows[1].part);
        assert!(report.compaction_pending);
        assert!(!report.truncated);
        assert!(project(&json!({})).is_err());
    }
    #[test]
    fn v104_context_previews_are_bounded_utf8_and_keep_upstream_totals() {
        let segment = json!({"category":"tool_definitions","label":"工具","tokenCount":9,"contentPreview":"🦀".repeat(10000)});
        let report = project(
            &json!({"model":{"contextLimit":0},"totalTokens":9000,"segments":vec![segment;1000]}),
        )
        .unwrap();
        assert_eq!(report.total, 9000);
        assert_eq!(report.limit, 0);
        assert_eq!(report.rows.len(), MAX_ROWS);
        assert!(report.truncated);
        assert!(
            report.rows.iter().map(|r| r.preview.len()).sum::<usize>()
                < MAX_TOTAL_PREVIEW + MAX_ROWS * 40
        );
    }
}
