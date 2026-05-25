//! Streamable HTTP + SSE transport for the OpenHuman MCP server.
//!
//! Reuses [`super::protocol`] for JSON-RPC dispatch. Session lifecycle and header
//! names match [`crate::openhuman::mcp_client::McpHttpClient`] so remote
//! MCP clients can talk to this server without custom glue.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        HeaderMap, StatusCode,
    },
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::post,
    Json, Router,
};
use parking_lot::Mutex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use tokio_stream::{wrappers::BroadcastStream, StreamExt};
use uuid::Uuid;

use super::protocol;

pub const HEADER_PROTOCOL_VERSION: &str = "MCP-Protocol-Version";
pub const HEADER_SESSION_ID: &str = "Mcp-Session-Id";

#[derive(Debug, Clone)]
pub struct HttpServerConfig {
    pub bind_addr: SocketAddr,
    pub auth_token: Option<String>,
}

#[derive(Debug, Default)]
struct SessionRecord {
    protocol_version: String,
}

#[derive(Clone)]
struct AppState {
    sessions: Arc<Mutex<HashMap<String, SessionRecord>>>,
    auth_token: Option<String>,
    event_tx: broadcast::Sender<McpSseEvent>,
}

#[derive(Debug, Clone)]
struct McpSseEvent {
    session_id: String,
    event: Option<String>,
    data: String,
}

pub async fn run_http(config: HttpServerConfig) -> Result<()> {
    let (event_tx, _) = broadcast::channel(128);
    let state = AppState {
        sessions: Arc::new(Mutex::new(HashMap::new())),
        auth_token: config.auth_token.clone(),
        event_tx,
    };

    let app = Router::new()
        .route("/", post(handle_post).get(handle_get).delete(handle_delete))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .with_context(|| format!("binding MCP HTTP server on {}", config.bind_addr))?;
    log::info!(
        "[mcp_server] HTTP/SSE listening on http://{}",
        listener.local_addr()?
    );

    axum::serve(listener, app)
        .await
        .context("MCP HTTP server exited with error")?;
    Ok(())
}

#[axum::debug_handler]
async fn handle_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Some(response) = check_auth(&state, &headers) {
        return response;
    }

    let session_id = header_value(&headers, HEADER_SESSION_ID);
    let protocol_version = header_value(&headers, HEADER_PROTOCOL_VERSION);
    let rpc_method = body.get("method").and_then(Value::as_str).unwrap_or("");
    let redacted_session_id = session_id.map(redact_session_id);

    log::debug!(
        "[mcp_server] HTTP POST method={rpc_method} session={:?} protocol={:?}",
        redacted_session_id.as_deref(),
        protocol_version
    );

    if rpc_method == "initialize" {
        return handle_initialize(&state, body).await;
    }

    let Some(session_id) = session_id else {
        log_request_rejected("missing/invalid session", None, protocol_version, None);
        return text_error(
            StatusCode::BAD_REQUEST,
            "missing or invalid Mcp-Session-Id header",
        );
    };

    let expected_protocol = {
        let sessions = state.sessions.lock();
        let Some(record) = sessions.get(session_id) else {
            log_request_rejected(
                "unknown/expired session",
                Some(session_id),
                protocol_version,
                None,
            );
            return text_error(StatusCode::NOT_FOUND, "unknown or expired MCP session");
        };
        record.protocol_version.clone()
    };

    if protocol_version.as_deref() != Some(expected_protocol.as_str()) {
        log_request_rejected(
            "protocol mismatch",
            Some(session_id),
            protocol_version,
            Some(expected_protocol.as_str()),
        );
        return text_error(
            StatusCode::BAD_REQUEST,
            "missing or invalid MCP-Protocol-Version header",
        );
    }

    if body.get("id").is_none() {
        let _ = protocol::handle_json_value(body).await;
        return StatusCode::NO_CONTENT.into_response();
    }

    match protocol::handle_json_value(body).await {
        responses if responses.is_empty() => StatusCode::NO_CONTENT.into_response(),
        responses if responses.len() == 1 => {
            Json(responses.into_iter().next().unwrap()).into_response()
        }
        responses => Json(Value::Array(responses)).into_response(),
    }
}

async fn handle_initialize(state: &AppState, body: Value) -> Response {
    let responses = protocol::handle_json_value(body).await;
    let Some(response) = responses.into_iter().next() else {
        return StatusCode::NO_CONTENT.into_response();
    };

    if response.get("error").is_some() {
        return Json(response).into_response();
    }

    let negotiated = response
        .get("result")
        .and_then(|result| result.get("protocolVersion"))
        .and_then(Value::as_str)
        .unwrap_or(protocol::LATEST_PROTOCOL_VERSION)
        .to_string();

    let session_id = Uuid::new_v4().to_string();
    let redacted_session_id = redact_session_id(&session_id);
    log::debug!("[mcp_server] HTTP session created id={redacted_session_id} protocol={negotiated}");
    state.sessions.lock().insert(
        session_id.clone(),
        SessionRecord {
            protocol_version: negotiated,
        },
    );

    ([(HEADER_SESSION_ID, session_id.as_str())], Json(response)).into_response()
}

