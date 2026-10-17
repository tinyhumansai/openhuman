//! Periodic poll scheduler for the `task_sources` domain.
//!
//! Spawned once at startup. On each tick it lists enabled sources and,
//! for any whose per-source `interval_secs` has elapsed since its last
//! poll, runs [`pipeline::run_source_once`] with
//! [`FetchReason::Periodic`]. Mirrors the composio periodic scheduler:
//! one global tick drives every source, per-source timing lives in a
//! process-global map, and errors are logged and swallowed so the loop
//! never unwinds.

use crate::core::runtime::DispatchCtx;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::time::interval;

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

/// The poll-tracking key for `source_id` in the current storage scope: the
/// same source id in two tenants' scopes (agents, or SaaS profiles) is two
/// different sources.
fn poll_key(source_id: &str) -> String {
    crate::core::runtime::tenant_key(
        &crate::core::runtime::tenant::current_tenant_or_isolated("task_sources"),
        source_id,
    )
}

/// Record a successful (or attempted) poll for a source id.
fn record_poll(source_id: &str) {
    if let Ok(mut map) = last_poll_map().lock() {
        map.insert(poll_key(source_id), Instant::now());
    }
}

/// Whether `source` is due for a poll given its last-poll timestamp.
fn is_due(source: &TaskSource) -> bool {
    let interval_secs = source.interval_secs.max(MIN_INTERVAL_SECONDS);
    let map = match last_poll_map().lock() {
        Ok(map) => map,
        Err(poisoned) => poisoned.into_inner(),
    };
    match map.get(&poll_key(&source.id)) {
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
    // The loop acts for the host: every tick visits `host_default()` and then
    // each live agent scope itself, so the spawned task needs no ambient scope.
    DispatchCtx::host_default().spawn(async move {
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
        // `local`, then every agent that keeps its sources in its own storage
        // scope (`crate::storage::agents`).
        for (agent, result) in crate::storage::agents::for_each_live_scope(
            &DispatchCtx::host_default(),
            "task_sources",
            |ctx| async move { run_one_tick(&ctx).await },
        )
        .await
        {
            if let Err(e) = result {
                tracing::warn!(error = %e, agent = agent.as_deref().unwrap_or("local"), "[task_sources:periodic] tick failed (continuing)");
            }
        }
    }
}

/// Run a single scheduler tick. `pub(crate)` so tests can drive ticks
/// without the real `interval`.
pub(crate) async fn run_one_tick(ctx: &DispatchCtx) -> Result<(), String> {
    let config = ctx
        .load_config()
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
        // A provider with no working fetch path is skipped before it can
        // record anything. `run_source_once` would refuse it, and that
        // refusal costs a `store::record_fetch` row and a
        // `TaskSourceFetchFailed` event on every tick, forever — an
        // unbounded write nobody asked for and nobody can act on.
        //
        // The manual `task_sources_fetch` RPC deliberately does NOT get this
        // gate: a user who presses Fetch is owed the explanation. Only the
        // timer is silenced, which is why the check lives here rather than
        // inside `run_source_once` (which both paths share).
        if !super::types::provider_can_fetch(source.provider) {
            tracing::debug!(
                source_id = %source.id,
                provider = %source.provider.as_str(),
                "[task_sources:periodic] skipping source — provider has no task-fetch path"
            );
            continue;
        }
        considered += 1;
        if !is_due(&source) {
            continue;
        }
        // Record the attempt up front so a slow/failing fetch doesn't
        // re-fire every tick.
        record_poll(&source.id);
        let outcome = pipeline::run_source_once(ctx, &config, &source, FetchReason::Periodic).await;
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
#[path = "periodic_tests.rs"]
mod tests;
