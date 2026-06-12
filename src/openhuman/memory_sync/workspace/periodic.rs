//! Periodic sync scheduler for workspace (non-Composio) memory sources.
//!
//! The Composio scheduler (`memory_sync::composio::periodic`) walks
//! Composio *connections* exclusively — GitHub repos, folders, RSS feeds
//! and web pages registered in `config.memory_sources` were only ever
//! synced when the user pressed "Sync now" (the `memory_sources.sync`
//! RPC). A GitHub source would sync once at setup and then silently go
//! stale forever. This loop closes that gap: it walks the registry on a
//! fixed tick and fires the existing [`sync_source`] dispatcher for every
//! enabled workspace-kind source whose cadence has elapsed.
//!
//! Cadence semantics mirror the Composio loop (#3302):
//! - `config.memory_sync_interval_secs == Some(0)` → "Manual only", the
//!   loop skips every source.
//! - `Some(n)` → sync every `max(n, 24h-default)` seconds.
//! - `None` → the 24h default.
//!
//! Due-check sources, in priority order:
//! 1. the in-memory fired-at map (most accurate within this process), and
//! 2. the persisted sync-audit log — keyed by `source_id` with the
//!    source's own `source_kind` — so a configured cadence survives app
//!    restarts instead of re-firing on every cold start.
//!
//! `sync_source` itself owns overlap protection (per-source `ACTIVE_SYNCS`
//! mutex), audit writes, and post-sync raw-coverage reconcile
//! (`check_and_rebuild_tree`), so this loop stays a thin cadence driver.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::time::interval;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::DEFAULT_MEMORY_SYNC_INTERVAL_SECS;
use crate::openhuman::memory_sources::sync::sync_source;
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};
use crate::openhuman::memory_sync::composio::periodic::{
    connection_is_due, effective_interval_secs, periodic_pause_reason,
};
use crate::openhuman::memory_sync::sources::audit::{read_audit_log, SyncAuditEntry};
use crate::openhuman::scheduler_gate::gate::resume_notify;

/// How often the scheduler wakes up to look for due syncs. Matches the
/// Composio loop's cadence — per-source intervals (24h default) bound the
/// actual sync frequency; this only bounds how far past due we can drift.
const TICK_SECONDS: u64 = 1200;

/// Process-wide guard: only the first call spawns the loop.
static SCHEDULER_STARTED: OnceLock<()> = OnceLock::new();

/// `source_id → last fired-at instant` for this process lifetime. Recorded
/// at *fire* time (the sync runs detached in `sync_source`'s spawned task),
/// so a failing source retries on the next due boundary, not every tick.
type FiredAtMap = Arc<Mutex<HashMap<String, Instant>>>;

static LAST_FIRED_AT: OnceLock<FiredAtMap> = OnceLock::new();

