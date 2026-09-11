use super::*;
use crate::openhuman::mcp::http_client::McpHttpClient;
use serde_json::json;
use tinymcp_bus::McpAuthConfig;

async fn spawn_test_server(auth_token: Option<&str>) -> String {
    spawn_test_server_with_events(auth_token).await.0
}

async fn spawn_test_server_with_events(
    auth_token: Option<&str>,
) -> (String, broadcast::Sender<McpSseEvent>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (event_tx, _) = broadcast::channel(128);
    let state = AppState {
        sessions: Arc::new(Mutex::new(HashMap::new())),
        auth_token: auth_token.map(str::to_string),
        event_tx: event_tx.clone(),
    };
    let app = Router::new()
        .route("/", post(handle_post).get(handle_get).delete(handle_delete))
        .with_state(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/"), event_tx)
}

#[tokio::test]
async fn http_client_round_trips_initialize_tools_list_and_ping() {
    let endpoint = spawn_test_server(None).await;
    let client = McpHttpClient::new(endpoint, 5).expect("a client builds");

    let init = client.initialize().await.expect("initialize");
    assert_eq!(init.protocol_version, protocol::LATEST_PROTOCOL_VERSION);
    assert_eq!(init.server_info["name"], "openhuman-core");

    let tools = client.list_tools().await.expect("tools/list");
    assert!(tools.iter().any(|tool| tool.name == "memory.search"));

    client.close_session().await.expect("DELETE session");
}

#[tokio::test]
async fn get_events_returns_long_lived_sse_stream() {
    let (endpoint, event_tx) = spawn_test_server_with_events(None).await;
    let http = reqwest::Client::new();
    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": protocol::LATEST_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}
        }
    });
    let init_response = http
        .post(&endpoint)
        .header(CONTENT_TYPE, "application/json")
        .json(&init)
        .send()
        .await
        .expect("initialize");
    assert_eq!(init_response.status(), StatusCode::OK);
    let session_id = init_response
        .headers()
        .get(HEADER_SESSION_ID)
        .and_then(|value| value.to_str().ok())
        .expect("session header")
        .to_string();

    let events_response = http
        .get(&endpoint)
        .header(HEADER_SESSION_ID, session_id.as_str())
        .header(HEADER_PROTOCOL_VERSION, protocol::LATEST_PROTOCOL_VERSION)
        .send()
        .await
        .expect("GET events");
    assert_eq!(events_response.status(), StatusCode::OK);
    assert!(events_response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream")));

    event_tx
        .send(McpSseEvent {
            session_id,
            event: Some("test".into()),
            data: "{\"ok\":true}".into(),
        })
        .expect("send test event");

    let mut stream = events_response.bytes_stream();
    let chunk = tokio::time::timeout(
        Duration::from_secs(2),
        futures_util::StreamExt::next(&mut stream),
    )
    .await
    .expect("timely event chunk")
    .expect("event chunk")
    .expect("event bytes");
    let text = String::from_utf8_lossy(&chunk);
    assert!(text.contains("event: test"), "{text}");
    assert!(text.contains("data: {\"ok\":true}"), "{text}");
}

#[tokio::test]
async fn http_rejects_requests_without_session_after_initialize() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState {
        sessions: Arc::new(Mutex::new(HashMap::new())),
        auth_token: None,
        event_tx: broadcast::channel(128).0,
    };
    let app = Router::new()
        .route("/", post(handle_post))
        .with_state(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let endpoint = format!("http://{addr}/");
    let http = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });
    let response = http
        .post(&endpoint)
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await
        .expect("post tools/list without session");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn http_bearer_auth_rejects_and_accepts() {
    let endpoint = spawn_test_server(Some("phase1-secret")).await;

    let denied = McpHttpClient::builder(endpoint.clone())
        .timeout_secs(5)
        .auth(McpAuthConfig::BearerToken {
            token: "wrong".into(),
        })
        .build()
        .expect("a client builds");
    let err = denied.initialize().await.expect_err("bad token");
    assert!(err.to_string().contains("401"), "expected 401, got {err}");

    let allowed = McpHttpClient::builder(endpoint)
        .timeout_secs(5)
        .auth(McpAuthConfig::BearerToken {
            token: "phase1-secret".into(),
        })
        .build()
        .expect("a client builds");
    allowed.initialize().await.expect("authorized initialize");
}
