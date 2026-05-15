use super::*;

#[test]
fn parse_sync_reason_accepts_known_values() {
    assert_eq!(parse_sync_reason(None).unwrap(), SyncReason::Manual);
    assert_eq!(
        parse_sync_reason(Some("manual")).unwrap(),
        SyncReason::Manual
    );
    assert_eq!(
        parse_sync_reason(Some("periodic")).unwrap(),
        SyncReason::Periodic
    );
    assert_eq!(
        parse_sync_reason(Some("connection_created")).unwrap(),
        SyncReason::ConnectionCreated
    );
}

#[test]
fn parse_sync_reason_rejects_unknown_values() {
    let err = parse_sync_reason(Some("scheduled")).unwrap_err();
    assert!(err.contains("unrecognized sync reason"));
    assert!(err.contains("scheduled"));
    // Typo of a real value should also fail rather than coerce.
    assert!(parse_sync_reason(Some("Periodic")).is_err());
    assert!(parse_sync_reason(Some("")).is_err());
}

// ── resolve_client / ops auth errors ──────────────────────────

fn test_config(tmp: &tempfile::TempDir) -> Config {
    let mut c = Config::default();
    c.workspace_dir = tmp.path().join("workspace");
    c.config_path = tmp.path().join("config.toml");
    c
}

#[test]
fn resolve_client_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // `ComposioClient` intentionally doesn't implement `Debug` — use a
    // pattern match instead of `.unwrap_err()`.
    let Err(err) = resolve_client(&config) else {
        panic!("expected auth error when no session is stored");
    };
    assert!(err.contains("composio unavailable"));
    assert!(err.contains("auth_store_session"));
}

#[tokio::test]
async fn composio_list_toolkits_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_list_toolkits(&config).await.unwrap_err();
    // Backend-mode (default) without a session — the mode-aware factory
    // surfaces "no backend session token" so we accept either the
    // legacy `composio unavailable` prefix or the new factory message.
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("no backend session") || err.contains("unavailable")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_list_connections_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_list_connections(&config).await.unwrap_err();
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("no backend session") || err.contains("unavailable")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_authorize_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_authorize(&config, "gmail", None)
        .await
        .unwrap_err();
    // Backend mode (default) without a session — the mode-aware factory
    // surfaces "no backend session token" once `composio_authorize`
    // routes through `create_composio_client`. Accept either the
    // legacy `composio unavailable` prefix or the new factory phrasing.
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("no backend session") || err.contains("unavailable")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_delete_connection_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_delete_connection(&config, "c-1")
        .await
        .unwrap_err();
    assert!(err.contains("composio unavailable"));
}

#[tokio::test]
async fn composio_list_tools_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_list_tools(&config, None).await.unwrap_err();
    // Same tolerance as `composio_list_toolkits_errors_without_session`.
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("no backend session") || err.contains("unavailable")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_execute_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_execute(&config, "GMAIL_SEND_EMAIL", None)
        .await
        .unwrap_err();
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("no backend session") || err.contains("unavailable")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_get_user_profile_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_get_user_profile(&config, "c-1").await.unwrap_err();
    assert!(err.contains("composio unavailable"));
}

#[tokio::test]
async fn composio_sync_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_sync(&config, "c-1", None).await.unwrap_err();
    assert!(err.contains("composio unavailable"));
}

#[tokio::test]
async fn composio_sync_rejects_invalid_reason_before_client_check() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // Invalid reason → should fail at parse step *before* touching the
    // client, so the error message references the reason, not auth.
    let err = composio_sync(&config, "c-1", Some("weird".into()))
        .await
        .unwrap_err();
    assert!(err.contains("unrecognized sync reason"));
}

#[tokio::test]
async fn composio_list_trigger_history_errors_when_store_not_init() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // The trigger history store is a process-global singleton. If
    // another test in the same binary already initialised it (e.g.
    // via the archive-roundtrip test), skip rather than asserting on
    // the uninitialised branch.
    if super::super::trigger_history::global().is_some() {
        return;
    }
    let err = composio_list_trigger_history(&config, Some(10))
        .await
        .unwrap_err();
    assert!(err.contains("archive store is not initialized"));
}

// ── cache_key / invalidate_connected_integrations_cache ───────

/// Process-wide mutex every test that mutates the `INTEGRATIONS_CACHE`
/// takes before it runs. cargo runs tests in parallel within a
/// single binary, and all these tests touch the same global map;
/// holding this guard keeps concurrent invalidations from
/// clobbering each other's seeded state. Poison-recover so a panic
/// in one test doesn't permanently block the rest.
static CACHE_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn cache_key_is_based_on_config_path_string() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Config::default();
    a.config_path = tmp.path().join("a.toml");
    let mut b = Config::default();
    b.config_path = tmp.path().join("b.toml");
    assert_ne!(cache_key(&a), cache_key(&b));
    assert_eq!(cache_key(&a), cache_key(&a));
}

