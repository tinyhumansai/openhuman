use super::*;
use crate::openhuman::config::{KalshiConfig, KalshiCredentials};
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{sleep, Duration};

#[derive(Clone)]
struct MockResponse {
    status: u16,
    body: String,
    delay_ms: u64,
}

impl MockResponse {
    fn json(status: u16, fixture_name: &str) -> Self {
        Self {
            status,
            body: fixture(fixture_name),
            delay_ms: 0,
        }
    }

    fn body(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_string(),
            delay_ms: 0,
        }
    }

    fn with_delay(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }
}

#[derive(Clone, Debug, Default)]
struct ObservedRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: String,
}

fn fixture(name: &str) -> String {
    let root = env!("CARGO_MANIFEST_DIR");
    let path = format!("{root}/tests/fixtures/kalshi/{name}.json");
    std::fs::read_to_string(path).expect("fixture must exist")
}

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        ..SecurityPolicy::default()
    })
}

fn test_tool(base_url: String, timeout_secs: u64) -> KalshiTool {
    let config = KalshiConfig {
        enabled: true,
        base_url,
        timeout_secs,
        credentials: None,
    };
    KalshiTool::new(&config, test_security())
}

fn authed_tool(base_url: String, timeout_secs: u64) -> KalshiTool {
    let config = KalshiConfig {
        enabled: true,
        base_url,
        timeout_secs,
        credentials: Some(KalshiCredentials {
            api_key: "test-key".to_string(),
            private_key_pem: String::new(),
            secret: "test-secret".to_string(),
        }),
    };
    KalshiTool::new(&config, test_security())
}

fn route(key: &str, responses: Vec<MockResponse>) -> HashMap<String, Vec<MockResponse>> {
    let mut routes = HashMap::new();
    routes.insert(key.to_string(), responses);
    routes
}

fn with_api_prefix(base: String) -> String {
    format!("{base}/trade-api/v2")
}

async fn start_mock_server(
    routes: HashMap<String, Vec<MockResponse>>,
) -> (String, Arc<AtomicUsize>) {
    let (base, calls, _captured) = start_mock_server_with_capture(routes).await;
    (base, calls)
}

async fn start_mock_server_with_capture(
    routes: HashMap<String, Vec<MockResponse>>,
) -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<ObservedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));

    let queues: HashMap<String, VecDeque<MockResponse>> = routes
        .into_iter()
        .map(|(path, responses)| (path, responses.into_iter().collect::<VecDeque<_>>()))
        .collect();

    let shared_routes = Arc::new(Mutex::new(queues));
    let shared_calls = Arc::clone(&calls);
    let shared_captured = Arc::clone(&captured);

    tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => return,
            };

            let routes = Arc::clone(&shared_routes);
            let calls = Arc::clone(&shared_calls);
            let captured = Arc::clone(&shared_captured);

            tokio::spawn(async move {
                let mut buf = Vec::with_capacity(32 * 1024);
                let mut chunk = [0_u8; 4096];
                loop {
                    let n = match socket.read(&mut chunk).await {
                        Ok(read) => read,
                        Err(_) => return,
                    };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if request_is_complete(&buf) {
                        break;
                    }
                }
                if buf.is_empty() {
                    return;
                }

                let request_raw = String::from_utf8_lossy(&buf).to_string();
                let observed = parse_request(&request_raw);
                let target = observed.target.clone();

                {
                    let mut guard = captured.lock().unwrap();
                    guard.push(observed);
                }

                calls.fetch_add(1, Ordering::Relaxed);

                let response = {
                    let mut guard = routes.lock().unwrap();
                    pop_response(&mut guard, &target)
                };

                if response.delay_ms > 0 {
                    sleep(Duration::from_millis(response.delay_ms)).await;
                }

                let reason = reason_phrase(response.status);
                let payload = response.body;
                let wire = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.status,
                    reason,
                    payload.len(),
                    payload
                );

                let _ = socket.write_all(wire.as_bytes()).await;
            });
        }
    });

    (format!("http://127.0.0.1:{}", addr.port()), calls, captured)
}

