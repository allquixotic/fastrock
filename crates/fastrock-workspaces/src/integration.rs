//! Explicit integration using vwork's original three-way merge. No checkout,
//! subprocess or source rewrite; both reviewed heads are checked under the
//! embedded owner's exclusive guard before calling the upstream engine.
use crate::embedded::Connection;
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::collections::BTreeSet;

const MAX_REVIEW_BYTES: usize = 128 * 1024;
const MAX_ANCESTORS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeReview {
    pub target: String,
    pub source: String,
    pub target_revision: String,
    pub source_revision: String,
    pub report: String,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeResult {
    Merged { revision: String },
    Conflicts { paths: Vec<String> },
}

pub async fn review(
    connection: &Connection,
    target: String,
    source: String,
) -> Result<MergeReview> {
    connection
        .run(move |store| {
            store.with_exclusive_engine(|engine| {
                ensure!(target != source, "Choose a different source workspace");
                let target = engine.workspace(&target)?;
                let source = engine.workspace(&source)?;
                ensure!(target.repo == source.repo, "Cannot merge different repositories");
                let mut ancestors = BTreeSet::new();
                let mut cursor = Some(target.head.clone());
                while let Some(head) = cursor {
                    ensure!(
                        ancestors.len() < MAX_ANCESTORS && ancestors.insert(head.clone()),
                        "Workspace ancestry exceeds the review limit"
                    );
                    cursor = engine.revision(&head)?.parent;
                }
                let mut seen = BTreeSet::new();
                let mut cursor = Some(source.head.clone());
                let mut common = None;
                while let Some(head) = cursor {
                    if ancestors.contains(&head) {
                        common = Some(head);
                        break;
                    }
                    ensure!(
                        seen.len() < MAX_ANCESTORS && seen.insert(head.clone()),
                        "Workspace ancestry exceeds the review limit"
                    );
                    cursor = engine.revision(&head)?.parent;
                }
                let common = common.context("No shared virtual ancestor; fork before merging")?;
                let (_, base) = engine.snapshot(&vwork_engine::model::Workspace {
                    head: common.clone(),
                    ..target.clone()
                })?;
                let mut report = format!(
                    "Merge {} into {}\nSource revision: {}\nTarget revision: {}\n\nChanges below are relative to the shared ancestor. Conflicting edits leave both workspaces unchanged.\n",
                    source.id, target.id, source.head, target.head
                );
                let mut complete = true;
                for (label, workspace) in [("Incoming", &source), ("Target", &target)] {
                    let value = engine.call(
                        "workspace_diff",
                        &json!({"workspace": workspace.id, "against_revision": common, "limit": 200}),
                    )?;
                    let diff = value["diff"].as_str().context("Missing workspace diff")?;
                    let changes = value["changes"].as_array().context("Missing changed paths")?;
                    let (_, tree) = engine.snapshot(workspace)?;
                    let heading = format!("\n{label} changes ({})\n", workspace.id);
                    report.push_str(&heading);
                    for change in changes {
                        let path = change["path"].as_str().context("Missing changed path")?;
                        report.push_str(&format!(
                            "{} {} · mode {} → {}\n",
                            change["status"].as_str().unwrap_or("?"), path,
                            base.get(path).map_or("absent", |entry| entry.mode.as_str()),
                            tree.get(path).map_or("absent", |entry| entry.mode.as_str()),
                        ));
                    }
                    let remaining = MAX_REVIEW_BYTES.saturating_sub(report.len());
                    let mut end = diff.len().min(remaining);
                    while !diff.is_char_boundary(end) {
                        end -= 1;
                    }
                    report.push_str(&diff[..end]);
                    complete &= end == diff.len()
                        && value["truncated"] == false
                        && diff.len() < engine.config.max_output_bytes
                        && changes.len() < 200.min(engine.config.max_results);
                }
                if report.len() > MAX_REVIEW_BYTES {
                    let mut end = MAX_REVIEW_BYTES;
                    while !report.is_char_boundary(end) {
                        end -= 1;
                    }
                    report.truncate(end);
                    complete = false;
                }
                if !complete {
                    report.push_str("\nReview exceeds the display limit. Apply is disabled; integrate a smaller set of changes.\n");
                }
                Ok(MergeReview {
                    target: target.id,
                    source: source.id,
                    target_revision: target.head,
                    source_revision: source.head,
                    report,
                    complete,
                })
            })
        })
        .await
}

pub async fn apply(connection: &Connection, review: MergeReview) -> Result<MergeResult> {
    connection
        .run(move |store| {
            store.with_exclusive_engine(|engine| {
                ensure!(review.complete, "Complete the change review before merging");
                ensure!(
                    review.source != review.target,
                    "Choose a different source workspace"
                );
                let target = engine.workspace(&review.target)?;
                let source = engine.workspace(&review.source)?;
                ensure!(
                    target.repo == source.repo,
                    "Cannot merge different repositories"
                );
                ensure!(
                    target.head == review.target_revision && source.head == review.source_revision,
                    "A workspace changed after review; review the changes again"
                );
                let value = engine.call(
                    "workspace_merge",
                    &json!({"workspace": review.target, "source": review.source,
                        "expected_revision": review.target_revision}),
                )?;
                if value["merged"] == true {
                    Ok(MergeResult::Merged {
                        revision: value["revision"]
                            .as_str()
                            .context("Missing merge revision")?
                            .into(),
                    })
                } else {
                    Ok(MergeResult::Conflicts {
                        paths: serde_json::from_value(value["conflicts"].clone())?,
                    })
                }
            })
        })
        .await
}
