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
    crate::openhuman::credentials::session_support::require_live_session_token(config)
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

pub async fn get_usage(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(config, Method::GET, "/teams/me/usage", None).await?;
    Ok(RpcOutcome::single_log(
        data,
        "team usage fetched from backend",
    ))
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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn budget_cache_get_returns_none_when_empty() {
        let cache = BudgetProbeCache::new();
        assert_eq!(cache.get(Instant::now(), Duration::from_secs(30)), None);
    }

    #[test]
    fn budget_cache_returns_value_within_ttl_and_expires_after() {
        let cache = BudgetProbeCache::new();
        let base = Instant::now();
        cache.put(base, true);
        // Within TTL → cached value.
        assert_eq!(
            cache.get(base + Duration::from_secs(5), Duration::from_secs(30)),
            Some(true)
        );
        // Past TTL → miss (caller must re-probe).
        assert_eq!(
            cache.get(base + Duration::from_secs(31), Duration::from_secs(30)),
            None
        );
    }

    #[tokio::test]
    async fn cache_hit_skips_fetch() {
        let cache = BudgetProbeCache::new();
        cache.put(Instant::now(), true);
        let calls = AtomicUsize::new(0);
        let result = budget_exhausted_with_cache(&cache, Duration::from_secs(30), || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Some(false) }
        })
        .await;
        assert!(result, "fresh cache value should be returned");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "fetch must be skipped on cache hit"
        );
    }

    #[tokio::test]
    async fn cache_miss_fetches_and_caches() {
        let cache = BudgetProbeCache::new();
        let calls = AtomicUsize::new(0);
        // First call: empty cache → fetch runs and result is cached.
        let first = budget_exhausted_with_cache(&cache, Duration::from_secs(30), || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Some(true) }
        })
        .await;
        assert!(first);
        // Second call: cache is now warm → fetch is skipped.
        let second = budget_exhausted_with_cache(&cache, Duration::from_secs(30), || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Some(false) }
        })
        .await;
        assert!(
            second,
            "second call should return the cached true, not the fresh false"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "only the first call should fetch"
        );
    }

    #[tokio::test]
    async fn failed_probe_is_not_cached_and_defers_to_backend() {
        let cache = BudgetProbeCache::new();
        let calls = AtomicUsize::new(0);
        // Probe failure (None) → returns false (defer to backend) and does not cache.
        let first = budget_exhausted_with_cache(&cache, Duration::from_secs(30), || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { None }
        })
        .await;
        assert!(!first, "failed probe defers to backend (not exhausted)");
        // Next call must re-probe because the failure wasn't cached.
        let second = budget_exhausted_with_cache(&cache, Duration::from_secs(30), || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Some(true) }
        })
        .await;
        assert!(second);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "failed probe is re-fetched next time"
        );
    }

    #[test]
    fn build_api_path_encodes_reserved_characters_in_segments() {
        let path = build_api_path(&["teams", "team/with?reserved", "members", "user#frag"])
            .expect("path should build");

        assert_eq!(path, "/teams/team%2Fwith%3Freserved/members/user%23frag");
    }

    #[test]
    fn build_api_path_empty_segments_list_is_root() {
        let path = build_api_path(&[]).expect("path should build");
        assert_eq!(path, "/");
    }

    #[test]
    fn build_api_path_preserves_segment_order() {
        let path = build_api_path(&["a", "b", "c"]).expect("path should build");
        assert_eq!(path, "/a/b/c");
    }

    #[test]
    fn build_api_path_percent_encodes_spaces_and_unicode() {
        let path = build_api_path(&["teams", "with space", "👥"]).expect("path should build");
        assert!(path.contains("with%20space"));
        // Unicode must be percent-encoded (UTF-8 bytes).
        assert!(!path.contains('👥'));
    }

    #[test]
    fn normalize_id_rejects_empty_with_field_name() {
        let err = normalize_id("", "teamId").unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[test]
    fn normalize_id_rejects_whitespace_only() {
        let err = normalize_id("   \t\n", "userId").unwrap_err();
        assert_eq!(err, "userId is required");
    }

    #[test]
    fn normalize_id_trims_and_keeps_body() {
        assert_eq!(normalize_id("  abc  ", "teamId").unwrap(), "abc");
    }

    #[test]
    fn normalize_id_preserves_internal_whitespace() {
        // Only leading/trailing whitespace is stripped — interior is preserved
        // so we don't silently corrupt caller-provided identifiers.
        assert_eq!(normalize_id("a b", "x").unwrap(), "a b");
    }

    #[test]
    fn usage_budget_exhausted_requires_real_cycle_signal() {
        assert!(!usage_budget_exhausted(&json!({
            "remainingUsd": 0,
            "cycleBudgetUsd": 0,
            "cycleSpentUsd": 0,
        })));
        assert!(usage_budget_exhausted(&json!({
            "remainingUsd": 0,
            "cycleBudgetUsd": 10,
            "cycleSpentUsd": 10,
        })));
        assert!(usage_budget_exhausted(&json!({
            "remainingUsd": 0,
            "cycleBudgetUsd": 0,
            "cycleSpentUsd": 2,
        })));
    }

    #[test]
    fn usage_budget_exhausted_honors_remaining_and_bypass() {
        assert!(!usage_budget_exhausted(&json!({
            "remainingUsd": 0.25,
            "cycleBudgetUsd": 10,
        })));
        assert!(!usage_budget_exhausted(&json!({
            "remainingUsd": 0,
            "cycleBudgetUsd": 10,
            "bypassCycleLimit": true,
        })));
    }

    // --- pre-HTTP input validation (no network) -----------------------------

    fn cfg() -> Config {
        Config::default()
    }

    #[tokio::test]
    async fn list_members_rejects_empty_team_id() {
        let err = list_members(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn list_members_rejects_whitespace_team_id() {
        let err = list_members(&cfg(), "   ").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn get_team_rejects_empty_team_id() {
        let err = get_team(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn create_team_rejects_empty_name() {
        let err = create_team(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "name is required");
    }

    #[tokio::test]
    async fn create_team_rejects_whitespace_name() {
        let err = create_team(&cfg(), "   ").await.unwrap_err();
        assert_eq!(err, "name is required");
    }

    #[tokio::test]
    async fn update_team_rejects_empty_team_id() {
        let err = update_team(&cfg(), "", Some("new")).await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn delete_team_rejects_empty_team_id() {
        let err = delete_team(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn switch_team_rejects_empty_team_id() {
        let err = switch_team(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn leave_team_rejects_empty_team_id() {
        let err = leave_team(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn join_team_rejects_empty_code() {
        let err = join_team(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "code is required");
    }

    #[tokio::test]
    async fn join_team_rejects_whitespace_code() {
        let err = join_team(&cfg(), "   ").await.unwrap_err();
        assert_eq!(err, "code is required");
    }

    #[tokio::test]
    async fn create_invite_rejects_empty_team_id() {
        let err = create_invite(&cfg(), "", None, None).await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn remove_member_validates_team_id_before_user_id() {
        // Failing input order must be deterministic: team_id is normalized
        // first, so an empty team_id reports the teamId error regardless of
        // the user_id.
        let err = remove_member(&cfg(), "", "someone").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn remove_member_rejects_empty_user_id_when_team_id_valid() {
        let err = remove_member(&cfg(), "t1", "").await.unwrap_err();
        assert_eq!(err, "userId is required");
    }

    #[tokio::test]
    async fn change_member_role_rejects_missing_role() {
        let err = change_member_role(&cfg(), "t1", "u1", "")
            .await
            .unwrap_err();
        assert_eq!(err, "role is required");
    }

    #[tokio::test]
    async fn change_member_role_validates_team_id_first() {
        let err = change_member_role(&cfg(), "", "u1", "admin")
            .await
            .unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn change_member_role_validates_user_id_before_role() {
        let err = change_member_role(&cfg(), "t1", "", "admin")
            .await
            .unwrap_err();
        assert_eq!(err, "userId is required");
    }

    #[tokio::test]
    async fn list_invites_rejects_empty_team_id() {
        let err = list_invites(&cfg(), "").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn revoke_invite_rejects_empty_team_id() {
        let err = revoke_invite(&cfg(), "", "inv1").await.unwrap_err();
        assert_eq!(err, "teamId is required");
    }

    #[tokio::test]
    async fn revoke_invite_rejects_empty_invite_id() {
        let err = revoke_invite(&cfg(), "t1", "").await.unwrap_err();
        assert_eq!(err, "inviteId is required");
    }
}
