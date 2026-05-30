use super::*;
use axum::{
    extract::State,
    http::{HeaderMap as AxumHeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicUsize, Ordering as AtomicOrdering},
    Arc,
};

#[derive(Clone)]
struct TestState {
    init_count: Arc<AtomicUsize>,
    call_count: Arc<AtomicUsize>,
}

fn has_streamable_http_accept(headers: &AxumHeaderMap) -> bool {
    headers
        .get(ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.contains("application/json") && value.contains("text/event-stream"))
        .unwrap_or(false)
}

async fn mcp_handler(
    State(state): State<TestState>,
    headers: AxumHeaderMap,
    method: Method,
    Json(body): Json<Value>,
) -> Response {
    if method == Method::POST && !has_streamable_http_accept(&headers) {
        return (
            StatusCode::NOT_ACCEPTABLE,
            "missing MCP Accept header".to_string(),
        )
            .into_response();
    }
    let rpc_method = body.get("method").and_then(Value::as_str).unwrap_or("");
    if method == Method::POST && rpc_method == "initialize" {
        state.init_count.fetch_add(1, AtomicOrdering::SeqCst);
        return (
            [(HEADER_SESSION_ID, "session-1")],
            Json(json!({
                "jsonrpc": "2.0",
                "id": body["id"].clone(),
                "result": {
                    "protocolVersion": LATEST_PROTOCOL_VERSION,
                    "capabilities": { "tools": { "listChanged": true } },
                    "serverInfo": { "name": "test-server", "version": "1.0.0" }
                }
            })),
        )
            .into_response();
    }

    if headers.get(HEADER_SESSION_ID).and_then(|v| v.to_str().ok()) != Some("session-1") {
        return (
            StatusCode::BAD_REQUEST,
            "missing or invalid session".to_string(),
        )
            .into_response();
    }

    if headers
        .get(HEADER_PROTOCOL_VERSION)
        .and_then(|v| v.to_str().ok())
        != Some(LATEST_PROTOCOL_VERSION)
    {
        return (
            StatusCode::BAD_REQUEST,
            "missing protocol version".to_string(),
        )
            .into_response();
    }

    match rpc_method {
        "notifications/initialized" => StatusCode::NO_CONTENT.into_response(),
        "tools/list" => Json(json!({
            "jsonrpc": "2.0",
            "id": body["id"].clone(),
            "result": {
                "tools": [{
                    "name": "needs_header",
                    "description": "needs x-mcp-header",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "tenant": {
                                "type": "string",
                                "x-mcp-header": "tenant"
                            }
                        }
                    }
                }]
            }
        }))
        .into_response(),
        "tools/call" => {
            state.call_count.fetch_add(1, AtomicOrdering::SeqCst);
            if headers
                .get("Mcp-Param-tenant")
                .and_then(|v| v.to_str().ok())
                != Some("acme")
            {
                return (
                    StatusCode::BAD_REQUEST,
                    "missing mirrored tenant header".to_string(),
                )
                    .into_response();
            }
            Json(json!({
                "jsonrpc": "2.0",
                "id": body["id"].clone(),
                "result": {
                    "content": [{
                        "type": "text",
                        "text": "remote result"
                    }]
                }
            }))
            .into_response()
        }
        _ => (
            StatusCode::BAD_REQUEST,
            format!("unexpected method {rpc_method}"),
        )
            .into_response(),
    }
}

async fn events_handler(headers: AxumHeaderMap) -> Response {
    if headers
        .get(ACCEPT)
        .and_then(|v| v.to_str().ok())
        .filter(|value| value.contains("text/event-stream"))
        .is_none()
    {
        return (
            StatusCode::NOT_ACCEPTABLE,
            "missing SSE Accept header".to_string(),
        )
            .into_response();
    }
    if headers.get(HEADER_SESSION_ID).is_none() {
        return (StatusCode::BAD_REQUEST, "no session".to_string()).into_response();
    }
    (
        [(CONTENT_TYPE.as_str(), "text/event-stream")],
        "id: 1\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"ok\":true}}\n\n",
    )
        .into_response()
}

async fn delete_handler() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

async fn bearer_required_handler(headers: AxumHeaderMap, Json(body): Json<Value>) -> Response {
    if headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer secret-token") {
        return (StatusCode::UNAUTHORIZED, "missing bearer".to_string()).into_response();
    }
    Json(json!({
        "jsonrpc": "2.0",
        "id": body["id"].clone(),
        "result": {
            "protocolVersion": LATEST_PROTOCOL_VERSION,
            "capabilities": {},
            "serverInfo": { "name": "bearer-server", "version": "1.0.0" }
        }
    }))
    .into_response()
}

