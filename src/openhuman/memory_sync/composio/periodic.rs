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
//!     restart; to keep a user-configured cadence (e.g. "Sync every 24h",
//!     #3302) from re-firing on every cold start, the due-check falls back
//!     to the **persisted** sync-audit timestamp ([`read_audit_log`]) when
//!     the in-memory record is absent — see [`persisted_since_last_sync`].
//!   * Errors are logged and swallowed; the scheduler must never panic
//!     out of its loop or periodic sync stops silently for the rest of
//!     the process lifetime.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::time::interval;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::DEFAULT_MEMORY_SYNC_INTERVAL_SECS;
use crate::openhuman::memory_sources::{
    memory_sync_defaults_for_toolkit, MemorySourceEntry, SourceKind,
};
use crate::openhuman::scheduler_gate::gate::{current_policy, resume_notify};
use crate::openhuman::scheduler_gate::policy::PauseReason;

use super::providers::{get_provider, ComposioUsage, ProviderContext, SyncReason};
use crate::openhuman::composio::client::{
    create_composio_client, direct_list_connections, ComposioClientKind,
};
use crate::openhuman::composio::ops;
use crate::openhuman::memory_sync::sources::audit::{
    append_audit_entry, read_audit_log, SyncAuditEntry,
};
use chrono::{DateTime, Utc};

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

/// Resolve the effective periodic sync interval (seconds) for one connection,
/// combining the provider's own default with the user's global
/// memory-sync cadence ([`Config::memory_sync_interval_secs`], #3302).
///
/// - `global == Some(0)` → `None`: "Manual only" — the scheduler skips this
///   source entirely (manual sync still works).
/// - `global == Some(n)` → `Some(max(n, provider_default))`: the user's
///   cadence overrides the provider default but is floored at it, so we never
///   sync *more* often than the provider intended.
/// - `global == None` → `Some(max(DEFAULT, provider_default))`: no explicit
///   user choice, so fall back to the 24h default cadence (also floored at the
///   provider default).
pub(crate) fn effective_interval_secs(provider_default: u64, global: Option<u64>) -> Option<u64> {
    match global {
        Some(0) => None,
        Some(n) => Some(n.max(provider_default)),
        None => Some(DEFAULT_MEMORY_SYNC_INTERVAL_SECS.max(provider_default)),
    }
}

/// Decide whether a connection is due for a periodic sync right now, given the
/// effective interval and how long ago it last synced this run.
///
/// `since_last_sync == None` means we have no record of a sync this process
/// lifetime, so we fire immediately (the restart-recovery path). Kept pure so
/// the due-check can be simulated without driving the real `Instant` clock.
pub(crate) fn connection_is_due(interval_secs: u64, since_last_sync: Option<Duration>) -> bool {
    match since_last_sync {
        Some(elapsed) => elapsed >= Duration::from_secs(interval_secs),
        None => true,
    }
}

/// Build an index of `connection_id → most recent successful Composio sync
/// timestamp` from the persisted sync audit log (#3302).
///
/// The periodic loop writes audit entries with `source_id = connection_id` and
/// `scope = "{toolkit}:{connection_id}"`; we accept either shape and key by the
/// connection id. Only successful syncs count — matching the in-memory
/// [`record_sync_success`] semantics, which never records a failed tick so the
/// next tick retries. This is the wall-clock record that lets the cadence
/// survive restarts (the in-memory monotonic map cannot).
fn index_last_success_by_connection(entries: &[SyncAuditEntry]) -> HashMap<String, DateTime<Utc>> {
    let mut idx: HashMap<String, DateTime<Utc>> = HashMap::new();
    for e in entries {
        if e.source_kind != "composio" || !e.success {
            continue;
        }
        let connection_id = e
            .scope
            .rsplit_once(':')
            .map(|(_, c)| c.to_string())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| e.source_id.clone());
        idx.entry(connection_id)
            .and_modify(|t| {
                if e.timestamp > *t {
                    *t = e.timestamp;
                }
            })
            .or_insert(e.timestamp);
    }
    idx
}

