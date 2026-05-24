//! SQLite insert helper for `mcp_writes`.
//!
//! Writes go through the existing memory-tree connection cache
//! (`tree::store::with_connection`) — Q1 = A per #2536 (colocated
//! storage). This keeps the cache, breaker, and migration story
//! aligned with the rest of the memory subsystem and avoids opening
//! a second SQLite handle just for audit rows.
//!
//! The actual `CREATE TABLE IF NOT EXISTS mcp_writes (...)` lives in
//! `tree::store::SCHEMA` so the table is initialised once per
//! connection (alongside the chunk / summary / embedding tables)
//! rather than re-checked on every audit insert.

use anyhow::{Context, Result};
use rusqlite::params;

use super::types::McpWriteRecord;
use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::store as tree_store;

/// Cap on `error_message` length at insert time. A runaway error stack
/// shouldn't bloat the audit table; 1 KiB is enough for any meaningful
/// upstream error (typical `doc_put` errors are < 200 chars).
const ERROR_MESSAGE_MAX_BYTES: usize = 1024;

/// Best-effort insert: returns an error rather than panicking so the
/// caller can `let _ = record_write(...).await` and keep going if the
/// audit insert fails (Q2 = A in #2536 — write availability is not
/// degraded by the audit subsystem).
///
/// The function is **synchronous** internally (SQLite-bound), but
/// exposed via `tokio::task::spawn_blocking` so the async caller doesn't
/// block its runtime thread on disk I/O — same pattern the rest of
/// `memory_store::chunks::store` follows for its sync rusqlite operations.
pub async fn record_write(config: &Config, record: McpWriteRecord) -> Result<()> {
    // Cap error_message before crossing the await boundary so the
    // serialised form going to the worker thread is already bounded.
    let mut record = record;
    if let Some(ref msg) = record.error_message {
        if msg.len() > ERROR_MESSAGE_MAX_BYTES {
            // Truncate at a UTF-8 char boundary to keep the resulting
            // string valid even if the slice point lands mid-codepoint.
            let mut end = ERROR_MESSAGE_MAX_BYTES;
            while end > 0 && !msg.is_char_boundary(end) {
                end -= 1;
            }
            record.error_message = Some(msg[..end].to_string());
        }
    }

    let config = config.clone();
    tokio::task::spawn_blocking(move || insert_blocking(&config, &record))
        .await
        .context("audit insert task panicked")?
}

fn insert_blocking(config: &Config, record: &McpWriteRecord) -> Result<()> {
    tree_store::with_connection::<()>(config, |conn| {
        conn.execute(
            "INSERT INTO mcp_writes (
                timestamp_ms,
                client_source_type,
                tool_name,
                args_summary,
                resulting_chunk_id,
                success,
                error_message
            ) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                record.timestamp_ms,
                record.client_source_type,
                record.tool_name,
                record.args_summary,
                record.resulting_chunk_id,
                record.success as i32,
                record.error_message,
            ],
        )
        .context("INSERT INTO mcp_writes failed")?;
        Ok(())
    })
}

/// Read-side helper used by tests (and, eventually, the
/// `openhuman.mcp_audit_list` RPC once #2536 Q4 is ratified). Returns
/// the most recent N rows ordered by `timestamp_ms DESC`.
///
/// Kept `pub(crate)` for now — once the RPC lands this graduates to
/// `pub` and the RPC handler wraps it.
#[cfg(test)]
pub(crate) async fn recent(config: &Config, limit: u32) -> Result<Vec<McpWriteRecord>> {
    let config = config.clone();
    tokio::task::spawn_blocking(move || recent_blocking(&config, limit))
        .await
        .context("audit recent task panicked")?
}

#[cfg(test)]
fn recent_blocking(config: &Config, limit: u32) -> Result<Vec<McpWriteRecord>> {
    tree_store::with_connection(config, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT timestamp_ms, client_source_type, tool_name,
                        args_summary, resulting_chunk_id, success, error_message
                   FROM mcp_writes
                  ORDER BY timestamp_ms DESC
                  LIMIT ?",
            )
            .context("prepare SELECT mcp_writes")?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(McpWriteRecord {
                    timestamp_ms: row.get(0)?,
                    client_source_type: row.get(1)?,
                    tool_name: row.get(2)?,
                    args_summary: row.get(3)?,
                    resulting_chunk_id: row.get(4)?,
                    success: row.get::<_, i32>(5)? != 0,
                    error_message: row.get(6)?,
                })
            })
            .context("query mcp_writes")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect mcp_writes rows")?;
        Ok(rows)
    })
}
