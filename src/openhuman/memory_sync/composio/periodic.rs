//! Periodic sync scheduler for the Composio domain.
//!
//! Spawned once at startup. The scheduler walks every active Composio
//! connection on a fixed tick, looks up the matching native provider,
//! and calls `provider.sync(ctx, SyncReason::Periodic)` if enough time
//! has elapsed since that connection's last sync (per the provider's
//! `sync_interval_secs`).
//!
//! ## Direct mode (`[composio-direct]`)
//!
//! As of #1710 Wave 1, the scheduler is **mode-aware**: it resolves the
//! client via [`create_composio_client`] each tick so a direct-mode
//! user's personal Composio v3 tenant gets walked (via
//! `direct_list_connections`) instead of returning an empty list from
//! the tinyhumans tenant. The per-connection sync calls go through
//! [`ProviderContext::execute`] which is itself mode-aware.
//!
//! Real-time trigger webhooks (`composio:trigger` socket.io events
//! fanned out from `wss://api.tinyhumans.ai`) still do not reach the
//! core when `config.composio.mode == "direct"`, because the backend
//! HMAC-verifies the Composio webhook and pushes it down a per-user
//! socket — direct-mode users see synchronous tool execution and
//! periodic poll-based sync, but not async trigger pushes in this
//! release. See the `composio.direct_mode_triggers_gap` capability
//! entry in `about_app/catalog.rs` for the user-visible status.
//!
//! Design notes:
//!
//!   * One global tick (5min) drives every provider — we don't spawn a
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::time::interval;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::scheduler_gate::gate::current_policy;
use crate::openhuman::scheduler_gate::policy::PauseReason;

use super::providers::{get_provider, ProviderContext, SyncReason};
use crate::openhuman::composio::client::{
    create_composio_client, direct_list_connections, ComposioClientKind,
};
use crate::openhuman::composio::ops;

/// How often the scheduler wakes up to look for due syncs. Independent
/// from per-provider `sync_interval_secs` — this just bounds how long
/// past a provider's interval we might fire.
///
/// 20 min trades a little staleness for noticeably less foreground load:
/// each tick triggers an HTTP fetch + DB write per due connection, and
/// for users with several connected providers the old 60s cadence kept
/// the laptop visibly busy. Per-provider `sync_interval_secs` still
/// caps the *minimum* delay between actual syncs — this only loosens
/// the upper bound.
const TICK_SECONDS: u64 = 1200;

/// Process-wide guard so the scheduler is only started once even
/// when both `start_channels` and `bootstrap_core_runtime` call into
/// us during startup. Without this we'd end up with two parallel tick
/// loops competing for the same connections.
static SCHEDULER_STARTED: OnceLock<()> = OnceLock::new();

/// Process-wide map of `(toolkit, connection_id) → last successful sync
/// instant`. Shared between the periodic scheduler loop and event-driven
/// sync paths (e.g. `ComposioConnectionCreatedSubscriber`,
/// `on_connection_created`) so that a recent non-periodic sync prevents
/// the scheduler from firing immediately on the next tick.
type SyncTimestampMap = Arc<Mutex<HashMap<(String, String), Instant>>>;

static LAST_SYNC_AT: OnceLock<SyncTimestampMap> = OnceLock::new();

