use super::super::build_url;
use super::super::types::{CoreAlertSeverity, CoreAlertStatus, CoreAlertStatusFilter};
use super::*;
use crate::rpc::StructuredRpcError;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use reqwest::StatusCode;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

#[derive(Debug, Clone)]
struct CapturedRequest {
    method: Method,
    path_and_query: String,
    authorization: Option<String>,
    actor: Option<String>,
    idempotency_key: Option<String>,
    body: Value,
}

type Requests = Arc<Mutex<Vec<CapturedRequest>>>;

const TEST_ALERT_ID: &str = "11111111-1111-4111-8111-111111111111";
const TEST_ACTOR_USER_ID: &str = "22222222-2222-4222-8222-222222222222";
const TEST_ACTION_REQUEST_ID: &str = "33333333-3333-4333-8333-333333333333";

fn test_config(tmp: &TempDir, base: String) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        youpet: crate::openhuman::config::YouPetConfig {
            core_api_url: base,
            service_token: Some("svc-token".into()),
            workbench_actor_id: "operator-workbench".into(),
            operator_user_id: Some(TEST_ACTOR_USER_ID.into()),
            tenant_id: Some("20000000-0000-0000-0000-000000000001".into()),
        },
        ..Config::default()
    }
}

fn sample_action_request_envelope() -> Value {
    json!({
        "action_request": {
            "id": TEST_ACTION_REQUEST_ID,
            "approval": { "state": "pending" },
            "execution": { "state": "not_started" }
        },
        "row_version": 1,
        "id": TEST_ACTION_REQUEST_ID,
        "tenant_id": "20000000-0000-0000-0000-000000000001",
        "approval_state": "pending",
        "execution_state": "not_started",
        "policy_outcome": "require_approval",
        "correlation_id": "corr_test",
        "created_at": "2026-08-08T12:00:00Z",
        "updated_at": "2026-08-08T12:00:00Z"
    })
}

async fn spawn_mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    format!("http://127.0.0.1:{}", addr.port())
}

async fn capture(
    State(requests): State<Requests>,
    method: Method,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let parsed_body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    requests.lock().unwrap().push(CapturedRequest {
        method,
        path_and_query: uri
            .path_and_query()
            .map(|v| v.as_str().to_string())
            .unwrap_or_else(|| uri.path().to_string()),
        authorization: headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        actor: headers
            .get("x-actor-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        idempotency_key: headers
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        body: parsed_body,
    });
    axum::Json(json!({
        "id": TEST_ALERT_ID,
        "alert_type": "missed_checkin",
        "severity": "high",
        "related_type": "task_instance",
        "related_id": "task-1",
        "status": "acknowledged",
        "created_at": "2026-06-01T00:00:00Z",
        "future_field": "tolerated"
    }))
}

async fn capture_trace(
    State(requests): State<Requests>,
    method: Method,
    uri: axum::http::Uri,
    headers: HeaderMap,
) -> impl IntoResponse {
    requests.lock().unwrap().push(CapturedRequest {
        method,
        path_and_query: uri
            .path_and_query()
            .map(|v| v.as_str().to_string())
            .unwrap_or_else(|| uri.path().to_string()),
        authorization: headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        actor: headers
            .get("x-actor-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        idempotency_key: headers
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        body: Value::Null,
    });
    axum::Json(json!({
        "alert_id": TEST_ALERT_ID,
        "workflow": {
            "type": "health_plan",
            "id": "plan-1",
            "task_id": "task-1",
            "openclaw_flow_id": "flow-plan-1"
        },
        "partial": true,
        "warnings": [{
            "code": "trace_truncated",
            "message": "Trace limited to 50 entries",
            "source": "event_outbox"
        }],
        "entries": [{
            "id": format!("alert:{TEST_ALERT_ID}"),
            "occurred_at": "2026-06-01T00:00:00Z",
            "kind": "alert_created",
            "source": "alerts",
            "title": "Alert created",
            "detail": null,
            "actor": null,
            "related_type": "task_instance",
            "related_id": "task-1",
            "severity": "high",
            "metadata": { "alert_type": "missed_checkin" }
        }]
    }))
}

#[path = "ops_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "ops_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "ops_tests_part_03_tests.rs"]
mod part_03_tests;
