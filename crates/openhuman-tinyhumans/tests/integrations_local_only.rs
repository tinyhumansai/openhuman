//! LocalOnly privacy enforcement through the integrations client's public verbs
//! (privacy epic S7, #4441).
//!
//! These tests install a process-global live policy, so they live in their own
//! test binary and serialize on one lock: no other test shares the policy.

use std::sync::Arc;

use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{http::StatusCode, Json, Router};
use openhuman_embed::__host::config::PrivacyMode;
use openhuman_embed::__host::security::live_policy;
use openhuman_embed::__host::security::SecurityPolicy;
use openhuman_tinyhumans::integrations::IntegrationClient;
use serde_json::json;

static POLICY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Installs a live policy carrying `mode`; restores `Standard` on drop.
struct PrivacyScope;

impl PrivacyScope {
    fn set(mode: PrivacyMode) -> Self {
        install(mode);
        Self
    }
}

impl Drop for PrivacyScope {
    fn drop(&mut self) {
        install(PrivacyMode::Standard);
    }
}

fn install(mode: PrivacyMode) {
    let policy = SecurityPolicy {
        privacy_mode: mode,
        ..SecurityPolicy::default()
    };
    live_policy::install(Arc::new(policy), Default::default(), Default::default());
}

async fn start_mock_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn client_for(base: String) -> IntegrationClient {
    IntegrationClient::new(base, "test-token".into())
}

/// Privacy epic S7 (#4441): the LocalOnly gate must fire through the PUBLIC verb
/// methods, not only via the `enforce_backend_egress` helper. Each of the six
/// verbs (`post`/`get`/`patch`/`delete`/`upload_multipart`/`get_bytes`) runs the
/// gate synchronously on first poll — before any `.await` — so a user-data path
/// is refused before the request leaves the device. The client points at an
/// unreachable address, so a leaked call would surface a transport error, not
/// the local-only message; asserting the policy string therefore proves the
/// short-circuit fired end-to-end through the verb, not just the helper.
#[tokio::test]
async fn verb_methods_block_user_data_egress_under_local_only() {
    let _serial = POLICY_LOCK.lock().await;
    let _mode = PrivacyScope::set(PrivacyMode::LocalOnly);
    let client = client_for("http://127.0.0.1:0".into());
    let path = "/agent-integrations/composio/execute"; // user-data egress (blocked)
    let body = json!({ "arguments": { "secret": "leak-me" } });

    let assert_blocked = |verb: &str, msg: String| {
        assert!(
            msg.contains("Local-only privacy mode is active"),
            "{verb}: expected a local-only block before network, got: {msg}"
        );
    };

    assert_blocked(
        "post",
        client
            .post::<serde_json::Value>(&Default::default(), path, &body)
            .await
            .unwrap_err()
            .to_string(),
    );
    assert_blocked(
        "get",
        client
            .get::<serde_json::Value>(&Default::default(), path)
            .await
            .unwrap_err()
            .to_string(),
    );
    assert_blocked(
        "patch",
        client
            .patch::<serde_json::Value>(&Default::default(), path, &body)
            .await
            .unwrap_err()
            .to_string(),
    );
    assert_blocked(
        "delete",
        client
            .delete::<serde_json::Value>(&Default::default(), path)
            .await
            .unwrap_err()
            .to_string(),
    );
    assert_blocked(
        "upload_multipart",
        client
            .upload_multipart::<serde_json::Value>(
                &Default::default(),
                path,
                reqwest::multipart::Form::new(),
            )
            .await
            .unwrap_err()
            .to_string(),
    );
    assert_blocked(
        "get_bytes",
        client
            .get_bytes(&Default::default(), path)
            .await
            .unwrap_err()
            .to_string(),
    );
}

#[tokio::test]
async fn local_only_rejects_user_data_verb_before_transport() {
    // End-to-end proof (privacy epic S7, #4441) that a PUBLIC verb
    // (`post`/`get`) — not just the private `enforce_backend_egress` helper —
    // honours LocalOnly and refuses a user-data backend call BEFORE it hits
    // transport. The mock 403s on every route it actually receives, so if the
    // gate short-circuits first the error is the policy message, never the
    // mock body.
    let _serial = POLICY_LOCK.lock().await;
    let app = Router::new()
        .route(
            "/agent-integrations/composio/execute",
            post(|| async {
                (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "success": false, "error": "mock must not be reached" })),
                )
                    .into_response()
            }),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "success": false, "error": "control-plane reached transport" })),
                )
                    .into_response()
            }),
        );
    let base = start_mock_backend(app).await;
    let client = client_for(base);

    let _mode = PrivacyScope::set(PrivacyMode::LocalOnly);

    // (1) User-data verb call is refused by the gate BEFORE transport — the
    //     error carries the policy message and NOT the mock's 403 body.
    let err = client
        .post::<serde_json::Value>(
            &Default::default(),
            "/agent-integrations/composio/execute",
            &json!({ "tool": "GMAIL_FETCH_EMAILS" }),
        )
        .await
        .expect_err("LocalOnly must block the user-data POST before transport");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("Local-only privacy mode is active"),
        "expected policy block, got: {msg}"
    );
    assert!(
        !msg.contains("mock must not be reached"),
        "gate must short-circuit before the request reaches the mock: {msg}"
    );

    // (2) A control-plane verb call under the SAME LocalOnly scope passes the
    //     gate and DOES reach transport (surfaces the mock's 403) — proving the
    //     gate is selective, not a blanket network kill.
    let err = client
        .get::<serde_json::Value>(
            &Default::default(),
            "/agent-integrations/composio/connections",
        )
        .await
        .expect_err("control-plane reaches transport and surfaces the mock 403");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("control-plane reached transport"),
        "control-plane must reach transport under LocalOnly, got: {msg}"
    );
}

#[tokio::test]
async fn standard_mode_lets_a_user_data_verb_reach_the_transport() {
    let _serial = POLICY_LOCK.lock().await;
    let _mode = PrivacyScope::set(PrivacyMode::Standard);
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|| async { Json(json!({ "success": true, "data": { "ok": true } })).into_response() }),
    );
    let client = client_for(start_mock_backend(app).await);
    let value = client
        .post::<serde_json::Value>(
            &Default::default(),
            "/agent-integrations/composio/execute",
            &json!({ "tool": "GMAIL_FETCH_EMAILS" }),
        )
        .await
        .expect("Standard privacy mode allows the call");
    assert_eq!(value, json!({ "ok": true }));
}
