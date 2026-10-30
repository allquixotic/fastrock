//! Immutable conversation checkpoints in the existing compressed vwork store.
//! Public workspace commits pin revisions without touching the original checkout.
pub mod journal;
use crate::embedded::Connection;
use anyhow::{Context, Result, ensure};
use redb::{ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Read;
use vwork_engine::engine::Engine;

const POINTS: TableDefinition<&str, &[u8]> = TableDefinition::new("fastrock_checkpoints_v1");
const MAX_RECORD_BYTES: usize = 64 * 1024;
const MAX_REVIEW_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Capture {
    /// Sortable time-prefixed unique identity, stable across retries.
    pub id: String,
    pub session: String,
    pub created_ms: u64,
    pub turn: String,
    pub label: String,
    pub phase: String,
    /// Provider-owned message boundary and prefix digest; never credentials.
    pub boundary: Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    #[serde(flatten)]
    pub capture: Capture,
    pub workspace: Option<String>,
    pub revision: Option<String>,
    pub commit: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub points: Vec<Checkpoint>,
    pub before: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub kind: String,
    pub checkpoint_mode: String,
    pub current_mode: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub checkpoint: Checkpoint,
    pub current_revision: Option<String>,
    pub files: Vec<FileChange>,
    pub diff: String,
    pub complete: bool,
}
fn key(session: &str, id: &str) -> Result<String> {
    for value in [session, id] {
        ensure!(
            !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control),
            "Invalid checkpoint identity"
        );
    }
    Ok(format!("{session}\0{id}"))
}
fn encode<T: Serialize>(engine: &Engine, value: &T) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() <= MAX_RECORD_BYTES,
        "Checkpoint metadata exceeds limit"
    );
    Ok(zstd::stream::encode_all(
        bytes.as_slice(),
        engine.config.compression_level,
    )?)
}
fn decode(bytes: &[u8]) -> Result<Checkpoint> {
    let mut output = Vec::new();
    zstd::stream::Decoder::new(bytes)?
        .take(MAX_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut output)?;
    ensure!(
        output.len() <= MAX_RECORD_BYTES,
        "Stored checkpoint exceeds limit"
    );
    Ok(serde_json::from_slice(&output)?)
}
fn read(engine: &Engine, session: &str, id: &str) -> Result<Option<Checkpoint>> {
    let key = key(session, id)?;
    let txn = engine.db.begin_read()?;
    let table = match txn.open_table(POINTS) {
        Ok(table) => table,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let point = table
        .get(key.as_str())?
        .map(|v| decode(v.value()))
        .transpose()?;
    if let Some(point) = &point {
        ensure!(
            point.capture.session == session && point.capture.id == id,
            "Checkpoint identity does not match its index"
        );
    }
    Ok(point)
}
fn same_workspace(checkpoint: &Checkpoint, workspace: Option<&str>) -> Result<()> {
    ensure!(
        checkpoint.workspace.as_deref() == workspace,
        "Checkpoint belongs to a different workspace"
    );
    Ok(())
}
pub async fn capture(
    connection: &Connection,
    workspace: Option<String>,
    capture: Capture,
) -> Result<Checkpoint> {
    ensure!(
        capture.label.chars().count() <= 512
            && capture.turn.len() <= 256
            && matches!(
                capture.phase.as_str(),
                "before" | "after" | "manual" | "restore-backup"
            ),
        "Invalid checkpoint metadata"
    );
    let record_key = key(&capture.session, &capture.id)?;
    connection.run(move |store|store.with_engine(|engine| {
        if let Some(previous)=read(engine,&capture.session,&capture.id)? {
            same_workspace(&previous,workspace.as_deref())?;
            ensure!(previous.capture==capture,"Checkpoint identity already has different content");
            return Ok(previous);
        }
        // Validate size before publishing a pin or any metadata.
        encode(engine,&capture)?;
        let mut checkpoint=Checkpoint{capture,workspace,revision:None,commit:None};
        if let Some(name)=checkpoint.workspace.as_deref() {
            let ws=engine.workspace(name)?;
            // A Git ref alone is not a GC root: the public commit operation also
            // persists vwork's export pin. Always complete that operation.
            let commit=engine.call("workspace_commit",&json!({"workspace":name,"expected_revision":ws.head,"message":"Fastrock checkpoint"}))?["commit"].as_str().context("Missing checkpoint commit")?.to_owned();
            checkpoint.revision=Some(ws.head);checkpoint.commit=Some(commit);
        }
        let bytes=encode(engine,&checkpoint)?;
        let txn=engine.db.begin_write()?;
        {
            let mut table=txn.open_table(POINTS)?;
            let previous=table.get(record_key.as_str())?.map(|v|decode(v.value())).transpose()?;
            if let Some(previous)=previous {
                ensure!(previous.capture==checkpoint.capture && previous.workspace==checkpoint.workspace,"Checkpoint identity already has different content");
                return Ok(previous);
            }
            table.insert(record_key.as_str(),bytes.as_slice())?;
        }
        txn.commit()?;Ok(checkpoint)
    })).await
}
/// Delete conversation metadata and restore-history backups. Virtual source
/// revisions/commits belong to the workspace and remain intact.
pub async fn forget(connection: &Connection, session: String) -> Result<()> {
    key(&session, "delete")?;
    connection
        .run(move |store| {
            store.with_engine(|engine| {
                let transaction = engine.db.begin_write()?;
                journal::forget_records(&transaction, &session)?;
                remove_records(&transaction, POINTS, &session)?;
                transaction.commit()?;
                Ok(())
            })
        })
        .await
}
fn remove_records(
    transaction: &redb::WriteTransaction,
    definition: TableDefinition<&str, &[u8]>,
    session: &str,
) -> Result<()> {
    let mut table = transaction.open_table(definition)?;
    let start = format!("{session}\0");
    let end = format!("{session}\u{1}");
    let keys = table
        .range(start.as_str()..end.as_str())?
        .map(|entry| entry.map(|(key, _)| key.value().to_owned()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for key in keys {
        table.remove(key.as_str())?;
    }
    Ok(())
}

pub async fn get(connection: &Connection, session: String, id: String) -> Result<Checkpoint> {
    connection
        .run(move |store| {
            store.with_engine(|engine| {
                read(engine, &session, &id)?.context("Checkpoint is unavailable")
            })
        })
        .await
}
pub async fn list(
    connection: &Connection,
    session: String,
    before: Option<String>,
    limit: usize,
) -> Result<Page> {
    key(&session, "page")?;
    let limit = limit.clamp(1, 100);
    connection
        .run(move |store| {
            store.with_engine(|engine| {
                let start = format!("{session}\0");
                let end = match before {
                    Some(id) => key(&session, &id)?,
                    None => format!("{session}\u{1}"),
                };
                let txn = engine.db.begin_read()?;
                let table = match txn.open_table(POINTS) {
                    Ok(table) => table,
                    Err(redb::TableError::TableDoesNotExist(_)) => {
                        return Ok(Page {
                            points: Vec::new(),
                            before: None,
                        });
                    }
                    Err(e) => return Err(e.into()),
                };
                let mut points = Vec::with_capacity(limit);
                let mut more = false;
                for row in table
                    .range(start.as_str()..end.as_str())?
                    .rev()
                    .take(limit + 1)
                {
                    let (_, value) = row?;
                    if points.len() == limit {
                        more = true;
                        break;
                    }
                    let point = decode(value.value())?;
                    ensure!(
                        point.capture.session == session,
                        "Checkpoint session does not match its index"
                    );
                    points.push(point);
                }
                let before = more.then(|| points.last().unwrap().capture.id.clone());
                Ok(Page { points, before })
            })
        })
        .await
}
fn review_files(
    engine: &Engine,
    checkpoint: Checkpoint,
    workspace: Option<&str>,
) -> Result<Review> {
    same_workspace(&checkpoint, workspace)?;
    let mut review = Review {
        checkpoint,
        current_revision: None,
        files: Vec::new(),
        diff: String::new(),
        complete: true,
    };
    let Some(name) = workspace else {
        return Ok(review);
    };
    let target = review
        .checkpoint
        .revision
        .as_deref()
        .context("Checkpoint has no file revision")?;
    let ws = engine.workspace(name)?;
    ensure!(
        engine.revision(target)?.repo == ws.repo,
        "Checkpoint belongs to a different repository"
    );
    let (_, before) = engine.snapshot(&vwork_engine::model::Workspace {
        head: target.into(),
        ..ws.clone()
    })?;
    let (_, current) = engine.snapshot(&ws)?;
    let value = engine.call(
        "workspace_diff",
        &json!({"workspace":name,"against_revision":target,"limit":200}),
    )?;
    let diff = value["diff"].as_str().context("Missing checkpoint diff")?;
    let changes = value["changes"]
        .as_array()
        .context("Missing checkpoint changes")?;
    for row in changes {
        let path = row["path"].as_str().context("Missing changed path")?;
        review.files.push(FileChange {
            path: path.into(),
            kind: row["status"].as_str().unwrap_or("modified").into(),
            checkpoint_mode: before
                .get(path)
                .map_or("absent", |e| e.mode.as_str())
                .into(),
            current_mode: current
                .get(path)
                .map_or("absent", |e| e.mode.as_str())
                .into(),
        });
    }
    let mut end = diff.len().min(MAX_REVIEW_BYTES);
    while !diff.is_char_boundary(end) {
        end -= 1;
    }
    review.diff = diff[..end].into();
    review.complete = end == diff.len()
        && value["truncated"] == false
        && diff.len() < engine.config.max_output_bytes
        && changes.len() < 200.min(engine.config.max_results);
    review.current_revision = Some(ws.head);
    Ok(review)
}
pub async fn review(
    connection: &Connection,
    session: String,
    id: String,
    workspace: Option<String>,
) -> Result<Review> {
    connection
        .run(move |store| {
            store.with_exclusive_engine(|engine| {
                let checkpoint =
                    read(engine, &session, &id)?.context("Checkpoint is unavailable")?;
                review_files(engine, checkpoint, workspace.as_deref())
            })
        })
        .await
}
/// Restore through the unchanged revision guard. A stale or incomplete preview
/// cannot mutate files; original immutable revisions and the checkout survive.
pub async fn restore_files(connection: &Connection, review: Review) -> Result<String> {
    ensure!(
        review.complete,
        "Review is incomplete; narrow the changes before restoring"
    );
    connection
        .run(move |store| {
            store.with_exclusive_engine(|engine| {
                let current = read(
                    engine,
                    &review.checkpoint.capture.session,
                    &review.checkpoint.capture.id,
                )?
                .context("Checkpoint is unavailable")?;
                ensure!(
                    current == review.checkpoint,
                    "Checkpoint changed after review"
                );
                let name = current
                    .workspace
                    .as_deref()
                    .context("This checkpoint has no virtual source files")?;
                let ws = engine.workspace(name)?;
                ensure!(
                    Some(ws.head.as_str()) == review.current_revision.as_deref(),
                    "Workspace changed after review; review it again"
                );
                ensure!(
                    review_files(engine, current.clone(), Some(name))?.complete,
                    "Review is incomplete; narrow the changes before restoring"
                );
                let revision = current
                    .revision
                    .context("Checkpoint has no file revision")?;
                let result = engine.call(
                    "workspace_restore",
                    &json!({"workspace":name,"expected_revision":ws.head,"revision":revision}),
                )?;
                Ok(result["revision"]
                    .as_str()
                    .context("Missing restored revision")?
                    .into())
            })
        })
        .await
}
