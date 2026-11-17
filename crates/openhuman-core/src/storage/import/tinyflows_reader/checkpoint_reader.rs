//! Bounded host-side readers for the retired SQLite and JSONL checkpointers.

use std::future::Future;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tinyflows_bus::catalog::{
    LegacyImportKind, LegacyImportRecord, LegacySqliteCheckpointWrite,
    MAX_IMPORT_RECORD_PAYLOAD_BYTES,
};

use super::flow_reader::BatchSender;
use super::legacy_db::LegacyDb;

const SQLITE_PAGE_ROWS: i64 = 8;

/// Reads SQLite and file/JSONL checkpoint sources as bounded callback batches.
///
/// A torn invalid final JSONL line is ignored as the retired file reader did.
/// The source paths remain host-side and are never serialized into records.
pub async fn read_legacy_checkpoint_batches<F, Fut>(
    sqlite_path: &Path,
    file_dir: &Path,
    send: F,
) -> Result<()>
where
    F: FnMut(Vec<LegacyImportRecord>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    read_sources(Some(sqlite_path), Some(file_dir), send).await
}

/// Reads only the retired SQLite checkpoint database as bounded raw batches.
pub async fn read_legacy_sqlite_checkpoint_batches<F, Fut>(
    sqlite_path: &Path,
    send: F,
) -> Result<()>
where
    F: FnMut(Vec<LegacyImportRecord>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    read_sources(Some(sqlite_path), None, send).await
}

/// Reads only the retired file/JSONL checkpoint directory as bounded raw batches.
pub async fn read_legacy_file_checkpoint_batches<F, Fut>(file_dir: &Path, send: F) -> Result<()>
where
    F: FnMut(Vec<LegacyImportRecord>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    read_sources(None, Some(file_dir), send).await
}

