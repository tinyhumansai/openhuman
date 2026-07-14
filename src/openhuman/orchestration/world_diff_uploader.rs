//! Periodic world-diff flusher.
//!
//! Drains the device `world_obs` buffer (compact observations produced from
//! locally-observed events) and uploads it to the hosted brain via
//! `POST /orchestration/v1/world-diff`, whose receipt schedules the subconscious
//! steering tick. Best-effort: a failed flush leaves rows buffered for the next
//! tick, and a replayed batch is an idempotent 202 server-side (dedupe on
//! `(userId, sessionId, seq)`).

use std::collections::BTreeMap;
use std::time::Duration;

use crate::openhuman::config::Config;

use super::store;
use super::wire::{WorldDiffBatchWire, WorldDiffEntryWire};

const LOG: &str = "orchestration";

/// Max observations drained per flush — bounds a single upload burst.
const DRAIN_LIMIT: u32 = 200;

/// Default cadence of the periodic flush loop.
pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_secs(30);

/// Drain the buffer once and upload it, grouped per session (the world-diff route
/// keys on a single `sessionId`). Returns the number of entries uploaded. A
/// session's rows are deleted only after its batch is accepted, so a transient
/// failure retries next tick.
pub async fn flush_once(config: &Config) -> Result<usize, String> {
    let rows = store::with_connection(&config.workspace_dir, |conn| {
        store::drain_world_obs(conn, DRAIN_LIMIT)
    })
    .map_err(|e| format!("drain world_obs: {e}"))?;
    if rows.is_empty() {
        return Ok(0);
    }

    let mut by_session: BTreeMap<String, Vec<store::WorldObs>> = BTreeMap::new();
    for row in rows {
        by_session
            .entry(row.session_id.clone())
            .or_default()
            .push(row);
    }

    let mut uploaded = 0usize;
    for (session_id, group) in by_session {
        let entries: Vec<WorldDiffEntryWire> = group
            .iter()
            .map(|r| WorldDiffEntryWire::build(r.seq, &r.note, r.ts))
            .collect();
        let batch = WorldDiffBatchWire::build(&session_id, entries);
        match super::cloud::push_world_diff(config, &batch).await {
            Ok(()) => {
                let ids: Vec<i64> = group.iter().map(|r| r.id).collect();
                let n = ids.len();
                if let Err(e) = store::with_connection(&config.workspace_dir, |conn| {
                    store::delete_world_obs(conn, &ids)
                }) {
                    // Uploaded but not cleared — a re-upload is an idempotent 202
                    // no-op server-side (dedupe on seq). Safe but noisy; log it.
                    log::warn!(
                        target: LOG,
                        "[orchestration] world_diff.flush.clear_failed session={session_id} err={e}"
                    );
                }
                uploaded += n;
            }
            Err(e) => {
                log::warn!(
                    target: LOG,
                    "[orchestration] world_diff.flush.upload_failed session={session_id} err={e}"
                );
                // Leave the rows buffered for the next tick.
            }
        }
    }
    log::debug!(target: LOG, "[orchestration] world_diff.flush uploaded={uploaded}");
    Ok(uploaded)
}

/// Run the periodic flush loop until the process exits. Spawned once after login.
/// The first (immediate) interval tick is skipped so the first flush waits a full
/// interval, giving the socket time to come up.
pub async fn run_flush_loop(config: Config, interval: Duration) {
    let mut tick = tokio::time::interval(interval);
    tick.tick().await;
    loop {
        tick.tick().await;
        if !config.orchestration.enabled {
            continue;
        }
        if let Err(e) = flush_once(&config).await {
            log::warn!(target: LOG, "[orchestration] world_diff.flush.error err={e}");
        }
    }
}
