//! JSON-RPC E2E cases for the auth security namespace.

#[path = "support/rpc_e2e_harness.rs"]
mod harness;
use harness::*;

#[tokio::test]
async fn notification_settings_roundtrip_and_disabled_ingest_skip() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let set = post_json_rpc(
        &rpc_base,
        4001,
        "openhuman.notification_settings_set",
        json!({
            "provider": "gmail",
            "enabled": false,
            "importance_threshold": 0.8,
            "route_to_orchestrator": false
        }),
    )
    .await;
    let set_result = assert_no_jsonrpc_error(&set, "notification_settings_set");
    assert_eq!(set_result.get("ok").and_then(Value::as_bool), Some(true));

    let get = post_json_rpc(
        &rpc_base,
        4002,
        "openhuman.notification_settings_get",
        json!({ "provider": "gmail" }),
    )
    .await;
    let get_result = assert_no_jsonrpc_error(&get, "notification_settings_get");
    let settings = get_result.get("settings").expect("settings object");
    assert_eq!(
        settings.get("enabled").and_then(Value::as_bool),
        Some(false)
    );
    let threshold = settings
        .get("importance_threshold")
        .and_then(Value::as_f64)
        .unwrap_or_default();
    assert!(
        (threshold - 0.8).abs() < 0.0001,
        "expected threshold ~= 0.8, got {threshold}"
    );
    assert_eq!(
        settings
            .get("route_to_orchestrator")
            .and_then(Value::as_bool),
        Some(false)
    );

    let ingest = post_json_rpc(
        &rpc_base,
        4003,
        "openhuman.notification_ingest",
        json!({
            "provider": "gmail",
            "account_id": "acct-1",
            "title": "subject",
            "body": "body",
            "raw_payload": { "source": "test" }
        }),
    )
    .await;
    let ingest_result = assert_no_jsonrpc_error(&ingest, "notification_ingest");
    assert_eq!(
        ingest_result.get("skipped").and_then(Value::as_bool),
        Some(true)
    );

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn credentials_crud_roundtrip() {
    // Tests the provider-credential lifecycle over the JSON-RPC transport:
    //   store → list → list-filtered → remove → verify-gone
    //
    // Provider credentials are stored locally (auth-profiles.json) and require
    // no upstream network calls, so no mock session/JWT is needed.
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    // A mock upstream is required so config validation passes and api_url is
    // well-formed, even though provider-credential calls don't hit the network.
    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── 1. store a provider credential ──────────────────────────────────────
    let store = post_json_rpc(
        &rpc_base,
        5001,
        "openhuman.auth_store_provider_credentials",
        json!({
            "provider": "openai",
            "profile": "default",
            "token": "sk-e2e-test-key",
            "setActive": true
        }),
    )
    .await;
    // assert_no_jsonrpc_error returns the JSON-RPC `result` field which is the
    // Outcome envelope: {"logs": [...], "result": { <AuthProfileSummary> }}.
    let store_outer = assert_no_jsonrpc_error(&store, "auth_store_provider_credentials");
    let store_result = store_outer.get("result").unwrap_or(store_outer);
    assert_eq!(
        store_result.get("provider").and_then(Value::as_str),
        Some("openai"),
        "stored profile should have provider=openai: {store_result}"
    );
    assert_eq!(
        store_result.get("profileName").and_then(Value::as_str),
        Some("default"),
        "stored profile should have profileName=default: {store_result}"
    );
    assert_eq!(
        store_result.get("hasToken").and_then(Value::as_bool),
        Some(true),
        "stored profile should report hasToken=true: {store_result}"
    );

    // ── 2. list all provider credentials — should find openai ───────────────
    let list_all = post_json_rpc(
        &rpc_base,
        5002,
        "openhuman.auth_list_provider_credentials",
        json!({}),
    )
    .await;
    let list_outer = assert_no_jsonrpc_error(&list_all, "auth_list_provider_credentials (all)");
    let list_result = list_outer.get("result").unwrap_or(list_outer);
    let profiles = list_result
        .as_array()
        .unwrap_or_else(|| panic!("expected array from list: {list_result}"));
    assert_eq!(profiles.len(), 1, "expected exactly one stored credential");
    assert_eq!(
        profiles[0].get("provider").and_then(Value::as_str),
        Some("openai")
    );

    // ── 3. list filtered by provider name ───────────────────────────────────
    let list_filtered = post_json_rpc(
        &rpc_base,
        5003,
        "openhuman.auth_list_provider_credentials",
        json!({ "provider": "openai" }),
    )
    .await;
    let filtered_outer =
        assert_no_jsonrpc_error(&list_filtered, "auth_list_provider_credentials (filtered)");
    let filtered_result = filtered_outer.get("result").unwrap_or(filtered_outer);
    let filtered_profiles = filtered_result
        .as_array()
        .unwrap_or_else(|| panic!("expected array from filtered list: {filtered_result}"));
    assert_eq!(
        filtered_profiles.len(),
        1,
        "filter by openai should return exactly one entry"
    );

    // ── 4. remove the stored credential ─────────────────────────────────────
    let remove = post_json_rpc(
        &rpc_base,
        5004,
        "openhuman.auth_remove_provider_credentials",
        json!({
            "provider": "openai",
            "profile": "default"
        }),
    )
    .await;
    let remove_outer = assert_no_jsonrpc_error(&remove, "auth_remove_provider_credentials");
    let remove_result = remove_outer.get("result").unwrap_or(remove_outer);
    assert_eq!(
        remove_result.get("removed").and_then(Value::as_bool),
        Some(true),
        "remove should report removed=true: {remove_result}"
    );

    // ── 5. verify the credential is gone ────────────────────────────────────
    let list_after = post_json_rpc(
        &rpc_base,
        5005,
        "openhuman.auth_list_provider_credentials",
        json!({}),
    )
    .await;
    let after_outer =
        assert_no_jsonrpc_error(&list_after, "auth_list_provider_credentials (after remove)");
    let after_result = after_outer.get("result").unwrap_or(after_outer);
    let after_profiles = after_result
        .as_array()
        .unwrap_or_else(|| panic!("expected array after remove: {after_result}"));
    assert!(
        after_profiles.is_empty(),
        "credentials list should be empty after remove, got {after_profiles:?}"
    );

    mock_join.abort();
    rpc_join.abort();
}

