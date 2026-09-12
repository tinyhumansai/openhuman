//! Team management RPC ops — thin adapters that call the hosted API.
//!
//! # Security
//! All methods require a valid app-session JWT stored via `auth_store_session`.
//! The JWT is sent as `Authorization: Bearer …` to the backend.
//! **No server-side authorization is replicated here**: the backend enforces team
//! ownership, role permissions, and tenant isolation on every request.
//! Callers without the required role (e.g. non-owner trying to remove a member)
//! receive a backend 401/403 surfaced verbatim as an RPC error string.
//! API keys / JWTs are never written to logs.

use std::sync::RwLock;
use std::time::{Duration, Instant};

use reqwest::{Method, Url};
use serde::Serialize;
use serde_json::{json, Value};

use crate::api::config::effective_backend_api_url;
use crate::api::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

/// Canonical authed-session guard. Delegates to `require_live_session_token`,
/// which rejects an expired token locally (publishing `SessionExpired`) instead
/// of firing a doomed backend 401 — see #3297 / `session_support`.
fn require_token(config: &Config) -> Result<String, String> {
    crate::openhuman::security::credentials::session_support::require_live_session_token(config)
}

fn normalize_id(input: &str, field: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} is required"));
    }
    Ok(trimmed.to_string())
}

fn build_api_path(segments: &[&str]) -> Result<String, String> {
    let mut url = Url::parse("https://openhuman.invalid")
        .map_err(|e| format!("failed to initialize URL path builder: {e}"))?;
    {
        let mut path_segments = url
            .path_segments_mut()
            .map_err(|_| "failed to initialize URL path builder".to_string())?;
        path_segments.clear();
        for segment in segments {
            path_segments.push(segment);
        }
    }
    Ok(url.path().to_string())
}

async fn get_authed_value(
    config: &Config,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let token = require_token(config)?;
    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| format!("{e:#}"))?;
    // `flatten_authed_error` maps the typed `BackendApiError::Unauthorized`
    // (expected session-lapse 401) onto the `SESSION_EXPIRED` sentinel so the
    // JSON-RPC layer classifies it as session expiry and skips Sentry (#3297,
    // TAURI-RUST-8WY on `/teams/me/usage`); every other error keeps its full
    // `{e:#}` anyhow chain. `authed_json` wraps the underlying reqwest error
    // with `.context(format!("backend request {} {}", …))`
    // (`api/rest.rs::authed_json`), so `{e:#}` (not `e.to_string()`) is required
    // to surface the cause (connect timeout, DNS failure, TLS handshake, non-2xx
    // status, …) before the JSON-RPC layer reports it to Sentry.
    // OPENHUMAN-TAURI-AD is the canonical instance: 2 events on `0.53.35` from a
    // Russia user, all with the truncated label and elapsed_ms=49 — far too
    // short for a real timeout, so the underlying cause is the only signal worth
    // surfacing. Same failure mode the `report_error` doc-string in
    // `core/observability.rs` calls out (TAURI-B2).
    client
        .authed_json(&token, method, path, body)
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

pub async fn get_usage(config: &Config) -> Result<RpcOutcome<Value>, String> {
    // Key the failure backoff by the effective backend URL so a failure on one
    // backend never suppresses probes after the backend is re-pointed (#4153).
    let backend_key = effective_backend_api_url(&config.api_url);
    get_usage_with_cache(
        &USAGE_FAILURE_CACHE,
        &backend_key,
        USAGE_FAILURE_BACKOFF,
        Instant::now(),
        || async { get_authed_value(config, Method::GET, "/teams/me/usage", None).await },
    )
    .await
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
        match get_usage(config).await {
            Ok(outcome) => Some(usage_budget_exhausted(&outcome.value)),
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

pub async fn list_members(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id, "members"])?;
    let data = get_authed_value(config, Method::GET, &path, None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team members fetched from backend",
    ))
}

pub async fn list_teams(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(config, Method::GET, "/teams", None).await?;
    Ok(RpcOutcome::single_log(data, "teams fetched from backend"))
}

pub async fn get_team(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id])?;
    let data = get_authed_value(config, Method::GET, &path, None).await?;
    Ok(RpcOutcome::single_log(data, "team fetched from backend"))
}

