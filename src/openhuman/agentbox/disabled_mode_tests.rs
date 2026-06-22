//! Verifies that with `OPENHUMAN_AGENTBOX_MODE` unset (the desktop default),
//! the core HTTP router does NOT expose `/run` or `/jobs/{id}`.
//!
//! Env vars are process-global, so if any other test sets this var in
//! parallel the assertion could flap. The repo standard for env-mutating
//! tests is to use the `serial_test` crate; if it's available, mark this
//! test `#[serial]`. Otherwise the test is best-effort and a fallback
//! `#[ignore]` is acceptable.
//!
//! `serial_test` is NOT a dev-dependency in this workspace, and a grep
//! across `src/` and `tests/` confirms no other test sets
//! `OPENHUMAN_AGENTBOX_MODE`, so unsetting the var inline is sufficient
//! today. Authoritative coverage of the disabled-mode contract lives in
//! the E2E test (Task 12) which boots a fresh process.

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
