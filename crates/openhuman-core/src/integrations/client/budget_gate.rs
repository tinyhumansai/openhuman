//! Managed-tool budget gate: refuses `/agent-integrations/*` calls once the
//! account's AI credits are exhausted, so a tool call never burns a backend
//! round-trip that the backend would reject anyway.
//!
//! Lives with the integrations client because that is its one consumer
//! (`requests.rs::ensure_budget_available`). The `team_get_usage` RPC lives in
//! `openhuman-tinyhumans` (`hosted::team::get_usage`, on the TinyHumans SDK)
//! and shares this module's failure backoff through
//! [`usage_with_failure_backoff`], so a persistent backend fault collapses to
//! about one probe a minute across both surfaces.
//!
//! The pre-call probe reads `GET /teams/me/usage` through
//! [`BackendOAuthClient`], so it rides the backend transport port. It resolves
//! the credential first and defers to the backend (allows the call) without
//! any request when there is no usable TinyHumans credential — the offline
//! local session, an API-less core, or a signed-out user.

use std::sync::RwLock;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::api::config::effective_backend_api_url;
use crate::api::BackendOAuthClient;
use crate::config::Config;
use crate::rpc::RpcOutcome;
use crate::security::credentials::session_support::BackendCredential;

/// Fetch `GET /teams/me/usage` for the pre-call probe with an
/// already-resolved credential. `flatten_authed_error` keeps the typed 401 on
/// the `SESSION_EXPIRED` sentinel so a lapse never anchors the backoff.
async fn fetch_usage(config: &Config, credential: BackendCredential) -> Result<Value, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| format!("{e:#}"))?;
    client
        .authed_json(&credential, reqwest::Method::GET, "/teams/me/usage", None)
        .await
        .map_err(crate::api::flatten_authed_error)
}

/// How long a *failed* usage fetch is short-circuited before the backend is
/// probed again. `get_usage` is hammered from two surfaces — the frontend usage
/// poll (the `team_get_usage` RPC) and the pre-call budget gate
/// (`managed_tool_budget_exhausted`) — so a *persistent* non-2xx (a misrouted
/// `BACKEND_URL`, a backend outage) otherwise re-fires on every poll and every
/// managed tool call, re-reporting to Sentry each time. That is the
/// `/teams/me/usage` flood in GH #4153 (TAURI-RUST-BSF/-8C/-HDS/-HW1/-JJ5).
///
/// This window collapses a persistent fault to ~one backend probe (and so
/// ~one Sentry event) per minute per process while staying responsive: the
/// FIRST failure of a streak still hits the backend and still reports (real
/// signal preserved — backpressure, not silent drop), and any success clears
/// the window immediately. This is defense-in-depth flood control; the actual
/// misroute fix lives in `effective_backend_api_url` (see GH #4153).
const USAGE_FAILURE_BACKOFF: Duration = Duration::from_secs(60);

/// Process-global anchor of the most recent *reportable* `get_usage` failure.
/// Session-expiry (401) failures are intentionally NOT recorded here — they are
/// handled by their own RPC arm and must keep driving auth recovery.
struct UsageFailureCache {
    /// Backend identity + `Instant` of the last reported failure for that
    /// backend, if a streak is active. Keying on the backend URL means a
    /// changed `config.api_url` (e.g. after the user fixes a misrouted
    /// `BACKEND_URL`, or auth/session context that re-points the backend) no
    /// longer matches the stored key, so the next probe hits the new backend
    /// immediately instead of inheriting the old backend's backoff (GH #4153).
    inner: RwLock<Option<(String, Instant)>>,
}

