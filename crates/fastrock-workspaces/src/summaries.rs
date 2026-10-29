//! One replaceable, lossless search summary in vwork's existing redb database.
use crate::embedded::Connection;
use anyhow::{Result, ensure};
use redb::{ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::io::Read;

const CURRENT: TableDefinition<&str, &[u8]> = TableDefinition::new("fastrock_search_current_v1");
const REQUESTED: TableDefinition<&str, u64> = TableDefinition::new("fastrock_search_requested_v1");
const MAX_SUMMARY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub revision: u64,
    pub text: String,
    pub profile: String,
    #[serde(default)]
    pub source_messages: usize,
    #[serde(default)]
    pub source_digest: String,
}

#[derive(Clone)]
pub struct Summaries(pub Connection);

impl Summaries {
    /// Allocate a durable generation even after a restart or a failed job.
    pub async fn advance(&self, id: String) -> Result<u64> {
        self.0
            .run(move |store| {
                store.with_engine(|engine| {
                    let transaction = engine.db.begin_write()?;
                    let next;
                    {
                        let mut requested = transaction.open_table(REQUESTED)?;
                        let previous = requested.get(id.as_str())?.map(|v| v.value()).unwrap_or(0);
                        next = previous
                            .checked_add(1)
                            .ok_or_else(|| anyhow::anyhow!("Summary generation exhausted"))?;
                        requested.insert(id.as_str(), next)?;
                        transaction.open_table(CURRENT)?;
                    }
                    transaction.commit()?;
                    Ok(next)
                })
            })
            .await
    }

    /// The caller's persisted user-turn ordinal is monotonic. Duplicate delivery
    /// is harmless, and a slow old completion cannot replace a newer summary.
    pub async fn request(&self, id: String, revision: u64) -> Result<()> {
        self.0
            .run(move |store| {
                store.with_engine(|engine| {
                    let transaction = engine.db.begin_write()?;
                    {
                        let mut table = transaction.open_table(REQUESTED)?;
                        let previous = table.get(id.as_str())?.map(|v| v.value());
                        if previous.is_none_or(|old| revision > old) {
                            table.insert(id.as_str(), revision)?;
                        }
                        transaction.open_table(CURRENT)?;
                    }
                    transaction.commit()?;
                    Ok(())
                })
            })
            .await
    }

    pub async fn replace(&self, id: String, summary: Summary) -> Result<bool> {
        self.0
            .run(move |store| {
                store.with_engine(|engine| {
                    let bytes = serde_json::to_vec(&summary)?;
                    ensure!(
                        bytes.len() <= MAX_SUMMARY_BYTES,
                        "Search summary exceeds storage limit"
                    );
                    let compressed = zstd::stream::encode_all(
                        bytes.as_slice(),
                        engine.config.compression_level,
                    )?;
                    let transaction = engine.db.begin_write()?;
                    {
                        let requested = transaction.open_table(REQUESTED)?;
                        if requested.get(id.as_str())?.map(|v| v.value()) != Some(summary.revision)
                        {
                            return Ok(false);
                        }
                        let mut current = transaction.open_table(CURRENT)?;
                        current.insert(id.as_str(), compressed.as_slice())?;
                    }
                    transaction.commit()?;
                    Ok(true)
                })
            })
            .await
    }

    pub async fn get(&self, id: String) -> Result<Option<Summary>> {
        self.0
            .run(move |store| {
                store.with_engine(|engine| {
                    let transaction = engine.db.begin_read()?;
                    let table = match transaction.open_table(CURRENT) {
                        Ok(table) => table,
                        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
                        Err(error) => return Err(error.into()),
                    };
                    table
                        .get(id.as_str())?
                        .map(|v| decode(v.value()))
                        .transpose()
                })
            })
            .await
    }

    /// Cursor paging bounds decompression and memory during index warm-up.
    pub async fn page(
        &self,
        after: Option<String>,
        limit: usize,
    ) -> Result<Vec<(String, Summary)>> {
        self.0
            .run(move |store| {
                store.with_engine(|engine| {
                    let transaction = engine.db.begin_read()?;
                    let table = match transaction.open_table(CURRENT) {
                        Ok(table) => table,
                        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(vec![]),
                        Err(error) => return Err(error.into()),
                    };
                    let start = after.as_deref().unwrap_or("");
                    let mut result = Vec::new();
                    for row in table.range(start..)? {
                        let (id, value) = row?;
                        if after.as_deref() == Some(id.value()) {
                            continue;
                        }
                        if result.len() >= limit.clamp(1, 128) {
                            break;
                        }
                        result.push((id.value().to_owned(), decode(value.value())?));
                    }
                    Ok(result)
                })
            })
            .await
    }
}

fn decode(bytes: &[u8]) -> Result<Summary> {
    let mut decoded = Vec::new();
    zstd::stream::Decoder::new(bytes)?
        .take(MAX_SUMMARY_BYTES as u64 + 1)
        .read_to_end(&mut decoded)?;
    ensure!(
        decoded.len() <= MAX_SUMMARY_BYTES,
        "Stored search summary exceeds limit"
    );
    Ok(serde_json::from_slice(&decoded)?)
}
