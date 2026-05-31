//! Round24 raw coverage for broad tools/composio cold branches.
//!
//! All HTTP traffic stays on loopback mocks. These tests drive public tool
//! APIs so coverage lands on the same paths an agent/tool call uses.

use std::sync::{Arc, Mutex};

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};

use openhuman_core::openhuman::config::{PolymarketClobCredentials, PolymarketConfig};
use openhuman_core::openhuman::security::{AutonomyLevel, SecurityPolicy};
use openhuman_core::openhuman::tools::{ComposioTool, PolymarketTool, Tool};

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: Method,
    path: String,
    query: String,
    body: Value,
}

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    market_failures_left: Arc<Mutex<usize>>,
}

#[tokio::test]
async fn round24_composio_direct_covers_v3_v2_fallbacks_and_account_shapes() {
    let state = MockState::default();
    let base = start_loopback(
        Router::new()
            .fallback(any(composio_handler))
            .with_state(state.clone()),
    )
    .await;
    let tool = ComposioTool::new_with_base_urls_for_loopback(
        "  ck_round24  ",
        Some(" entity-round24 "),
        Arc::new(SecurityPolicy::default()),
        format!("{base}/api/v2"),
        format!("{base}/api/v3"),
    )
    .expect("loopback composio tool");

    let connected = tool
        .list_connected_accounts()
        .await
        .expect("connected accounts");
    assert_eq!(connected.len(), 3, "blank id row should be dropped");
    assert_eq!(
        connected
            .iter()
            .map(|account| account.toolkit_slug().unwrap())
            .collect::<Vec<_>>(),
        vec!["gmail", "github", "slack"]
    );

    let execute = tool
        .execute(json!({
            "action": "execute",
            "tool_slug": "GMAIL_SEND_EMAIL",
            "params": { "to": "a@example.test" },
            "connected_account_id": "conn-secret"
        }))
        .await
        .expect("execute falls back to v2");
    assert!(!execute.is_error, "{}", execute.output());
    assert!(execute.output().contains("v2-fallback-ok"));

    let connect = tool
        .execute(json!({
            "action": "connect",
            "app": "gmail"
        }))
        .await
        .expect("connect resolves auth config");
    assert!(!connect.is_error, "{}", connect.output());
    assert!(connect
        .output()
        .contains("https://connect.example.test/round24"));

    let requests = state.requests.lock().expect("requests").clone();
    let v3_execute = requests
        .iter()
        .find(|request| {
            request.method == Method::POST
                && request.path == "/api/v3/tools/gmail-send-email/execute"
        })
        .expect("v3 execute request");
    assert_eq!(v3_execute.body["connected_account_id"], "conn-secret");
    assert_eq!(v3_execute.body["user_id"], "entity-round24");

    let v2_execute = requests
        .iter()
        .find(|request| {
            request.method == Method::POST
                && request.path == "/api/v2/actions/GMAIL_SEND_EMAIL/execute"
        })
        .expect("v2 execute fallback request");
    assert_eq!(v2_execute.body["entityId"], "entity-round24");

    assert!(requests.iter().any(|request| {
        request.method == Method::GET
            && request.path == "/api/v3/auth_configs"
            && request.query.contains("toolkit_slug=gmail")
            && request.query.contains("show_disabled=true")
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::POST
            && request.path == "/api/v3/connected_accounts/link"
            && request.body["auth_config_id"] == "auth-enabled"
            && request.body["user_id"] == "entity-round24"
    }));
}

#[tokio::test]
async fn round24_polymarket_covers_retries_errors_and_signed_read_paths() {
    let state = MockState::default();
    *state.market_failures_left.lock().expect("failure counter") = 1;
    let base = start_loopback(
        Router::new()
            .fallback(any(polymarket_handler))
            .with_state(state.clone()),
    )
    .await;
    let tool = polymarket_tool(&base);

    let markets = tool
        .execute(json!({
            "action": "list_markets",
            "slug": "will-it-rain",
            "event_id": "evt-1",
            "limit": 2,
            "offset": 1,
            "cursor": "next",
            "active": true,
            "closed": false,
            "tag": "weather"
        }))
        .await
        .expect("market list after retry");
    assert!(!markets.is_error, "{}", markets.output());
    assert!(markets.output().contains("will-it-rain"));

    let missing_slug = tool
        .execute(json!({ "action": "get_market", "slug": "missing-market" }))
        .await
        .expect("missing slug result");
    assert!(missing_slug.is_error);
    assert!(missing_slug
        .output()
        .contains("No Polymarket market found for slug"));

    let bad_side = tool
        .execute(json!({
            "action": "get_price",
            "token_id": "token-1",
            "side": "hold"
        }))
        .await
        .expect("bad side result");
    assert!(bad_side.is_error);
    assert!(bad_side.output().contains("Invalid 'side'"));

    let balance = tool
        .execute(json!({
            "action": "get_balance",
            "user": "0x0000000000000000000000000000000000000001"
        }))
        .await
        .expect("signed balance read");
    assert!(!balance.is_error, "{}", balance.output());
    assert!(balance.output().contains("42.00"));

    let allowance = tool
        .execute(json!({
            "action": "get_usdc_allowance",
            "user": "0x0000000000000000000000000000000000000001"
        }))
        .await
        .expect("allowance read");
    assert!(!allowance.is_error, "{}", allowance.output());
    let allowance_json: Value = serde_json::from_str(&allowance.output()).expect("allowance json");
    assert_eq!(allowance_json["allowance"], "16");

    let empty_orderbook_token = tool
        .execute(json!({ "action": "get_orderbook", "token_id": "   " }))
        .await
        .expect("empty orderbook token result");
    assert!(empty_orderbook_token.is_error);
    assert!(empty_orderbook_token
        .output()
        .contains("'token_id' cannot be empty"));

    let requests = state.requests.lock().expect("requests").clone();
    let market_gets = requests
        .iter()
        .filter(|request| request.method == Method::GET && request.path == "/markets")
        .count();
    assert!(
        market_gets >= 3,
        "429 retry plus slug lookup should hit /markets"
    );
    assert!(requests.iter().any(|request| {
        request.method == Method::GET
            && request.path == "/data/balance"
            && request.query.contains("token=usdce")
            && request
                .query
                .contains("user=0x0000000000000000000000000000000000000001")
    }));
    assert!(requests.iter().any(|request| {
        request.method == Method::POST
            && request.path == "/"
            && request.body["method"] == "eth_call"
    }));
}