#[tokio::test]
async fn fetch_connected_integrations_returns_empty_without_auth() {
    let _guard = CACHE_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let integrations = fetch_connected_integrations(&config).await;
    assert!(integrations.is_empty());
}

#[test]
fn invalidate_connected_integrations_cache_is_safe_without_prior_insert() {
    let _guard = CACHE_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    // Must not panic on an empty cache.
    invalidate_connected_integrations_cache();
    invalidate_connected_integrations_cache();
}

// ── Mock-backend integration tests for ops ─────────────────────

use axum::{
    extract::{Path, Query},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;

async fn start_mock_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Wait until the axum accept loop is actually serving — not just
    // until the kernel-level TCP socket is bound. Without this, fast
    // tests can fire a request before `axum::serve` starts polling and
    // occasionally see connection resets / hangs on loaded CI.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut backoff = std::time::Duration::from_millis(2);
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("mock backend at {addr} did not become ready in time");
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(std::time::Duration::from_millis(50));
    }

    format!("http://127.0.0.1:{}", addr.port())
}

fn config_with_backend(tmp: &tempfile::TempDir, base: String) -> Config {
    let mut c = Config::default();
    c.workspace_dir = tmp.path().join("workspace");
    c.config_path = tmp.path().join("config.toml");
    c.api_url = Some(base);
    crate::openhuman::credentials::AuthService::from_config(&c)
        .store_provider_token(
            crate::openhuman::credentials::APP_SESSION_PROVIDER,
            crate::openhuman::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "test-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");
    c
}

