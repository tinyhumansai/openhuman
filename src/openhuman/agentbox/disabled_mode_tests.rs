//! Verifies that with `OPENHUMAN_AGENTBOX_MODE` unset (the desktop default),
//! the core HTTP router does NOT expose `/run` or `/jobs/{id}`.
//!
//! Env vars are process-global, so this test holds the AgentBox test env lock
//! while it clears `OPENHUMAN_AGENTBOX_MODE` and builds the router.
//! Authoritative coverage of the disabled-mode contract lives in the E2E test
//! (Task 12) which boots a fresh process.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn run_route_absent_when_mode_off() {
    let router = {
        let _lock = super::test_support::test_env_lock();
        // Ensure flag is OFF while the router is built.
        std::env::remove_var("OPENHUMAN_AGENTBOX_MODE");
        crate::core::jsonrpc::build_core_http_router(false)
    };
    let req = Request::builder()
        .method("POST")
        .uri("/run")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"payload":{"message":"x"}}"#))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    // Router's fallback returns 404 for unmounted routes.
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