/// End-to-end coverage for `openhuman.skills_uninstall`.
///
/// Validates that the RPC method is registered, wire-decodes
/// `UninstallSkillParams`, resolves the slug against
/// `~/.openhuman/skills/<slug>/`, removes the directory on success, and
/// forwards the core error message verbatim for the two documented
/// failure modes (missing SKILL.md and path traversal). Previously only
/// the `uninstall_skill(...)` helper was tested — the wire layer
/// (controller registration, param decoding, response shape) was not.

#[tokio::test]
async fn rpc_rejects_unauthenticated_request() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    ensure_test_rpc_auth();

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("http://{rpc_addr}/rpc"))
        .header("Content-Type", "application/json")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"core.ping","params":{}}"#)
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 401, "missing Authorization must yield 401");
    let body: Value = resp.json().await.expect("json body");
    assert_eq!(
        body["error"], "unauthorized",
        "error field must be 'unauthorized'"
    );

    rpc_join.abort();
}

/// POST /rpc with a syntactically valid but wrong bearer token → 401.

#[tokio::test]
async fn rpc_rejects_wrong_token() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    ensure_test_rpc_auth();

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("http://{rpc_addr}/rpc"))
        .header(
            AUTHORIZATION,
            "Bearer deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        )
        .header("Content-Type", "application/json")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"core.ping","params":{}}"#)
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 401, "wrong token must yield 401");
    let body: Value = resp.json().await.expect("json body");
    assert_eq!(body["error"], "unauthorized");

    rpc_join.abort();
}

/// Every path in PUBLIC_PATHS must bypass the auth middleware — i.e. never
/// return 401 — even without an Authorization header.  Some paths return
/// non-2xx for other reasons (missing query params, no WebSocket upgrade
/// headers) so the assertion is `!= 401`, not `.is_success()`.

#[tokio::test]
async fn public_paths_accessible_without_token() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    ensure_test_rpc_auth();

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let client = reqwest::Client::new();
    let base = format!("http://{rpc_addr}");

    // Paths that return 200 without any extra params.
    // `/events/webhooks` was REMOVED from this list when issue #1922 wired
    // bearer auth onto it (header or `?token=…`). Coverage for that path
    // lives in the dedicated `webhook_sse_*` tests below.
    for path in ["/", "/health", "/schema"] {
        let resp = client
            .get(format!("{base}{path}"))
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert!(
            resp.status().is_success(),
            "public path {path} must return 2xx without auth, got {}",
            resp.status()
        );
    }

    // Paths that bypass the *middleware* header check but return non-2xx for
    // unrelated reasons (missing required query params, no WebSocket upgrade
    // headers, etc.). The invariant here is that the auth middleware does NOT
    // reject them with 401.
    //
    // NOTE: `/events` enforces its own credential inside the handler, so an
    // unauthenticated GET returns 401 from the HANDLER (not the middleware).
    // It requires a `client_id` query param, so an empty request is rejected
    // by Axum's extractor (400) before the handler's own auth runs.
    for path in ["/events"] {
        let resp = client
            .get(format!("{base}{path}"))
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert_ne!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "public path {path} must not be auth-gated by the middleware (got {})",
            resp.status()
        );
    }

    // The removed dictation socket must fall through to the normal 404.
    let resp = client
        .get(format!("{base}/ws/dictation"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET /ws/dictation: {e}"));
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "removed /ws/dictation endpoint must return 404 (got {})",
        resp.status()
    );

    rpc_join.abort();
}

// ---------------------------------------------------------------------------
// Webhook SSE auth (issue #1922) — /events/webhooks now requires bearer auth
// via either the Authorization header OR a `?token=…` query param. The
// query-param fallback exists because browser `EventSource` cannot attach
// custom headers (whatwg/html §10.7). See `QUERY_TOKEN_PATHS` in
// crates/openhuman-core/src/core/auth.rs.
// ---------------------------------------------------------------------------

/// GET /events/webhooks with neither header nor query token → 401.

#[tokio::test]
async fn webhook_sse_rejects_unauthenticated() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    ensure_test_rpc_auth();

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{rpc_addr}/events/webhooks"))
        .send()
        .await
        .expect("request");

    assert_eq!(
        resp.status(),
        401,
        "missing credentials on /events/webhooks must yield 401"
    );
    let body: Value = resp.json().await.expect("json body");
    assert_eq!(body["error"], "unauthorized");

    rpc_join.abort();
}

/// GET /events/webhooks?token= (empty value) → 401. Defends against
/// `encodeURIComponent(null)` / `encodeURIComponent("")` mishaps on the FE.

#[tokio::test]
async fn webhook_sse_rejects_empty_query_token() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    ensure_test_rpc_auth();

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{rpc_addr}/events/webhooks?token="))
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 401, "empty token value must be 401");

    rpc_join.abort();
}