/// Wall-clock elapsed since a connection's last persisted successful sync, if
/// any. Saturates at zero for a future timestamp (clock skew), so a skewed
/// record never reads as "wildly overdue". Returns `None` when the connection
/// has no persisted sync — letting the caller treat it as never-synced.
fn persisted_since_last_sync(
    idx: &HashMap<String, DateTime<Utc>>,
    connection_id: &str,
    now: DateTime<Utc>,
) -> Option<Duration> {
    idx.get(connection_id).map(|ts| {
        let secs = (now - *ts).num_seconds().max(0) as u64;
        Duration::from_secs(secs)
    })
}

/// Outcome of consulting the per-source registry for one Composio connection
/// during a periodic tick (#2831).
#[derive(Debug, PartialEq, Eq)]
enum PeriodicSourceDecision {
    /// The user toggled this source **off** — skip background sync entirely.
    /// Manual `memory_sources_sync` still works (it has its own `enabled`
    /// guard); only the automatic loop honours this here.
    Skip,
    /// Sync this connection with the given caps (`None` = uncapped for that
    /// dimension).
    Sync {
        max_items: Option<u32>,
        sync_depth_days: Option<u32>,
    },
}

/// Decide whether — and with what caps — to periodically sync one connection,
/// honouring the per-source `enabled` toggle (#2831). Pure so the three
/// branches can be unit-tested without async/registry I/O.
///
/// - **disabled row** → [`PeriodicSourceDecision::Skip`]: the background loop
///   must not sync a source the user switched off (this was the row-2 leak —
///   previously the loop only read the registry for caps and synced disabled
///   sources anyway, uncapped).
/// - **enabled row** → `Sync` with the row's caps.
/// - **no row yet** → `Sync` with conservative per-toolkit defaults
///   ([`memory_sync_defaults_for_toolkit`]). `reconcile` normally backfills an
///   enabled, capped row for every connection; this covers the brief
///   pre-reconcile window. The no-match path defaults to *sync-bounded*, never
///   *skip* and never *uncapped*, so a missing/mismatched row degrades safely
///   (data keeps flowing, just capped) instead of silently going dark.
fn decide_periodic_source(
    source: Option<&MemorySourceEntry>,
    toolkit: &str,
) -> PeriodicSourceDecision {
    match source {
        Some(s) if !s.enabled => PeriodicSourceDecision::Skip,
        Some(s) => PeriodicSourceDecision::Sync {
            max_items: s.max_items,
            sync_depth_days: s.sync_depth_days,
        },
        None => {
            let (max_items, sync_depth_days) = memory_sync_defaults_for_toolkit(toolkit);
            PeriodicSourceDecision::Sync {
                max_items,
                sync_depth_days,
            }
        }
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
///
/// Each iteration waits on whichever comes first (#2831):
///   * the 20-min `ticker` — the steady-state cadence, or
///   * the scheduler-gate **resume** notify — fired when the user toggles
///     Memory Tree back on or signs back in.
///
/// On a resume wake we run a tick **immediately** (so sync restarts within
/// seconds, not at the next ≤20-min boundary) and `reset()` the ticker so the
/// *next* scheduled tick is a full `TICK_SECONDS` out. The reset is what stops
/// rapid off-on toggling from bunch-firing: many wakes collapse into at most
/// one extra tick (the `Notify` stores a single permit), and the cadence
/// re-bases from the last actual tick.
async fn run_loop() {
    let mut ticker = interval(Duration::from_secs(TICK_SECONDS));
    let resume = resume_notify();
    // Skip the immediate-fire tick so startup isn't slammed before the
    // user even has time to sign in.
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = resume.notified() => {
                // Woke early on a resume transition. Re-base the cadence so the
                // next scheduled tick is TICK_SECONDS from now, then fall
                // through and run the tick immediately.
                ticker.reset();
            }
        }
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
pub(crate) fn periodic_pause_reason() -> Option<PauseReason> {
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

    // Global, user-configurable memory-sync cadence (#3302). Applied to every
    // opted-in source as a floor/override over the provider's own default; a
    // value of `Some(0)` disables periodic auto-sync ("Manual only").
    let global_interval = config.memory_sync_interval_secs;

    // Persisted last-sync fallback (#3302). The in-memory `LAST_SYNC_AT` map is
    // rebuilt empty on every launch, so without this a cold start would re-fire
    // every connection on the first tick — silently breaking the configured
    // "Sync every 24h" gap across app restarts. We index the persisted sync
    // audit log (wall-clock timestamps that survive restarts) and use it as the
    // due-check fallback whenever the in-memory monotonic record is absent.
    let audit_index = index_last_success_by_connection(&read_audit_log(&config));
    let now = Utc::now();

    // Per-source registry snapshot (#2831). The periodic loop gates on the
    // per-source `enabled` toggle so a source the user switched off stops
    // syncing in the background — matching the manual paths
    // (`memory_sources::sync_source`, `memory_sources_sync_all`), which already
    // early-return on `!enabled`. Index every Composio source (enabled and
    // disabled) by connection id; the per-connection branch below resolves
    // skip/caps via `decide_periodic_source`.
    //
    // Built from the **already-loaded** `config` snapshot (Step 1), not a second
    // `list_sources()` read. A separate read whose error we swallowed to an
    // empty map would make every disabled source fall through to the
    // `decide_periodic_source(None, ..)` default-caps path — silently
    // re-enabling background sync for sources the user switched off on a
    // transient config-read failure. Reusing the tick's snapshot is fail-closed
    // (a disabled row stays disabled) and avoids the extra read entirely.
    let composio_sources: HashMap<String, MemorySourceEntry> = config
        .memory_sources
        .iter()
        .filter(|s| s.kind == SourceKind::Composio)
        .filter_map(|s| s.connection_id.clone().map(|id| (id, s.clone())))
        .collect();

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

        let Some(provider_default) = provider.sync_interval_secs() else {
            // Provider opted out of periodic sync entirely.
            continue;
        };

        let Some(interval_secs) = effective_interval_secs(provider_default, global_interval) else {
            // User selected "Manual only" — skip auto-sync for this source.
            // Manual `memory_sources_sync` still works.
            tracing::debug!(
                toolkit = %toolkit,
                connection_id = %conn.id,
                "[composio:periodic] manual-only mode — skipping periodic sync"
            );
            continue;
        };

        let key = (toolkit.clone(), conn.id.clone());
        // Prefer the in-memory monotonic record (most accurate within this run);
        // fall back to the persisted audit timestamp so the configured cadence
        // is honoured across restarts instead of re-firing on every cold start.
        let since_last_sync = {
            let map = sync_map.lock().unwrap_or_else(|e| e.into_inner());
            map.get(&key).map(|when| when.elapsed())
        }
        .or_else(|| persisted_since_last_sync(&audit_index, &conn.id, now));
        if !connection_is_due(interval_secs, since_last_sync) {
            continue;
        }

        // Per-source gate + caps from the memory_sources registry (#2831).
        // A disabled source is skipped here (the background-sync half of the
        // toggle); enabled sources sync with their caps; a connection with no
        // registry row yet syncs with conservative per-toolkit defaults.
        let (src_max_items, src_sync_depth_days) =
            match decide_periodic_source(composio_sources.get(&conn.id), &toolkit) {
                PeriodicSourceDecision::Skip => {
                    tracing::debug!(
                        toolkit = %toolkit,
                        connection_id = %conn.id,
                        "[composio:periodic] source disabled — skipping periodic sync"
                    );
                    continue;
                }
                PeriodicSourceDecision::Sync {
                    max_items,
                    sync_depth_days,
                } => (max_items, sync_depth_days),
            };

        tracing::debug!(
            toolkit = %toolkit,
            connection_id = %conn.id,
            max_items = ?src_max_items,
            sync_depth_days = ?src_sync_depth_days,
            "[composio:periodic] caps from registry"
        );

        // Build a context tied to this specific connection and dispatch.
        // `ProviderContext` no longer caches a pre-baked
        // `ComposioClient` — provider methods resolve a fresh handle per
        // call via `ctx.execute(...)` so a mid-session
        // `composio.mode` toggle is honoured immediately (#1710).
        let ctx = ProviderContext {
            config: Arc::clone(&config),
            toolkit: toolkit.clone(),
            connection_id: Some(conn.id.clone()),
            usage: Default::default(),
            max_items: src_max_items,
            sync_depth_days: src_sync_depth_days,
        };

        tracing::debug!(
            toolkit = %conn.toolkit,
            connection_id = %conn.id,
            interval_secs,
            "[composio:periodic] firing sync"
        );
        let sync_started = Instant::now();
        let result = provider.sync(&ctx, SyncReason::Periodic).await;
        let duration_ms = sync_started.elapsed().as_millis() as u64;

        // Read the Composio billable-action tally the sync accumulated at the
        // `execute` chokepoint (#3111). Periodic is where most Composio cost
        // accrues (this loop fires every 20 min per connection vs. rare manual
        // syncs), but periodic ticks weren't recorded in the sync audit at all
        // — so the audit under-counted real cost. Record each tick that ran a
        // fetch, success or failure, so the Sync History panel reflects the
        // background spend too (#3111 follow-up; raised in the #3138 review).
        let usage = ctx.usage.lock().map(|u| u.clone()).unwrap_or_default();

        match result {
            Ok(outcome) => {
                tracing::debug!(
                    toolkit = %conn.toolkit,
                    connection_id = %conn.id,
                    items = outcome.items_ingested,
                    elapsed_ms = outcome.elapsed_ms(),
                    composio_actions = usage.actions_called,
                    "[composio:periodic] sync ok"
                );
                let entry = build_periodic_audit_entry(
                    &toolkit,
                    &conn.id,
                    &usage,
                    outcome.items_ingested,
                    duration_ms,
                    None,
                );
                append_audit_entry(&config, &entry);
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
                // A failed tick may still have fired billable fetch actions
                // before erroring — audit the partial cost so it isn't lost.
                let entry =
                    build_periodic_audit_entry(&toolkit, &conn.id, &usage, 0, duration_ms, Some(e));
                append_audit_entry(&config, &entry);
                // Intentionally do NOT update last_sync_at on failure
                // so the next tick retries immediately.
            }
        }
    }

    tracing::debug!(considered, fired, "[composio:periodic] tick complete");
    Ok(())
}

/// Build a [`SyncAuditEntry`] for one periodic Composio sync tick (#3111
/// follow-up).
///
/// Periodic syncs only fetch + ingest; summarisation runs later in the async
/// job worker, so the LLM-cost columns (tokens, estimated / actual charge)
/// are zero here. The meaningful spend is the Composio billable actions the
/// fetch fired, carried in `usage`. `scope` is `{toolkit}:{connection_id}` to
/// match the owner shape the per-source memory-tree ingest uses, and
/// `source_kind` is `"composio"` so the Sync History panel groups periodic
/// rows alongside the manual-sync rows the dispatcher already writes.
fn build_periodic_audit_entry(
    toolkit: &str,
    connection_id: &str,
    usage: &ComposioUsage,
    items_ingested: usize,
    duration_ms: u64,
    error: Option<String>,
) -> SyncAuditEntry {
    SyncAuditEntry {
        timestamp: chrono::Utc::now(),
        source_id: connection_id.to_string(),
        source_kind: "composio".to_string(),
        scope: format!("{toolkit}:{connection_id}"),
        items_fetched: items_ingested as u32,
        batches: 0,
        input_tokens: 0,
        output_tokens: 0,
        estimated_cost_usd: 0.0,
        composio_actions_called: usage.actions_called,
        composio_cost_usd: usage.cost_usd,
        actual_charged_usd: None,
        duration_ms,
        success: error.is_none(),
        error,
    }
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
    fn effective_interval_none_falls_back_to_default() {
        // No user choice → 24h default, floored at the provider default.
        assert_eq!(
            effective_interval_secs(15 * 60, None),
            Some(DEFAULT_MEMORY_SYNC_INTERVAL_SECS)
        );
    }

    #[test]
    fn effective_interval_manual_disables_sync() {
        // Some(0) is the "Manual only" sentinel — periodic sync is skipped.
        assert_eq!(effective_interval_secs(15 * 60, Some(0)), None);
    }

    #[test]
    fn effective_interval_override_is_floored_at_provider_default() {
        // A user cadence longer than the provider default is honoured as-is.
        assert_eq!(
            effective_interval_secs(15 * 60, Some(4 * 3600)),
            Some(4 * 3600)
        );
        // A user cadence shorter than the provider default is clamped up to it
        // so we never sync more often than the provider intends.
        assert_eq!(effective_interval_secs(30 * 60, Some(60)), Some(30 * 60));
        // Exactly equal stays equal.
        assert_eq!(effective_interval_secs(1800, Some(1800)), Some(1800));
    }

    #[test]
    fn effective_interval_default_is_floored_at_a_longer_provider_default() {
        // If a provider ever defaults to longer than 24h, that wins under None.
        let long = DEFAULT_MEMORY_SYNC_INTERVAL_SECS + 3600;
        assert_eq!(effective_interval_secs(long, None), Some(long));
    }

    #[test]
    fn connection_is_due_compares_elapsed_against_interval() {
        let interval = 4 * 3600;
        // Never synced this run → always due.
        assert!(connection_is_due(interval, None));
        // Synced more recently than the interval → not due.
        assert!(!connection_is_due(
            interval,
            Some(Duration::from_secs(3600))
        ));
        // Synced exactly at the interval boundary → due.
        assert!(connection_is_due(
            interval,
            Some(Duration::from_secs(interval))
        ));
        // Synced longer ago than the interval → due.
        assert!(connection_is_due(
            interval,
            Some(Duration::from_secs(interval + 1))
        ));
    }

    /// Build a minimal Composio `MemorySourceEntry` for the per-source gate
    /// tests — only the fields `decide_periodic_source` reads are meaningful.
    fn composio_source(
        enabled: bool,
        max_items: Option<u32>,
        sync_depth_days: Option<u32>,
    ) -> MemorySourceEntry {
        MemorySourceEntry {
            id: "src_test".to_string(),
            kind: SourceKind::Composio,
            label: "test".to_string(),
            enabled,
            toolkit: Some("gmail".to_string()),
            connection_id: Some("cmp-1".to_string()),
            path: None,
            glob: None,
            url: None,
            branch: None,
            paths: Vec::new(),
            max_commits: None,
            max_issues: None,
            max_prs: None,
            query: None,
            since_days: None,
            max_items,
            selector: None,
            max_tokens_per_sync: None,
            max_cost_per_sync_usd: None,
            sync_depth_days,
        }
    }

    /// #2831 row 2: a source explicitly toggled **off** must be skipped by the
    /// background loop — this is the leak the gate closes.
    #[test]
    fn decide_periodic_source_skips_disabled_source() {
        let src = composio_source(false, Some(100), Some(30));
        assert_eq!(
            decide_periodic_source(Some(&src), "gmail"),
            PeriodicSourceDecision::Skip
        );
    }

    /// An enabled source syncs with exactly its configured caps (no defaulting).
    #[test]
    fn decide_periodic_source_uses_enabled_source_caps() {
        let src = composio_source(true, Some(42), Some(7));
        assert_eq!(
            decide_periodic_source(Some(&src), "gmail"),
            PeriodicSourceDecision::Sync {
                max_items: Some(42),
                sync_depth_days: Some(7),
            }
        );
    }

    /// A connection with no registry row yet (pre-reconcile window) syncs with
    /// the conservative per-toolkit defaults — **bounded**, never uncapped, and
    /// never skipped. This is the safe-direction fallback for a missing match.
    #[test]
    fn decide_periodic_source_defaults_caps_when_no_row() {
        let (want_items, want_depth) = memory_sync_defaults_for_toolkit("gmail");
        assert_eq!(
            decide_periodic_source(None, "gmail"),
            PeriodicSourceDecision::Sync {
                max_items: want_items,
                sync_depth_days: want_depth,
            }
        );
        // The defaults are bounded for a known toolkit (regression guard against
        // an accidental return to uncapped background fetches).
        assert!(want_items.is_some());
    }

    /// Multi-account regression (#3443 added multiple account connections per
    /// toolkit): two live connections of the *same* toolkit must be gated
    /// **independently** by their own per-`connection_id` source rows. This
    /// pins the loop's connection-id keying — re-keying the lookup by toolkit
    /// would collapse the two accounts and is the regression this guards.
    #[test]
    fn per_connection_gate_is_independent_across_accounts_of_same_toolkit() {
        // gmail account A: enabled with caps; gmail account B: disabled.
        let mut a = composio_source(true, Some(10), Some(5));
        a.connection_id = Some("conn-A".to_string());
        a.toolkit = Some("gmail".to_string());
        let mut b = composio_source(false, Some(99), Some(99));
        b.connection_id = Some("conn-B".to_string());
        b.toolkit = Some("gmail".to_string());

        // Build the same connection_id → entry index the live tick builds.
        let index: HashMap<String, MemorySourceEntry> = [a, b]
            .into_iter()
            .filter_map(|s| s.connection_id.clone().map(|id| (id, s)))
            .collect();

        // Account A (enabled) syncs with its own caps...
        assert_eq!(
            decide_periodic_source(index.get("conn-A"), "gmail"),
            PeriodicSourceDecision::Sync {
                max_items: Some(10),
                sync_depth_days: Some(5),
            }
        );
        // ...account B (disabled) is skipped, even though it shares the toolkit.
        assert_eq!(
            decide_periodic_source(index.get("conn-B"), "gmail"),
            PeriodicSourceDecision::Skip
        );
        // A third, not-yet-registered account of the same toolkit falls back to
        // bounded defaults (never skipped, never uncapped).
        let (def_items, def_depth) = memory_sync_defaults_for_toolkit("gmail");
        assert_eq!(
            decide_periodic_source(index.get("conn-C"), "gmail"),
            PeriodicSourceDecision::Sync {
                max_items: def_items,
                sync_depth_days: def_depth,
            }
        );
    }

    /// End-to-end simulation of the scheduler's per-connection decision: prove
    /// that **changing the global setting changes when the next sync fires**
    /// (issue #3302 acceptance criterion). We drive the same two pure helpers
    /// the live tick uses (`effective_interval_secs` → `connection_is_due`)
    /// across realistic last-sync ages, so no clock or network is needed.
    #[test]
    fn scheduler_decision_honors_the_global_setting() {
        // A chatty provider that natively wants to sync every 15 minutes.
        let provider_default = 15 * 60;

        // Helper mirroring the live loop: returns whether the connection would
        // fire right now, or `None` for "Manual only" (skipped entirely).
        let decide = |global: Option<u64>, since: Option<Duration>| -> Option<bool> {
            effective_interval_secs(provider_default, global)
                .map(|interval| connection_is_due(interval, since))
        };

        let one_hour_ago = Some(Duration::from_secs(3600));
        let five_hours_ago = Some(Duration::from_secs(5 * 3600));

        // Baseline (no global override): with only the 15m provider default, a
        // connection synced an hour ago is already overdue and WOULD fire.
        // (This is the behavior the feature is reining in.)
        assert!(connection_is_due(provider_default, one_hour_ago));

        // User picks "every 4h": now that same hour-old connection must NOT
        // fire — the global cadence (not the 15m default) governs the gap…
        assert_eq!(decide(Some(4 * 3600), one_hour_ago), Some(false));
        // …but once 5h have passed it fires again.
        assert_eq!(decide(Some(4 * 3600), five_hours_ago), Some(true));

        // User picks "Manual only" (0): never auto-fires, no matter how stale.
        assert_eq!(decide(Some(0), five_hours_ago), None);
        assert_eq!(decide(Some(0), None), None);

        // Unset (None) → 24h default: the hour-old connection is not yet due,
        // confirming the default is far more conservative than the 15m native
        // cadence.
        assert_eq!(decide(None, one_hour_ago), Some(false));
        assert_eq!(
            decide(None, Some(Duration::from_secs(25 * 3600))),
            Some(true)
        );

        // A never-synced connection fires on any non-manual setting (the
        // restart-recovery path).
        assert_eq!(decide(Some(4 * 3600), None), Some(true));
    }

    fn audit_entry(
        connection_id: &str,
        scope: &str,
        success: bool,
        ts: DateTime<Utc>,
    ) -> SyncAuditEntry {
        SyncAuditEntry {
            timestamp: ts,
            source_id: connection_id.to_string(),
            source_kind: "composio".to_string(),
            scope: scope.to_string(),
            items_fetched: 1,
            batches: 0,
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost_usd: 0.0,
            composio_actions_called: 1,
            composio_cost_usd: 0.0,
            actual_charged_usd: None,
            duration_ms: 10,
            success,
            error: None,
        }
    }

    #[test]
    fn index_last_success_keeps_latest_success_and_ignores_failures() {
        let now = Utc::now();
        let older = now - chrono::Duration::hours(6);
        let newer = now - chrono::Duration::hours(1);
        let entries = vec![
            audit_entry("cmp-1", "gmail:cmp-1", true, older),
            audit_entry("cmp-1", "gmail:cmp-1", true, newer), // newer success wins
            audit_entry("cmp-1", "gmail:cmp-1", false, now),  // failure ignored
            audit_entry("cmp-2", "slack:cmp-2", false, now),  // only-failure → absent
        ];
        let idx = index_last_success_by_connection(&entries);
        assert_eq!(idx.get("cmp-1"), Some(&newer));
        assert!(
            !idx.contains_key("cmp-2"),
            "a connection with only failed syncs is not indexed"
        );
    }

    #[test]
    fn index_last_success_falls_back_to_source_id_without_scope_suffix() {
        let now = Utc::now();
        // A non-composio kind is skipped entirely.
        let entries = vec![
            SyncAuditEntry {
                source_kind: "github_repo".to_string(),
                ..audit_entry("ignored", "github:org/repo", true, now)
            },
            // Composio entry whose scope has no ':' → key by source_id.
            audit_entry("cmp-3", "noscope", true, now),
        ];
        let idx = index_last_success_by_connection(&entries);
        assert!(idx.contains_key("cmp-3"));
        assert!(!idx.contains_key("ignored"));
    }

    #[test]
    fn persisted_since_last_sync_computes_and_saturates() {
        let now = Utc::now();
        let mut idx = HashMap::new();
        idx.insert("cmp-1".to_string(), now - chrono::Duration::hours(3));
        idx.insert("future".to_string(), now + chrono::Duration::hours(2));

        let elapsed = persisted_since_last_sync(&idx, "cmp-1", now).unwrap();
        // ~3h, allow a small window for test execution time.
        assert!(elapsed >= Duration::from_secs(3 * 3600 - 5));
        assert!(elapsed <= Duration::from_secs(3 * 3600 + 5));
        // Clock skew (future timestamp) saturates to zero, not a huge value.
        assert_eq!(
            persisted_since_last_sync(&idx, "future", now),
            Some(Duration::ZERO)
        );
        // Unknown connection → None (treated as never synced).
        assert_eq!(persisted_since_last_sync(&idx, "unknown", now), None);
    }

    /// The cadence must survive a restart: with the in-memory map cold, the
    /// persisted audit timestamp drives the due-check so a connection synced
    /// 1h ago does NOT re-fire under a 4h setting, but one synced 5h ago does.
    #[test]
    fn cadence_survives_restart_via_persisted_audit() {
        let now = Utc::now();
        let mut idx = HashMap::new();
        idx.insert("cmp-1".to_string(), now - chrono::Duration::hours(1));
        idx.insert("cmp-2".to_string(), now - chrono::Duration::hours(5));

        let interval = effective_interval_secs(15 * 60, Some(4 * 3600)).unwrap();

        // cmp-1 (synced 1h ago) — in-memory cold, persisted fallback says NOT due.
        let cmp1 = None.or_else(|| persisted_since_last_sync(&idx, "cmp-1", now));
        assert!(!connection_is_due(interval, cmp1));

        // cmp-2 (synced 5h ago) — persisted fallback says due.
        let cmp2 = None.or_else(|| persisted_since_last_sync(&idx, "cmp-2", now));
        assert!(connection_is_due(interval, cmp2));

        // A connection with no persisted record still fires (truly fresh).
        let fresh = None.or_else(|| persisted_since_last_sync(&idx, "cmp-new", now));
        assert!(connection_is_due(interval, fresh));
    }

    /// A successful periodic tick produces a Composio-kind audit entry that
    /// carries the billable-action tally + cost and zeroes the LLM-cost
    /// columns (summarisation happens later in the job worker). Pins the
    /// shape the Sync History panel reads (#3111 follow-up).
    #[test]
    fn periodic_audit_entry_records_composio_cost_on_success() {
        let usage = ComposioUsage {
            actions_called: 3,
            cost_usd: 0.042,
        };
        let entry = build_periodic_audit_entry("gmail", "cmp-123", &usage, 17, 1234, None);

        assert_eq!(entry.source_kind, "composio");
        assert_eq!(entry.source_id, "cmp-123");
        assert_eq!(entry.scope, "gmail:cmp-123");
        assert_eq!(entry.items_fetched, 17);
        assert_eq!(entry.composio_actions_called, 3);
        assert!((entry.composio_cost_usd - 0.042).abs() < f64::EPSILON);
        assert!(entry.success);
        assert!(entry.error.is_none());
        // Periodic fetch does no summarisation — LLM cost columns stay zero,
        // and the Composio spend is the whole combined cost.
        assert_eq!(entry.input_tokens, 0);
        assert_eq!(entry.estimated_cost_usd, 0.0);
        assert!((entry.combined_cost_usd() - 0.042).abs() < f64::EPSILON);
    }

    /// A failed periodic tick still records the partial billable cost it
    /// incurred before erroring (the fetch may have fired actions), with
    /// `success = false` and the error message preserved.
    #[test]
    fn periodic_audit_entry_preserves_partial_cost_on_failure() {
        let usage = ComposioUsage {
            actions_called: 1,
            cost_usd: 0.01,
        };
        let entry = build_periodic_audit_entry(
            "notion",
            "cmp-9",
            &usage,
            0,
            500,
            Some("fetch timed out".to_string()),
        );

        assert!(!entry.success);
        assert_eq!(entry.error.as_deref(), Some("fetch timed out"));
        assert_eq!(entry.items_fetched, 0);
        // The billable action it managed to fire before failing is still
        // recorded so cost isn't under-reported on failures.
        assert_eq!(entry.composio_actions_called, 1);
        assert!((entry.composio_cost_usd - 0.01).abs() < f64::EPSILON);
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