async fn read_sources<F, Fut>(
    sqlite_path: Option<&Path>,
    file_dir: Option<&Path>,
    mut send: F,
) -> Result<()>
where
    F: FnMut(Vec<LegacyImportRecord>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut batches = BatchSender::default();
    if let Some(sqlite_path) = sqlite_path {
        if let Some(db) = LegacyDb::open(sqlite_path)? {
            let tables = db.tables().await?;
            if tables.contains("checkpoints") {
                let mut offset = 0_i64;
                loop {
                    let rows = db
                    .read("checkpoints", move |connection| {
                        let mut statement = connection.prepare(
                            "SELECT CASE WHEN length(CAST(record AS BLOB)) <= ?1 THEN CAST(record AS BLOB) END, seq FROM checkpoints ORDER BY seq LIMIT ?2 OFFSET ?3",
                        )?;
                        let rows = statement
                            .query_map(
                                [MAX_IMPORT_RECORD_PAYLOAD_BYTES as i64, SQLITE_PAGE_ROWS, offset],
                                |row| {
                                    Ok((
                                        row.get::<_, Option<Vec<u8>>>(0)?,
                                        row.get::<_, i64>(1)?,
                                    ))
                                },
                            )?
                            .collect::<std::result::Result<Vec<_>, _>>();
                        rows
                    })
                    .await?;
                    if rows.is_empty() {
                        break;
                    }
                    offset += rows.len() as i64;
                    for (raw, sequence) in rows {
                        let record = match raw.and_then(|bytes| String::from_utf8(bytes).ok()) {
                        Some(raw_json) => LegacyImportRecord::Checkpoint { raw_json },
                        None => LegacyImportRecord::Invalid {
                            record_kind: LegacyImportKind::Checkpoint,
                            identity: Some(format!("seq:{sequence}")),
                            message: "checkpoint source record exceeds the import byte limit or is not UTF-8".into(),
                        },
                    };
                        batches.push(record, &mut send).await?;
                    }
                }
            }
            if tables.contains("checkpoint_writes") {
                let mut offset = 0_i64;
                loop {
                    let rows = db.read("checkpoint writes", move |connection| {
                    let mut statement = connection.prepare(
                        "WITH source_rows AS (SELECT rowid, thread_id, namespace, checkpoint_id, task_id, idx, node, channel, payload, length(COALESCE(CAST(thread_id AS BLOB), X'')) + length(COALESCE(CAST(namespace AS BLOB), X'')) + length(COALESCE(CAST(checkpoint_id AS BLOB), X'')) + length(COALESCE(CAST(task_id AS BLOB), X'')) + length(COALESCE(CAST(node AS BLOB), X'')) + length(COALESCE(CAST(channel AS BLOB), X'')) + length(COALESCE(CAST(payload AS BLOB), X'')) AS raw_bytes FROM checkpoint_writes) SELECT rowid, raw_bytes <= ?1, CASE WHEN raw_bytes <= ?1 THEN CAST(thread_id AS BLOB) END, CASE WHEN raw_bytes <= ?1 THEN CAST(namespace AS BLOB) END, CASE WHEN raw_bytes <= ?1 THEN CAST(checkpoint_id AS BLOB) END, CASE WHEN raw_bytes <= ?1 THEN CAST(task_id AS BLOB) END, idx, CASE WHEN raw_bytes <= ?1 THEN CAST(node AS BLOB) END, CASE WHEN raw_bytes <= ?1 THEN CAST(channel AS BLOB) END, CASE WHEN raw_bytes <= ?1 THEN CAST(payload AS BLOB) END FROM source_rows ORDER BY rowid LIMIT ?2 OFFSET ?3",
                    )?;
                    let mapped = statement.query_map(
                        [
                            MAX_IMPORT_RECORD_PAYLOAD_BYTES as i64,
                            SQLITE_PAGE_ROWS,
                            offset,
                        ],
                        |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, bool>(1)?,
                            row.get::<_, Option<Vec<u8>>>(2)?,
                            row.get::<_, Option<Vec<u8>>>(3)?,
                            row.get::<_, Option<Vec<u8>>>(4)?,
                            row.get::<_, Option<Vec<u8>>>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, Option<Vec<u8>>>(7)?,
                            row.get::<_, Option<Vec<u8>>>(8)?,
                            row.get::<_, Option<Vec<u8>>>(9)?,
                        ))
                        },
                    )?;
                    mapped.collect::<std::result::Result<Vec<_>, _>>()
                }).await?;
                    if rows.is_empty() {
                        break;
                    }
                    offset += rows.len() as i64;
                    for (
                        row_id,
                        within_limit,
                        thread,
                        namespace,
                        checkpoint,
                        task,
                        idx,
                        node,
                        channel,
                        payload,
                    ) in rows
                    {
                        let record = if within_limit {
                            let decoded = (|| {
                                Some(LegacySqliteCheckpointWrite {
                                    thread_id: String::from_utf8(thread?).ok()?,
                                    namespace_json: String::from_utf8(namespace?).ok()?,
                                    checkpoint_id: String::from_utf8(checkpoint?).ok()?,
                                    task_id: String::from_utf8(task?).ok()?,
                                    idx,
                                    node: String::from_utf8(node?).ok()?,
                                    channel: String::from_utf8(channel?).ok()?,
                                    payload_json: String::from_utf8(payload?).ok()?,
                                })
                            })();
                            decoded.map_or_else(
                                || LegacyImportRecord::Invalid {
                                    record_kind: LegacyImportKind::Write,
                                    identity: Some(format!("row:{row_id}")),
                                    message:
                                        "checkpoint write row contains a missing or non-UTF-8 field"
                                            .into(),
                                },
                                LegacyImportRecord::SqliteCheckpointWrite,
                            )
                        } else {
                            LegacyImportRecord::Invalid {
                                record_kind: LegacyImportKind::Write,
                                identity: Some(format!("row:{row_id}")),
                                message: "checkpoint write row exceeds the import byte limit"
                                    .into(),
                            }
                        };
                        batches.push(record, &mut send).await?;
                    }
                }
            }
        }
    }
    if let Some(file_dir) = file_dir {
        read_file_checkpoints(file_dir, &mut batches, &mut send).await?;
    }
    batches.flush(&mut send).await
}