/// GET /events/webhooks?token=garbage → 401.

#[tokio::test]
async fn webhook_sse_rejects_wrong_query_token() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    ensure_test_rpc_auth();

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let client = reqwest::Client::new();

    let bad = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    assert_ne!(bad, TEST_RPC_TOKEN);

    let resp = client
        .get(format!("http://{rpc_addr}/events/webhooks?token={bad}"))
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 401, "wrong query token must be 401");
    let body: Value = resp.json().await.expect("json body");
    assert_eq!(body["error"], "unauthorized");

    rpc_join.abort();
}

/// GET /events/webhooks?token=<valid> → 200 (SSE stream opens).

#[tokio::test]
async fn webhook_sse_accepts_valid_query_token() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    ensure_test_rpc_auth();

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!(
            "http://{rpc_addr}/events/webhooks?token={TEST_RPC_TOKEN}"
        ))
        .send()
        .await
        .expect("request");

    assert_eq!(
        resp.status(),
        200,
        "valid query token must open the SSE stream"
    );
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("text/event-stream"),
        "expected SSE content-type, got {ct}"
    );

    rpc_join.abort();
}

/// GET /events/webhooks with a percent-encoded query token → 200. Locks the
/// URL-decoding contract `EventSource` callers depend on (the FE uses
/// `encodeURIComponent`, which percent-encodes reserved characters even
/// when the token itself is hex-only).

#[tokio::test]
async fn webhook_sse_accepts_percent_encoded_query_token() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    ensure_test_rpc_auth();

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let client = reqwest::Client::new();

    let encoded = urlencoding::encode(TEST_RPC_TOKEN);
    // Sanity: encoding the canonical hex test token must remain a non-empty
    // string. The encoder may pass-through (hex is URL-safe) — that's fine;
    // the test still proves the decode path doesn't double-decode or strip.
    assert!(!encoded.is_empty());

    let resp = client
        .get(format!("http://{rpc_addr}/events/webhooks?token={encoded}"))
        .send()
        .await
        .expect("request");

    assert_eq!(
        resp.status(),
        200,
        "url-encoded valid query token must open the SSE stream"
    );
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("text/event-stream"),
        "expected SSE content-type, got {ct}"
    );

    rpc_join.abort();
}

/// GET /events/webhooks with `Authorization: Bearer <valid>` → 200.
/// CLI / non-browser callers should still be able to subscribe the header way.

#[tokio::test]
async fn webhook_sse_accepts_valid_bearer_header() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    ensure_test_rpc_auth();

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://{rpc_addr}/events/webhooks"))
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
        .send()
        .await
        .expect("request");

    assert_eq!(
        resp.status(),
        200,
        "valid Bearer header must open the SSE stream"
    );

    rpc_join.abort();
}

/// Simulate an external process using a guessed token — must be rejected.

#[tokio::test]
async fn external_process_with_guessed_token_is_rejected() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    ensure_test_rpc_auth(); // server validates against TEST_RPC_TOKEN

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let client = reqwest::Client::new();

    // An attacker process trying a plausible-looking token that isn't the real one.
    let attacker_token = "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
    assert_ne!(
        attacker_token, TEST_RPC_TOKEN,
        "attacker token must differ from real one"
    );

    let resp = client
        .post(format!("http://{rpc_addr}/rpc"))
        .header(AUTHORIZATION, format!("Bearer {attacker_token}"))
        .header("Content-Type", "application/json")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"core.ping","params":{}}"#)
        .send()
        .await
        .expect("request");

    assert_eq!(
        resp.status(),
        401,
        "external process with wrong token must be rejected"
    );

    rpc_join.abort();
}

#[tokio::test]
async fn rpc_update_apply_can_be_disabled_by_config_policy() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    ensure_test_rpc_auth();

    let tmp = tempdir().expect("tempdir");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());

    let mut config = openhuman_core::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..openhuman_core::config::Config::default()
    };
    config.update.rpc_mutations_enabled = false;
    config
        .save()
        .await
        .expect("persist config with update rpc disabled");

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("http://{rpc_addr}/rpc"))
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
        .header("Content-Type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "openhuman.update_apply",
            "params": {
                "download_url": "https://github.com/owner/repo/releases/download/v1/x",
                "asset_name": "openhuman-core-x86_64-unknown-linux-gnu"
            }
        }))
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.expect("json body");
    let error_msg = body
        .get("result")
        .and_then(|outer| outer.get("result"))
        .and_then(|inner| inner.get("error"))
        .and_then(Value::as_str)
        .expect("policy error result message");
    assert!(
        error_msg.contains("rpc_mutations_enabled=false"),
        "unexpected error: {body}"
    );

    rpc_join.abort();
}

/// End-to-end coverage for issue #1149: storing a managed-DM channel
/// credential under `channel:<slug>:<mode>` and immediately observing
/// `connected:true` from `openhuman.channels_status`.
///
/// Before the fix, `channels_status` always returned `connected:false`
/// because the underlying `list_provider_credentials` call used an
/// exact-match filter (`provider == "channel:"`) that never matched
/// the real credential keys (`channel:telegram:managed_dm`,
/// `channel:slack:bot_token`, …). The user could connect Telegram in
/// the UI but the chat / Settings page would still report it
/// disconnected on the next reload.
///
/// This test exercises the full RPC wire path so a regression in
/// either the prefix helper or the channels controller is caught at
/// the transport layer, not just at the unit level.

