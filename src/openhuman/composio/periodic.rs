//! Periodic sync scheduler for the Composio domain.
//!
//! Spawned once at startup. The scheduler walks every active Composio
//! connection on a fixed tick, looks up the matching native provider,
//! and calls `provider.sync(ctx, SyncReason::Periodic)` if enough time
//! has elapsed since that connection's last sync (per the provider's
//! `sync_interval_secs`).
//!
//! Design notes:
//!
//!   * One global tick (60s) drives every provider — we don't spawn a
//!     task per connection, because the number of connections per user
//!     is small and a single tick keeps the bookkeeping trivial.
//!   * Per-connection state (last sync timestamp) lives in a
//!     process-global `Arc<Mutex<HashMap>>` keyed by `(toolkit,
//!     connection_id)`. The map is shared with event-driven sync paths
//!     (bus subscribers, `on_connection_created`) via
//!     [`record_sync_success`] so a recent non-periodic sync prevents
//!     the scheduler from redundantly re-firing. The map is rebuilt on
//!     restart, which is fine — a missed periodic sync is harmless
//!     because the next tick after restart picks it back up immediately.
//!   * Errors are logged and swallowed; the scheduler must never panic
//!     out of its loop or periodic sync stops silently for the rest of
//!     the process lifetime.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::time::interval;

use crate::openhuman::config::rpc as config_rpc;

use super::providers::{get_provider, ProviderContext, SyncReason};

/// How often the scheduler wakes up to look for due syncs. Independent
/// from per-provider `sync_interval_secs` — this just bounds how long
/// past a provider's interval we might fire.
const TICK_SECONDS: u64 = 60;

/// Process-wide guard so the scheduler is only started once even
/// when both `start_channels` and `bootstrap_skill_runtime` call into
/// us during startup. Without this we'd end up with two parallel tick
/// loops competing for the same connections.
static SCHEDULER_STARTED: OnceLock<()> = OnceLock::new();

/// Process-wide map of `(toolkit, connection_id) → last successful sync
/// instant`. Shared between the periodic scheduler loop and event-driven
/// sync paths (e.g. `ComposioConnectionCreatedSubscriber`,
/// `on_connection_created`) so that a recent non-periodic sync prevents
/// the scheduler from firing immediately on the next tick.
static LAST_SYNC_AT: OnceLock<Arc<Mutex<HashMap<(String, String), Instant>>>> = OnceLock::new();