fn request_is_complete(buf: &[u8]) -> bool {
    let raw = String::from_utf8_lossy(buf);
    let Some((head, body)) = raw.split_once("\r\n\r\n") else {
        return false;
    };

    let content_length = head
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            if k.trim().eq_ignore_ascii_case("content-length") {
                v.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    body.as_bytes().len() >= content_length
}

fn parse_request(raw: &str) -> ObservedRequest {
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let mut lines = head.lines();
    let first_line = lines.next().unwrap_or_default();

    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or("/").to_string();

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    ObservedRequest {
        method,
        target,
        headers,
        body: body.to_string(),
    }
}

fn pop_response(
    routes: &mut HashMap<String, VecDeque<MockResponse>>,
    target: &str,
) -> MockResponse {
    if let Some(response) = pop_from_queue(routes.get_mut(target)) {
        return response;
    }

    let path_only = target.split('?').next().unwrap_or(target);
    if let Some(response) = pop_from_queue(routes.get_mut(path_only)) {
        return response;
    }

    MockResponse {
        status: 404,
        body: r#"{"error":"not found"}"#.to_string(),
        delay_ms: 0,
    }
}

fn pop_from_queue(queue: Option<&mut VecDeque<MockResponse>>) -> Option<MockResponse> {
    let queue = queue?;
    if queue.len() <= 1 {
        return queue.front().cloned();
    }
    queue.pop_front()
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    }
}

fn parse_tool_output(result: &ToolResult) -> Value {
    serde_json::from_str::<Value>(&result.output()).expect("tool output should be valid json")
}

fn header<'a>(request: &'a ObservedRequest, key: &str) -> Option<&'a str> {
    request
        .headers
        .get(&key.to_ascii_lowercase())
        .map(|value| value.as_str())
}

#[tokio::test]
async fn list_markets_happy_path() {
    let (base, _) = start_mock_server(route(
        "/trade-api/v2/markets?status=open&limit=2",
        vec![MockResponse::json(200, "markets_list")],
    ))
    .await;

    let tool = test_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "list_markets",
            "limit": 2
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "list_markets");
    assert_eq!(output["data"]["markets"][0]["ticker"], "INXD-23DEC29-T4500");
}

#[tokio::test]
async fn get_market_happy_path() {
    let (base, _) = start_mock_server(route(
        "/trade-api/v2/markets/INXD-23DEC29-T4500",
        vec![MockResponse::json(200, "market_by_ticker")],
    ))
    .await;

    let tool = test_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "get_market",
            "ticker": "INXD-23DEC29-T4500"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_market");
    assert_eq!(output["data"]["market"]["ticker"], "INXD-23DEC29-T4500");
}

#[tokio::test]
async fn list_events_happy_path() {
    let (base, _) = start_mock_server(route(
        "/trade-api/v2/events?status=open&limit=2",
        vec![MockResponse::json(200, "events_list")],
    ))
    .await;

    let tool = test_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "list_events",
            "limit": 2
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "list_events");
    assert_eq!(output["data"]["events"][0]["event_ticker"], "INXD-23DEC29");
}

#[tokio::test]
async fn get_event_happy_path() {
    let (base, _) = start_mock_server(route(
        "/trade-api/v2/events/INXD-23DEC29",
        vec![MockResponse::json(200, "event_by_ticker")],
    ))
    .await;

    let tool = test_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "get_event",
            "event_ticker": "INXD-23DEC29"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_event");
    assert_eq!(output["data"]["event"]["event_ticker"], "INXD-23DEC29");
}

#[tokio::test]
async fn get_orderbook_happy_path() {
    let (base, _) = start_mock_server(route(
        "/trade-api/v2/markets/INXD-23DEC29-T4500/orderbook?depth=5",
        vec![MockResponse::json(200, "orderbook")],
    ))
    .await;

    let tool = test_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "get_orderbook",
            "ticker": "INXD-23DEC29-T4500",
            "depth": 5
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_orderbook");
    assert_eq!(output["data"]["orderbook"]["yes"][0]["price"], 52);
}

#[tokio::test]
async fn get_positions_happy_path_signs_headers() {
    let (base, _calls, captured) = start_mock_server_with_capture(route(
        "/trade-api/v2/portfolio/positions",
        vec![MockResponse::json(200, "positions")],
    ))
    .await;

    let tool = authed_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({ "action": "get_positions" }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_positions");
    assert_eq!(
        output["data"]["positions"][0]["ticker"],
        "INXD-23DEC29-T4500"
    );

    let requests = captured.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "GET");
    assert_eq!(request.target, "/trade-api/v2/portfolio/positions");
    assert_eq!(header(request, "kalshi-access-key"), Some("test-key"));
    assert!(header(request, "kalshi-access-signature")
        .map(|sig| !sig.trim().is_empty())
        .unwrap_or(false));
    assert!(header(request, "kalshi-access-timestamp")
        .and_then(|v| v.parse::<u64>().ok())
        .is_some());
}

