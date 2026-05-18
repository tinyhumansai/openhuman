use super::*;
use crate::openhuman::config::PolymarketConfig;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
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

    fn with_delay(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }
}

fn fixture(name: &str) -> String {
    let root = env!("CARGO_MANIFEST_DIR");
    let path = format!("{root}/tests/fixtures/polymarket/{name}.json");
    std::fs::read_to_string(path).expect("fixture must exist")
}

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        ..SecurityPolicy::default()
    })
}

fn test_tool(gamma_base_url: String, clob_base_url: String, timeout_secs: u64) -> PolymarketTool {
    let config = PolymarketConfig {
        enabled: true,
        gamma_base_url,
        clob_base_url,
        timeout_secs,
    };

    PolymarketTool::new(&config, test_security())
}

fn route(key: &str, responses: Vec<MockResponse>) -> HashMap<String, Vec<MockResponse>> {
    let mut routes = HashMap::new();
    routes.insert(key.to_string(), responses);
    routes
}

async fn start_mock_server(
    routes: HashMap<String, Vec<MockResponse>>,
) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));

    let queues: HashMap<String, VecDeque<MockResponse>> = routes
        .into_iter()
        .map(|(path, responses)| (path, responses.into_iter().collect::<VecDeque<_>>()))
        .collect();

    let shared_routes = Arc::new(Mutex::new(queues));
    let shared_calls = Arc::clone(&calls);

    tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => return,
            };

            let routes = Arc::clone(&shared_routes);
            let calls = Arc::clone(&shared_calls);

            tokio::spawn(async move {
                let mut buf = vec![0_u8; 8192];
                let n = match socket.read(&mut buf).await {
                    Ok(read) => read,
                    Err(_) => return,
                };
                if n == 0 {
                    return;
                }

                let request = String::from_utf8_lossy(&buf[..n]);
                let target = request_target(&request);
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

    (format!("http://127.0.0.1:{}", addr.port()), calls)
}

fn request_target(request: &str) -> String {
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string()
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

#[tokio::test]
async fn list_markets_happy_path() {
    let (gamma_base, _) = start_mock_server(route(
        "/markets?limit=2&offset=0&active=true",
        vec![MockResponse::json(200, "markets_list")],
    ))
    .await;

    let tool = test_tool(gamma_base.clone(), gamma_base, 15);
    let result = tool
        .execute(json!({
            "action": "list_markets",
            "limit": 2,
            "offset": 0,
            "active": true
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = serde_json::from_str::<serde_json::Value>(&result.output()).unwrap();
    assert_eq!(output["action"], "list_markets");
    assert!(output["data"].is_array());
    assert_eq!(output["data"][0]["slug"], "will-eth-hit-10k");
}

#[tokio::test]
async fn get_market_by_id_happy_path() {
    let (gamma_base, _) = start_mock_server(route(
        "/markets/12345",
        vec![MockResponse::json(200, "market_by_id")],
    ))
    .await;

    let tool = test_tool(gamma_base.clone(), gamma_base, 15);
    let result = tool
        .execute(json!({
            "action": "get_market",
            "market_id": "12345"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = serde_json::from_str::<serde_json::Value>(&result.output()).unwrap();
    assert_eq!(output["action"], "get_market");
    assert_eq!(output["lookup"], "market_id");
    assert_eq!(output["data"]["id"], "12345");
}

#[tokio::test]
async fn get_market_by_slug_happy_path() {
    let (gamma_base, _) = start_mock_server(route(
        "/markets?slug=will-eth-hit-10k",
        vec![MockResponse::json(200, "market_by_slug")],
    ))
    .await;

    let tool = test_tool(gamma_base.clone(), gamma_base, 15);
    let result = tool
        .execute(json!({
            "action": "get_market",
            "slug": "will-eth-hit-10k"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = serde_json::from_str::<serde_json::Value>(&result.output()).unwrap();
    assert_eq!(output["lookup"], "slug");
    assert_eq!(output["data"]["id"], "12345");
    assert_eq!(output["data"]["slug"], "will-eth-hit-10k");
}

#[tokio::test]
async fn list_events_happy_path() {
    let (gamma_base, _) = start_mock_server(route(
        "/events?limit=2",
        vec![MockResponse::json(200, "events_list")],
    ))
    .await;

    let tool = test_tool(gamma_base.clone(), gamma_base, 15);
    let result = tool
        .execute(json!({
            "action": "list_events",
            "limit": 2
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = serde_json::from_str::<serde_json::Value>(&result.output()).unwrap();
    assert_eq!(output["action"], "list_events");
    assert!(output["data"].is_array());
    assert_eq!(output["data"][0]["id"], "event-1");
}

#[tokio::test]
async fn get_orderbook_happy_path() {
    let (clob_base, _) = start_mock_server(route(
        "/book?token_id=1001",
        vec![MockResponse::json(200, "orderbook")],
    ))
    .await;

    let tool = test_tool(clob_base.clone(), clob_base, 15);
    let result = tool
        .execute(json!({
            "action": "get_orderbook",
            "token_id": "1001"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = serde_json::from_str::<serde_json::Value>(&result.output()).unwrap();
    assert_eq!(output["action"], "get_orderbook");
    assert_eq!(output["data"]["token_id"], "1001");
}

#[tokio::test]
async fn get_price_happy_path() {
    let (clob_base, _) = start_mock_server(route(
        "/price?token_id=1001&side=buy",
        vec![MockResponse::json(200, "price")],
    ))
    .await;

    let tool = test_tool(clob_base.clone(), clob_base, 15);
    let result = tool
        .execute(json!({
            "action": "get_price",
            "token_id": "1001",
            "side": "buy"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = serde_json::from_str::<serde_json::Value>(&result.output()).unwrap();
    assert_eq!(output["action"], "get_price");
    assert_eq!(output["data"]["price"], "0.47");
}

#[tokio::test]
async fn client_error_4xx_returns_error_not_retried() {
    let (clob_base, calls) = start_mock_server(route(
        "/book?token_id=bad-token",
        vec![MockResponse::json(400, "error_client")],
    ))
    .await;

    let tool = test_tool(clob_base.clone(), clob_base, 15);
    let result = tool
        .execute(json!({
            "action": "get_orderbook",
            "token_id": "bad-token"
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("client error 400"));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn server_error_5xx_returns_transient_error() {
    let (clob_base, calls) = start_mock_server(route(
        "/price?token_id=1001&side=sell",
        vec![
            MockResponse::json(500, "error_server"),
            MockResponse::json(500, "error_server"),
            MockResponse::json(500, "error_server"),
        ],
    ))
    .await;

    let tool = test_tool(clob_base.clone(), clob_base, 15);
    let result = tool
        .execute(json!({
            "action": "get_price",
            "token_id": "1001",
            "side": "sell"
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("transient server error 500"));
    assert_eq!(calls.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn timeout_returns_deadline_error() {
    let (gamma_base, _) = start_mock_server(route(
        "/markets?limit=1",
        vec![MockResponse::json(200, "markets_list").with_delay(1_500)],
    ))
    .await;

    let tool = test_tool(gamma_base.clone(), gamma_base, 1);
    let result = tool
        .execute(json!({
            "action": "list_markets",
            "limit": 1
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("timed out"));
}

#[test]
fn parameters_schema_deserializes_for_all_5_actions() {
    let config = PolymarketConfig::default();
    let tool = PolymarketTool::new(&config, test_security());

    let schema = tool.parameters_schema();
    let actions = schema["properties"]["action"]["enum"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    for expected in [
        "list_markets",
        "get_market",
        "list_events",
        "get_orderbook",
        "get_price",
    ] {
        assert!(actions.contains(&json!(expected)));
    }

    let samples = vec![
        json!({"action": "list_markets", "limit": 1}),
        json!({"action": "get_market", "market_id": "123"}),
        json!({"action": "list_events", "limit": 1}),
        json!({"action": "get_orderbook", "token_id": "1001"}),
        json!({"action": "get_price", "token_id": "1001", "side": "buy"}),
    ];

    for sample in samples {
        let parsed: PolymarketRequest = serde_json::from_value(sample).unwrap();
        assert!(matches!(
            parsed,
            PolymarketRequest::ListMarkets { .. }
                | PolymarketRequest::GetMarket { .. }
                | PolymarketRequest::ListEvents { .. }
                | PolymarketRequest::GetOrderbook { .. }
                | PolymarketRequest::GetPrice { .. }
        ));
    }
}