#[tokio::test]
async fn channels_status_reflects_managed_dm_credential_e2e() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── 1. baseline: telegram should report disconnected ────────────────────
    let baseline = post_json_rpc(
        &rpc_base,
        7001,
        "openhuman.channels_status",
        json!({ "channel": "telegram" }),
    )
    .await;
    let baseline_outer = assert_no_jsonrpc_error(&baseline, "channels_status (baseline)");
    let baseline_result = baseline_outer.get("result").unwrap_or(baseline_outer);
    let baseline_entries = baseline_result
        .as_array()
        .unwrap_or_else(|| panic!("expected array: {baseline_result}"));
    let baseline_managed = baseline_entries
        .iter()
        .find(|e| e.get("auth_mode").and_then(Value::as_str) == Some("managed_dm"))
        .expect("managed_dm entry should exist for telegram");
    assert_eq!(
        baseline_managed.get("connected").and_then(Value::as_bool),
        Some(false),
        "fresh config should report telegram managed_dm disconnected: {baseline_managed}"
    );

    // ── 2. simulate a successful managed-DM link by storing the credential
    //      marker the way `telegram_login_check` does in production ─────────
    let store = post_json_rpc(
        &rpc_base,
        7002,
        "openhuman.auth_store_provider_credentials",
        json!({
            "provider": "channel:telegram:managed_dm",
            "profile": "default",
            "token": "managed",
            "fields": { "linked": true },
            "setActive": true,
        }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "auth_store_provider_credentials");

    // ── 3. channels_status must now report telegram managed_dm connected ─
    let after = post_json_rpc(
        &rpc_base,
        7003,
        "openhuman.channels_status",
        json!({ "channel": "telegram" }),
    )
    .await;
    let after_outer = assert_no_jsonrpc_error(&after, "channels_status (after link)");
    let after_result = after_outer.get("result").unwrap_or(after_outer);
    let after_entries = after_result
        .as_array()
        .unwrap_or_else(|| panic!("expected array: {after_result}"));
    let after_managed = after_entries
        .iter()
        .find(|e| e.get("auth_mode").and_then(Value::as_str) == Some("managed_dm"))
        .expect("managed_dm entry should exist for telegram");
    assert_eq!(
        after_managed.get("connected").and_then(Value::as_bool),
        Some(true),
        "managed-DM credential should surface as connected: {after_managed}"
    );
    assert_eq!(
        after_managed
            .get("has_credentials")
            .and_then(Value::as_bool),
        Some(true)
    );

    mock_join.abort();
    rpc_join.abort();
}