#[tokio::test]
async fn get_balance_happy_path_authenticated() {
    let (base, _calls, captured) = start_mock_server_with_capture(route(
        "/trade-api/v2/portfolio/balance",
        vec![MockResponse::json(200, "balance")],
    ))
    .await;

    let tool = authed_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({ "action": "get_balance" }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_balance");
    assert_eq!(output["data"]["balance"]["cash"], 123450);

    let requests = captured.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].target, "/trade-api/v2/portfolio/balance");
    assert_eq!(header(&requests[0], "kalshi-access-key"), Some("test-key"));
}

#[tokio::test]
async fn get_open_orders_happy_path_uses_resting_status() {
    let (base, _calls, captured) = start_mock_server_with_capture(route(
        "/trade-api/v2/portfolio/orders?status=resting&limit=2",
        vec![MockResponse::json(200, "orders")],
    ))
    .await;

    let tool = authed_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "get_open_orders",
            "limit": 2
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_open_orders");
    assert_eq!(output["data"]["orders"][0]["status"], "resting");

    let requests = captured.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].target,
        "/trade-api/v2/portfolio/orders?status=resting&limit=2"
    );
}

#[tokio::test]
async fn get_fills_happy_path_authenticated() {
    let (base, _calls, captured) = start_mock_server_with_capture(route(
        "/trade-api/v2/portfolio/fills?limit=1",
        vec![MockResponse::json(200, "fills")],
    ))
    .await;

    let tool = authed_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "get_fills",
            "limit": 1
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_fills");
    assert_eq!(output["data"]["fills"][0]["fill_id"], "fill-1");

    let requests = captured.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].target, "/trade-api/v2/portfolio/fills?limit=1");
}

#[tokio::test]
async fn place_order_requires_approval_and_does_not_issue_http() {
    let (base, calls) = start_mock_server(route(
        "/trade-api/v2/portfolio/orders",
        vec![MockResponse::json(200, "place_order_ok")],
    ))
    .await;

    let tool = authed_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "place_order",
            "ticker": "INXD-23DEC29-T4500",
            "side": "yes",
            "order_action": "buy",
            "type": "limit",
            "count": 2,
            "yes_price": 54
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("requires explicit user approval"));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn place_order_happy_path_posts_signed_payload() {
    let (base, _calls, captured) = start_mock_server_with_capture(route(
        "/trade-api/v2/portfolio/orders",
        vec![MockResponse::json(200, "place_order_ok")],
    ))
    .await;

    let tool = authed_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "place_order",
            "ticker": "INXD-23DEC29-T4500",
            "side": "yes",
            "order_action": "buy",
            "type": "limit",
            "count": 2,
            "yes_price": 54,
            "expiration_ts": 1735689600,
            "approved": true
        }))
        .await
        .unwrap();

    assert!(
        !result.is_error,
        "expected success, got: {}",
        result.output()
    );
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "place_order");
    assert_eq!(output["data"]["order"]["order_id"], "ord-new-1");
    assert!(output["client_order_id"].as_str().is_some());

    let requests = captured.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/trade-api/v2/portfolio/orders");
    assert_eq!(header(request, "kalshi-access-key"), Some("test-key"));
    assert_eq!(header(request, "content-type"), Some("application/json"));

    let posted: Value = serde_json::from_str(&request.body).expect("valid json body");
    assert_eq!(posted["ticker"], "INXD-23DEC29-T4500");
    assert_eq!(posted["side"], "yes");
    assert_eq!(posted["action"], "buy");
    assert_eq!(posted["type"], "limit");
    assert_eq!(posted["count"], 2);
    assert_eq!(posted["yes_price"], 54);
    assert_eq!(posted["expiration_ts"], 1735689600);

    let client_order_id = posted["client_order_id"]
        .as_str()
        .expect("client_order_id should be a string");
    assert!(uuid::Uuid::from_str(client_order_id).is_ok());
}

#[tokio::test]
async fn cancel_order_requires_approval_and_does_not_issue_http() {
    let (base, calls) = start_mock_server(route(
        "/trade-api/v2/portfolio/orders/ord-1",
        vec![MockResponse::json(200, "cancel_order_ok")],
    ))
    .await;

    let tool = authed_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "cancel_order",
            "order_id": "ord-1"
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("requires explicit user approval"));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn cancel_order_happy_path_signed_delete() {
    let (base, _calls, captured) = start_mock_server_with_capture(route(
        "/trade-api/v2/portfolio/orders/ord-1",
        vec![MockResponse::json(200, "cancel_order_ok")],
    ))
    .await;

    let tool = authed_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "cancel_order",
            "order_id": "ord-1",
            "approved": true
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "cancel_order");
    assert_eq!(output["data"]["canceled"]["status"], "canceled");

    let requests = captured.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "DELETE");
    assert_eq!(request.target, "/trade-api/v2/portfolio/orders/ord-1");
    assert_eq!(header(request, "kalshi-access-key"), Some("test-key"));
}