#[derive(Debug, Serialize)]
struct TeamNameBody<'a> {
    name: &'a str,
}

pub async fn create_team(config: &Config, name: &str) -> Result<RpcOutcome<Value>, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("name is required".to_string());
    }
    let data = get_authed_value(
        config,
        Method::POST,
        "/teams",
        Some(json!(TeamNameBody { name: trimmed })),
    )
    .await?;
    Ok(RpcOutcome::single_log(data, "team created via backend"))
}

pub async fn update_team(
    config: &Config,
    team_id: &str,
    name: Option<&str>,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id])?;
    let mut body = serde_json::Map::new();
    if let Some(name) = name.map(str::trim).filter(|value| !value.is_empty()) {
        body.insert("name".to_string(), Value::String(name.to_string()));
    }
    let data = get_authed_value(config, Method::PUT, &path, Some(Value::Object(body))).await?;
    Ok(RpcOutcome::single_log(data, "team updated via backend"))
}

pub async fn delete_team(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id])?;
    let data = get_authed_value(config, Method::DELETE, &path, None).await?;
    Ok(RpcOutcome::single_log(data, "team deleted via backend"))
}

pub async fn switch_team(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id, "switch"])?;
    let data = get_authed_value(config, Method::POST, &path, Some(json!({}))).await?;
    Ok(RpcOutcome::single_log(
        data,
        "active team switched via backend",
    ))
}

pub async fn leave_team(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id, "leave"])?;
    let data = get_authed_value(config, Method::POST, &path, Some(json!({}))).await?;
    Ok(RpcOutcome::single_log(data, "team left via backend"))
}

pub async fn join_team(config: &Config, code: &str) -> Result<RpcOutcome<Value>, String> {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return Err("code is required".to_string());
    }
    let data = get_authed_value(
        config,
        Method::POST,
        "/teams/join",
        Some(json!({ "code": trimmed })),
    )
    .await?;
    Ok(RpcOutcome::single_log(data, "team joined via backend"))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InviteBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_uses: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in_days: Option<u64>,
}

pub async fn create_invite(
    config: &Config,
    team_id: &str,
    max_uses: Option<u64>,
    expires_in_days: Option<u64>,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id, "invites"])?;
    let body = json!(InviteBody {
        max_uses,
        expires_in_days,
    });
    let data = get_authed_value(config, Method::POST, &path, Some(body)).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team invite created via backend",
    ))
}

pub async fn remove_member(
    config: &Config,
    team_id: &str,
    user_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let user_id = normalize_id(user_id, "userId")?;
    let path = build_api_path(&["teams", &team_id, "members", &user_id])?;
    let data = get_authed_value(config, Method::DELETE, &path, None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team member removed via backend",
    ))
}

pub async fn change_member_role(
    config: &Config,
    team_id: &str,
    user_id: &str,
    role: &str,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let user_id = normalize_id(user_id, "userId")?;
    let role = normalize_id(role, "role")?;
    let path = build_api_path(&["teams", &team_id, "members", &user_id, "role"])?;
    let body = json!({ "role": role });
    let data = get_authed_value(config, Method::PUT, &path, Some(body)).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team member role updated via backend",
    ))
}

/// List all active invites for a team.
/// Maps to `GET /teams/:teamId/invites` — matches `teamApi.getInvites`.
pub async fn list_invites(config: &Config, team_id: &str) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let path = build_api_path(&["teams", &team_id, "invites"])?;
    let data = get_authed_value(config, Method::GET, &path, None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team invites listed from backend",
    ))
}

/// Revoke (delete) an existing invite by id.
/// Maps to `DELETE /teams/:teamId/invites/:inviteId` — matches `teamApi.revokeInvite`.
pub async fn revoke_invite(
    config: &Config,
    team_id: &str,
    invite_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let team_id = normalize_id(team_id, "teamId")?;
    let invite_id = normalize_id(invite_id, "inviteId")?;
    let path = build_api_path(&["teams", &team_id, "invites", &invite_id])?;
    let data = get_authed_value(config, Method::DELETE, &path, None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team invite revoked via backend",
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
