use super::http::router;
use super::invoker::{AgentInvoker, InvocationOutput};
use super::store::JobStore;
use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

struct EchoInvoker;

#[async_trait]
impl AgentInvoker for EchoInvoker {
    async fn invoke(
        &self,
        thread_id: Option<&str>,
        message: &str,
    ) -> Result<InvocationOutput, String> {
        Ok(InvocationOutput {
            assistant_message: format!("echo: {message}"),
            thread_id: thread_id.unwrap_or("t-new").to_string(),
        })
    }
}

fn make_app() -> (axum::Router, JobStore) {
    let store = JobStore::new(Duration::from_secs(3600));
    let invoker: Arc<dyn AgentInvoker> = Arc::new(EchoInvoker);
    let app = router(store.clone(), invoker, Duration::from_secs(5));
    (app, store)
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn post_run_with_valid_body_returns_202_with_job_id() {
    let (app, _store) = make_app();
    let req = Request::builder()
        .method("POST")
        .uri("/run")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "payload": { "message": "hi" } }).to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = body_json(resp).await;
    let id = body.get("job_id").and_then(|v| v.as_str()).unwrap();
    assert_eq!(id.len(), 36);
}

#[tokio::test]
async fn post_run_missing_payload_returns_400() {
    let (app, _store) = make_app();
    let req = Request::builder()
        .method("POST")
        .uri("/run")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn post_run_empty_message_returns_400() {
    let (app, _store) = make_app();
    let req = Request::builder()
        .method("POST")
        .uri("/run")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "payload": { "message": "" } }).to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert!(body.get("error").is_some());
}

#[tokio::test]
async fn post_run_malformed_json_returns_400() {
    let (app, _store) = make_app();
    let req = Request::builder()
        .method("POST")
        .uri("/run")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