fn fired_map() -> FiredAtMap {
    LAST_FIRED_AT
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

/// Source kinds this loop schedules. Composio is owned by the Composio
/// scheduler; Conversation/Twitter have no periodic pull semantics today.
fn is_workspace_synced_kind(kind: &SourceKind) -> bool {
    matches!(
        kind,
        SourceKind::GithubRepo | SourceKind::Folder | SourceKind::RssFeed | SourceKind::WebPage
    )
}

/// Index `source_id → most recent successful sync timestamp` from the
/// persisted audit log, restricted to workspace source kinds. Failed runs
/// are skipped (matching the in-memory semantics — a failure retries at
/// the next tick after the cadence elapses).
fn index_last_success_by_source_id(entries: &[SyncAuditEntry]) -> HashMap<String, DateTime<Utc>> {
    let mut idx: HashMap<String, DateTime<Utc>> = HashMap::new();
    for e in entries {
        if !e.success {
            continue;
        }
        let is_workspace_kind = matches!(
            e.source_kind.as_str(),
            "github_repo" | "folder" | "rss_feed" | "web_page"
        );
        if !is_workspace_kind {
            continue;
        }
        idx.entry(e.source_id.clone())
            .and_modify(|t| {
                if e.timestamp > *t {
                    *t = e.timestamp;
                }
            })
            .or_insert(e.timestamp);
    }
    idx
}

/// Wall-clock elapsed since the persisted last success, saturating at zero
/// for clock skew. `None` when the source has never successfully synced.
fn persisted_since_last_sync(
    idx: &HashMap<String, DateTime<Utc>>,
    source_id: &str,
    now: DateTime<Utc>,
) -> Option<Duration> {
    idx.get(source_id).map(|ts| {
        let secs = (now - *ts).num_seconds().max(0) as u64;
        Duration::from_secs(secs)
    })
}

/// Spawn the workspace-source periodic sync task. Idempotent.
pub fn start_workspace_periodic_sync() {
    if SCHEDULER_STARTED.set(()).is_err() {
        tracing::debug!("[memory_sync:workspace:periodic] scheduler already running");
        return;
    }
    tokio::spawn(async move {
        tracing::info!(
            tick_seconds = TICK_SECONDS,
            "[memory_sync:workspace:periodic] scheduler starting"
        );
        run_loop().await;
        tracing::error!("[memory_sync:workspace:periodic] scheduler loop exited");
    });
}

/// Tick loop: wakes on the steady cadence or a scheduler-gate resume
/// (Memory Tree toggled back on / sign-in), same shape as the Composio
/// loop — resume runs a tick immediately and re-bases the ticker.
async fn run_loop() {
    let mut ticker = interval(Duration::from_secs(TICK_SECONDS));
    let resume = resume_notify();
    // Skip the immediate-fire tick so startup isn't slammed before sign-in.
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = resume.notified() => {
                ticker.reset();
            }
        }
        if let Err(e) = run_one_tick().await {
            tracing::warn!(
                error = %e,
                "[memory_sync:workspace:periodic] tick failed (continuing)"
            );
        }
    }
}

