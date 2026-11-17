//! `/socket.io` on a core started with Socket.IO disabled (`--jsonrpc-only`)
//! answers with a specific 503, not the bearer middleware's 401 (#5656).

use std::sync::Once;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::{build_core_http_router, SOCKETIO_DISABLED_ERROR};
use crate::core_host::core::auth::CORE_TOKEN_ENV_VAR;

fn ensure_rpc_token() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // SAFETY: test-only init serialized via `Once`.
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, "socketio-disabled-tests-token") };
        let tmp = tempfile::tempdir().expect("tempdir");
        crate::core_host::core::auth::init_rpc_token(tmp.path()).expect("init rpc token");
    });
}

async fn get(router: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = router
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

#[tokio::test]
async fn disabled_socketio_answers_503_with_cause_not_401() {
    ensure_rpc_token();
    for uri in [
        "/socket.io/?EIO=4&transport=polling",
        "/socket.io/",
        "/socket.io",
    ] {
        let (status, body) = get(build_core_http_router(false), uri).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
        assert_eq!(body["error"], SOCKETIO_DISABLED_ERROR, "{uri}");
        assert!(
            body["message"].as_str().unwrap().contains("--jsonrpc-only"),
            "{uri}"
        );
    }
}

#[tokio::test]
async fn disabled_socketio_does_not_change_other_routes() {
    ensure_rpc_token();
    // A protected route still demands the bearer.
    let resp = build_core_http_router(false)
        .oneshot(Request::post("/rpc").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    // A look-alike prefix is not treated as Socket.IO.
    let (status, body) = get(build_core_http_router(false), "/socket.iox").await;
    assert_ne!(body["error"], SOCKETIO_DISABLED_ERROR);
    assert_ne!(status, StatusCode::SERVICE_UNAVAILABLE);
}