impl UsageFailureCache {
    const fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }

    /// True when a failure for `key` was recorded within `ttl` of `now`. A
    /// failure anchored under a different backend key never counts as fresh.
    fn is_fresh(&self, key: &str, now: Instant, ttl: Duration) -> bool {
        let guard = self.inner.read().unwrap_or_else(|e| e.into_inner());
        guard
            .as_ref()
            .is_some_and(|(k, at)| k == key && now.duration_since(*at) < ttl)
    }

    /// Anchor a fresh failure for `key` (start / keep a streak).
    fn record(&self, key: &str, now: Instant) {
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        *guard = Some((key.to_string(), now));
    }

    /// Clear the streak — the endpoint recovered.
    fn clear(&self) {
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    }
}

static USAGE_FAILURE_CACHE: UsageFailureCache = UsageFailureCache::new();

/// Run a `/teams/me/usage` fetch behind the process-wide failure backoff.
///
/// `backend_key` identifies the backend (the effective API URL) so a failure
/// on one backend never suppresses a probe after the backend is re-pointed
/// (#4153). Shared by the pre-call probe here and the hosted `team_get_usage`
/// RPC, which supplies an SDK-backed `fetch`. Callers resolve the credential
/// *before* calling this, so a missing credential never opens a streak.
pub async fn usage_with_failure_backoff<F, Fut>(
    backend_key: &str,
    fetch: F,
) -> Result<RpcOutcome<Value>, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    get_usage_with_cache(
        &USAGE_FAILURE_CACHE,
        backend_key,
        USAGE_FAILURE_BACKOFF,
        Instant::now(),
        fetch,
    )
    .await
}

/// The pre-call usage probe: `None` (allow, defer to the backend) when there is
/// no usable credential — no request, no backoff streak — else the usage
/// document behind the shared failure backoff.
async fn probe_usage(config: &Config) -> Result<Value, String> {
    let credential =
        crate::security::credentials::session_support::resolve_backend_credential(config)?;
    let backend_key = effective_backend_api_url(&config.api_url);
    usage_with_failure_backoff(&backend_key, || fetch_usage(config, credential))
        .await
        .map(|outcome| outcome.value)
}

/// Cache-aware usage fetch. Mirrors the backoff/backpressure shape of
/// [`budget_exhausted_with_cache`] but for *failures*:
///
/// - Within `ttl` of a recorded failure → short-circuit WITHOUT calling
///   `fetch` (network backpressure) and return the
///   [`crate::core::observability::USAGE_PROBE_BACKOFF_PREFIX`] sentinel, which
///   the JSON-RPC boundary demotes (no re-report).
/// - Otherwise call `fetch`: `Ok` clears the streak; a non-session-expiry `Err`
///   anchors a fresh streak and propagates verbatim (first-of-streak reports);
///   a session-expiry `Err` propagates verbatim WITHOUT anchoring (its own RPC
///   arm handles it / drives auth recovery).
async fn get_usage_with_cache<F, Fut>(
    cache: &UsageFailureCache,
    key: &str,
    ttl: Duration,
    now: Instant,
    fetch: F,
) -> Result<RpcOutcome<Value>, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    if cache.is_fresh(key, now, ttl) {
        tracing::debug!(
            "[team] usage probe in failure-backoff window — skipping backend call (suppressing repeat report)"
        );
        return Err(format!(
            "{} recent /teams/me/usage failure suppressed (backoff)",
            crate::core::observability::USAGE_PROBE_BACKOFF_PREFIX
        ));
    }

    match fetch().await {
        Ok(data) => {
            cache.clear();
            Ok(RpcOutcome::single_log(
                data,
                "team usage fetched from backend",
            ))
        }
        Err(err) => {
            if crate::core::observability::is_session_expired_message(&err) {
                // Session lapse — handled by the session-expired RPC arm and
                // drives local session cleanup. Never enter the failure window.
                tracing::debug!(
                    "[team] usage probe failed with session-expiry — not anchoring backoff"
                );
            } else {
                cache.record(key, now);
            }
            Err(err)
        }
    }
}