async fn retrying_mcp_handler(
    State(state): State<TestState>,
    headers: AxumHeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !has_streamable_http_accept(&headers) {
        return (
            StatusCode::NOT_ACCEPTABLE,
            "missing MCP Accept header".to_string(),
        )
            .into_response();
    }
    let rpc_method = body.get("method").and_then(Value::as_str).unwrap_or("");
    if rpc_method == "initialize" {
        state.init_count.fetch_add(1, AtomicOrdering::SeqCst);
        return (
            [(HEADER_SESSION_ID, "session-retry")],
            Json(json!({
                "jsonrpc": "2.0",
                "id": body["id"].clone(),
                "result": {
                    "protocolVersion": LATEST_PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "retry-server", "version": "1.0.0" }
                }
            })),
        )
            .into_response();
    }
    if rpc_method == "notifications/initialized" {
        return StatusCode::NO_CONTENT.into_response();
    }
    if rpc_method == "tools/list" {
        let call_number = state.call_count.fetch_add(1, AtomicOrdering::SeqCst);
        if call_number == 0
            && headers.get(HEADER_SESSION_ID).and_then(|v| v.to_str().ok()) == Some("session-retry")
        {
            return (StatusCode::NOT_FOUND, "expired".to_string()).into_response();
        }
        return Json(json!({
            "jsonrpc": "2.0",
            "id": body["id"].clone(),
            "result": { "tools": [] }
        }))
        .into_response();
    }
    (StatusCode::BAD_REQUEST, "unexpected".to_string()).into_response()
}