/// Get (or lazily initialise) the shared last-sync-at map.
fn last_sync_map() -> Arc<Mutex<HashMap<(String, String), Instant>>> {
    LAST_SYNC_AT
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

/// Record a successful sync for the given `(toolkit, connection_id)` key.
/// Called by the periodic scheduler after a successful sync and by
/// event-driven paths (bus subscribers, `on_connection_created`) so the
/// periodic ticker respects recent non-periodic syncs.
pub fn record_sync_success(toolkit: &str, connection_id: &str) {
    if let Ok(mut map) = last_sync_map().lock() {
        map.insert(
            (toolkit.to_string(), connection_id.to_string()),
            Instant::now(),
        );
    }
}

/// Spawn the periodic sync background task. Idempotent: only the
/// first call actually spawns the loop, every subsequent call is a
/// cheap no-op (logged at `debug` so it's visible during startup
/// tracing without spamming `info`).
pub fn start_periodic_sync() {
    if SCHEDULER_STARTED.get().is_some() {
        tracing::debug!("[composio:periodic] scheduler already running, skipping start");
        return;
    }
    // Race-safe: only the thread that wins `set` runs the spawn body.
    if SCHEDULER_STARTED.set(()).is_err() {
        tracing::debug!("[composio:periodic] scheduler already running (race), skipping start");
        return;
    }

    tokio::spawn(async move {
        tracing::info!(
            tick_seconds = TICK_SECONDS,
            "[composio:periodic] scheduler starting"
        );
        run_loop().await;
        // run_loop only returns on a fatal error in the bus — log it
        // so the silent stop is at least visible in the trace.
        tracing::error!("[composio:periodic] scheduler loop exited");
    });
}

/// Inner loop, broken out so it's easy to mock-replace in tests if we
/// ever want to drive ticks deterministically.
async fn run_loop() {
    let mut ticker = interval(Duration::from_secs(TICK_SECONDS));
    // Skip the immediate-fire tick so startup isn't slammed before the
    // user even has time to sign in.
    ticker.tick().await;

    loop {
        ticker.tick().await;
        if let Err(e) = run_one_tick().await {
            tracing::warn!(
                error = %e,
                "[composio:periodic] tick failed (continuing)"
            );
        }
    }
}

/// Run a single scheduler tick. Public-ish (`pub(crate)`) so the test
/// module can drive ticks without spinning up the real `interval`.
pub(crate) async fn run_one_tick() -> Result<(), String> {
    // Step 1: load config (also gives us the auth token via the
    // shared integrations client builder).
    let config = config_rpc::load_config_with_timeout()
        .await
        .map_err(|e| format!("load_config: {e}"))?;
    let config = Arc::new(config);

    // Step 2: list active connections from the backend.
    let Some(client) = super::client::build_composio_client(&config) else {
        tracing::debug!("[composio:periodic] no client (not signed in?), skipping tick");
        return Ok(());
    };
    let resp = client
        .list_connections()
        .await
        .map_err(|e| format!("list_connections: {e}"))?;

    let sync_map = last_sync_map();

    let mut considered = 0usize;
    let mut fired = 0usize;
    for conn in resp.connections {
        considered += 1;

        // Skip connections that aren't actually live yet.
        if !matches!(conn.status.as_str(), "ACTIVE" | "CONNECTED") {
            continue;
        }

        let Some(provider) = get_provider(&conn.toolkit) else {
            // No provider registered for this toolkit — that's fine,
            // we just don't have native code for it. Tools still work
            // through `composio_execute`.
            continue;
        };

        let Some(interval_secs) = provider.sync_interval_secs() else {
            // Provider opted out of periodic sync entirely.
            continue;
        };

        let key = (conn.toolkit.clone(), conn.id.clone());
        let due = {
            let map = sync_map.lock().unwrap_or_else(|e| e.into_inner());
            match map.get(&key) {
                Some(when) => when.elapsed() >= Duration::from_secs(interval_secs),
                None => true, // never synced this run — fire immediately
            }
        };
        if !due {
            continue;
        }

        // Build a context tied to this specific connection and dispatch.
        let ctx = ProviderContext {
            config: Arc::clone(&config),
            client: client.clone(),
            toolkit: conn.toolkit.clone(),
            connection_id: Some(conn.id.clone()),
        };

        tracing::debug!(
            toolkit = %conn.toolkit,
            connection_id = %conn.id,
            interval_secs,
            "[composio:periodic] firing sync"
        );
        match provider.sync(&ctx, SyncReason::Periodic).await {
            Ok(outcome) => {
                tracing::debug!(
                    toolkit = %conn.toolkit,
                    connection_id = %conn.id,
                    items = outcome.items_ingested,
                    elapsed_ms = outcome.elapsed_ms(),
                    "[composio:periodic] sync ok"
                );
                record_sync_success(&conn.toolkit, &conn.id);
                fired += 1;
            }
            Err(e) => {
                tracing::warn!(
                    toolkit = %conn.toolkit,
                    connection_id = %conn.id,
                    error = %e,
                    "[composio:periodic] sync failed (will retry next tick)"
                );
                // Intentionally do NOT update last_sync_at on failure
                // so the next tick retries immediately.
            }
        }
    }

    tracing::debug!(considered, fired, "[composio:periodic] tick complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_seconds_is_sane_default() {
        // Sanity check: don't accidentally ship a 1-second tick.
        assert!(TICK_SECONDS >= 30);
        assert!(TICK_SECONDS <= 300);
    }
}
