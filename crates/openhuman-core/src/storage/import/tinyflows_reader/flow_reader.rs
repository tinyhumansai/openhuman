//! Bounded, host-side readers for the retired flow catalog and draft files.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tinyflows_bus::catalog::{
    LegacyDraftRecord, LegacyFlowRecord, LegacyImportRecord, LegacyRevisionRecord, LegacyRunRecord,
    LegacyStateRecord, LegacySuggestionRecord, MAX_IMPORT_BATCH_RECORDS,
    MAX_IMPORT_RECORD_PAYLOAD_BYTES,
};

use super::legacy_db::{column_or, LegacyDb};

/// Reads legacy flow database rows and draft files as bounded ordered batches.
///
/// The caller owns source discovery, the target storage scope and migration
/// marker. Only file contents and safe file names cross the callback boundary.
/// Each callback completes before the next batch is read and sent.
pub async fn read_legacy_flow_catalog<F, Fut>(dir: &Path, mut send: F) -> Result<()>
where
    F: FnMut(Vec<LegacyImportRecord>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut batches = BatchSender::default();
    if let Some(db) = LegacyDb::open(&dir.join("flows.db"))? {
        let tables = db.tables().await?;
        if tables.contains("flow_definitions") {
            let columns = db.columns("flow_definitions").await?;
            let select = format!(
                "SELECT id,name,graph_json,enabled,created_at,updated_at,last_run_at,last_status,{} FROM flow_definitions ORDER BY created_at,id LIMIT ?1 OFFSET ?2",
                column_or(&columns, "require_approval", "0")
            );
            let mut offset = 0_i64;
            loop {
                let query = select.clone();
                let rows = db
                    .read("flow definitions", move |conn| {
                        let mut statement = conn.prepare(&query)?;
                        let mapped = statement.query_map([64_i64, offset], |row| {
                            Ok(LegacyFlowRecord {
                                id: row.get(0)?,
                                name: row.get(1)?,
                                graph_json: row.get(2)?,
                                enabled: row.get::<_, i64>(3)? != 0,
                                created_at: row.get(4)?,
                                updated_at: row.get(5)?,
                                last_run_at: row.get(6)?,
                                last_status: row.get(7)?,
                                require_approval: row.get::<_, i64>(8)? != 0,
                            })
                        })?;
                        mapped.collect::<std::result::Result<Vec<_>, _>>()
                    })
                    .await?;
                if rows.is_empty() {
                    break;
                }
                offset += rows.len() as i64;
                for row in rows {
                    batches
                        .push(LegacyImportRecord::Flow(row), &mut send)
                        .await?;
                }
            }
        }
        if tables.contains("flow_revisions") {
            page_rows(&db, "flow revisions", "SELECT id,flow_id,graph_json,name,require_approval,created_at FROM flow_revisions ORDER BY created_at,id LIMIT ?1 OFFSET ?2", |row| {
                Ok(LegacyRevisionRecord { id: row.get(0)?, flow_id: row.get(1)?, graph_json: row.get(2)?, name: row.get(3)?, require_approval: row.get::<_, i64>(4)? != 0, created_at: row.get(5)? })
            }, LegacyImportRecord::Revision, &mut batches, &mut send).await?;
        }
        if tables.contains("flow_runs") {
            let columns = db.columns("flow_runs").await?;
            let select = format!(
                "SELECT id,flow_id,thread_id,status,started_at,finished_at,steps_json,pending_approvals_json,error,{} FROM flow_runs ORDER BY started_at,id LIMIT ?1 OFFSET ?2",
                column_or(&columns, "graph_hash", "NULL")
            );
            page_rows(
                &db,
                "flow runs",
                &select,
                |row| {
                    Ok(LegacyRunRecord {
                        id: row.get(0)?,
                        flow_id: row.get(1)?,
                        thread_id: row.get(2)?,
                        status: row.get(3)?,
                        started_at: row.get(4)?,
                        finished_at: row.get(5)?,
                        steps_json: row.get(6)?,
                        pending_approvals_json: row.get(7)?,
                        error: row.get(8)?,
                        graph_hash: row.get(9)?,
                    })
                },
                LegacyImportRecord::Run,
                &mut batches,
                &mut send,
            )
            .await?;
        }
        if tables.contains("flow_state") {
            page_rows(&db, "flow state", "SELECT namespace,key,value FROM flow_state ORDER BY namespace,key LIMIT ?1 OFFSET ?2", |row| {
                Ok(LegacyStateRecord { namespace: row.get(0)?, key: row.get(1)?, value_json: row.get(2)? })
            }, LegacyImportRecord::State, &mut batches, &mut send).await?;
        }
        if tables.contains("flow_suggestions") {
            page_rows(&db, "flow suggestions", "SELECT id,title,one_liner,rationale,trigger_hint,steps_json,connections_json,slugs_json,build_prompt,confidence,status,created_at,source_run_id FROM flow_suggestions ORDER BY created_at,id LIMIT ?1 OFFSET ?2", |row| {
                Ok(LegacySuggestionRecord { id: row.get(0)?, title: row.get(1)?, one_liner: row.get(2)?, rationale: row.get(3)?, trigger_hint: row.get(4)?, steps_json: row.get(5)?, connections_json: row.get(6)?, slugs_json: row.get(7)?, build_prompt: row.get(8)?, confidence: row.get(9)?, status: row.get(10)?, created_at: row.get(11)?, source_run_id: row.get(12)? })
            }, LegacyImportRecord::Suggestion, &mut batches, &mut send).await?;
        }
    }
    read_drafts(&dir.join("drafts"), &mut batches, &mut send).await?;
    batches.flush(&mut send).await
}