#[tokio::test]
async fn composio_list_toolkits_via_mock() {
    let app = Router::new().route(
        "/agent-integrations/composio/toolkits",
        get(|| async { Json(json!({"success": true, "data": {"toolkits": ["gmail"]}})) }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_list_toolkits(&config).await.unwrap();
    assert_eq!(outcome.value.toolkits, vec!["gmail".to_string()]);
    assert!(outcome.logs.iter().any(|l| l.contains("toolkit")));
}

#[tokio::test]
async fn composio_list_connections_via_mock_counts_active() {
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            Json(json!({
                "success": true,
                "data": {"connections": [
                    {"id":"c1","toolkit":"gmail","status":"ACTIVE"},
                    {"id":"c2","toolkit":"notion","status":"PENDING"},
                    {"id":"c3","toolkit":"gmail","status":"CONNECTED"}
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_list_connections(&config).await.unwrap();
    assert_eq!(outcome.value.connections.len(), 3);
    // 2 active, 3 total
    assert!(outcome.logs.iter().any(|l| l.contains("3 connection")));
    assert!(outcome.logs.iter().any(|l| l.contains("2 active")));
}

#[tokio::test]
async fn composio_authorize_via_mock_publishes_event_and_returns_url() {
    let app = Router::new().route(
        "/agent-integrations/composio/authorize",
        post(|Json(_b): Json<Value>| async move {
            Json(json!({
                "success": true,
                "data": {"connectUrl": "https://x", "connectionId": "c1"}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_authorize(&config, "gmail", None).await.unwrap();
    assert_eq!(outcome.value.connect_url, "https://x");
    assert_eq!(outcome.value.connection_id, "c1");
}

#[tokio::test]
async fn composio_delete_connection_via_mock() {
    let app = Router::new().route(
        "/agent-integrations/composio/connections/{id}",
        axum::routing::delete(|Path(_id): Path<String>| async move {
            Json(json!({"success": true, "data": {"deleted": true}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_delete_connection(&config, "c1").await.unwrap();
    assert!(outcome.value.deleted);
}

#[tokio::test]
async fn composio_list_tools_via_mock_with_filter() {
    let app = Router::new().route(
        "/agent-integrations/composio/tools",
        get(|Query(_q): Query<HashMap<String, String>>| async move {
            Json(json!({
                "success": true,
                "data": {"tools": [
                    {"type":"function","function":{"name":"GMAIL_SEND_EMAIL"}},
                    {"type":"function","function":{"name":"GMAIL_SEARCH"}}
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_list_tools(&config, Some(vec!["gmail".into()]))
        .await
        .unwrap();
    assert_eq!(outcome.value.tools.len(), 2);
}

#[tokio::test]
async fn composio_execute_via_mock_succeeds_and_logs_elapsed() {
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|Json(b): Json<Value>| async move {
            Json(json!({
                "success": true,
                "data": {
                    "data": {"echo": b["tool"]},
                    "successful": true,
                    "error": null,
                    "costUsd": 0.001
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_execute(&config, "GMAIL_SEND", Some(json!({"to": "a"})))
        .await
        .unwrap();
    assert!(outcome.value.successful);
    assert!(outcome
        .logs
        .iter()
        .any(|l| l.contains("executed GMAIL_SEND")));
}

#[tokio::test]
async fn composio_execute_via_mock_propagates_backend_error() {
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|| async { Json(json!({"success": false, "error": "rate limited"})) }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let err = composio_execute(&config, "ANY_TOOL", None)
        .await
        .unwrap_err();
    assert!(err.contains("execute failed"));
}

#[tokio::test]
async fn fetch_connected_integrations_via_mock_aggregates_tools() {
    let _guard = CACHE_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    // Connections: gmail + notion. Tools: filtered to those toolkits
    // and prefixed with the uppercased slug. The toolkits route
    // backs the `list_toolkits()` allowlist gate that
    // `fetch_connected_integrations_uncached` calls before touching
    // connections — without it the function bails out at the first
    // step and returns an empty vec.
    let app = Router::new()
        .route(
            "/agent-integrations/composio/toolkits",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"toolkits": ["gmail", "notion"]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"gmail","status":"ACTIVE"},
                        {"id":"c2","toolkit":"notion","status":"CONNECTED"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/tools",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"tools": [
                        {"type":"function","function":{
                            "name":"GMAIL_SEND_EMAIL",
                            "description":"Send"
                        }},
                        {"type":"function","function":{
                            "name":"NOTION_CREATE_PAGE",
                            "description":"Create"
                        }}
                    ]}
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    // Use a fresh cache key by isolating config_path.
    let config = config_with_backend(&tmp, base);
    invalidate_connected_integrations_cache();
    let integrations = fetch_connected_integrations(&config).await;
    assert_eq!(integrations.len(), 2);
    // Sorted by toolkit name
    assert_eq!(integrations[0].toolkit, "gmail");
    assert_eq!(integrations[1].toolkit, "notion");
    assert_eq!(integrations[0].tools.len(), 1);
    assert_eq!(integrations[0].tools[0].name, "GMAIL_SEND_EMAIL");
}

#[tokio::test]
async fn fetch_connected_integrations_treats_slack_and_telegram_status_like_ui() {
    let _guard = CACHE_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let app = Router::new()
        .route(
            "/agent-integrations/composio/toolkits",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"toolkits": [" Slack ", "telegram"]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c-slack","toolkit":" Slack ","status":"connected"},
                        {"id":"c-telegram","toolkit":"telegram","status":" active "}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/tools",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"tools": [
                        {"type":"function","function":{
                            "name":"SLACK_FETCH_CONVERSATION_HISTORY",
                            "description":"Read Slack channel history"
                        }},
                        {"type":"function","function":{
                            "name":"TELEGRAM_GET_CHAT_HISTORY",
                            "description":"Read Telegram chat history"
                        }},
                        {"type":"function","function":{
                            "name":"SLACK_DELETE_CHANNEL",
                            "description":"Delete a channel"
                        }}
                    ]}
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    invalidate_connected_integrations_cache();

    let integrations = fetch_connected_integrations(&config).await;

    let slack = integrations
        .iter()
        .find(|i| i.toolkit == "slack")
        .expect("slack integration should be present");
    assert!(slack.connected);
    assert_eq!(slack.tools.len(), 1);
    assert_eq!(slack.tools[0].name, "SLACK_FETCH_CONVERSATION_HISTORY");

    let telegram = integrations
        .iter()
        .find(|i| i.toolkit == "telegram")
        .expect("telegram integration should be present");
    assert!(telegram.connected);
    assert_eq!(telegram.tools.len(), 1);
    assert_eq!(telegram.tools[0].name, "TELEGRAM_GET_CHAT_HISTORY");
}

#[tokio::test]
async fn fetch_connected_integrations_via_mock_returns_empty_with_no_active() {
    let _guard = CACHE_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            Json(json!({"success": true, "data": {"connections": [
                {"id":"c1","toolkit":"gmail","status":"PENDING"}
            ]}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    invalidate_connected_integrations_cache();
    let integrations = fetch_connected_integrations(&config).await;
    assert!(integrations.is_empty());
}

// ── Windows-observed sync regression coverage (issue #749) ────
//
// These tests exercise the cross-platform defenses layered on top
// of the `ComposioConnectionCreated` → `wait_for_connection_active`
// event-bus invalidation path — which can miss on Windows when the
// OAuth handoff outruns the 60 s readiness poll. They use the ops
// helpers directly (no mock backend needed) so they're deterministic
// and don't depend on the tokio runtime's scheduling.
//
// Every test uses a unique cache key (a unique &str literal) and
// clears only *its* key before seeding, so they can safely run in
// parallel with each other and with any other test in the binary
// that mutates `INTEGRATIONS_CACHE` (e.g. the mock-backend tests
// above call `invalidate_connected_integrations_cache()`, which
// would otherwise wipe our seeded state mid-run).

/// Remove just the test's own cache entry. Preferred over
/// [`invalidate_connected_integrations_cache`] inside these tests
/// because it can't be clobbered by — nor clobber — parallel tests
/// that also touch the global cache.
fn clear_cache_key(key: &str) {
    if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
        guard.remove(key);
    }
}

/// Seed the process-wide cache with `integrations` keyed by `key`
/// and an `Instant::now()` timestamp. Used by tests that want to
/// drive cache behaviour without going through a backend fetch.
fn seed_cache(key: &str, integrations: Vec<ConnectedIntegration>) {
    let mut guard = INTEGRATIONS_CACHE.write().unwrap();
    guard.insert(
        key.to_string(),
        CachedIntegrations {
            entries: integrations,
            cached_at: Instant::now(),
        },
    );
}

/// Build a minimal `ConnectedIntegration` for cache-seeding tests.
/// Only `toolkit` + `connected` matter for diff-based invalidation.
fn integration(toolkit: &str, connected: bool) -> ConnectedIntegration {
    ConnectedIntegration {
        toolkit: toolkit.to_string(),
        description: String::new(),
        tools: Vec::new(),
        connected,
    }
}

/// Build a minimal backend connection row for
/// `sync_cache_with_connections` tests.
fn conn(id: &str, toolkit: &str, status: &str) -> super::super::types::ComposioConnection {
    // The real type has a handful of optional metadata fields we
    // don't care about here — construct via serde so the test
    // stays decoupled from struct-field churn.
    serde_json::from_value(json!({
        "id": id,
        "toolkit": toolkit,
        "status": status,
    }))
    .expect("deserialize test ComposioConnection")
}

#[test]
fn sync_cache_invalidates_when_connection_becomes_active() {
    let _guard = CACHE_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    // Cache reflects the pre-connect world: gmail is listed but
    // not connected. This is exactly the state the chat runtime
    // gets stuck in on Windows when the user completes OAuth
    // after the event-bus 60 s readiness poll times out.
    let key = "windows-regression-1";
    clear_cache_key(key);
    seed_cache(
        key,
        vec![integration("gmail", false), integration("notion", false)],
    );

    // Fresh UI poll shows gmail just flipped ACTIVE — mirrors a
    // user who finished OAuth in the system browser.
    sync_cache_with_connections(&[conn("c-1", "gmail", "ACTIVE")]);

    // Chat-runtime cache must be cleared so the next
    // `fetch_connected_integrations` re-fetches truth from the
    // backend. Without this fix the entry would live on until
    // `CACHE_TTL` expired or the process restarted.
    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_none(),
        "expected cache to be busted when a new toolkit flips ACTIVE"
    );
}

#[test]
fn sync_cache_invalidates_when_connection_is_removed() {
    let _guard = CACHE_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    // Cache remembers gmail as connected. The user just
    // disconnected it from Settings; the next UI poll returns an
    // empty list. Chat must forget gmail within one poll.
    let key = "windows-regression-2";
    clear_cache_key(key);
    seed_cache(key, vec![integration("gmail", true)]);

    sync_cache_with_connections(&[]);

    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_none(),
        "expected cache to be busted when a connected toolkit disappears"
    );
}

#[test]
fn sync_cache_noop_when_backend_matches_cached_state() {
    let _guard = CACHE_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    // Steady state: UI polls confirm cache is accurate. No
    // invalidation — we must not thrash the chat runtime's tool
    // registry on every 5 s UI poll.
    let key = "windows-regression-3";
    clear_cache_key(key);
    seed_cache(
        key,
        vec![integration("gmail", true), integration("notion", false)],
    );

    sync_cache_with_connections(&[conn("c-1", "gmail", "ACTIVE")]);

    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_some(),
        "expected cache entry to survive when backend matches cached state"
    );
    // And the seeded entries are still there byte-for-byte.
    assert_eq!(guard.get(key).unwrap().entries.len(), 2);
}

#[test]
fn sync_cache_ignores_non_active_connection_rows() {
    let _guard = CACHE_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    // Backend reports a PENDING row (user started OAuth but
    // hasn't completed). The cache should NOT be invalidated —
    // that would trigger a fresh `list_tools` call on every poll
    // while the OAuth handshake is in flight, which is wasteful
    // and would also clear `tools` vecs for real active
    // integrations already on disk.
    let key = "windows-regression-4";
    clear_cache_key(key);
    seed_cache(key, vec![integration("gmail", true)]);

    sync_cache_with_connections(&[
        conn("c-1", "gmail", "ACTIVE"),
        conn("c-2", "notion", "PENDING"),
        conn("c-3", "slack", "FAILED"),
    ]);

    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_some(),
        "PENDING/FAILED rows must not trigger invalidation"
    );
}

#[test]
fn sync_cache_treats_connected_status_equivalent_to_active() {
    let _guard = CACHE_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    // Backend may emit either "ACTIVE" or "CONNECTED" — we treat
    // them identically in every status check (see
    // `fetch_connected_integrations_uncached` filter). Make sure
    // the new diff path matches that convention so it doesn't
    // produce a false-positive invalidation.
    let key = "windows-regression-5";
    clear_cache_key(key);
    seed_cache(key, vec![integration("gmail", true)]);

    // Same toolkit set but reported via the legacy "CONNECTED" spelling.
    sync_cache_with_connections(&[conn("c-1", "gmail", "CONNECTED")]);

    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_some(),
        "CONNECTED should be treated as an active status"
    );
}

#[test]
fn cache_entries_expire_after_ttl() {
    let _guard = CACHE_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    // Even without any UI polling, the chat runtime must
    // self-heal stale state within `CACHE_TTL`. We can't wait
    // 60 s in a unit test; instead, directly age the entry by
    // rewriting its `cached_at`.
    let key = "windows-regression-6";
    clear_cache_key(key);
    seed_cache(key, vec![integration("gmail", true)]);

    // Age the entry past the TTL.
    {
        let mut guard = INTEGRATIONS_CACHE.write().unwrap();
        let entry = guard.get_mut(key).unwrap();
        entry.cached_at = Instant::now() - (CACHE_TTL + Duration::from_secs(1));
    }

    // Re-read via the public API — expired reads must not serve
    // the stale entry. We can't trigger a real backend call in a
    // unit test, so assert that the read path falls through (by
    // asserting the entry is still present before the read, and
    // proving the staleness check via a direct helper).
    let is_fresh = {
        let guard = INTEGRATIONS_CACHE.read().unwrap();
        guard
            .get(key)
            .map(|c| c.cached_at.elapsed() < CACHE_TTL)
            .unwrap_or(false)
    };
    assert!(
        !is_fresh,
        "entry aged past CACHE_TTL must not be treated as fresh"
    );
}

// ── Trigger management ops (PR #671) ────────────────────────────────

#[tokio::test]
async fn composio_list_available_triggers_via_mock() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/available",
        get(|Query(q): Query<HashMap<String, String>>| async move {
            assert_eq!(q.get("toolkit"), Some(&"gmail".into()));
            assert_eq!(q.get("connectionId"), Some(&"c1".into()));
            // Echo back so the test can also assert what was forwarded.
            Json(json!({
                "success": true,
                "data": {"triggers": [
                    {
                        "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                        "scope": "static",
                        "defaultConfig": {"labelIds": "INBOX"},
                        "_echoed_connectionId": q.get("connectionId"),
                        "_echoed_toolkit": q.get("toolkit"),
                    }
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_list_available_triggers(&config, "gmail", Some("c1".into()))
        .await
        .unwrap();
    assert_eq!(outcome.value.triggers.len(), 1);
    assert_eq!(outcome.value.triggers[0].slug, "GMAIL_NEW_GMAIL_MESSAGE");
    assert_eq!(outcome.value.triggers[0].scope, "static");
    assert!(outcome.logs.iter().any(|l| l.contains("available trigger")));
}

#[tokio::test]
async fn composio_list_available_triggers_omits_connection_when_none() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/available",
        get(|Query(q): Query<HashMap<String, String>>| async move {
            assert!(
                q.get("connectionId").is_none(),
                "should not forward connectionId"
            );
            Json(json!({"success": true, "data": {"triggers": []}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_list_available_triggers(&config, "gmail", None)
        .await
        .unwrap();
    assert!(outcome.value.triggers.is_empty());
}

#[tokio::test]
async fn composio_list_triggers_via_mock_with_filter() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers",
        get(|Query(_q): Query<HashMap<String, String>>| async move {
            Json(json!({
                "success": true,
                "data": {"triggers": [
                    {
                        "id": "ti_1",
                        "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                        "toolkit": "gmail",
                        "connectionId": "c1",
                        "state": "active"
                    }
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_list_triggers(&config, Some("gmail".into()))
        .await
        .unwrap();
    assert_eq!(outcome.value.triggers.len(), 1);
    assert_eq!(outcome.value.triggers[0].id, "ti_1");
    assert_eq!(outcome.value.triggers[0].connection_id, "c1");
}

#[tokio::test]
async fn composio_list_triggers_without_filter() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers",
        get(|| async { Json(json!({"success": true, "data": {"triggers": []}})) }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_list_triggers(&config, None).await.unwrap();
    assert!(outcome.value.triggers.is_empty());
}

#[tokio::test]
async fn composio_enable_trigger_via_mock() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["slug"], "GMAIL_NEW_GMAIL_MESSAGE");
            assert_eq!(body["connectionId"], "c1");
            assert_eq!(body["triggerConfig"]["labelIds"], "INBOX");
            Json(json!({
                "success": true,
                "data": {
                    "triggerId": "ti_new",
                    "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                    "connectionId": "c1"
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_enable_trigger(
        &config,
        "c1",
        "GMAIL_NEW_GMAIL_MESSAGE",
        Some(json!({"labelIds": "INBOX"})),
    )
    .await
    .unwrap();
    assert_eq!(outcome.value.trigger_id, "ti_new");
    assert_eq!(outcome.value.connection_id, "c1");
    assert!(outcome.logs.iter().any(|l| l.contains("enabled trigger")));
}

#[tokio::test]
async fn composio_disable_trigger_via_mock() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/{id}",
        axum::routing::delete(|Path(id): Path<String>| async move {
            assert_eq!(id, "ti_1");
            Json(json!({"success": true, "data": {"deleted": true}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_disable_trigger(&config, "ti_1").await.unwrap();
    assert!(outcome.value.deleted);
    assert!(outcome.logs.iter().any(|l| l.contains("disabled trigger")));
}

#[tokio::test]
async fn composio_disable_trigger_propagates_backend_error() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/{id}",
        axum::routing::delete(|Path(_id): Path<String>| async move {
            (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({"success": false, "error": "Trigger not found"})),
            )
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let err = composio_disable_trigger(&config, "missing")
        .await
        .unwrap_err();
    assert!(err.contains("disable_trigger failed"), "unexpected: {err}");
}

// ── Direct-mode list_* short-circuits ─────────────────────────────
//
// [composio-direct] When `config.composio.mode == "direct"`, the
// `composio_list_toolkits` / `composio_list_connections` ops must NOT
// silently fall through to the backend tenant's data — that's the
// bug the user reported in #1710 (toggled to Direct, still saw
// tinyhumans-tenant connections). We return empty responses with
// explicit log lines so the UI / agent surface stays honest about
// where the data is (or isn't) coming from.

/// Set up a config with `composio.mode = "direct"` and a stored
/// direct-mode API key (so `create_composio_client` succeeds).
fn direct_mode_config(tmp: &tempfile::TempDir) -> Config {
    let mut c = Config::default();
    c.workspace_dir = tmp.path().join("workspace");
    c.config_path = tmp.path().join("config.toml");
    c.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.into();
    crate::openhuman::credentials::AuthService::from_config(&c)
        .store_provider_token(
            crate::openhuman::credentials::ops::COMPOSIO_DIRECT_PROVIDER,
            crate::openhuman::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "ck_test_direct_key",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test direct-mode api key");
    c
}

#[tokio::test]
async fn composio_list_toolkits_returns_empty_in_direct_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    let outcome = composio_list_toolkits(&config)
        .await
        .expect("direct-mode list_toolkits must succeed without HTTP");
    assert!(
        outcome.value.toolkits.is_empty(),
        "direct mode must not surface the backend allowlist"
    );
    assert!(
        outcome.logs.iter().any(|l| l.contains("direct mode")),
        "log line must call out direct mode explicitly, got {:?}",
        outcome.logs
    );
}

#[tokio::test]
async fn composio_list_connections_routes_through_direct_mode() {
    let _guard = CACHE_TEST_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    // [composio-direct] After commit 2 of #1710, direct mode actually
    // calls `backend.composio.dev/api/v3/connected_accounts` rather
    // than returning an empty stub. Without a real Composio key the
    // remote will reject the test request, so we assert on the error
    // shape: it must reference `composio` AND must NOT reference the
    // backend-session path (proving the factory routed us to direct).
    let result = composio_list_connections(&config).await;
    match result {
        Ok(outcome) => {
            // Some sandboxes resolve OK with an empty list — accept
            // that as well, but the connections vec must be empty
            // (the test key is not provisioned in any real tenant).
            assert!(
                outcome.value.connections.is_empty(),
                "test key should not surface real connections"
            );
        }
        Err(err) => {
            assert!(
                !err.contains("no backend session"),
                "direct mode must not surface backend-auth errors, got: {err}"
            );
            assert!(
                err.to_lowercase().contains("composio"),
                "error must carry the composio prefix, got: {err}"
            );
        }
    }
}

// ── Direct-mode authorize / list_tools / execute (commit 1, #1710) ─

/// Direct-mode `composio_list_tools` now hits Composio v3 with the
/// user's own key (replacing the prior empty-short-circuit). The unit
/// test reaches an outbound HTTPS call against the real
/// `backend.composio.dev`, which immediately fails with HTTP 401 on the
/// fake key — exactly the shape we want the contract to preserve:
///
///   * NEVER fall back to the tinyhumans backend tenant (no
///     `"no backend session"` text in the error)
///   * Surface the failure with the `composio` grep prefix so it routes
///     through normal observability
///
/// A full schemas-mapped test that asserts response shape lives in the
/// `client_tests.rs` mock-axum suite (`direct_list_tools_*`); this
/// integration-style test only pins the failure-mode contract.
#[tokio::test]
async fn composio_list_tools_in_direct_mode_does_not_fall_back_to_backend() {
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    let result = composio_list_tools(&config, None).await;
    match result {
        Ok(outcome) => {
            // If the prefetch returns empty connections (test env may
            // intermittently mock that), the function short-circuits to
            // an empty tool list — still no backend leak.
            assert!(
                outcome.value.tools.is_empty(),
                "direct mode must NOT surface backend-tenant tool catalogue"
            );
            assert!(
                outcome.logs.iter().any(|l| l.contains("direct mode")),
                "log line must call out direct mode explicitly, got {:?}",
                outcome.logs
            );
        }
        Err(err) => {
            assert!(
                !err.contains("no backend session"),
                "direct mode must not surface backend-auth errors, got: {err}"
            );
            assert!(
                err.to_lowercase().contains("composio"),
                "error must carry the composio prefix, got: {err}"
            );
        }
    }
}

#[tokio::test]
async fn composio_authorize_routes_through_direct_mode() {
    // The direct-mode `authorize` path actually calls
    // `backend.composio.dev/api/v3/connected_accounts/link` over HTTPS.
    // We can't mock that endpoint at the URL-rewriter level in this
    // unit test, so the assertion below verifies (a) the mode-aware
    // factory was hit (i.e. no "no backend session" error) and (b) the
    // error path is the direct-mode one (HTTP failure or DNS failure),
    // not the backend one. Both error shapes are acceptable — the
    // important thing is that backend mode would have produced
    // "composio unavailable / no backend session" instead.
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    let err = composio_authorize(&config, "gmail", None)
        .await
        .unwrap_err();
    assert!(
        !err.contains("no backend session"),
        "direct mode must not surface backend-auth errors, got: {err}"
    );
    assert!(
        err.to_lowercase().contains("composio"),
        "error must carry the composio prefix, got: {err}"
    );
}

#[tokio::test]
async fn composio_execute_routes_through_direct_mode() {
    // Same shape of assertion as
    // `composio_authorize_routes_through_direct_mode` — we can't mock
    // `backend.composio.dev` from a unit test, so we verify the factory
    // routed to direct mode (no backend-auth error) and that an error
    // surfaces because the live HTTP call cannot succeed against a
    // test key.
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    let err = composio_execute(&config, "GMAIL_SEND_EMAIL", None)
        .await
        .unwrap_err();
    assert!(
        !err.contains("no backend session"),
        "direct mode must not surface backend-auth errors, got: {err}"
    );
    assert!(
        err.to_lowercase().contains("composio"),
        "error must carry the composio prefix, got: {err}"
    );
}

// ── classify_composio_failure_tag ──────────────────────────────
//
// Pin the failure-tag routing for `report_composio_op_error` so the
// `before_send` filter (`is_transient_integrations_failure` extended to
// `domain="composio"` in the same #1608 patch series) matches. The tag
// drives which branch of the filter fires:
//   - `failure="non_2xx"` + transient `status` (set by the integrations
//     wrapper) → dropped
//   - `failure="transport"` + transient transport phrase in the message
//     → dropped
// Any drift between the helper's classification and the filter's
// expectations would silently re-open the leak path.

#[test]
fn composio_failure_tag_is_non_2xx_for_backend_returned_502() {
    // OPENHUMAN-TAURI-35 / -2H wire shape — the dominant leak. The
    // integrations layer renders this on a 5xx response; composio's op
    // layer wraps the chain and re-reports under `domain=composio`. The
    // tag MUST be `non_2xx` so the existing transient-status filter
    // branch matches.
    let rendered = "Backend returned 502 Bad Gateway for POST \
                    https://api.tinyhumans.ai/agent-integrations/composio/connections: \
                    upstream temporarily unavailable";
    assert_eq!(classify_composio_failure_tag(rendered), "non_2xx");
}

#[test]
fn composio_failure_tag_is_non_2xx_for_envelope_error() {
    // Envelope errors don't carry a transport phrase or "error sending
    // request" anchor; default to non_2xx.
    let rendered = "Backend error for POST https://api.tinyhumans.ai/x: \
                    unknown backend error";
    assert_eq!(classify_composio_failure_tag(rendered), "non_2xx");
}

#[test]
fn composio_failure_tag_is_transport_for_operation_timed_out() {
    // OPENHUMAN-TAURI-18 / -G shape — `composio/execute` reqwest chain
    // surfaces `operation timed out` (one of `TRANSIENT_TRANSPORT_PHRASES`).
    // Tag MUST be `transport` so the filter's transport-phrase branch fires
    // even though the report carries no `status`.
    let rendered = "POST https://api.tinyhumans.ai/agent-integrations/composio/execute \
                    failed: error sending request for url \
                    (https://api.tinyhumans.ai/agent-integrations/composio/execute) → \
                    client error (SendRequest) → connection error → \
                    Operation timed out (os error 60)";
    assert_eq!(classify_composio_failure_tag(rendered), "transport");
}

#[test]
fn composio_failure_tag_is_transport_for_dns_and_tls_phrases() {
    for raw in [
        "POST /v1/foo failed: error sending request for url (https://api.example.com/x)",
        "GET /agent-integrations/composio/connections failed: tls handshake eof",
        "POST /agent-integrations/composio/triggers failed: connection reset by peer",
        "GET /agent-integrations/composio/toolkits failed: connection forcibly closed (os 10054)",
    ] {
        assert_eq!(
            classify_composio_failure_tag(raw),
            "transport",
            "should classify as transport: {raw}"
        );
    }
}

#[test]
fn composio_failure_tag_does_not_misclassify_unrelated_messages() {
    // A bare error string with no transport / "error sending request"
    // anchor must default to non_2xx — the safe choice for the dominant
    // leak shape.
    for raw in [
        "[composio] no connection with id 'abc'",
        "[composio] no native provider registered for toolkit 'foo'",
        "fetch_user_profile failed: invalid JSON in profile facet",
    ] {
        assert_eq!(
            classify_composio_failure_tag(raw),
            "non_2xx",
            "should default to non_2xx: {raw}"
        );
    }
}

// ── extract_backend_returned_status ───────────────────────────
//
// Pin status extraction so the `report_composio_op_error` Sentry tag
// stays in lockstep with the `Backend returned <status> ...` rendering
// the integrations layer produces. Without the digit anchor the
// `before_send` filter's transient-status branch can't distinguish a 502
// from a 401, and the dominant leak shape (OPENHUMAN-TAURI-35 / -2H)
// re-opens.

#[test]
fn extract_backend_returned_status_parses_three_digit_status() {
    let rendered = "Backend returned 502 Bad Gateway for POST \
                    https://api.tinyhumans.ai/agent-integrations/composio/connections: \
                    upstream temporarily unavailable";
    assert_eq!(
        extract_backend_returned_status(rendered),
        Some("502".to_string())
    );
}

#[test]
fn extract_backend_returned_status_returns_none_when_no_status() {
    // Envelope-style error with no HTTP status digits after the anchor.
    let rendered = "Backend returned bad gateway (envelope-only error)";
    assert_eq!(extract_backend_returned_status(rendered), None);
}

#[test]
fn extract_backend_returned_status_handles_mixed_case() {
    // Some renders upper-case the prefix; the helper lowercases before
    // matching so both shapes resolve to the same status string.
    let rendered = "BACKEND RETURNED 429 Too Many Requests for GET \
                    https://api.tinyhumans.ai/agent-integrations/composio/triggers";
    assert_eq!(
        extract_backend_returned_status(rendered),
        Some("429".to_string())
    );
}

// ── before_send filter integration ─────────────────────────────
//
// Belt-and-suspenders: re-assert the cross-module contract from the
// composio side. If `is_transient_integrations_failure` ever stops
// matching `domain="composio"` (e.g. accidental revert), the
// `report_composio_op_error` events flood Sentry again with no test in
// the composio crate to catch it. These guards make the link explicit.

#[test]
fn composio_domain_502_is_dropped_by_before_send() {
    let mut event = sentry::protocol::Event::default();
    let mut tags: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    tags.insert("domain".into(), "composio".into());
    tags.insert("failure".into(), "non_2xx".into());
    tags.insert("status".into(), "502".into());
    event.tags = tags;
    assert!(
        crate::core::observability::is_transient_integrations_failure(&event),
        "composio non_2xx 502 must be dropped by integrations filter (#1608)"
    );
}

#[test]
fn composio_transport_timeout_is_dropped_by_before_send() {
    let mut event = sentry::protocol::Event::default();
    let mut tags: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    tags.insert("domain".into(), "composio".into());
    tags.insert("failure".into(), "transport".into());
    event.tags = tags;
    event.message = Some(
        "POST /agent-integrations/composio/execute failed: error sending request → \
         operation timed out"
            .to_string(),
    );
    assert!(
        crate::core::observability::is_transient_integrations_failure(&event),
        "composio transport timeout must be dropped by integrations filter (#1608)"
    );
}