async fn start_loopback(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("loopback server");
    });
    format!("http://127.0.0.1:{}", addr.port())
}

async fn composio_handler(State(state): State<MockState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let method = parts.method;
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().unwrap_or_default().to_string();
    let body_bytes = to_bytes(body, 1024 * 1024).await.expect("body bytes");
    let body_json = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or_else(|_| Value::Null)
    };

    state
        .requests
        .lock()
        .expect("requests")
        .push(RecordedRequest {
            method: method.clone(),
            path: path.clone(),
            query,
            body: body_json,
        });

    match (method, path.as_str()) {
        (Method::GET, "/api/v3/connected_accounts") => Json(json!({
            "items": [
                { "id": "conn-gmail", "status": "ACTIVE", "toolkit": { "slug": "gmail" }, "created_at": "2026-05-30T00:00:00Z" },
                { "id": "", "status": "ACTIVE", "toolkit": "dropme" },
                { "id": "conn-github", "status": "INITIATED", "toolkit": "github", "createdAt": "2026-05-30T00:00:00Z" },
                { "id": "conn-slack", "status": "ACTIVE", "appName": "slack" }
            ]
        }))
        .into_response(),
        (Method::POST, "/api/v3/tools/gmail-send-email/execute") => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": "bad connected_account_id conn-secret for entity_id entity-round24"
                }
            })),
        )
            .into_response(),
        (Method::POST, "/api/v2/actions/GMAIL_SEND_EMAIL/execute") => Json(json!({
            "successful": true,
            "data": { "message": "v2-fallback-ok" }
        }))
        .into_response(),
        (Method::GET, "/api/v3/auth_configs") => Json(json!({
            "items": [
                { "id": "auth-disabled", "status": "disabled", "enabled": false },
                { "id": "auth-enabled", "status": "enabled", "enabled": true }
            ]
        }))
        .into_response(),
        (Method::POST, "/api/v3/connected_accounts/link") => Json(json!({
            "data": {
                "redirect_url": "https://connect.example.test/round24"
            }
        }))
        .into_response(),
        _ => (StatusCode::NOT_FOUND, Json(json!({ "message": "not found" }))).into_response(),
    }
}

fn polymarket_tool(base: &str) -> PolymarketTool {
    let config = PolymarketConfig {
        enabled: true,
        gamma_base_url: base.to_string(),
        clob_base_url: base.to_string(),
        polygon_rpc_url: base.to_string(),
        timeout_secs: 15,
        eoa_address: Some("0x0000000000000000000000000000000000000001".to_string()),
        usdc_contract: "0x0000000000000000000000000000000000000002".to_string(),
        clob_exchange_contract: "0x0000000000000000000000000000000000000003".to_string(),
        derived_clob_credentials: Some(PolymarketClobCredentials {
            api_key: "round24-key".to_string(),
            secret: "cm91bmQyNC1zZWNyZXQ=".to_string(),
            passphrase: "round24-passphrase".to_string(),
        }),
        ..PolymarketConfig::default()
    };
    PolymarketTool::new(
        &config,
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            ..SecurityPolicy::default()
        }),
    )
}

async fn polymarket_handler(State(state): State<MockState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let method = parts.method;
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().unwrap_or_default().to_string();
    let body_bytes = to_bytes(body, 1024 * 1024).await.expect("body bytes");
    let body_json = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or_else(|_| Value::Null)
    };

    state
        .requests
        .lock()
        .expect("requests")
        .push(RecordedRequest {
            method: method.clone(),
            path: path.clone(),
            query: query.clone(),
            body: body_json,
        });

    match (method, path.as_str()) {
        (Method::GET, "/markets") if query.contains("slug=missing-market") => {
            Json(json!([])).into_response()
        }
        (Method::GET, "/markets") => {
            let mut failures = state
                .market_failures_left
                .lock()
                .expect("market failure counter");
            if *failures > 0 {
                *failures -= 1;
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({ "error": "try again" })),
                )
                    .into_response();
            }
            Json(json!([
                { "id": "m-1", "slug": "will-it-rain", "active": true }
            ]))
            .into_response()
        }
        (Method::GET, "/data/balance") => Json(json!({
            "balance": "42.00",
            "token": "usdce"
        }))
        .into_response(),
        (Method::POST, "/") => Json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "0x10"
        }))
        .into_response(),
        _ => (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response(),
    }
}