/// Exercise the Telegram bot-token lifecycle through the host JSON-RPC wire
/// using the registry-pinned TinyChannels release staged by the standard Rust
/// test runner. Telegram credential validation is local to the module and
/// does not call the Bot API.
#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    feature = "modules",
    feature = "channels"
))]
#[tokio::test]
async fn telegram_bot_token_connect_reject_disconnect_and_reconnect_over_json_rpc_e2e() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let pinned = openhuman_core::modules::registry::find("tinychannels")
        .expect("TinyChannels is registered in the module asset registry");
    assert!(
        std::env::var_os("OPENHUMAN_MODULE_SOURCE").is_none()
            && std::env::var_os("OPENHUMAN_MODULE_SOURCE_DIR").is_none()
            && std::env::var_os("OPENHUMAN_MODULE_SOURCE_TINYCHANNELS").is_none(),
        "the integration must use the staged release, not a source or linked override"
    );
    let module_paths_env = std::env::var_os("OPENHUMAN_MODULE_PATH")
        .expect("the standard Rust test runner must stage TinyChannels on OPENHUMAN_MODULE_PATH");
    let module_paths = std::env::split_paths(&module_paths_env).collect::<Vec<_>>();
    let artifact = openhuman_core::modules::local_artifact("tinychannels")
        .expect("the standard Rust test runner must stage the pinned TinyChannels library");
    let artifact_dir = artifact
        .parent()
        .expect("the staged TinyChannels artifact has a parent directory");
    assert!(
        module_paths.iter().any(|path| path == artifact_dir),
        "TinyChannels must resolve to the standard runner's staged release path: {artifact:?}"
    );
    let module_cache = tempdir().expect("isolated module cache");
    let _module_cache_guard = EnvVarGuard::set_to_path("XDG_CACHE_HOME", module_cache.path());
    let module_data = tempdir().expect("isolated module data directory");
    let _module_data_guard = EnvVarGuard::set_to_path("XDG_DATA_HOME", module_data.path());

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);
    let local_openhuman_home = openhuman_home.join("users/local");
    for config_dir in [&openhuman_home, &local_openhuman_home] {
        let config_path = config_dir.join("config.toml");
        let mut config_text = std::fs::read_to_string(&config_path).expect("minimal config exists");
        config_text.push_str("\n[modules]\nsource = \"release\"\nallow_download = false\n");
        std::fs::write(config_path, config_text).expect("disable module downloads in test config");
    }

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    let connect = post_json_rpc(
        &rpc_base,
        7151,
        "openhuman.channels_connect",
        json!({
            "channel": "telegram",
            "authMode": "bot_token",
            "credentials": { "bot_token": "fixture-A" }
        }),
    )
    .await;
    let connected = peel_logs_envelope(assert_no_jsonrpc_error(
        &connect,
        "Telegram channels_connect fixture A",
    ));
    assert_eq!(
        connected.get("status").and_then(Value::as_str),
        Some("connected")
    );
    assert_eq!(
        connected.get("restart_required").and_then(Value::as_bool),
        Some(true)
    );

    let status_a = channel_status_entry(
        &post_json_rpc(
            &rpc_base,
            7152,
            "openhuman.channels_status",
            json!({ "channel": "telegram" }),
        )
        .await,
        "Telegram status after fixture A",
        "telegram",
        "bot_token",
    );
    assert_eq!(
        status_a.get("connected").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        status_a.get("has_credentials").and_then(Value::as_bool),
        Some(true)
    );

    let rejected = post_json_rpc(
        &rpc_base,
        7153,
        "openhuman.channels_connect",
        json!({
            "channel": "telegram",
            "authMode": "bot_token",
            "credentials": { "bot_token": "" }
        }),
    )
    .await;
    assert!(
        rejected.get("error").is_some(),
        "an empty Telegram bot token must be rejected: {rejected}"
    );
    let preserved = channel_status_entry(
        &post_json_rpc(
            &rpc_base,
            7154,
            "openhuman.channels_status",
            json!({ "channel": "telegram" }),
        )
        .await,
        "Telegram status after rejected empty token",
        "telegram",
        "bot_token",
    );
    assert_eq!(
        preserved.get("connected").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        preserved.get("has_credentials").and_then(Value::as_bool),
        Some(true)
    );

    let disconnect = post_json_rpc(
        &rpc_base,
        7155,
        "openhuman.channels_disconnect",
        json!({ "channel": "telegram", "authMode": "bot_token" }),
    )
    .await;
    let disconnected = peel_logs_envelope(assert_no_jsonrpc_error(
        &disconnect,
        "Telegram channels_disconnect",
    ));
    assert_eq!(
        disconnected.get("disconnected").and_then(Value::as_bool),
        Some(true)
    );
    let status_disconnected = channel_status_entry(
        &post_json_rpc(
            &rpc_base,
            7156,
            "openhuman.channels_status",
            json!({ "channel": "telegram" }),
        )
        .await,
        "Telegram status after disconnect",
        "telegram",
        "bot_token",
    );
    assert_eq!(
        status_disconnected
            .get("connected")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        status_disconnected
            .get("has_credentials")
            .and_then(Value::as_bool),
        Some(false)
    );

    let reconnect = post_json_rpc(
        &rpc_base,
        7157,
        "openhuman.channels_connect",
        json!({
            "channel": "telegram",
            "authMode": "bot_token",
            "credentials": { "bot_token": "fixture-B" }
        }),
    )
    .await;
    let reconnected = peel_logs_envelope(assert_no_jsonrpc_error(
        &reconnect,
        "Telegram channels_connect fixture B",
    ));
    assert_eq!(
        reconnected.get("status").and_then(Value::as_str),
        Some("connected")
    );
    let status_b = channel_status_entry(
        &post_json_rpc(
            &rpc_base,
            7158,
            "openhuman.channels_status",
            json!({ "channel": "telegram" }),
        )
        .await,
        "Telegram status after fixture B",
        "telegram",
        "bot_token",
    );
    assert_eq!(
        status_b.get("connected").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        status_b.get("has_credentials").and_then(Value::as_bool),
        Some(true)
    );

    let modules_response =
        post_json_rpc(&rpc_base, 7159, "openhuman.modules_list", json!({})).await;
    let modules_response =
        assert_no_jsonrpc_error(&modules_response, "modules_list after Telegram");
    let tinychannels = modules_response
        .get("modules")
        .or_else(|| modules_response.pointer("/result/modules"))
        .and_then(Value::as_array)
        .and_then(|modules| {
            modules
                .iter()
                .find(|module| module.get("id").and_then(Value::as_str) == Some("tinychannels"))
        })
        .unwrap_or_else(|| panic!("modules_list omitted TinyChannels: {modules_response}"));
    assert_eq!(
        tinychannels.get("version").and_then(Value::as_str),
        Some(pinned.version),
        "the loaded TinyChannels must be the registry-pinned release"
    );
    assert_eq!(
        tinychannels.get("state").and_then(Value::as_str),
        Some("ready")
    );
    assert_eq!(
        tinychannels.get("source").and_then(Value::as_str),
        Some("release")
    );

    mock_join.abort();
    rpc_join.abort();
}