/// Run a single scheduler tick. `pub(crate)` so tests can drive ticks
/// without the real interval.
pub(crate) async fn run_one_tick() -> Result<(), String> {
    // Honour the same pause reasons as the Composio loop: user toggled
    // Memory Tree off, or signed out.
    if let Some(reason) = periodic_pause_reason() {
        tracing::debug!(
            reason = reason.as_str(),
            "[memory_sync:workspace:periodic] scheduler-gate paused — skipping tick"
        );
        return Ok(());
    }

    let config = config_rpc::load_config_with_timeout()
        .await
        .map_err(|e| format!("load_config: {e}"))?;

    let global_interval = config.memory_sync_interval_secs;
    let Some(interval_secs) =
        effective_interval_secs(DEFAULT_MEMORY_SYNC_INTERVAL_SECS, global_interval)
    else {
        tracing::debug!(
            "[memory_sync:workspace:periodic] manual-only mode — skipping all workspace sources"
        );
        return Ok(());
    };

    let audit_index = index_last_success_by_source_id(&read_audit_log(&config));
    let now = Utc::now();
    let map = fired_map();

    let due_sources: Vec<MemorySourceEntry> = config
        .memory_sources
        .iter()
        .filter(|s| s.enabled && is_workspace_synced_kind(&s.kind))
        .filter(|s| {
            let since = {
                let guard = map.lock().unwrap_or_else(|e| e.into_inner());
                guard.get(&s.id).map(|when| when.elapsed())
            }
            .or_else(|| persisted_since_last_sync(&audit_index, &s.id, now));
            connection_is_due(interval_secs, since)
        })
        .cloned()
        .collect();

    if due_sources.is_empty() {
        tracing::debug!("[memory_sync:workspace:periodic] tick complete — nothing due");
        return Ok(());
    }

    let mut fired = 0usize;
    for source in due_sources {
        let source_id = source.id.clone();
        let kind = source.kind.as_str();
        tracing::info!(
            source_id = %source_id,
            kind = %kind,
            interval_secs,
            "[memory_sync:workspace:periodic] firing sync"
        );
        // sync_source spawns the actual work and returns immediately; it
        // rejects overlapping syncs of the same source internally.
        match sync_source(source, config.clone()).await {
            Ok(()) => {
                if let Ok(mut guard) = map.lock() {
                    guard.insert(source_id, Instant::now());
                }
                fired += 1;
            }
            Err(e) => {
                tracing::warn!(
                    source_id = %source_id,
                    kind = %kind,
                    error = %e,
                    "[memory_sync:workspace:periodic] sync dispatch failed (will retry next tick)"
                );
            }
        }
    }

    tracing::debug!(fired, "[memory_sync:workspace:periodic] tick complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source_id: &str, kind: &str, success: bool, ts: DateTime<Utc>) -> SyncAuditEntry {
        SyncAuditEntry {
            timestamp: ts,
            source_id: source_id.to_string(),
            source_kind: kind.to_string(),
            scope: format!("{kind}:{source_id}"),
            items_fetched: 1,
            batches: 0,
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost_usd: 0.0,
            composio_actions_called: 0,
            composio_cost_usd: 0.0,
            actual_charged_usd: None,
            duration_ms: 10,
            success,
            error: None,
        }
    }

    #[test]
    fn workspace_kinds_are_scheduled_composio_is_not() {
        assert!(is_workspace_synced_kind(&SourceKind::GithubRepo));
        assert!(is_workspace_synced_kind(&SourceKind::Folder));
        assert!(is_workspace_synced_kind(&SourceKind::RssFeed));
        assert!(is_workspace_synced_kind(&SourceKind::WebPage));
        assert!(!is_workspace_synced_kind(&SourceKind::Composio));
        assert!(!is_workspace_synced_kind(&SourceKind::Conversation));
        assert!(!is_workspace_synced_kind(&SourceKind::TwitterQuery));
    }

    #[test]
    fn audit_index_keeps_latest_workspace_success_and_skips_others() {
        let now = Utc::now();
        let older = now - chrono::Duration::hours(30);
        let newer = now - chrono::Duration::hours(2);
        let entries = vec![
            entry("src_gh", "github_repo", true, older),
            entry("src_gh", "github_repo", true, newer), // newest success wins
            entry("src_gh", "github_repo", false, now),  // failure ignored
            entry("conn_1", "composio", true, now),      // composio kind ignored
        ];
        let idx = index_last_success_by_source_id(&entries);
        assert_eq!(idx.get("src_gh"), Some(&newer));
        assert!(!idx.contains_key("conn_1"));
    }

    /// The headline regression: a GitHub source that synced once long ago
    /// must read as DUE under the default 24h cadence — before this loop
    /// existed, nothing ever consulted that staleness, so the source went
    /// permanently dark after its first manual sync.
    #[test]
    fn stale_github_source_is_due_fresh_one_is_not() {
        let now = Utc::now();
        let mut idx = HashMap::new();
        idx.insert("src_stale".to_string(), now - chrono::Duration::days(5));
        idx.insert("src_fresh".to_string(), now - chrono::Duration::hours(1));

        let interval =
            effective_interval_secs(DEFAULT_MEMORY_SYNC_INTERVAL_SECS, None).expect("interval");

        let stale = persisted_since_last_sync(&idx, "src_stale", now);
        assert!(connection_is_due(interval, stale), "5-day-old sync is due");

        let fresh = persisted_since_last_sync(&idx, "src_fresh", now);
        assert!(
            !connection_is_due(interval, fresh),
            "1h-old sync is not due"
        );

        // Never-synced source fires immediately.
        let never = persisted_since_last_sync(&idx, "src_new", now);
        assert!(connection_is_due(interval, never));
    }

    #[test]
    fn manual_only_global_setting_disables_the_loop() {
        assert_eq!(
            effective_interval_secs(DEFAULT_MEMORY_SYNC_INTERVAL_SECS, Some(0)),
            None
        );
    }

    #[test]
    fn persisted_since_last_sync_saturates_clock_skew() {
        let now = Utc::now();
        let mut idx = HashMap::new();
        idx.insert("future".to_string(), now + chrono::Duration::hours(2));
        assert_eq!(
            persisted_since_last_sync(&idx, "future", now),
            Some(Duration::ZERO)
        );
        assert_eq!(persisted_since_last_sync(&idx, "missing", now), None);
    }

    #[tokio::test]
    async fn start_workspace_periodic_sync_is_idempotent() {
        start_workspace_periodic_sync();
        start_workspace_periodic_sync();
        assert!(SCHEDULER_STARTED.get().is_some());
    }
}
