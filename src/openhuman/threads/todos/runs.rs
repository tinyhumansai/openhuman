//! Compatibility facade over [`tinyagents_graph::todos::runs`].
//!
//! TinyAgents owns the durable task-run record, the heartbeat, the staleness
//! policy, and the reclaim sweep (card back to `todo`, or parked at `blocked`
//! once a card has burned through its reclaim budget). What stays here is
//! OpenHuman's own shape around it: [`BoardLocation`] addressing (including the
//! process-global scratch board), RFC 3339 timestamps on the wire, the
//! `TaskRunReclaimed` domain event, and the one-time import of the retired
//! `{workspace}/agent_task_boards/<hex>.runs.json` ledger.
//!
//! Run records live in the crate KV store beside the board itself
//! (`graph.todos.runs`), so a board and its run log can no longer drift apart
//! across a restart.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tinyagents_graph::todos::runs as crate_runs;

pub use tinyagents_graph::todos::runs::{
    ReclaimDetail, ReclaimResult, RunLimits, RunOutcome, TaskRun, DEFAULT_CLAIM_TTL_SECS,
    DEFAULT_HEARTBEAT_STALE_SECS, DEFAULT_MAX_RECLAIM_COUNT,
};

use crate::openhuman::agent::task_board::normalize_timestamp_for_wire;

use super::ops::{target, BoardLocation};

/// Cadence of the background heartbeat spawned alongside an autonomous run.
const HEARTBEAT_TICK: std::time::Duration = crate_runs::DEFAULT_HEARTBEAT_TICK;

/// Legacy on-disk ledger the crate store replaced.
const TASK_BOARD_DIR: &str = "agent_task_boards";

fn map_err<T>(result: tinyagents_harness::error::Result<T>) -> Result<T, String> {
    result.map_err(|error| error.to_string())
}

/// Crate stamps are unix-epoch milliseconds; the `openhuman.todos_run_*` RPC
/// surface has always spoken RFC 3339, so translate on the way out.
fn for_wire(mut run: TaskRun) -> TaskRun {
    run.started_at = normalize_timestamp_for_wire(&run.started_at);
    run.last_heartbeat_at = normalize_timestamp_for_wire(&run.last_heartbeat_at);
    run.completed_at = run
        .completed_at
        .as_deref()
        .map(normalize_timestamp_for_wire);
    run
}

pub async fn create_run(
    location: &BoardLocation,
    run_id: &str,
    card_id: &str,
    claimed_by: &str,
) -> Result<TaskRun, String> {
    let (store, thread_id) = target(location);
    let run = map_err(
        crate_runs::create_run(&store, thread_id, Some(run_id), card_id, claimed_by).await,
    )?;
    Ok(for_wire(run))
}

pub async fn update_heartbeat(location: &BoardLocation, run_id: &str) -> Result<(), String> {
    let (store, thread_id) = target(location);
    map_err(crate_runs::update_heartbeat(&store, thread_id, run_id).await)
}

pub async fn complete_run(
    location: &BoardLocation,
    run_id: &str,
    outcome: RunOutcome,
    error: Option<String>,
    evidence: Vec<String>,
) -> Result<TaskRun, String> {
    let (store, thread_id) = target(location);
    let run = map_err(
        crate_runs::complete_run(&store, thread_id, run_id, outcome, error, evidence).await,
    )?;
    Ok(for_wire(run))
}

pub async fn list_runs(
    location: &BoardLocation,
    card_id: Option<&str>,
) -> Result<Vec<TaskRun>, String> {
    let (store, thread_id) = target(location);
    let runs = map_err(crate_runs::list_runs(&store, thread_id, card_id).await)?;
    Ok(runs.into_iter().map(for_wire).collect())
}

pub async fn get_run(location: &BoardLocation, run_id: &str) -> Result<Option<TaskRun>, String> {
    let (store, thread_id) = target(location);
    let run = map_err(crate_runs::get_run(&store, thread_id, run_id).await)?;
    Ok(run.map(for_wire))
}

pub async fn find_stale_runs(
    location: &BoardLocation,
    limits: &RunLimits,
) -> Result<Vec<(TaskRun, String)>, String> {
    let (store, thread_id) = target(location);
    let stale = map_err(crate_runs::find_stale_runs(&store, thread_id, limits).await)?;
    Ok(stale
        .into_iter()
        .map(|(run, reason)| (for_wire(run), reason))
        .collect())
}

