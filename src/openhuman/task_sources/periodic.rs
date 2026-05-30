//! Periodic poll scheduler for the `task_sources` domain.
//!
//! Spawned once at startup. On each tick it lists enabled sources and,
//! for any whose per-source `interval_secs` has elapsed since its last
//! poll, runs [`pipeline::run_source_once`] with
//! [`FetchReason::Periodic`]. Mirrors the composio periodic scheduler:
//! one global tick drives every source, per-source timing lives in a
//! process-global map, and errors are logged and swallowed so the loop
//! never unwinds.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::time::interval;

use crate::openhuman::config::rpc as config_rpc;

use super::pipeline;
use super::types::{FetchReason, TaskSource};

/// How often the scheduler wakes to look for due sources.
///
/// This is also the *effective lower bound* on polling frequency: a
/// source's `interval_secs` only governs how many ticks must elapse
/// before it is due again, so any `interval_secs` shorter than
/// `TICK_SECONDS` is effectively rounded up to this tick cadence (e.g. a
/// 60s source is still only polled every ~10 minutes). This keeps
/// background load bounded; sub-tick intervals are intentionally not
/// honoured more frequently.
const TICK_SECONDS: u64 = 600;

/// Floor on a source's effective poll interval, so a misconfigured
/// `interval_secs = 0` can't hammer the provider every tick.
const MIN_INTERVAL_SECONDS: u64 = 60;

static SCHEDULER_STARTED: OnceLock<()> = OnceLock::new();

type LastPollMap = Mutex<HashMap<String, Instant>>;
static LAST_POLL_AT: OnceLock<LastPollMap> = OnceLock::new();

fn last_poll_map() -> &'static LastPollMap {
    LAST_POLL_AT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record a successful (or attempted) poll for a source id.
fn record_poll(source_id: &str) {
    if let Ok(mut map) = last_poll_map().lock() {
        map.insert(source_id.to_string(), Instant::now());
    }
}

/// Whether `source` is due for a poll given its last-poll timestamp.
fn is_due(source: &TaskSource) -> bool {
    let interval_secs = source.interval_secs.max(MIN_INTERVAL_SECONDS);
    let map = match last_poll_map().lock() {
        Ok(map) => map,
        Err(poisoned) => poisoned.into_inner(),
    };
    match map.get(&source.id) {
        Some(when) => when.elapsed() >= Duration::from_secs(interval_secs),
        None => true, // never polled this run — fire immediately
    }
}

/// Spawn the periodic poll task. Idempotent — only the first call
/// installs the loop.
pub fn start_periodic_poll() {
    if SCHEDULER_STARTED.set(()).is_err() {
        tracing::debug!("[task_sources:periodic] scheduler already running, skipping start");
        return;
    }
    tokio::spawn(async move {
        tracing::info!(
            tick_seconds = TICK_SECONDS,
            "[task_sources:periodic] scheduler starting"
        );
        run_loop().await;
        tracing::error!("[task_sources:periodic] scheduler loop exited");
    });
}

async fn run_loop() {
    let mut ticker = interval(Duration::from_secs(TICK_SECONDS));
    // Skip the immediate-fire tick so startup isn't slammed before the
    // user signs in.
    ticker.tick().await;
    loop {
        ticker.tick().await;
        if let Err(e) = run_one_tick().await {
            tracing::warn!(error = %e, "[task_sources:periodic] tick failed (continuing)");
        }
    }
}

/// Run a single scheduler tick. `pub(crate)` so tests can drive ticks
/// without the real `interval`.
pub(crate) async fn run_one_tick() -> Result<(), String> {
    let config = config_rpc::load_config_with_timeout()
        .await
        .map_err(|e| format!("load_config: {e}"))?;

    if !config.task_sources.enabled {
        tracing::debug!("[task_sources:periodic] domain disabled in config, skipping tick");
        return Ok(());
    }

    let sources = match super::store::list_sources(&config) {
        Ok(sources) => sources,
        Err(e) => {
            tracing::debug!(error = %e, "[task_sources:periodic] list_sources failed, skipping tick");
            return Ok(());
        }
    };

    let mut considered = 0usize;
    let mut fired = 0usize;
    for source in sources {
        if !source.enabled {
            continue;
        }
        considered += 1;
        if !is_due(&source) {
            continue;
        }
        // Record the attempt up front so a slow/failing fetch doesn't
        // re-fire every tick.
        record_poll(&source.id);
        let outcome = pipeline::run_source_once(&config, &source, FetchReason::Periodic).await;
        fired += 1;
        tracing::debug!(
            source_id = %source.id,
            fetched = outcome.fetched,
            routed = outcome.routed,
            error = ?outcome.error,
            "[task_sources:periodic] source polled"
        );
    }

    tracing::debug!(considered, fired, "[task_sources:periodic] tick complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::task_sources::types::{FilterSpec, ProviderSlug, SourceTarget};
    use chrono::Utc;
    use serde_json::json;

    fn source(id: &str, interval_secs: u64) -> TaskSource {
        TaskSource {
            id: id.into(),
            provider: ProviderSlug::Github,
            connection_id: None,
            name: None,
            enabled: true,
            filter: FilterSpec::Github {
                repo: None,
                labels: vec![],
                assignee_is_me: true,
                state: None,
                extra: json!({}),
            },
            interval_secs,
            target: SourceTarget::TodoOnly,
            max_tasks_per_fetch: 25,
            assigned_executor: None,
            created_at: Utc::now(),
            last_fetch_at: None,
            last_status: None,
        }
    }

    #[test]
    fn tick_seconds_is_sane() {
        assert!(TICK_SECONDS >= 60);
        assert!(TICK_SECONDS <= 3600);
    }

    #[test]
    fn never_polled_source_is_due() {
        let s = source("ts-never-polled-xyz", 1800);
        assert!(is_due(&s));
    }

    #[test]
    fn recently_polled_source_is_not_due() {
        let s = source("ts-recent-poll-xyz", 1800);
        record_poll(&s.id);
        assert!(!is_due(&s), "just-recorded poll should not be due again");
    }

    #[test]
    fn zero_interval_is_floored_not_always_due() {
        let s = source("ts-zero-interval-xyz", 0);
        record_poll(&s.id);
        // With the MIN_INTERVAL_SECONDS floor a just-polled zero-interval
        // source is not immediately due again.
        assert!(!is_due(&s));
    }
}