async fn read_file_checkpoints<F, Fut>(
    dir: &Path,
    batches: &mut BatchSender,
    send: &mut F,
) -> Result<()>
where
    F: FnMut(Vec<LegacyImportRecord>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("listing {}", dir.display())),
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry
            .with_context(|| format!("reading an entry under {}", dir.display()))?
            .path();
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    for path in files {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Some(stem) = name.strip_suffix(".writes.jsonl") {
            let Some(thread_id) = unescape_thread_id(stem) else {
                batches
                    .push(
                        LegacyImportRecord::Invalid {
                            record_kind: LegacyImportKind::Write,
                            identity: Some(name.chars().take(128).collect()),
                            message: "not a valid checkpoint thread filename".into(),
                        },
                        send,
                    )
                    .await?;
                continue;
            };
            read_jsonl(
                &path,
                name,
                thread_id,
                LegacyImportKind::Write,
                batches,
                send,
            )
            .await?;
        } else if name.ends_with(".jsonl") {
            let Some(thread_id) = name.strip_suffix(".jsonl").and_then(unescape_thread_id) else {
                batches
                    .push(
                        LegacyImportRecord::Invalid {
                            record_kind: LegacyImportKind::Checkpoint,
                            identity: Some(name.chars().take(128).collect()),
                            message: "not a valid checkpoint thread filename".into(),
                        },
                        send,
                    )
                    .await?;
                continue;
            };
            read_jsonl(
                &path,
                name,
                thread_id,
                LegacyImportKind::Checkpoint,
                batches,
                send,
            )
            .await?;
        }
    }
    Ok(())
}

async fn read_jsonl<F, Fut>(
    path: &Path,
    name: &str,
    thread_id: String,
    kind: LegacyImportKind,
    batches: &mut BatchSender,
    send: &mut F,
) -> Result<()>
where
    F: FnMut(Vec<LegacyImportRecord>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut reader = BufReader::new(file);
    while let Some((line, terminated, oversized)) = read_bounded_line(&mut reader)? {
        if oversized {
            batches
                .push(
                    LegacyImportRecord::Invalid {
                        record_kind: kind,
                        identity: Some(thread_id.chars().take(128).collect()),
                        message: "checkpoint JSONL record exceeds the import byte limit".into(),
                    },
                    send,
                )
                .await?;
            continue;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if !terminated && serde_json::from_slice::<serde_json::Value>(&line).is_err() {
            tracing::warn!(target: "tinyflows_import", file = name, "[import] ignoring a torn trailing checkpoint line");
            continue;
        }
        let Ok(raw_json) = String::from_utf8(line) else {
            batches
                .push(
                    LegacyImportRecord::Invalid {
                        record_kind: kind,
                        identity: Some(thread_id.chars().take(128).collect()),
                        message: "checkpoint JSONL record is not UTF-8".into(),
                    },
                    send,
                )
                .await?;
            continue;
        };
        let record = match kind {
            LegacyImportKind::Checkpoint => LegacyImportRecord::Checkpoint { raw_json },
            LegacyImportKind::Write => LegacyImportRecord::FileCheckpointWrite {
                thread_id: thread_id.clone(),
                raw_json,
            },
            _ => unreachable!("only checkpoint and write files are read here"),
        };
        batches.push(record, send).await?;
    }
    Ok(())
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<Option<(Vec<u8>, bool, bool)>> {
    let mut line = Vec::new();
    let mut saw_bytes = false;
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(saw_bytes.then_some((line, false, oversized)));
        }
        saw_bytes = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let content_len = newline.unwrap_or(consumed);
        if !oversized {
            if line.len() + content_len > MAX_IMPORT_RECORD_PAYLOAD_BYTES {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(&available[..content_len]);
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some((line, true, oversized)));
        }
    }
}

pub(crate) fn unescape_thread_id(escaped: &str) -> Option<String> {
    let bytes = escaped.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = escaped.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}