/// Credential channel descriptions are consumed by the setup form to build
/// its credential fields. Keep the email and Yuanbao field contracts reachable
/// through the same JSON-RPC surface used by the app.
#[tokio::test]
async fn credential_channel_describe_fields_are_wired_over_json_rpc_e2e() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    let list = post_json_rpc(&rpc_base, 7003, "openhuman.channels_list", json!({})).await;
    let list_result = peel_logs_envelope(assert_no_jsonrpc_error(&list, "channels_list"));
    let channels = list_result
        .as_array()
        .or_else(|| list_result.get("channels").and_then(Value::as_array))
        .unwrap_or_else(|| panic!("channels_list should return an array: {list_result}"));
    for channel in ["email", "yuanbao"] {
        let definition = channels
            .iter()
            .find(|entry| entry.get("id").and_then(Value::as_str) == Some(channel))
            .unwrap_or_else(|| panic!("channels_list omitted {channel}: {channels:?}"));
        let modes = definition
            .get("auth_modes")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{channel} has no auth_modes: {definition}"));
        assert!(
            modes
                .iter()
                .any(|mode| { mode.get("mode").and_then(Value::as_str) == Some("api_key") }),
            "{channel} must advertise api_key credentials: {definition}"
        );
    }

    for (id, channel, required_fields, optional_default) in [
        (
            7004,
            "email",
            vec!["imap_host", "username", "password", "smtp_host"],
            Some(("smtp_tls", true)),
        ),
        (7005, "yuanbao", vec!["app_key", "app_secret"], None),
    ] {
        let response = post_json_rpc(
            &rpc_base,
            id,
            "openhuman.channels_describe",
            json!({ "channel": channel }),
        )
        .await;
        let outer = assert_no_jsonrpc_error(&response, "channels_describe credential channel");
        let description = peel_logs_envelope(outer.get("result").unwrap_or(outer));
        assert_eq!(
            description.get("id").and_then(Value::as_str),
            Some(channel),
            "unexpected channel description: {description}"
        );
        let api_key = description
            .get("auth_modes")
            .and_then(Value::as_array)
            .and_then(|modes| {
                modes
                    .iter()
                    .find(|mode| mode.get("mode").and_then(Value::as_str) == Some("api_key"))
            })
            .unwrap_or_else(|| panic!("{channel} has no api_key auth mode: {description}"));
        let fields = api_key
            .get("fields")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{channel} api_key mode has no fields: {api_key}"));
        let keys = fields
            .iter()
            .filter_map(|field| field.get("key").and_then(Value::as_str))
            .collect::<Vec<_>>();
        for required in required_fields {
            assert!(
                keys.contains(&required),
                "{channel} description omits setup field {required}: {description}"
            );
            assert_eq!(
                fields
                    .iter()
                    .find(|field| field.get("key").and_then(Value::as_str) == Some(required))
                    .and_then(|field| field.get("required"))
                    .and_then(Value::as_bool),
                Some(true),
                "{channel} field {required} must be required: {description}"
            );
        }
        if let Some((key, expected)) = optional_default {
            assert_eq!(
                fields
                    .iter()
                    .find(|field| field.get("key").and_then(Value::as_str) == Some(key))
                    .and_then(|field| field.get("default_bool"))
                    .and_then(Value::as_bool),
                Some(expected),
                "{channel} should describe the default for {key}: {description}"
            );
        }
    }

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn credential_yuanbao_connect_and_disconnect_roundtrip_over_json_rpc_e2e() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let sign_token_requests = Arc::new(AtomicUsize::new(0));
    let requests_for_route = Arc::clone(&sign_token_requests);
    let yuanbao_mock = Router::new().route(
        "/api/v5/robotLogic/sign-token",
        post(move || {
            let requests = Arc::clone(&requests_for_route);
            async move {
                requests.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "code": 0,
                    "data": { "token": "local-yuanbao-token", "bot_id": "local-bot", "duration": 600 }
                }))
            }
        }),
    );
    let (yuanbao_addr, yuanbao_join) = serve_on_ephemeral(yuanbao_mock).await;
    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    let connect = post_json_rpc(
        &rpc_base,
        7101,
        "openhuman.channels_connect",
        json!({
            "channel": "yuanbao",
            "authMode": "api_key",
            "credentials": {
                "app_key": "e2e-yuanbao-key",
                "app_secret": "e2e-yuanbao-secret",
                "api_domain": format!("http://{}", yuanbao_addr),
            }
        }),
    )
    .await;
    let connected = peel_logs_envelope(assert_no_jsonrpc_error(
        &connect,
        "channels_connect Yuanbao",
    ));
    assert_eq!(
        connected.get("status").and_then(Value::as_str),
        Some("connected"),
        "Yuanbao connect should report connected: {connected}"
    );
    assert_eq!(
        connected.get("restart_required").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        sign_token_requests.load(Ordering::SeqCst),
        1,
        "the pinned module must verify credentials through the test-owned endpoint"
    );

    let status = channel_status_entry(
        &post_json_rpc(
            &rpc_base,
            7102,
            "openhuman.channels_status",
            json!({ "channel": "yuanbao" }),
        )
        .await,
        "Yuanbao status after connect",
        "yuanbao",
        "api_key",
    );
    assert_eq!(status.get("connected").and_then(Value::as_bool), Some(true));
    assert_eq!(
        status.get("has_credentials").and_then(Value::as_bool),
        Some(true)
    );

    let user_config = openhuman_home.join("users/local/config.toml");
    let config_text = std::fs::read_to_string(&user_config).expect("Yuanbao config persisted");
    let parsed_config: toml::Value = toml::from_str(&config_text).expect("config parses");
    let yuanbao_config = parsed_config
        .get("channels_config")
        .and_then(|channels| channels.get("yuanbao"))
        .expect("Yuanbao config saved");
    assert_eq!(
        yuanbao_config.get("app_key").and_then(toml::Value::as_str),
        Some("e2e-yuanbao-key")
    );
    assert!(
        yuanbao_config
            .get("app_secret")
            .and_then(toml::Value::as_str)
            .is_none_or(str::is_empty),
        "Yuanbao app_secret must not be persisted in config.toml: {yuanbao_config}"
    );
    assert!(
        !config_text.contains("e2e-yuanbao-secret"),
        "Yuanbao secret must not be persisted in config.toml"
    );

    let disconnect = post_json_rpc(
        &rpc_base,
        7103,
        "openhuman.channels_disconnect",
        json!({ "channel": "yuanbao", "authMode": "api_key" }),
    )
    .await;
    let disconnected = peel_logs_envelope(assert_no_jsonrpc_error(
        &disconnect,
        "channels_disconnect Yuanbao",
    ));
    assert_eq!(
        disconnected.get("disconnected").and_then(Value::as_bool),
        Some(true)
    );
    let status = channel_status_entry(
        &post_json_rpc(
            &rpc_base,
            7104,
            "openhuman.channels_status",
            json!({ "channel": "yuanbao" }),
        )
        .await,
        "Yuanbao status after disconnect",
        "yuanbao",
        "api_key",
    );
    assert_eq!(
        status.get("connected").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        status.get("has_credentials").and_then(Value::as_bool),
        Some(false)
    );
    let config_text = std::fs::read_to_string(&user_config).expect("config remains present");
    let parsed_config: toml::Value = toml::from_str(&config_text).expect("config parses");
    assert!(
        parsed_config
            .get("channels_config")
            .and_then(|channels| channels.get("yuanbao"))
            .is_none(),
        "Yuanbao disconnect should remove its config"
    );

    mock_join.abort();
    yuanbao_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn credential_channels_reject_missing_required_fields_over_json_rpc_e2e() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    for (id, channel, credentials) in [
        (
            7201,
            "yuanbao",
            json!({ "app_key": "incomplete-yuanbao-key" }),
        ),
        (
            7203,
            "email",
            json!({
                "imap_host": "127.0.0.1",
                "smtp_host": "smtp.example.invalid",
                "password": "incomplete-email-password",
            }),
        ),
    ] {
        let response = post_json_rpc(
            &rpc_base,
            id,
            "openhuman.channels_connect",
            json!({ "channel": channel, "authMode": "api_key", "credentials": credentials }),
        )
        .await;
        assert!(
            response.get("error").is_some(),
            "{channel} must reject missing required credentials: {response}"
        );
        let status = channel_status_entry(
            &post_json_rpc(
                &rpc_base,
                id + 1,
                "openhuman.channels_status",
                json!({ "channel": channel }),
            )
            .await,
            &format!("{channel} status after rejected connect"),
            channel,
            "api_key",
        );
        assert_eq!(
            status.get("connected").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            status.get("has_credentials").and_then(Value::as_bool),
            Some(false),
            "{channel} must not store incomplete credentials"
        );
    }

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn credential_email_connect_refusal_does_not_store_credentials_over_json_rpc_e2e() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let imap_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback IMAP fixture");
    let imap_port = imap_listener
        .local_addr()
        .expect("IMAP fixture address")
        .port();
    let mut accept_task = tokio::spawn(async move {
        let (socket, _) = imap_listener.accept().await.expect("accept IMAP probe");
        drop(socket); // Refuse the TLS handshake before any credentials can be sent.
    });

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    let credentials = json!({
        "imap_host": "127.0.0.1",
        "imap_port": imap_port.to_string(),
        "smtp_host": "smtp.example.invalid",
        "smtp_port": "465",
        "username": "e2e@example.invalid",
        "password": "e2e-email-password",
    });
    let response = post_json_rpc(
        &rpc_base,
        7301,
        "openhuman.channels_connect",
        json!({ "channel": "email", "authMode": "api_key", "credentials": credentials }),
    )
    .await;
    assert!(
        response.get("error").is_some(),
        "Email connect should surface the local IMAP verifier refusal: {response}"
    );
    match tokio::time::timeout(Duration::from_secs(5), &mut accept_task).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("IMAP fixture task failed: {error}"),
        Err(_) => {
            accept_task.abort();
            let _ = accept_task.await;
            panic!("email verifier should reach the test-owned IMAP endpoint");
        }
    }

    let error_text = response.get("error").expect("RPC error").to_string();
    for secret in ["e2e-email-password", "e2e@example.invalid"] {
        assert!(
            !error_text.contains(secret),
            "email verifier error must not expose credentials: {error_text}"
        );
    }
    let status = channel_status_entry(
        &post_json_rpc(
            &rpc_base,
            7302,
            "openhuman.channels_status",
            json!({ "channel": "email" }),
        )
        .await,
        "Email status after local verifier refusal",
        "email",
        "api_key",
    );
    assert_eq!(
        status.get("connected").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        status.get("has_credentials").and_then(Value::as_bool),
        Some(false),
        "failed IMAP verification must not store email credentials"
    );

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn credential_email_disconnect_clears_config_over_json_rpc_e2e() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);
    let user_config = openhuman_home.join("users/local/config.toml");
    let mut config_text = std::fs::read_to_string(&user_config).expect("base config exists");
    config_text.push_str(
        "\n[channels_config.email]\nimap_host = \"127.0.0.1\"\nsmtp_host = \"127.0.0.1\"\nusername = \"e2e@example.invalid\"\npassword = \"\"\nfrom_address = \"e2e@example.invalid\"\nallowed_senders = [\"*\"]\n",
    );
    std::fs::write(&user_config, config_text).expect("seed email config for disconnect");

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    let disconnect = post_json_rpc(
        &rpc_base,
        7401,
        "openhuman.channels_disconnect",
        json!({ "channel": "email", "authMode": "api_key" }),
    )
    .await;
    assert_no_jsonrpc_error(&disconnect, "channels_disconnect Email");

    let config_text = std::fs::read_to_string(&user_config).expect("config remains present");
    let parsed_config: toml::Value = toml::from_str(&config_text).expect("config parses");
    assert!(
        parsed_config
            .get("channels_config")
            .and_then(|channels| channels.get("email"))
            .is_none(),
        "Email disconnect should remove its config"
    );
    let status = channel_status_entry(
        &post_json_rpc(
            &rpc_base,
            7402,
            "openhuman.channels_status",
            json!({ "channel": "email" }),
        )
        .await,
        "Email status after disconnect",
        "email",
        "api_key",
    );
    assert_eq!(
        status.get("connected").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        status.get("has_credentials").and_then(Value::as_bool),
        Some(false)
    );

    mock_join.abort();
    rpc_join.abort();
}