#[tokio::test]
async fn client_error_4xx_returns_error_not_retried() {
    let (base, calls) = start_mock_server(route(
        "/trade-api/v2/markets/INVALID",
        vec![MockResponse::json(400, "error_client")],
    ))
    .await;

    let tool = test_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "get_market",
            "ticker": "INVALID"
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("client error 400"));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn server_error_5xx_retries_three_times() {
    let (base, calls) = start_mock_server(route(
        "/trade-api/v2/events?status=open&limit=1",
        vec![
            MockResponse::json(500, "error_server"),
            MockResponse::json(500, "error_server"),
            MockResponse::json(500, "error_server"),
        ],
    ))
    .await;

    let tool = test_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({
            "action": "list_events",
            "limit": 1
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("transient server error 500"));
    assert_eq!(calls.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn timeout_returns_deadline_error() {
    let (base, _) = start_mock_server(route(
        "/trade-api/v2/events?status=open&limit=1",
        vec![MockResponse::json(200, "events_list").with_delay(1_500)],
    ))
    .await;

    let tool = test_tool(with_api_prefix(base), 1);
    let result = tool
        .execute(json!({
            "action": "list_events",
            "limit": 1
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("timed out"));
}

#[test]
fn parameters_schema_deserializes_for_all_actions() {
    let config = KalshiConfig::default();
    let tool = KalshiTool::new(&config, test_security());

    let schema = tool.parameters_schema();
    let actions = schema["properties"]["action"]["enum"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    for expected in [
        "list_markets",
        "get_market",
        "list_events",
        "get_event",
        "get_orderbook",
        "get_positions",
        "get_balance",
        "get_open_orders",
        "get_fills",
        "place_order",
        "cancel_order",
    ] {
        assert!(
            actions.contains(&json!(expected)),
            "missing action {expected}"
        );
    }

    let samples = vec![
        json!({"action": "list_markets", "limit": 1}),
        json!({"action": "get_market", "ticker": "INXD-23DEC29-T4500"}),
        json!({"action": "list_events", "limit": 1}),
        json!({"action": "get_event", "event_ticker": "INXD-23DEC29"}),
        json!({"action": "get_orderbook", "ticker": "INXD-23DEC29-T4500", "depth": 5}),
        json!({"action": "get_positions"}),
        json!({"action": "get_balance"}),
        json!({"action": "get_open_orders", "limit": 2}),
        json!({"action": "get_fills", "limit": 2}),
        json!({"action": "place_order", "ticker": "INXD-23DEC29-T4500", "side": "yes", "order_action": "buy", "type": "limit", "count": 1, "yes_price": 55, "approved": true}),
        json!({"action": "cancel_order", "order_id": "ord-1", "approved": true}),
    ];

    for sample in samples {
        let parsed: KalshiRequest = serde_json::from_value(sample).unwrap();
        assert!(matches!(
            parsed,
            KalshiRequest::ListMarkets { .. }
                | KalshiRequest::GetMarket { .. }
                | KalshiRequest::ListEvents { .. }
                | KalshiRequest::GetEvent { .. }
                | KalshiRequest::GetOrderbook { .. }
                | KalshiRequest::GetPositions
                | KalshiRequest::GetBalance
                | KalshiRequest::GetOpenOrders { .. }
                | KalshiRequest::GetFills { .. }
                | KalshiRequest::PlaceOrder { .. }
                | KalshiRequest::CancelOrder { .. }
        ));
    }
}

#[tokio::test]
async fn signed_reads_require_credentials() {
    let (base, calls) = start_mock_server(route(
        "/trade-api/v2/portfolio/positions",
        vec![MockResponse::json(200, "positions")],
    ))
    .await;

    let tool = test_tool(with_api_prefix(base), 15);
    let result = tool
        .execute(json!({ "action": "get_positions" }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("credentials are required"));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn ensure_https_blocks_signed_reads_on_non_loopback_http() {
    let tool = authed_tool("http://example.com:8080/trade-api/v2".to_string(), 15);

    let result = tool
        .execute(json!({ "action": "get_positions" }))
        .await
        .unwrap();

    assert!(result.is_error, "expected error, got success");
    assert!(
        result.output().contains("non-HTTPS URL"),
        "expected HTTPS guard rejection, got: {}",
        result.output()
    );
}