async fn handle_get(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(response) = check_auth(&state, &headers) {
        return response;
    }

    let protocol_version = header_value(&headers, HEADER_PROTOCOL_VERSION);
    let Some(session_id) = header_value(&headers, HEADER_SESSION_ID) else {
        log_request_rejected("missing/invalid session", None, protocol_version, None);
        return text_error(StatusCode::BAD_REQUEST, "missing Mcp-Session-Id header");
    };

    let expected_protocol = {
        let sessions = state.sessions.lock();
        let Some(record) = sessions.get(session_id) else {
            log_request_rejected(
                "unknown/expired session",
                Some(session_id),
                protocol_version,
                None,
            );
            return text_error(StatusCode::NOT_FOUND, "unknown or expired MCP session");
        };
        record.protocol_version.clone()
    };

    if protocol_version.as_deref() != Some(expected_protocol.as_str()) {
        log_request_rejected(
            "protocol mismatch",
            Some(session_id),
            protocol_version,
            Some(expected_protocol.as_str()),
        );
        return text_error(
            StatusCode::BAD_REQUEST,
            "missing or invalid MCP-Protocol-Version header",
        );
    }

    let redacted_session_id = redact_session_id(session_id);
    log::debug!("[mcp_server] HTTP events stream opened session={redacted_session_id}");

    let session_id = session_id.to_string();
    let stream = BroadcastStream::new(state.event_tx.subscribe()).filter_map(move |message| {
        let event = match message {
            Ok(event) if event.session_id == session_id => event,
            _ => return None,
        };
        let mut sse_event = Event::default().data(event.data);
        if let Some(name) = event.event {
            sse_event = sse_event.event(name);
        }
        Some(Ok::<Event, Infallible>(sse_event))
    });

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(10))
                .text("keepalive"),
        )
        .into_response()
}

async fn handle_delete(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(response) = check_auth(&state, &headers) {
        return response;
    }

    let Some(session_id) = header_value(&headers, HEADER_SESSION_ID) else {
        log_request_rejected(
            "missing/invalid session",
            None,
            header_value(&headers, HEADER_PROTOCOL_VERSION),
            None,
        );
        return text_error(StatusCode::BAD_REQUEST, "missing Mcp-Session-Id header");
    };

    if state.sessions.lock().remove(session_id).is_some() {
        let redacted_session_id = redact_session_id(session_id);
        log::debug!("[mcp_server] HTTP session closed id={redacted_session_id}");
    }
    StatusCode::NO_CONTENT.into_response()
}

fn check_auth(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let expected = state.auth_token.as_deref()?;
    let provided = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    if provided == Some(expected) {
        return None;
    }
    log::debug!("[mcp_server] HTTP request rejected: bearer auth mismatch");
    Some(
        (
            StatusCode::UNAUTHORIZED,
            [(CONTENT_TYPE.as_str(), "text/plain")],
            "unauthorized",
        )
            .into_response(),
    )
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn redact_session_id(session_id: &str) -> String {
    let digest = Sha256::digest(session_id.as_bytes());
    format!("sha256:{}", hex::encode(&digest[..4]))
}

fn log_request_rejected(
    reason: &str,
    session_id: Option<&str>,
    protocol_version: Option<&str>,
    expected_protocol: Option<&str>,
) {
    let redacted_session_id = session_id.map(redact_session_id);
    log::debug!(
        "[mcp_server] HTTP request rejected reason={reason} session={:?} protocol={:?} expected_protocol={:?}",
        redacted_session_id.as_deref(),
        protocol_version,
        expected_protocol
    );
}

fn text_error(status: StatusCode, message: &str) -> Response {
    (status, message.to_string()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::{McpAuthConfig, McpClientIdentityConfig};
    use crate::openhuman::mcp_client::McpHttpClient;
    use serde_json::json;

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
        let client = McpHttpClient::new(endpoint, 5);

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

        let denied = McpHttpClient::with_options(
            endpoint.clone(),
            5,
            McpAuthConfig::BearerToken {
                token: "wrong".into(),
            },
            McpClientIdentityConfig::default(),
        );
        let err = denied.initialize().await.expect_err("bad token");
        assert!(err.to_string().contains("401"), "expected 401, got {err}");

        let allowed = McpHttpClient::with_options(
            endpoint,
            5,
            McpAuthConfig::BearerToken {
                token: "phase1-secret".into(),
            },
            McpClientIdentityConfig::default(),
        );
        allowed.initialize().await.expect("authorized initialize");
    }
}
