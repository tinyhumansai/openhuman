//! Streamable HTTP + SSE transport for the OpenHuman MCP server.
//!
//! Reuses [`super::protocol`] for JSON-RPC dispatch. Session lifecycle and header
//! names match [`crate::openhuman::mcp_client::client::McpHttpClient`] so remote
//! MCP clients can talk to this server without custom glue.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        HeaderMap, StatusCode,
    },
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use parking_lot::Mutex;
use serde_json::Value;
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
}

pub async fn run_http(config: HttpServerConfig) -> Result<()> {
    let state = AppState {
        sessions: Arc::new(Mutex::new(HashMap::new())),
        auth_token: config.auth_token.clone(),
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

    log::debug!(
        "[mcp_server] HTTP POST method={rpc_method} session={:?} protocol={:?}",
        session_id,
        protocol_version
    );

    if rpc_method == "initialize" {
        return handle_initialize(&state, body).await;
    }

    let Some(session_id) = session_id else {
        return text_error(
            StatusCode::BAD_REQUEST,
            "missing or invalid Mcp-Session-Id header",
        );
    };

    let expected_protocol = {
        let sessions = state.sessions.lock();
        let Some(record) = sessions.get(session_id) else {
            return text_error(StatusCode::NOT_FOUND, "unknown or expired MCP session");
        };
        record.protocol_version.clone()
    };

    if protocol_version.as_deref() != Some(expected_protocol.as_str()) {
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
    log::debug!("[mcp_server] HTTP session created id={session_id} protocol={negotiated}");
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

    let Some(session_id) = header_value(&headers, HEADER_SESSION_ID) else {
        return text_error(StatusCode::BAD_REQUEST, "missing Mcp-Session-Id header");
    };
    if !state.sessions.lock().contains_key(session_id) {
        return text_error(StatusCode::NOT_FOUND, "unknown or expired MCP session");
    }

    // Phase 1: no server-initiated notifications yet; return an empty SSE stream
    // so clients can open the events channel without error.
    (
        [(CONTENT_TYPE.as_str(), "text/event-stream")],
        ": openhuman mcp events\n\n",
    )
        .into_response()
}

async fn handle_delete(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(response) = check_auth(&state, &headers) {
        return response;
    }

    let Some(session_id) = header_value(&headers, HEADER_SESSION_ID) else {
        return text_error(StatusCode::BAD_REQUEST, "missing Mcp-Session-Id header");
    };

    if state.sessions.lock().remove(session_id).is_some() {
        log::debug!("[mcp_server] HTTP session closed id={session_id}");
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = AppState {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            auth_token: auth_token.map(str::to_string),
        };
        let app = Router::new()
            .route("/", post(handle_post).get(handle_get).delete(handle_delete))
            .with_state(state);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}/")
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

        let events = client.drain_events(None).await.expect("GET events");
        assert!(events.is_empty());

        client.close_session().await.expect("DELETE session");
    }

    #[tokio::test]
    async fn http_rejects_requests_without_session_after_initialize() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = AppState {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            auth_token: None,
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