fn usage_number(data: &Value, key: &str) -> f64 {
    data.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Returns true when backend usage says OpenHuman-managed spend should stop.
///
/// A brand-new free account can legitimately have `remainingUsd == 0` and no
/// recurring budget; that should not disable managed tools on its own. We only
/// gate once there is an actual cycle budget/spend signal, matching the
/// frontend's exhausted-budget semantics while covering spend-only payloads.
pub fn usage_budget_exhausted(data: &Value) -> bool {
    let remaining = usage_number(data, "remainingUsd");
    let cycle_budget = usage_number(data, "cycleBudgetUsd");
    let cycle_spent = usage_number(data, "cycleSpentUsd");
    let cycle_limit_7day = usage_number(data, "cycleLimit7day");
    let bypass = data
        .get("bypassCycleLimit")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    !bypass
        && remaining <= 0.01
        && (cycle_budget > 0.01 || cycle_spent > 0.01 || cycle_limit_7day > 0.01)
}

/// How long a managed-tool budget probe result is reused before re-fetching.
///
/// `ensure_budget_available` runs before **every** managed `post`/`get`, so a
/// burst of managed tool calls in one agent turn would otherwise fire one
/// `GET /teams/me/usage` round-trip *per call* — doubling the network cost of
/// each managed integration request and re-fetching identical usage data. A
/// short TTL collapses that burst to one probe while keeping the gate
/// responsive: the backend remains the authoritative gate (it rejects spend
/// once credits run out), so the worst case of a stale cache is a handful of
/// extra calls within the window that the backend itself still blocks.
const BUDGET_PROBE_TTL: Duration = Duration::from_secs(30);

/// Process-global cache for the managed-tool budget probe.
struct BudgetProbeCache {
    /// `(fetched_at, exhausted)` of the last successful probe, if any.
    inner: RwLock<Option<(Instant, bool)>>,
}

impl BudgetProbeCache {
    const fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }

    /// Cached `exhausted` flag if the last probe is still within `ttl`.
    fn get(&self, now: Instant, ttl: Duration) -> Option<bool> {
        let guard = self.inner.read().unwrap_or_else(|e| e.into_inner());
        guard.and_then(|(fetched_at, exhausted)| {
            (now.duration_since(fetched_at) < ttl).then_some(exhausted)
        })
    }

    /// Record a fresh probe result.
    fn put(&self, now: Instant, exhausted: bool) {
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        *guard = Some((now, exhausted));
    }
}

static BUDGET_PROBE_CACHE: BudgetProbeCache = BudgetProbeCache::new();

pub async fn managed_tool_budget_exhausted(config: &Config) -> bool {
    budget_exhausted_with_cache(&BUDGET_PROBE_CACHE, BUDGET_PROBE_TTL, || async {
        match probe_usage(config).await {
            Ok(usage) => Some(usage_budget_exhausted(&usage)),
            Err(err) => {
                tracing::debug!(
                    error = %err,
                    "[budget-gate] usage probe failed; allowing managed tool to defer to backend"
                );
                None
            }
        }
    })
    .await
}

/// Cache-aware budget gate. Returns the cached `exhausted` flag when fresh;
/// otherwise calls `fetch` and caches a successful result. A failed probe
/// (`fetch` returns `None`) is **not** cached and reports "not exhausted" so
/// the call defers to the backend gate — identical to the pre-cache behaviour.
async fn budget_exhausted_with_cache<F, Fut>(
    cache: &BudgetProbeCache,
    ttl: Duration,
    fetch: F,
) -> bool
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Option<bool>>,
{
    if let Some(hit) = cache.get(Instant::now(), ttl) {
        tracing::debug!(exhausted = hit, "[team] budget probe cache hit");
        return hit;
    }
    match fetch().await {
        Some(exhausted) => {
            tracing::debug!(
                exhausted,
                "[team] budget probe cache miss; caching fresh probe"
            );
            cache.put(Instant::now(), exhausted);
            exhausted
        }
        None => {
            tracing::debug!("[team] budget probe failed; deferring to backend gate (not cached)");
            false
        }
    }
}

#[cfg(test)]
#[path = "budget_gate_tests.rs"]
mod tests;