fn channel_status_entry(response: &Value, context: &str, channel: &str, mode: &str) -> Value {
    let result = assert_no_jsonrpc_error(response, context);
    let statuses = peel_logs_envelope(result)
        .as_array()
        .unwrap_or_else(|| panic!("{context}: expected channel status array, got {result}"));
    statuses
        .iter()
        .find(|entry| {
            entry.get("channel_id").and_then(Value::as_str) == Some(channel)
                && entry.get("auth_mode").and_then(Value::as_str) == Some(mode)
        })
        .cloned()
        .unwrap_or_else(|| panic!("{context}: missing {channel}/{mode} entry: {statuses:?}"))
}

/// Set on the re-exec below so the child runs the test body instead of
/// spawning another child.
const OWNER_ONLY_CHILD_ENV: &str = "OPENHUMAN_E2E_OWNER_ONLY_CHILD";
#[cfg(unix)]
const OWNER_ONLY_TEST_NAME: &str =
    "auth_store_provider_credentials_writes_an_owner_only_store_file";

/// Find `auth-profiles.json` beneath a temp `HOME`.
///
/// `cfg(unix)` to match its only caller: the mode assertion below has nothing
/// to check on a platform without permission bits, and an uncalled helper
/// would be dead code there.
///
/// Searched rather than hard-coded: the state dir is resolved from config and
/// user scope, and pinning that layout here would make this test fail for a
/// reason that has nothing to do with file modes.
#[cfg(unix)]
fn find_auth_profiles_store(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name() == Some(std::ffi::OsStr::new("auth-profiles.json")) {
                return Some(path);
            }
        }
    }
    None
}