async fn page_rows<T, ToRecord, F, Fut>(
    db: &LegacyDb,
    what: &'static str,
    query: &str,
    decode: impl Fn(&crate::storage::rusqlite::Row<'_>) -> crate::storage::rusqlite::Result<T>
        + Send
        + Sync
        + 'static,
    to_record: ToRecord,
    batches: &mut BatchSender,
    send: &mut F,
) -> Result<()>
where
    T: Send + 'static,
    ToRecord: Fn(T) -> LegacyImportRecord,
    F: FnMut(Vec<LegacyImportRecord>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let query = query.to_string();
    let decode = Arc::new(decode);
    let mut offset = 0_i64;
    loop {
        let statement_query = query.clone();
        let decode = Arc::clone(&decode);
        let rows = db
            .read(what, move |conn| {
                let mut statement = conn.prepare(&statement_query)?;
                let mapped = statement.query_map([64_i64, offset], |row| decode(row))?;
                mapped.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await?;
        if rows.is_empty() {
            break;
        }
        offset += rows.len() as i64;
        for row in rows {
            batches.push(to_record(row), send).await?;
        }
    }
    Ok(())
}

async fn read_drafts<F, Fut>(dir: &Path, batches: &mut BatchSender, send: &mut F) -> Result<()>
where
    F: FnMut(Vec<LegacyImportRecord>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("listing {}", dir.display())),
    };
    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path.extension().is_some_and(|ext| ext == "json")
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        })
        .collect();
    paths.sort();
    for path in paths {
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        batches
            .push(
                LegacyImportRecord::Draft(LegacyDraftRecord { file_name, bytes }),
                send,
            )
            .await?;
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct BatchSender {
    current: Vec<LegacyImportRecord>,
    bytes: usize,
}

impl BatchSender {
    pub(crate) async fn push<F, Fut>(
        &mut self,
        mut record: LegacyImportRecord,
        send: &mut F,
    ) -> Result<()>
    where
        F: FnMut(Vec<LegacyImportRecord>) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let mut size = serde_json::to_vec(&record)?.len();
        if size > MAX_IMPORT_RECORD_PAYLOAD_BYTES {
            let kind = record.record_kind();
            let identity = record_identity(&record);
            record = LegacyImportRecord::Invalid {
                record_kind: kind,
                identity,
                message: "record exceeds the import byte limit".into(),
            };
            size = serde_json::to_vec(&record)?.len();
        }
        if !self.current.is_empty()
            && (self.current.len() >= MAX_IMPORT_BATCH_RECORDS
                || self.bytes + size > MAX_IMPORT_RECORD_PAYLOAD_BYTES)
        {
            self.flush(send).await?;
        }
        self.bytes += size;
        self.current.push(record);
        Ok(())
    }

    pub(crate) async fn flush<F, Fut>(&mut self, send: &mut F) -> Result<()>
    where
        F: FnMut(Vec<LegacyImportRecord>) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        if self.current.is_empty() {
            return Ok(());
        }
        self.bytes = 0;
        send(std::mem::take(&mut self.current)).await
    }
}

fn record_identity(record: &LegacyImportRecord) -> Option<String> {
    let value = match record {
        LegacyImportRecord::Flow(row) => Some(row.id.clone()),
        LegacyImportRecord::Revision(row) => Some(row.id.clone()),
        LegacyImportRecord::Run(row) => Some(row.id.clone()),
        LegacyImportRecord::State(row) => Some(row.key.clone()),
        LegacyImportRecord::Suggestion(row) => Some(row.id.clone()),
        LegacyImportRecord::Draft(row) => Some(row.file_name.clone()),
        LegacyImportRecord::FileCheckpointWrite { thread_id, .. } => Some(thread_id.clone()),
        LegacyImportRecord::SqliteCheckpointWrite(row) => Some(row.task_id.clone()),
        LegacyImportRecord::Checkpoint { .. } | LegacyImportRecord::Invalid { .. } => None,
    };
    value.map(|identity| identity.chars().take(128).collect())
}
