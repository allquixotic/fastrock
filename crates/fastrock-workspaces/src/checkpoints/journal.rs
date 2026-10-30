//! Cross-store restore intent. The intent is durable before either store changes.
//! A failed/interrupted operation stays pending until explicit review and action.
use super::*;
const JOURNALS: TableDefinition<&str, &[u8]> = TableDefinition::new("fastrock_restore_journals_v1");
const PENDING: TableDefinition<&str, &str> = TableDefinition::new("fastrock_restore_pending_v1");
const MAX_BACKUP: u64 = 256 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Journal {
    pub backup: Checkpoint,
    pub target: Checkpoint,
    pub files: bool,
    pub conversation: bool,
    pub original: Value,
    pub status: String,
}
fn load(engine: &Engine, session: &str, id: &str) -> Result<Journal> {
    let k = key(session, id)?;
    let txn = engine.db.begin_read()?;
    let table = txn.open_table(JOURNALS)?;
    let value = table
        .get(k.as_str())?
        .context("Restore backup is unavailable")?;
    let mut bytes = Vec::new();
    zstd::stream::Decoder::new(value.value())?
        .take(MAX_BACKUP + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BACKUP,
        "Restore backup exceeds limit"
    );
    let journal: Journal = serde_json::from_slice(&bytes)?;
    ensure!(
        journal.backup.capture.session == session && journal.backup.capture.id == id,
        "Restore backup identity mismatch"
    );
    Ok(journal)
}
fn pending_id(engine: &Engine, session: &str) -> Result<Option<String>> {
    key(session, "pending")?;
    let txn = engine.db.begin_read()?;
    let table = match txn.open_table(PENDING) {
        Ok(t) => t,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    Ok(table.get(session)?.map(|v| v.value().to_owned()))
}
fn save(engine: &Engine, journal: &Journal) -> Result<()> {
    let bytes = serde_json::to_vec(journal)?;
    ensure!(
        bytes.len() as u64 <= MAX_BACKUP,
        "Restore backup exceeds 256 MiB; files and conversation unchanged"
    );
    let bytes = zstd::stream::encode_all(bytes.as_slice(), engine.config.compression_level)?;
    let k = key(&journal.backup.capture.session, &journal.backup.capture.id)?;
    let txn = engine.db.begin_write()?;
    {
        txn.open_table(JOURNALS)?
            .insert(k.as_str(), bytes.as_slice())?;
        let mut table = txn.open_table(PENDING)?;
        if journal.status == "pending" {
            table.insert(
                journal.backup.capture.session.as_str(),
                journal.backup.capture.id.as_str(),
            )?;
        } else {
            table.remove(journal.backup.capture.session.as_str())?;
        }
    }
    txn.commit()?;
    Ok(())
}
pub(super) fn forget_records(transaction: &redb::WriteTransaction, session: &str) -> Result<()> {
    ensure!(
        transaction.open_table(PENDING)?.get(session)?.is_none(),
        "Resolve the pending restore before deleting its conversation"
    );
    super::remove_records(transaction, JOURNALS, session)
}
pub async fn get(connection: &Connection, session: String, id: String) -> Result<Journal> {
    connection
        .run(move |store| store.with_engine(|e| load(e, &session, &id)))
        .await
}
pub async fn pending(connection: &Connection, session: String) -> Result<Option<Journal>> {
    connection
        .run(move |store| {
            store.with_engine(|e| {
                pending_id(e, &session)?
                    .map(|id| load(e, &session, &id))
                    .transpose()
            })
        })
        .await
}
fn verified(engine: &Engine, review: &Review, files: bool) -> Result<()> {
    let point = read(
        engine,
        &review.checkpoint.capture.session,
        &review.checkpoint.capture.id,
    )?
    .context("Checkpoint is unavailable")?;
    ensure!(
        point == review.checkpoint,
        "Checkpoint changed after review"
    );
    if let Some(workspace) = point.workspace.as_deref() {
        ensure!(
            Some(engine.workspace(workspace)?.head.as_str()) == review.current_revision.as_deref(),
            "Workspace changed after review; review it again"
        );
        if files {
            ensure!(
                review.complete && review_files(engine, point.clone(), Some(workspace))?.complete,
                "Review is incomplete; files cannot be restored"
            );
            let leases = engine.call("temporary_list", &json!({}))?;
            ensure!(
                !leases["active"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|l| l["workspace"] == workspace),
                "Close the workspace's terminal and temporary tools before restoring files"
            );
        }
    } else {
        ensure!(!files, "Checkpoint has no virtual source files");
    }
    Ok(())
}
fn apply_files(engine: &Engine, review: &Review) -> Result<()> {
    engine.call("workspace_restore",&json!({"workspace":review.checkpoint.workspace,"expected_revision":review.current_revision,"revision":review.checkpoint.revision}))?;
    Ok(())
}
/// The provider callback runs on a blocking worker under the store's exclusive
/// mutation guard. It must only access provider storage, never call vwork/RPC.
pub async fn apply<F>(
    connection: &Connection,
    review: Review,
    mut journal: Journal,
    provider: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()> + Send + 'static,
{
    connection
        .run(move |store| {
            store.with_exclusive_engine(|engine| {
                ensure!(
                    pending_id(engine, &review.checkpoint.capture.session)?.is_none(),
                    "Review the interrupted restore first"
                );
                verified(engine, &review, journal.files)?;
                ensure!(
                    journal.target == review.checkpoint
                        && journal.backup.capture.session == review.checkpoint.capture.session
                        && journal.backup.workspace == review.checkpoint.workspace
                        && journal.backup.revision == review.current_revision,
                    "Restore backup does not match the reviewed state"
                );
                ensure!(
                    read(
                        engine,
                        &journal.backup.capture.session,
                        &journal.backup.capture.id
                    )?
                    .as_ref()
                        == Some(&journal.backup),
                    "Restore backup is unavailable"
                );
                ensure!(
                    journal.files || journal.conversation,
                    "Choose files or conversation to restore"
                );
                journal.status = "pending".into();
                save(engine, &journal)?;
                if journal.files {
                    apply_files(engine, &review)?;
                }
                provider()?;
                journal.status = "completed".into();
                save(engine, &journal)
            })
        })
        .await
}
/// Explicit recovery can restore the retained backup or keep the reviewed state.
/// A second interruption retains the same original backup and pending marker.
pub async fn resolve<F>(
    connection: &Connection,
    review: Review,
    id: String,
    rollback: bool,
    provider: F,
) -> Result<()>
where
    F: FnOnce(&Journal) -> Result<()> + Send + 'static,
{
    connection
        .run(move |store| {
            store.with_exclusive_engine(|engine| {
                let session = &review.checkpoint.capture.session;
                ensure!(
                    pending_id(engine, session)?.as_deref() == Some(&id),
                    "Interrupted restore changed; review it again"
                );
                let mut journal = load(engine, session, &id)?;
                ensure!(
                    review.checkpoint == journal.backup,
                    "Review the retained backup before resolving this restore"
                );
                verified(engine, &review, rollback && journal.files)?;
                if rollback && journal.files {
                    apply_files(engine, &review)?;
                }
                provider(&journal)?;
                journal.status = if rollback {
                    "rolled-back"
                } else {
                    "kept-current"
                }
                .into();
                save(engine, &journal)
            })
        })
        .await
}
