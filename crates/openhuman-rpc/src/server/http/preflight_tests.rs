use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::build_core_http_router_with_preflight;

#[tokio::test]
async fn startup_preflight_serves_health_and_releases_the_normal_router_afterward() {
    let gate = crate::server::auth::PreflightGate::new_active();
    let router = build_core_http_router_with_preflight(false, gate.clone());

    let health = router
        .clone()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);

    let blocked = router
        .clone()
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::SERVICE_UNAVAILABLE);

    gate.open();
    let normal = router
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(normal.status(), StatusCode::OK);
}