/// #5795 — the credential store must not be group/world readable.
///
/// `fs::write` creates at `0o666 & ~umask` (0644 under the usual 022) and
/// `fs::rename` carries the source mode onto the destination, so before the fix
/// every save left OAuth-token ciphertext readable by any UID on the box.
///
/// Driven through `auth.store_provider_credentials` — the RPC a user hits when
/// they paste a provider key — because the existing unit cover calls
/// `AuthProfilesStore` directly and would not notice the RPC path acquiring a
/// different writer.
#[cfg(unix)]
#[tokio::test]
async fn auth_store_provider_credentials_writes_an_owner_only_store_file() {
    use std::os::unix::fs::PermissionsExt as _;

    // Re-exec this one test under a known-permissive umask before doing
    // anything else.
    //
    // umask is process-global and libtest runs tests as threads, so it cannot
    // be set in-process without racing every other test in this binary. It has
    // to be controlled, though: a reverted `fs::write` creates at
    // `0o666 & ~umask`, which is 0644 under the usual 022 but ALSO 0600 under a
    // hardened 077 — and under 077 an owner-only assertion passes whether or not
    // the fix is present. Skipping in that case is not a way out either: libtest
    // records an early `return` as a PASS, so on a hardened runner this
    // security regression test would go green having checked nothing.
    if std::env::var(OWNER_ONLY_CHILD_ENV).is_err() {
        let exe = std::env::current_exe().expect("current_exe");
        // The exe path and the test name go in as POSITIONAL PARAMETERS rather
        // than interpolated into the command string. A repository or
        // CARGO_TARGET_DIR containing a single quote (`/home/o'connor/openhuman`)
        // would close the literal `'...'` early and hand `sh` invalid syntax, so
        // this test would fail before its child ever started — on exactly the
        // machines whose paths are least likely to be noticed. `$0` is the
        // conventional placeholder slot for `sh -c`, so the real arguments start
        // at `$1`.
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg("umask 022 && exec \"$1\" --exact \"$2\" --nocapture --test-threads=1")
            .arg("sh")
            .arg(&exe)
            .arg(OWNER_ONLY_TEST_NAME)
            .env(OWNER_ONLY_CHILD_ENV, "1")
            .status()
            .expect("re-exec the owner-only test under umask 022");
        assert!(
            status.success(),
            "the owner-only credential-store check failed under umask 022 (see child output above)"
        );
        return;
    }

    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    // Precondition, asserted rather than skipped: with umask 022 an ordinary
    // create must land group/world-readable. If it does not, the umask did not
    // take and the assertion below could not tell the fix from its absence —
    // that is a failure, not a pass.
    let probe = home.join("umask-probe");
    std::fs::write(&probe, b"probe").expect("write probe");
    let probe_mode = std::fs::metadata(&probe)
        .expect("probe metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_ne!(
        probe_mode & 0o077,
        0,
        "umask 022 should leave a plain create group-readable, got {probe_mode:#o}; \
         without that this test cannot distinguish the owner-only fix from its absence"
    );

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let stored = post_json_rpc(
        &rpc_base,
        9_795,
        "openhuman.auth_store_provider_credentials",
        json!({
            "provider": "provider:openai",
            "profile": "default",
            "token": "sk-owner-only-e2e",
            "setActive": true
        }),
    )
    .await;
    assert_no_jsonrpc_error(&stored, "auth_store_provider_credentials");

    let store_path =
        find_auth_profiles_store(home).expect("the save must have produced an auth-profiles.json");
    let mode = std::fs::metadata(&store_path)
        .expect("store metadata")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(
        mode,
        0o600,
        "the credential store at {} must be owner-only, got {mode:#o} \
         (a plain create in this environment yields {probe_mode:#o})",
        store_path.display()
    );

    mock_join.abort();
    rpc_join.abort();
}