/// Get (or lazily initialise) the shared last-sync-at map.
fn last_sync_map() -> SyncTimestampMap {
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

/// Inspect the scheduler-gate policy and decide whether this tick should
/// fire at all. Returns `Some(reason)` for paused states so the caller can
/// log a single, attributable line instead of doing the work and discovering
/// per-LLM-call later that everything's gated.
///
/// Covers two reasons the memory subsystem treats as "do no background
/// work":
/// - [`PauseReason::UserDisabled`] — user flipped the Memory Tree toggle off
///   in Settings (#1856 Part 1). The 20-min Composio fetch loop honouring
///   this flag is the explicit follow-up listed in the #2719 PR body.
/// - [`PauseReason::SignedOut`] — no live session; periodic work would just
///   401-loop against the backend.
///
/// Other [`PauseReason`] variants:
/// - `OnBattery` / `CpuPressure` (future, per #1073) — intentionally **not**
///   gated here; periodic Composio fetch is network-light, so battery / CPU
///   pressure shouldn't stop the user's data flowing in. Those signals
///   already throttle LLM-bound work through the regular gate.
/// - `Unknown` — documented in `scheduler_gate::policy` as a safe fallback;
///   `Policy::pause_reason()` returns it only when the gate state is in a
///   transitional / not-yet-resolved condition. Letting the tick proceed
///   here keeps periodic sync running through brief transitions instead of
///   pausing on stale unresolved state.
fn periodic_pause_reason() -> Option<PauseReason> {
    // Delegate the `Policy::Paused { .. }` → `PauseReason` extraction to
    // the existing `Policy::pause_reason()` helper (avoids re-implementing
    // the same destructure twice). The allow-list below is the only thing
    // this site has to own — future `PauseReason` variants stay opt-in.
    let reason = current_policy().pause_reason()?;
    matches!(reason, PauseReason::UserDisabled | PauseReason::SignedOut).then_some(reason)
}

/// Process-level "was the last tick paused?" tracker for transition logging.
///
/// We want `info!` *once* when the periodic loop crosses the pause boundary
/// (so fleet operators investigating "why is Composio not syncing?" see a
/// breadcrumb at default log level), without spamming `info` every 20 min
/// while the user has the toggle off. `Relaxed` ordering is fine because
/// the only consumer is the inside of `run_one_tick`, which is serialised
/// by the singleton scheduler loop.
static LAST_TICK_WAS_PAUSED: AtomicBool = AtomicBool::new(false);

/// Run a single scheduler tick. Public-ish (`pub(crate)`) so the test
/// module can drive ticks without spinning up the real `interval`.
pub(crate) async fn run_one_tick() -> Result<(), String> {
    // Step 0: scheduler-gate check. When the user has paused Memory Tree
    // via the Settings toggle, every subsequent tick should be a cheap
    // no-op — no `list_connections` call, no provider walk, no API budget
    // burn. The check runs **before** config load + auth-client build so
    // a paused session never even resolves the API token.
    //
    // Transition logging: emit `info!` once when the loop crosses the
    // pause boundary in either direction; stay at `debug!` for the
    // already-paused / already-running steady state. Without this, fleet
    // operators investigating "why is Composio not syncing?" see nothing
    // at default log level.
    if let Some(reason) = periodic_pause_reason() {
        let was_paused = LAST_TICK_WAS_PAUSED.swap(true, Ordering::Relaxed);
        if was_paused {
            tracing::debug!(
                reason = reason.as_str(),
                "[composio:periodic] scheduler-gate paused — skipping tick"
            );
        } else {
            tracing::info!(
                reason = reason.as_str(),
                "[composio:periodic] scheduler-gate paused — pausing periodic Composio sync"
            );
        }
        return Ok(());
    } else {
        let was_paused = LAST_TICK_WAS_PAUSED.swap(false, Ordering::Relaxed);
        if was_paused {
            tracing::info!(
                "[composio:periodic] scheduler-gate resumed — periodic Composio sync re-enabled"
            );
        }
    }

    // Step 1: load config (also gives us the auth token via the
    // shared integrations client builder).
    let config = config_rpc::load_config_with_timeout()
        .await
        .map_err(|e| format!("load_config: {e}"))?;
    let config = Arc::new(config);

    // Step 2: list active connections — mode-aware. Backend mode walks
    // the tinyhumans tenant; direct mode walks the user's personal
    // Composio v3 tenant. Mirrors `ops::composio_list_connections` so
    // direct-mode users get periodic sync against their own connections
    // instead of seeing an empty list (#1710).
    let kind = match create_composio_client(&config) {
        Ok(kind) => kind,
        Err(e) => {
            tracing::debug!(
                error = %e,
                "[composio:periodic] no client (not signed in? no direct key?), skipping tick"
            );
            return Ok(());
        }
    };
    let resp = match &kind {
        ComposioClientKind::Backend(client) => client
            .list_connections()
            .await
            .map_err(|e| format!("list_connections (backend): {e}"))?,
        ComposioClientKind::Direct(direct) => {
            direct_list_connections(direct).await.map_err(|e| {
                // [#1166 / Sentry TAURI-RUST-X9] The server-side periodic
                // tick re-renders the same v3 `/connected_accounts` 401
                // shape that `ops::composio_list_connections` emits, so
                // route it through the observability classifier too.
                // Without this, the tick-side 401s leak as unclassified
                // Sentry events even when the UI poll's identical failure
                // is correctly classified. Render WITH the
                // `[composio-direct]` anchor so the classifier arm in
                // `is_provider_user_state_message` actually fires.
                let rendered = format!("[composio-direct] list_connections (direct): {e:#}");
                ops::report_composio_op_error("list_connections", &rendered);
                rendered
            })?
        }
    };

    let sync_map = last_sync_map();

    let mut considered = 0usize;
    let mut fired = 0usize;
    for conn in resp.connections {
        considered += 1;

        // Skip connections that aren't actually live yet.
        if !conn.is_active() {
            continue;
        }

        let toolkit = conn.normalized_toolkit();
        let Some(provider) = get_provider(&toolkit) else {
            // No provider registered for this toolkit — that's fine,
            // we just don't have native code for it. Tools still work
            // through `composio_execute`.
            continue;
        };

        let Some(interval_secs) = provider.sync_interval_secs() else {
            // Provider opted out of periodic sync entirely.
            continue;
        };

        let key = (toolkit.clone(), conn.id.clone());
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
        // `ProviderContext` no longer caches a pre-baked
        // `ComposioClient` — provider methods resolve a fresh handle per
        // call via `ctx.execute(...)` so a mid-session
        // `composio.mode` toggle is honoured immediately (#1710).
        let ctx = ProviderContext {
            config: Arc::clone(&config),
            toolkit: toolkit.clone(),
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
    use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;
    use tempfile::tempdir;

    #[test]
    fn tick_seconds_is_sane_default() {
        // Sanity check: don't accidentally ship a 1-second tick.
        assert!(TICK_SECONDS >= 30);
        assert!(TICK_SECONDS <= 3600);
    }

    #[test]
    fn record_sync_success_stores_timestamp_keyed_by_toolkit_and_connection() {
        // Use unique keys so this test doesn't collide with other tests
        // writing into the process-wide map.
        let toolkit = "test_periodic_toolkit_a";
        let conn = "test-conn-a";
        record_sync_success(toolkit, conn);
        let map = last_sync_map();
        let guard = map.lock().expect("lock");
        let ts = guard
            .get(&(toolkit.to_string(), conn.to_string()))
            .expect("entry recorded");
        // Just-recorded timestamps should be very recent.
        assert!(ts.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn record_sync_success_overwrites_previous_timestamp() {
        let toolkit = "test_periodic_toolkit_b";
        let conn = "test-conn-b";
        record_sync_success(toolkit, conn);
        let first = last_sync_map()
            .lock()
            .expect("lock")
            .get(&(toolkit.to_string(), conn.to_string()))
            .copied()
            .expect("first entry");
        // Second call must replace (not keep the older) timestamp.
        std::thread::sleep(Duration::from_millis(5));
        record_sync_success(toolkit, conn);
        let second = last_sync_map()
            .lock()
            .expect("lock")
            .get(&(toolkit.to_string(), conn.to_string()))
            .copied()
            .expect("second entry");
        assert!(
            second >= first,
            "record_sync_success should advance the stored Instant"
        );
    }

    #[tokio::test]
    async fn run_one_tick_returns_ok_when_no_client() {
        // Isolate the workspace/env so config loading doesn't contend with
        // sibling tests mutating OPENHUMAN_WORKSPACE in parallel.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }

        // With no session stored in the isolated workspace,
        // `build_composio_client` returns None and the tick should
        // silently skip (returning Ok). This covers the early-return
        // path that's otherwise only hit in production.
        let inner = tokio::time::timeout(Duration::from_secs(5), run_one_tick())
            .await
            .expect("run_one_tick should not hang indefinitely during tests");
        assert!(
            inner.is_ok(),
            "run_one_tick should return Ok when no client is available: {inner:?}"
        );

        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn start_periodic_sync_is_idempotent() {
        // First call installs the scheduler via the OnceLock; subsequent
        // calls must be cheap no-ops without panicking. `tokio::spawn`
        // needs an ambient runtime, so this test runs under `tokio::test`.
        start_periodic_sync();
        start_periodic_sync();
        assert!(SCHEDULER_STARTED.get().is_some());
    }

    #[test]
    fn record_sync_success_distinguishes_connections() {
        let toolkit = "test_periodic_toolkit_c";
        record_sync_success(toolkit, "conn-1");
        record_sync_success(toolkit, "conn-2");
        let map = last_sync_map();
        let guard = map.lock().expect("lock");
        assert!(guard
            .get(&(toolkit.to_string(), "conn-1".to_string()))
            .is_some());
        assert!(guard
            .get(&(toolkit.to_string(), "conn-2".to_string()))
            .is_some());
        // Unrelated key should be absent.
        assert!(guard
            .get(&(toolkit.to_string(), "conn-3".to_string()))
            .is_none());
    }

    /// In unit tests `scheduler_gate::STATE` is never initialised, so
    /// `current_policy()` returns `Policy::Normal` and the helper must
    /// return `None` — i.e. the tick is allowed to proceed. This pins the
    /// happy-path wiring; an accidental "always pause" regression in the
    /// helper would break every `run_one_tick`-driven test that follows it.
    ///
    /// (The redundant "does-not-short-circuit" tick-level test that was
    /// here in the first review pass was dropped per @oxoxDev's
    /// [#2825 review](https://github.com/tinyhumansai/openhuman/pull/2825):
    /// it duplicated `run_one_tick_returns_ok_when_no_client` because
    /// both exited at the same `create_composio_client` no-client branch,
    /// so neither actually proved the new gate-check arm fired in the
    /// right direction. Asserting log-line absence via `tracing-test`
    /// would prove it but adds a new dev-dependency for one assertion —
    /// the helper-level test below already pins the wiring.)
    #[test]
    fn periodic_pause_reason_returns_none_when_gate_not_initialised() {
        // Calling without `scheduler_gate::init_global(...)` exercises the
        // OnceLock-uninitialised branch in `current_policy`, which is the
        // realistic test-environment state.
        assert!(
            periodic_pause_reason().is_none(),
            "expected None (i.e. tick proceeds) when scheduler_gate is in default Normal state, \
             got {:?}",
            periodic_pause_reason()
        );
    }
}