async fn spawn_test_server() -> (String, TestState) {
    let state = TestState {
        init_count: Arc::new(AtomicUsize::new(0)),
        call_count: Arc::new(AtomicUsize::new(0)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/",
            post(mcp_handler).get(events_handler).delete(delete_handler),
        )
        .with_state(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/"), state)
}

async fn spawn_retry_server() -> (String, TestState) {
    let state = TestState {
        init_count: Arc::new(AtomicUsize::new(0)),
        call_count: Arc::new(AtomicUsize::new(0)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/", post(retrying_mcp_handler))
        .with_state(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/"), state)
}

async fn spawn_discovery_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let auth_header = format!(
        "Bearer realm=\"mcp\", resource_metadata=\"{base}/.well-known/oauth-protected-resource\""
    );
    let prm_base = base.clone();
    let issuer_base = base.clone();
    let app = Router::new()
        .route(
            "/",
            post(move || {
                let auth_header = auth_header.clone();
                async move {
                    (
                        StatusCode::UNAUTHORIZED,
                        [("WWW-Authenticate", auth_header.as_str())],
                        "",
                    )
                        .into_response()
                }
            }),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(move || {
                let prm_base = prm_base.clone();
                async move {
                    let resource = format!("{prm_base}/");
                    Json(json!({
                        "resource": resource,
                        "authorization_servers": [prm_base],
                        "scopes_supported": ["mcp:tools"]
                    }))
                }
            }),
        )
        .route(
            "/.well-known/openid-configuration",
            get(move || {
                let issuer_base = issuer_base.clone();
                async move {
                    let authorization_endpoint = format!("{}/authorize", issuer_base);
                    let token_endpoint = format!("{}/token", issuer_base);
                    Json(json!({
                        "issuer": issuer_base,
                        "authorization_endpoint": authorization_endpoint,
                        "token_endpoint": token_endpoint,
                        "grant_types_supported": ["authorization_code"],
                        "code_challenge_methods_supported": ["S256"]
                    }))
                }
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/")
}

#[tokio::test]
async fn initialize_and_list_tools_negotiate_session() {
    let (endpoint, state) = spawn_test_server().await;
    let client = McpHttpClient::new(endpoint, 5);
    let tools = client.list_tools().await.expect("list_tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(state.init_count.load(AtomicOrdering::SeqCst), 1);
    let snapshot = client.initialize_snapshot().expect("snapshot");
    assert_eq!(snapshot.protocol_version, LATEST_PROTOCOL_VERSION);
}

#[tokio::test]
async fn call_tool_mirrors_x_mcp_header_parameters() {
    let (endpoint, state) = spawn_test_server().await;
    let client = McpHttpClient::new(endpoint, 5);
    let result = client
        .call_tool("needs_header", json!({"tenant": "acme"}))
        .await
        .expect("call_tool");
    assert_eq!(result.rendered.output(), "remote result");
    assert_eq!(state.call_count.load(AtomicOrdering::SeqCst), 1);
}

#[tokio::test]
async fn session_404_triggers_reinitialize_and_retry() {
    let (endpoint, state) = spawn_retry_server().await;
    let client = McpHttpClient::new(endpoint, 5);
    let tools = client.list_tools().await.expect("list_tools");
    assert!(tools.is_empty());
    assert_eq!(state.init_count.load(AtomicOrdering::SeqCst), 2);
    assert_eq!(state.call_count.load(AtomicOrdering::SeqCst), 2);
}

#[tokio::test]
async fn drain_events_parses_sse_stream() {
    let (endpoint, _) = spawn_test_server().await;
    let client = McpHttpClient::new(endpoint, 5);
    let events = client.drain_events(None).await.expect("drain events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id.as_deref(), Some("1"));
    assert_eq!(events[0].event.as_deref(), Some("message"));
    assert_eq!(events[0].data.as_ref().unwrap()["params"]["ok"], true);
}

#[tokio::test]
async fn close_session_sends_delete() {
    let (endpoint, _) = spawn_test_server().await;
    let client = McpHttpClient::new(endpoint, 5);
    client.initialize().await.expect("initialize");
    client.close_session().await.expect("close_session");
    assert!(client.initialize_snapshot().is_none());
}

#[test]
fn redact_endpoint_hides_paths_and_credentials() {
    assert_eq!(
        redact_endpoint("https://example.com/path?x=1"),
        "https://example.com"
    );
    assert_eq!(
        redact_endpoint("https://user:pw@example.com/a"),
        "<redacted>"
    );
}

#[test]
fn parse_sse_events_handles_multiple_frames() {
    let body = "id: 1\nevent: message\ndata: {\"a\":1}\n\ndata: {\"b\":2}\n\n";
    let events = parse_sse_events(body).expect("events");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].id.as_deref(), Some("1"));
    assert_eq!(events[1].data.as_ref().unwrap()["b"], 2);
}

#[test]
fn parse_www_authenticate_extracts_resource_metadata() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "WWW-Authenticate",
        HeaderValue::from_static(
            "Bearer realm=\"mcp\", resource_metadata=\"https://example.com/.well-known/oauth-protected-resource\"",
        ),
    );
    let challenge = parse_www_authenticate_challenge(&headers).expect("challenge");
    assert_eq!(challenge.scheme, "Bearer");
    assert_eq!(challenge.realm.as_deref(), Some("mcp"));
    assert_eq!(
        challenge.resource_metadata.as_deref(),
        Some("https://example.com/.well-known/oauth-protected-resource")
    );
}

#[tokio::test]
async fn discover_authorization_returns_none_when_not_401() {
    let (endpoint, _) = spawn_test_server().await;
    let client = McpHttpClient::new(endpoint, 5);
    assert!(client.discover_authorization().await.unwrap().is_none());
}

#[tokio::test]
async fn discover_authorization_fetches_metadata() {
    let endpoint = spawn_discovery_server().await;
    let client = McpHttpClient::new(endpoint, 2);
    let ctx = client
        .discover_authorization()
        .await
        .expect("discover")
        .expect("some");
    assert_eq!(ctx.challenge.scheme, "Bearer");
    assert_eq!(
        ctx.protected_resource_metadata
            .as_ref()
            .unwrap()
            .scopes_supported,
        vec!["mcp:tools"]
    );
    assert_eq!(ctx.authorization_server_metadata.len(), 1);
    let expected_authorization_endpoint = format!(
        "{}/authorize",
        ctx.protected_resource_metadata
            .as_ref()
            .unwrap()
            .authorization_servers[0]
    );
    assert_eq!(
        ctx.authorization_server_metadata[0]
            .authorization_endpoint
            .as_deref(),
        Some(expected_authorization_endpoint.as_str())
    );
}

#[tokio::test]
async fn bearer_auth_is_attached_to_initialize() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/", post(bearer_required_handler));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = McpHttpClient::with_options(
        format!("http://{addr}/"),
        2,
        McpAuthConfig::BearerToken {
            token: "secret-token".into(),
        },
        McpClientIdentityConfig::default(),
    );
    let init = client.initialize().await.expect("initialize");
    assert_eq!(init.server_info["name"], "bearer-server");
}
