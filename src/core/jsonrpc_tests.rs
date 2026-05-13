use serde_json::json;
use std::ffi::OsString;
use std::sync::Arc;
use std::sync::MutexGuard;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use super::{
    build_http_schema_dump, default_state, escape_html, invoke_method, is_param_validation_error,
    is_session_expired_error, params_to_object, parse_json_params, rpc_handler, type_name,
};

struct EnvVarGuard {
    old_values: Vec<(&'static str, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvVarGuard {
    fn set_many(vars: Vec<(&'static str, OsString)>) -> Self {
        let lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .expect("test env lock poisoned");
        let mut old_values = Vec::with_capacity(vars.len());
        for (key, value) in vars {
            let old = std::env::var_os(key);
            std::env::set_var(key, value);
            old_values.push((key, old));
        }
        Self {
            old_values,
            _lock: lock,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        for (key, old) in self.old_values.iter().rev() {
            match old {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

async fn wait_until_port_accepts(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "server did not start accepting on port {port}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_until_port_released(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "server did not release port {port}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Regression test for issue #920 — the embedded server's `axum::serve`
/// accept loop must stop within the cancellation timeout when its
/// `CancellationToken` is fired.
///
/// **Ignored by default.** This test calls `run_server_embedded`,
/// which triggers the full production bootstrap (`bootstrap_skill_runtime`
/// → `register_domain_subscribers` → `scheduler_gate::init_global` +
/// `memory::tree::jobs::start` + `composio::start_periodic_sync` +
/// cron scheduler). Those code paths spawn detached `tokio::spawn`
/// background tasks and write to several process-global statics
/// (`STATE: OnceLock`, `SIGNED_OUT: AtomicBool`, `LLM_PERMITS`
/// semaphore, `GLOBAL_REGISTRY` agent.run_turn handler, `STARTED`
/// `std::sync::Once`s, …) — *none of which have teardown semantics*.
/// In a unit-test binary the leaked tasks then race with every other
/// test, multiplying CI wall time by 10–20× (PR #1552 thread). The
/// right shape for this regression is an integration test in a
/// dedicated `tests/` binary where global pollution doesn't affect
/// siblings — tracked as a follow-up.
///
/// To run manually: `cargo test --lib -p openhuman -- --ignored
/// shutdown_token`.
#[tokio::test]
#[ignore = "calls full server bootstrap; leaks process-global state into sibling tests (#1552). Re-cover via integration test."]
async fn shutdown_token_stops_axum_listener_within_timeout() {
    let _signed_out_restore = crate::openhuman::scheduler_gate::SignedOutTestGuard::set(false);

    let workspace = tempfile::tempdir().expect("workspace tempdir");

    // Pin scheduler-gate policy to Aggressive while this test runs so
    // the bootstrap's `init_global` snapshot can't capture transient
    // CPU pressure and freeze the cached policy at Paused.
    std::fs::write(
        workspace.path().join("config.toml"),
        "[scheduler_gate]\nmode = \"always_on\"\n",
    )
    .expect("seed scheduler_gate=always_on config.toml");
    let _env = EnvVarGuard::set_many(vec![
        (
            "OPENHUMAN_WORKSPACE",
            workspace.path().as_os_str().to_os_string(),
        ),
        ("OPENHUMAN_DISABLE_CHANNEL_LISTENERS", OsString::from("1")),
        (
            "OPENHUMAN_CORE_TOKEN",
            OsString::from("test-token-shutdown"),
        ),
    ]);

    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("allocate test port");
    let port = probe.local_addr().expect("local addr").port();
    drop(probe);

    let shutdown_token = CancellationToken::new();
    let server_token = shutdown_token.clone();
    let server = tokio::spawn(async move {
        super::run_server_embedded(Some("127.0.0.1"), Some(port), false, server_token).await
    });

    wait_until_port_accepts(port).await;
    shutdown_token.cancel();

    let result = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("embedded server task should stop within timeout")
        .expect("embedded server task should not panic");
    result.expect("embedded server should shut down cleanly");
    wait_until_port_released(port).await;
}

#[tokio::test]
async fn invoke_health_snapshot_via_registry() {
    let result = invoke_method(default_state(), "openhuman.health_snapshot", json!({}))
        .await
        .expect("health snapshot should succeed");
    assert!(result.get("result").is_some());
}

#[tokio::test]
async fn invoke_encrypt_secret_missing_required_param_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.encrypt_secret", json!({}))
        .await
        .expect_err("missing plaintext should fail");
    assert!(err.contains("missing required param 'plaintext'"));
}

#[tokio::test]
async fn invoke_doctor_models_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.doctor_models",
        json!({ "invalid": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'invalid'"));
}

#[tokio::test]
async fn invoke_config_get_runtime_flags_via_registry() {
    let result = invoke_method(
        default_state(),
        "openhuman.config_get_runtime_flags",
        json!({}),
    )
    .await
    .expect("runtime flags should succeed");
    assert!(result.get("result").is_some());
}

#[tokio::test]
async fn invoke_autocomplete_status_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.autocomplete_status",
        json!({ "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'extra'"));
}

#[tokio::test]
async fn invoke_auth_store_session_missing_token_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.auth_store_session", json!({}))
        .await
        .expect_err("missing token should fail");
    assert!(err.contains("missing required param 'token'"));
}

#[tokio::test]
async fn invoke_service_status_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.service_status",
        json!({ "x": 1 }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'x'"));
}

#[tokio::test]
async fn invoke_memory_init_accepts_empty_params() {
    // jwt_token is optional (accepted for backward compat but ignored).
    // The call may still fail for workspace reasons in test, but must NOT
    // fail with a missing-param error for jwt_token.
    let result = invoke_method(default_state(), "openhuman.memory_init", json!({})).await;
    if let Err(ref e) = result {
        assert!(
            !e.contains("missing required param") || !e.contains("jwt_token"),
            "jwt_token should be optional, got: {e}"
        );
    }
}

#[tokio::test]
async fn invoke_memory_list_namespaces_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.memory_list_namespaces",
        json!({ "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("extra"));
}

#[tokio::test]
async fn invoke_memory_query_namespace_missing_namespace_fails() {
    let err = invoke_method(
        default_state(),
        "openhuman.memory_query_namespace",
        json!({ "query": "who owns atlas" }),
    )
    .await
    .expect_err("missing namespace should fail");
    assert!(err.contains("namespace"));
}

#[tokio::test]
async fn invoke_memory_recall_memories_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.memory_recall_memories",
        json!({ "namespace": "team", "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("extra"));
}

#[tokio::test]
async fn invoke_migrate_openclaw_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.migrate_openclaw",
        json!({ "x": 1 }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'x'"));
}

#[tokio::test]
async fn invoke_local_ai_download_asset_missing_required_param_fails_validation() {
    let err = invoke_method(
        default_state(),
        "openhuman.local_ai_download_asset",
        json!({}),
    )
    .await
    .expect_err("missing capability should fail");
    assert!(err.contains("missing required param 'capability'"));
}

#[test]
fn http_schema_dump_includes_openhuman_and_core_methods() {
    let dump = build_http_schema_dump();
    let methods = dump.methods;
    assert!(
        methods
            .iter()
            .any(|m| m.method == "core.version" && m.namespace == "core"),
        "schema dump should include core methods"
    );

    assert!(
        methods
            .iter()
            .any(|m| m.method == "openhuman.health_snapshot"),
        "schema dump should include migrated openhuman methods"
    );

    assert!(
        methods
            .iter()
            .any(|m| m.method == "openhuman.billing_get_current_plan"),
        "schema dump should include billing methods"
    );

    assert!(
        methods
            .iter()
            .any(|m| m.method == "openhuman.team_list_members"),
        "schema dump should include team methods"
    );
}

#[tokio::test]
async fn billing_get_current_plan_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.billing_get_current_plan",
        json!({ "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'extra'"));
}

#[tokio::test]
async fn billing_purchase_plan_missing_plan_fails_validation() {
    let err = invoke_method(
        default_state(),
        "openhuman.billing_purchase_plan",
        json!({}),
    )
    .await
    .expect_err("missing plan should fail");
    assert!(err.contains("missing required param 'plan'"));
}

#[tokio::test]
async fn billing_top_up_missing_amount_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.billing_top_up", json!({}))
        .await
        .expect_err("missing amountUsd should fail");
    assert!(err.contains("missing required param 'amountUsd'"));
}

#[tokio::test]
async fn billing_top_up_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.billing_top_up",
        json!({ "amountUsd": 10.0, "unknownField": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'unknownField'"));
}

#[tokio::test]
async fn billing_create_portal_session_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.billing_create_portal_session",
        json!({ "x": 1 }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'x'"));
}

#[tokio::test]
async fn team_list_members_missing_team_id_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.team_list_members", json!({}))
        .await
        .expect_err("missing teamId should fail");
    assert!(err.contains("missing required param 'teamId'"));
}

#[tokio::test]
async fn team_list_members_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.team_list_members",
        json!({ "teamId": "t1", "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'extra'"));
}

#[tokio::test]
async fn team_create_invite_missing_team_id_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.team_create_invite", json!({}))
        .await
        .expect_err("missing teamId should fail");
    assert!(err.contains("missing required param 'teamId'"));
}

#[tokio::test]
async fn team_remove_member_missing_required_params_fails_validation() {
    let err = invoke_method(
        default_state(),
        "openhuman.team_remove_member",
        json!({ "teamId": "t1" }),
    )
    .await
    .expect_err("missing userId should fail");
    assert!(err.contains("missing required param 'userId'"));
}

#[tokio::test]
async fn team_change_member_role_missing_role_fails_validation() {
    let err = invoke_method(
        default_state(),
        "openhuman.team_change_member_role",
        json!({ "teamId": "t1", "userId": "u1" }),
    )
    .await
    .expect_err("missing role should fail");
    assert!(err.contains("missing required param 'role'"));
}

#[tokio::test]
async fn billing_create_coinbase_charge_missing_plan_fails_validation() {
    let err = invoke_method(
        default_state(),
        "openhuman.billing_create_coinbase_charge",
        json!({}),
    )
    .await
    .expect_err("missing plan should fail");
    assert!(err.contains("missing required param 'plan'"));
}

#[tokio::test]
async fn billing_create_coinbase_charge_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.billing_create_coinbase_charge",
        json!({ "plan": "pro", "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'extra'"));
}

#[tokio::test]
async fn team_list_invites_missing_team_id_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.team_list_invites", json!({}))
        .await
        .expect_err("missing teamId should fail");
    assert!(err.contains("missing required param 'teamId'"));
}

#[tokio::test]
async fn team_list_invites_rejects_unknown_param() {
    let err = invoke_method(
        default_state(),
        "openhuman.team_list_invites",
        json!({ "teamId": "t1", "extra": true }),
    )
    .await
    .expect_err("unknown param should fail");
    assert!(err.contains("unknown param 'extra'"));
}

#[tokio::test]
async fn team_revoke_invite_missing_team_id_fails_validation() {
    let err = invoke_method(default_state(), "openhuman.team_revoke_invite", json!({}))
        .await
        .expect_err("missing teamId should fail");
    assert!(err.contains("missing required param 'teamId'"));
}

#[tokio::test]
async fn team_revoke_invite_missing_invite_id_fails_validation() {
    let err = invoke_method(
        default_state(),
        "openhuman.team_revoke_invite",
        json!({ "teamId": "t1" }),
    )
    .await
    .expect_err("missing inviteId should fail");
    assert!(err.contains("missing required param 'inviteId'"));
}

#[tokio::test]
async fn schema_dump_includes_new_billing_and_team_methods() {
    let dump = build_http_schema_dump();
    let methods: Vec<&str> = dump.methods.iter().map(|m| m.method.as_str()).collect();
    for expected in &[
        "openhuman.billing_get_current_plan",
        "openhuman.billing_purchase_plan",
        "openhuman.billing_create_portal_session",
        "openhuman.billing_top_up",
        "openhuman.billing_create_coinbase_charge",
        "openhuman.team_list_members",
        "openhuman.team_create_invite",
        "openhuman.team_list_invites",
        "openhuman.team_revoke_invite",
        "openhuman.team_remove_member",
        "openhuman.team_change_member_role",
    ] {
        assert!(
            methods.contains(expected),
            "schema dump missing expected method: {expected}"
        );
    }
}

// --- helper coverage -----------------------------------------------------

#[test]
fn params_to_object_accepts_object() {
    let map = params_to_object(json!({"a": 1, "b": "x"})).unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map.get("a"), Some(&json!(1)));
}

#[test]
fn params_to_object_accepts_null_as_empty_map() {
    let map = params_to_object(json!(null)).unwrap();
    assert!(map.is_empty());
}

#[test]
fn params_to_object_rejects_array() {
    let err = params_to_object(json!([1, 2, 3])).unwrap_err();
    assert!(err.contains("invalid params"));
    assert!(err.contains("array"));
}

#[test]
fn params_to_object_rejects_scalars() {
    assert!(params_to_object(json!(42)).unwrap_err().contains("number"));
    assert!(params_to_object(json!("hi"))
        .unwrap_err()
        .contains("string"));
    assert!(params_to_object(json!(true)).unwrap_err().contains("bool"));
}

#[test]
fn type_name_labels_every_json_variant() {
    assert_eq!(type_name(&json!(null)), "null");
    assert_eq!(type_name(&json!(true)), "bool");
    assert_eq!(type_name(&json!(3)), "number");
    assert_eq!(type_name(&json!("s")), "string");
    assert_eq!(type_name(&json!([])), "array");
    assert_eq!(type_name(&json!({})), "object");
}

#[test]
fn parse_json_params_roundtrips_object() {
    let v = parse_json_params(r#"{"k":1}"#).unwrap();
    assert_eq!(v, json!({"k": 1}));
}

#[test]
fn parse_json_params_reports_error_message() {
    let err = parse_json_params("{not json").unwrap_err();
    assert!(err.contains("invalid JSON params"));
}

#[test]
fn is_session_expired_error_matches_401_unauthorized() {
    assert!(is_session_expired_error(
        "backend returned 401 Unauthorized"
    ));
    assert!(is_session_expired_error("401 UNAUTHORIZED"));
    assert!(is_session_expired_error("got 401 and unauthorized body"));
}

#[test]
fn is_session_expired_error_requires_both_401_and_unauthorized() {
    // 401 alone is not sufficient — could be HTTP/3.01 nonsense or
    // unrelated text. We require the string "unauthorized" too.
    assert!(!is_session_expired_error("server returned 401"));
    assert!(!is_session_expired_error("unauthorized without code"));
}

#[test]
fn is_session_expired_error_matches_invalid_token_case_insensitive() {
    assert!(is_session_expired_error("Invalid Token"));
    assert!(is_session_expired_error("got an invalid token here"));
}

#[test]
fn is_session_expired_error_matches_session_expired_sentinel() {
    // The SESSION_EXPIRED sentinel is case-sensitive by design.
    assert!(is_session_expired_error("SESSION_EXPIRED: please re-auth"));
    assert!(!is_session_expired_error("session_expired lowercase"));
}

#[test]
fn is_session_expired_error_does_not_match_unrelated_errors() {
    assert!(!is_session_expired_error("network timeout"));
    assert!(!is_session_expired_error("500 internal server error"));
    assert!(!is_session_expired_error(""));
}

#[test]
fn is_param_validation_error_matches_the_three_validator_shapes() {
    // Regression guard for OPENHUMAN-TAURI-20: pre-#1467 cores rejected
    // `api_key` because it wasn't in the schema yet. The error string
    // must keep matching here so it gets logged at info level and never
    // reaches Sentry as an unactionable client/server skew event.
    assert!(is_param_validation_error(
        "unknown param 'api_key' for config.update_model_settings"
    ));
    // `all::validate_params` — missing required field.
    assert!(is_param_validation_error(
        "missing required param 'session_id': active session identifier"
    ));
    // `params_to_object` — params field is the wrong JSON shape.
    assert!(is_param_validation_error(
        "invalid params: expected object or null, got array"
    ));
}

#[test]
fn is_param_validation_error_does_not_match_unrelated_errors() {
    // Handler-side / network / auth failures must still be reported.
    assert!(!is_param_validation_error(
        "backend returned 401 Unauthorized"
    ));
    assert!(!is_param_validation_error("network timeout"));
    assert!(!is_param_validation_error(
        "config.update_model_settings: store write failed"
    ));
    // Empty and substring-only matches don't qualify either.
    assert!(!is_param_validation_error(""));
    assert!(!is_param_validation_error(
        "rpc failed: unknown param 'x' for ns.fn"
    ));
}

#[test]
fn is_session_expired_error_matches_missing_backend_session_token() {
    // Composio / web search / billing / team / webhooks / referral all surface
    // a "no backend session token" variant when the auth profile is gone. Each
    // of these should funnel into the auto-cleanup path instead of being
    // reported to Sentry as a fresh error on every 5 s poll.
    assert!(is_session_expired_error(
        "composio unavailable: no backend session token. Sign in first (auth_store_session)."
    ));
    assert!(is_session_expired_error(
        "no backend session token; run auth_store_session first"
    ));
    assert!(is_session_expired_error(
        "Web search unavailable: no backend session token. Sign in first so the server can proxy search."
    ));
    // Case-insensitive match — the helper lowercases first.
    assert!(is_session_expired_error("NO BACKEND SESSION TOKEN"));
}

#[tokio::test(flavor = "current_thread")]
async fn structured_rpc_error_envelope_passes_through_generic_dispatch() {
    // The transport layer must surface any controller-emitted
    // `StructuredRpcError` payload without inspecting the method name —
    // this is what makes the boundary domain-agnostic. We register a
    // throwaway method-name on a thread-scoped op and confirm the
    // wire-shape carries the `kind`/`thread_id` data verbatim.
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::Json;

    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let _env = EnvVarGuard::set_many(vec![(
        "OPENHUMAN_WORKSPACE",
        workspace.path().as_os_str().to_os_string(),
    )]);

    let stale_thread_request = crate::core::types::RpcRequest {
        jsonrpc: "2.0".to_string(),
        id: json!(7),
        method: "openhuman.threads_generate_title".to_string(),
        params: json!({ "thread_id": "thread-ghost" }),
    };
    let response = rpc_handler(State(default_state()), Json(stale_thread_request)).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json response");
    assert_eq!(body["error"]["data"]["kind"], "ThreadNotFound");
    assert_eq!(body["error"]["data"]["thread_id"], "thread-ghost");
    // The structured-error message must be human-readable on the wire —
    // never the encoded sentinel envelope.
    let message = body["error"]["message"].as_str().expect("error message");
    assert!(
        !message.contains("__OPENHUMAN_STRUCTURED_RPC_ERROR_V1__"),
        "sentinel-encoded envelope leaked onto the wire: {message}"
    );
    assert!(message.contains("thread-ghost"));
}

#[tokio::test(flavor = "current_thread")]
async fn thread_not_found_rpc_error_does_not_report_to_sentry() {
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::Json;
    use sentry::test::TestTransport;
    use tracing::Level;
    use tracing_subscriber::layer::SubscriberExt;

    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let _env = EnvVarGuard::set_many(vec![(
        "OPENHUMAN_WORKSPACE",
        workspace.path().as_os_str().to_os_string(),
    )]);

    let transport = TestTransport::new();
    let sentry_options = sentry::ClientOptions {
        dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
        transport: Some(Arc::new(transport.clone())),
        ..Default::default()
    };
    let sentry_hub = Arc::new(sentry::Hub::new(
        Some(Arc::new(sentry_options.into())),
        Arc::new(Default::default()),
    ));
    let _sentry_guard = sentry::HubSwitchGuard::new(sentry_hub);

    let subscriber = tracing_subscriber::registry().with(
        sentry::integrations::tracing::layer().event_filter(|metadata| match *metadata.level() {
            Level::ERROR => sentry::integrations::tracing::EventFilter::Event,
            Level::WARN | Level::INFO => sentry::integrations::tracing::EventFilter::Breadcrumb,
            _ => sentry::integrations::tracing::EventFilter::Ignore,
        }),
    );
    let _subscriber_guard = tracing::subscriber::set_default(subscriber);

    let stale_thread_request = crate::core::types::RpcRequest {
        jsonrpc: "2.0".to_string(),
        id: json!(1),
        method: "openhuman.threads_message_append".to_string(),
        params: json!({
            "thread_id": "thread-missing",
            "message": {
                "id": "msg-1",
                "content": "hello",
                "type": "text",
                "extraMetadata": {},
                "sender": "user",
                "createdAt": "2026-01-01T00:00:00Z"
            }
        }),
    };
    let response = rpc_handler(State(default_state()), Json(stale_thread_request)).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json response");
    assert_eq!(body["error"]["data"]["kind"], "ThreadNotFound");
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "ThreadNotFound should not reach Sentry"
    );

    let unrelated_error_request = crate::core::types::RpcRequest {
        jsonrpc: "2.0".to_string(),
        id: json!(2),
        method: "core.not_a_real_method".to_string(),
        params: json!({}),
    };
    let response = rpc_handler(State(default_state()), Json(unrelated_error_request)).await;
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json response");
    assert_eq!(body["error"]["data"], serde_json::Value::Null);

    let events = transport.fetch_and_clear_events();
    assert_eq!(
        events.len(),
        1,
        "unrelated RPC errors should still reach Sentry"
    );
    assert_eq!(
        events[0].tags.get("domain").map(String::as_str),
        Some("rpc")
    );
    assert_eq!(
        events[0].tags.get("operation").map(String::as_str),
        Some("invoke_method")
    );
    assert_eq!(
        events[0].tags.get("method").map(String::as_str),
        Some("core.not_a_real_method")
    );
}

#[test]
fn is_session_expired_error_matches_session_jwt_required() {
    // Regression: Sentry issue 7472592145.
    // A prior 401 clears the stored JWT; the very next RPC call (e.g.
    // channels_telegram_login_start) finds no token and returns "session JWT
    // required; complete login first". This is the same auth-boundary condition
    // and must not be reported to Sentry.
    assert!(is_session_expired_error(
        "session JWT required; complete login first"
    ));
    assert!(is_session_expired_error(
        "session JWT required; complete login and store_session first"
    ));
    assert!(is_session_expired_error("session JWT required"));
    // Case-insensitive.
    assert!(is_session_expired_error("SESSION JWT REQUIRED"));
}

#[test]
fn escape_html_escapes_all_special_chars() {
    let raw = r#"<script>alert("x&y'z")</script>"#;
    let escaped = escape_html(raw);
    assert!(!escaped.contains('<'));
    assert!(!escaped.contains('>'));
    assert!(!escaped.contains('"'));
    assert!(!escaped.contains('\''));
    assert!(escaped.contains("&lt;"));
    assert!(escaped.contains("&gt;"));
    assert!(escaped.contains("&quot;"));
    assert!(escaped.contains("&#x27;"));
    // `&` must be escaped first so later substitutions don't double-encode.
    assert!(escaped.contains("&amp;y"));
}

#[test]
fn escape_html_is_noop_for_safe_text() {
    assert_eq!(escape_html("safe text 123"), "safe text 123");
    assert_eq!(escape_html(""), "");
}

// --- invoke_method parameter-shape errors ---------------------------------

#[tokio::test]
async fn invoke_method_rejects_array_params_for_registered_method() {
    // Registered controllers expect named-argument style (JSON object).
    // Passing an array must fail with a clear "invalid params" error
    // instead of silently calling the handler with no args.
    let err = invoke_method(
        default_state(),
        "openhuman.health_snapshot",
        json!([1, 2, 3]),
    )
    .await
    .expect_err("array params should be rejected");
    assert!(err.contains("invalid params"));
    assert!(err.contains("array"));
}

#[tokio::test]
async fn invoke_method_rejects_string_params_for_registered_method() {
    let err = invoke_method(default_state(), "openhuman.health_snapshot", json!("oops"))
        .await
        .expect_err("string params should be rejected");
    assert!(err.contains("invalid params"));
    assert!(err.contains("string"));
}

#[tokio::test]
async fn invoke_method_accepts_null_params_for_registered_method() {
    // JSON-RPC 2.0 allows omitting params; null must be treated like {}.
    let result = invoke_method(default_state(), "openhuman.health_snapshot", json!(null)).await;
    // Call should succeed or fail for domain reasons — but must NOT
    // fail with the "invalid params" shape error.
    if let Err(e) = result {
        assert!(
            !e.contains("invalid params"),
            "null should be accepted as empty object, got: {e}"
        );
    }
}

#[tokio::test]
async fn invoke_method_unknown_method_returns_unknown_error() {
    let err = invoke_method(default_state(), "openhuman.totally_made_up_xyz", json!({}))
        .await
        .expect_err("unknown methods must error");
    assert!(err.contains("unknown method"));
}

#[tokio::test]
async fn invoke_method_core_ping_via_tier1() {
    // core.* methods aren't in the registry; they route through tier 1.
    let result = invoke_method(default_state(), "core.ping", json!({}))
        .await
        .expect("core.ping should succeed via tier 1");
    assert_eq!(result, json!({ "ok": true }));
}

#[tokio::test]
async fn invoke_method_core_version_via_tier1_reflects_state() {
    let state = super::AppState {
        core_version: "0.0.1-abc".into(),
    };
    let result = invoke_method(state, "core.version", json!({}))
        .await
        .expect("core.version should succeed");
    assert_eq!(result, json!({ "version": "0.0.1-abc" }));
}
