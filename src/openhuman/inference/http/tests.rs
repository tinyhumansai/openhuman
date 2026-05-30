//! Integration tests for the OpenAI-compatible `/v1` HTTP endpoint.
//!
//! These tests spin up an in-process axum router (no network), send
//! crafted HTTP requests via `tower::ServiceExt::oneshot`, and assert on
//! the response status codes.
//!
//! A running inference backend is NOT required — the tests exercise the
//! routing and auth-middleware layers only.

use std::sync::Once;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use tower::ServiceExt;

use crate::core::auth::CORE_TOKEN_ENV_VAR;
use crate::core::jsonrpc::build_core_http_router;
use crate::openhuman::inference::http::server::strip_temperature_suffix;

const TEST_RPC_TOKEN: &str = "inference-http-tests-token";

/// Initialize the per-process RPC bearer token exactly once, so that the
/// auth middleware can answer 401 instead of 500 ("auth subsystem not
/// initialized") in tests that don't spin up a real core.
///
/// Returns the bearer that is actually installed in `RPC_TOKEN` after init.
/// This may not equal `TEST_RPC_TOKEN`: the `RPC_TOKEN` `OnceLock` is shared
/// across the whole test binary, and a sibling test (e.g. the in-memory-seed
/// regression in `src/core/auth.rs`) may have populated it first. Token-
/// agnostic callers should treat the return value as the source of truth.
fn ensure_test_rpc_auth() -> String {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // SAFETY: test-only init; we serialize via `Once`, and live_routing_e2e
        // uses its own env lock + a different token value so the two test
        // binaries don't collide (they run in separate processes anyway).
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN) };
        let tmp = tempfile::tempdir().expect("tempdir for token file");
        crate::core::auth::init_rpc_token(tmp.path()).expect("init rpc auth token for http tests");
    });
    crate::core::auth::get_rpc_token()
        .expect("rpc bearer must be installed after ensure_test_rpc_auth")
        .to_string()
}

/// Build the test router (Socket.IO disabled — no real runtime needed).
fn test_router() -> axum::Router {
    let _ = ensure_test_rpc_auth();
    build_core_http_router(false)
}

/// Convenience: dispatch a single request through the in-process router.
async fn dispatch(req: Request<Body>) -> axum::response::Response {
    test_router().oneshot(req).await.unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Requests to `POST /v1/chat/completions` without any `Authorization` header
/// must be rejected with `401 Unauthorized`.
#[tokio::test]
async fn test_chat_completions_no_bearer_returns_401() {
    let body = serde_json::json!({
        "model": "ollama:llama3",
        "messages": [{ "role": "user", "content": "hello" }]
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();

    let resp = dispatch(req).await;
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "POST /v1/chat/completions without bearer must return 401"
    );
}

/// Requests to `GET /v1/models` without any `Authorization` header must be
/// rejected with `401 Unauthorized`.
#[tokio::test]
async fn test_models_no_bearer_returns_401() {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/models")
        .body(Body::empty())
        .unwrap();

    let resp = dispatch(req).await;
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "GET /v1/models without bearer must return 401"
    );
}

/// A request with a bearer token must not be rejected as 401/403. The actual
/// response code depends on whether a live inference backend is running; the
/// test only asserts that auth passed.
#[tokio::test]
async fn test_chat_completions_with_bearer_not_rejected_as_auth_error() {
    // Read whatever token is actually installed in `RPC_TOKEN`. A sibling
    // test in `src/core/auth.rs` may seed the `OnceLock` first with its
    // own value, so we cannot rely on `TEST_RPC_TOKEN` matching.
    let token = ensure_test_rpc_auth();

    let body = serde_json::json!({
        "model": "ollama:llama3",
        "messages": [{ "role": "user", "content": "ping" }],
        "stream": false
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();

    let resp = dispatch(req).await;
    let status = resp.status();
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "401 must not fire when bearer is present"
    );
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "403 must not fire when bearer is present"
    );
}

#[test]
fn strip_temperature_suffix_only_removes_numeric_suffixes() {
    assert_eq!(strip_temperature_suffix("gpt-4o@0.7"), "gpt-4o");
    assert_eq!(strip_temperature_suffix("llama3.1:8b@1"), "llama3.1:8b");
    assert_eq!(strip_temperature_suffix("gpt@beta"), "gpt@beta");
}