/// Reclaim stale runs and publish a `TaskRunReclaimed` event per reclaimed
/// card, so the Tasks board UI sees a wedged card come back without a refresh.
pub async fn reclaim_stale(
    location: &BoardLocation,
    limits: &RunLimits,
) -> Result<ReclaimResult, String> {
    let (store, thread_id) = target(location);
    let result = map_err(crate_runs::reclaim_stale(&store, thread_id, limits).await)?;

    if let Some(thread_id) = location.thread_id() {
        for detail in &result.details {
            crate::core::bus::BUS.publish(crate::core::events::DomainEvent::TaskRunReclaimed {
                run_id: detail.run_id.clone(),
                card_id: detail.card_id.clone(),
                thread_id: thread_id.to_string(),
                reason: detail.reason.clone(),
            });
        }
    }
    Ok(result)
}

/// Tick the run's heartbeat in the background until it completes or `cancel`
/// fires. Board-location addressing is resolved once, here, so the crate task
/// carries only a store and a thread id.
pub fn spawn_heartbeat_task(
    location: BoardLocation,
    run_id: String,
    cancel: tokio::sync::watch::Receiver<bool>,
) {
    let (store, thread_id) = target(&location);
    crate_runs::spawn_heartbeat_task(store, thread_id.to_string(), run_id, cancel, HEARTBEAT_TICK);
}

// ── Legacy ledger migration ────────────────────────────────────────────

/// Outcome of the one-time `<hex>.runs.json` import.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRunMigrationReport {
    pub total: usize,
    pub copied: usize,
    pub skipped: usize,
}

/// Copy any run ledgers left in the retired file tree into the crate store,
/// without replacing runs the crate already holds.
///
/// A thread whose crate log is non-empty is skipped wholesale: the crate log is
/// authoritative, and merging two histories would double-count the reclaims the
/// sweep's `max_reclaim_count` budget is derived from.
pub async fn migrate_legacy_task_runs(
    workspace_dir: &Path,
) -> Result<TaskRunMigrationReport, String> {
    let dir = workspace_dir.join(TASK_BOARD_DIR);
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TaskRunMigrationReport::default());
        }
        Err(error) => return Err(format!("read legacy runs dir {}: {error}", dir.display())),
    };

    let mut report = TaskRunMigrationReport::default();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| format!("iterate legacy runs dir: {error}"))?
    {
        let path = entry.path();
        let Some(thread_id) = legacy_thread_id(&path) else {
            continue;
        };
        report.total += 1;

        let runs: Vec<TaskRun> = match tokio::fs::read_to_string(&path).await {
            Ok(body) => match serde_json::from_str(&body) {
                Ok(runs) => runs,
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "skip invalid legacy run ledger");
                    report.skipped += 1;
                    continue;
                }
            },
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "skip unreadable legacy run ledger");
                report.skipped += 1;
                continue;
            }
        };

        let location = BoardLocation::Thread {
            workspace_dir: workspace_dir.to_path_buf(),
            thread_id: thread_id.clone(),
        };
        let (store, thread_id) = target(&location);
        match map_err(crate_runs::import_if_absent(&store, thread_id, runs).await) {
            Ok(true) => report.copied += 1,
            Ok(false) => report.skipped += 1,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "skip legacy run ledger: store write failed");
                report.skipped += 1;
            }
        }
    }
    Ok(report)
}

/// Decode the thread id encoded in a `<hex>.runs.json` file name.
///
/// Only strict, ASCII, even-length lowercase hex is accepted. The inner two
/// bytes of every pair must both be hexadecimal digits (`0-9a-f`), so a signed
/// or malformed stem such as `+f` is rejected rather than accepted by
/// `u8::from_str_radix`. Decoding walks `hex.as_bytes()` in whole pairs, never
/// slicing a multi-byte UTF-8 character, so a non-ASCII stem like `aéb` returns
/// `None` instead of panicking mid-startup.
fn legacy_thread_id(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let hex = name.strip_suffix(".runs.json")?;
    if hex.is_empty() || hex.len() % 2 != 0 || !hex.is_ascii() {
        return None;
    }
    let bytes: Vec<u8> = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let hi = lowercase_hex_nibble(pair[0])?;
            let lo = lowercase_hex_nibble(pair[1])?;
            Some(hi * 16 + lo)
        })
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

/// Decode one ASCII byte as a lowercase hexadecimal nibble (`0-9a-f`), or
/// `None` for any other byte (including uppercase `A-F`).
fn lowercase_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
