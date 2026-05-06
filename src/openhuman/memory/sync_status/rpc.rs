//! JSON-RPC handler for `openhuman.memory_sync_status_list` (#1136).
//!
//! Single SQL query against `mem_tree_chunks`. Two layers of metrics:
//!
//!   * **Lifetime** — `chunks_synced` (total ingested), `chunks_pending`
//!     (`embedding IS NULL` = still in the extract+embed queue, not
//!     yet appended to the source-tree buffer).
//!
//!   * **Active sync wave** — `batch_total` / `batch_processed`. The
//!     wave is identified by a *time-cluster anchor*: the earliest
//!     chunk within `WAVE_WINDOW_MS` of the most recent chunk (per
//!     provider). A typical sync ingests its whole batch in seconds,
//!     so a 10-minute window cleanly captures one wave; if no new
//!     chunks arrive, the anchor stays put. Two syncs <10min apart
//!     merge into one wave (acceptable — they're contiguous activity).
//!
//! Stateless: no per-process Mutex, no persisted side table. Pure SQL
//! + the chunks table. Survives restart, safe across multiple core
//! processes.
//!
//! Trade-off: pending chunks older than `WAVE_WINDOW_MS` (e.g.,
//! leftovers from a stuck earlier wave when the worker was offline)
//! show up in lifetime `chunks_pending` but not in `batch_total` —
//! deliberately, since they shouldn't pollute the active wave's
//! progress signal.

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::store::with_connection;
use crate::rpc::RpcOutcome;

use super::types::{FreshnessLabel, MemorySyncStatus, StatusListResponse};

/// Sliding window used to identify a "current sync wave". Chunks
/// within this many ms of `MAX(created_at_ms)` for a provider count
/// as part of the wave; older chunks fall out.
const WAVE_WINDOW_MS: i64 = 10 * 60 * 1000;

/// `openhuman.memory_sync_status_list` — one row per provider that
/// has chunks, with lifetime + active-wave counters and a freshness
/// label.
pub async fn status_list_rpc(config: &Config) -> Result<RpcOutcome<StatusListResponse>, String> {
    tracing::debug!("[memory_sync_status][rpc] status_list");

    let config = config.clone();
    let statuses: Vec<MemorySyncStatus> = tokio::task::spawn_blocking(move || {
        with_connection(&config, |conn| -> anyhow::Result<Vec<MemorySyncStatus>> {
            // Provider parsed from `source_id` prefix (substring before
            // first ':'); falls back to `source_kind` when no prefix.
            //
            // `provider_chunks` projects per-row provider + the columns
            // we need. `provider_pending` flags providers that still
            // have at least one chunk waiting for an embedding —
            // `wave_anchors` is gated on this so a fully-drained
            // provider gets `batch_total = batch_processed = 0` (the
            // UI then hides the progress bar instead of rendering a
            // completed one for an idle connection). `wave_anchors`
            // finds the earliest chunk within WAVE_WINDOW_MS of the
            // most recent — the wave's start. The outer SELECT joins
            // back to count both lifetime and in-wave totals.
            let mut stmt = conn.prepare(
                "WITH provider_chunks AS ( \
                    SELECT \
                        CASE \
                            WHEN INSTR(source_id, ':') > 0 \
                                THEN SUBSTR(source_id, 1, INSTR(source_id, ':') - 1) \
                            ELSE source_kind \
                        END AS provider, \
                        created_at_ms, \
                        embedding, \
                        timestamp_ms \
                    FROM mem_tree_chunks \
                 ), \
                 provider_max AS ( \
                    SELECT provider, MAX(created_at_ms) AS max_created \
                    FROM provider_chunks \
                    GROUP BY provider \
                 ), \
                 provider_pending AS ( \
                    SELECT provider, \
                           SUM(CASE WHEN embedding IS NULL THEN 1 ELSE 0 END) AS pending \
                    FROM provider_chunks \
                    GROUP BY provider \
                 ), \
                 wave_anchors AS ( \
                    SELECT p.provider, MIN(p.created_at_ms) AS anchor \
                    FROM provider_chunks p \
                    JOIN provider_max m ON p.provider = m.provider \
                    JOIN provider_pending pp ON p.provider = pp.provider \
                    WHERE pp.pending > 0 \
                      AND p.created_at_ms >= m.max_created - ?1 \
                    GROUP BY p.provider \
                 ) \
                 SELECT \
                    p.provider, \
                    COUNT(*) AS chunks_synced, \
                    SUM(CASE WHEN p.embedding IS NULL THEN 1 ELSE 0 END) AS chunks_pending, \
                    SUM(CASE WHEN w.anchor IS NOT NULL \
                             AND p.created_at_ms >= w.anchor \
                             THEN 1 ELSE 0 END) AS batch_total, \
                    SUM(CASE WHEN w.anchor IS NOT NULL \
                             AND p.created_at_ms >= w.anchor \
                             AND p.embedding IS NOT NULL \
                             THEN 1 ELSE 0 END) AS batch_processed, \
                    MAX(p.timestamp_ms) AS last_chunk_at_ms \
                 FROM provider_chunks p \
                 LEFT JOIN wave_anchors w ON p.provider = w.provider \
                 GROUP BY p.provider \
                 ORDER BY last_chunk_at_ms DESC",
            )?;
            let now_ms = chrono::Utc::now().timestamp_millis();
            let iter = stmt.query_map([WAVE_WINDOW_MS], |row| {
                let provider: String = row.get(0)?;
                let chunks_synced: i64 = row.get(1)?;
                let chunks_pending: i64 = row.get(2)?;
                let batch_total: i64 = row.get(3)?;
                let batch_processed: i64 = row.get(4)?;
                let last_chunk_at_ms: Option<i64> = row.get(5)?;
                Ok(MemorySyncStatus {
                    provider,
                    chunks_synced: chunks_synced.max(0) as u64,
                    chunks_pending: chunks_pending.max(0) as u64,
                    batch_total: batch_total.max(0) as u64,
                    batch_processed: batch_processed.max(0) as u64,
                    last_chunk_at_ms,
                    freshness: FreshnessLabel::from_age_ms(last_chunk_at_ms, now_ms),
                })
            })?;
            let out = iter.collect::<Result<Vec<_>, _>>()?;
            Ok(out)
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking join failed: {e}"))?
    .map_err(|e| format!("memory_tree DB access failed: {e:#}"))?;

    tracing::debug!(
        "[memory_sync_status][rpc] status_list returning {} row(s)",
        statuses.len()
    );
    // No `single_log` wrapper: the controller serializes
    // `RpcOutcome::into_cli_compatible_json`, and a non-empty `logs` list
    // wraps the value in `{ result, logs }`. The frontend reads
    // `resp.statuses` directly, so any envelope here breaks parsing.
    Ok(RpcOutcome::new(StatusListResponse { statuses }, vec![]))
}
