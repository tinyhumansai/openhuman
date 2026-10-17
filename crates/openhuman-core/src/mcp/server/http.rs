//! OpenHuman's thin Streamable HTTP adapter over TinyMCP's server contract.

use std::{collections::HashMap, convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    body::Bytes,
    extract::State,
    http::{header::AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::post,
    Router,
};
use futures_util::stream;
use serde_json::{Map, Value};
use tinymcp_bus::{ServerHttpSessionMethod, ServerHttpSessionRequest, HEADER_SESSION_ID};

use crate::config::Config;
use tinybus::module::ModuleCallError;

use super::module::{headers_object, ServerError, ServerPort, ServerSession};

/// HTTP listener configuration kept at OpenHuman's transport boundary.
#[derive(Debug, Clone)]
pub struct HttpServerConfig {
    pub bind_addr: SocketAddr,
    pub auth_token: Option<String>,
}

#[derive(Clone)]
struct HttpState {
    port: Arc<dyn ServerPort>,
    auth_token: Option<String>,
    sessions: Arc<tokio::sync::Mutex<HashMap<String, Arc<ServerSession>>>>,
}

/// Serve the OpenHuman MCP surface over module-owned HTTP protocol state.
pub async fn run_http(config: HttpServerConfig) -> anyhow::Result<()> {
    run_http_reporting(config, None).await
}

/// Serve the endpoint and report the bound address after the listener is ready.
pub async fn run_http_reporting(
    config: HttpServerConfig,
    ready: Option<tokio::sync::oneshot::Sender<SocketAddr>>,
) -> anyhow::Result<()> {
    let host_config = Config::load_or_init().await?;
    let port = ServerSession::port_for_config(&host_config);
    let state = HttpState {
        port,
        auth_token: config.auth_token,
        sessions: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    let local_addr = listener.local_addr()?;
    if let Some(sender) = ready {
        let _ = sender.send(local_addr);
    }
    let result = axum::serve(listener, router(state.clone())).await;
    close_http_sessions(&state).await;
    result.map_err(anyhow::Error::from)
}

fn router(state: HttpState) -> Router {
    Router::new()
        .route("/", post(handle_post).get(handle_get).delete(handle_delete))
        .with_state(state)
}

async fn handle_post(State(state): State<HttpState>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(response) = check_auth(&state, &headers) {
        return response;
    }
    let line = match String::from_utf8(body.to_vec()) {
        Ok(line) => line,
        Err(_) => return text_error(StatusCode::BAD_REQUEST, "invalid request body"),
    };
    let requested_session_id = header_value(&headers, HEADER_SESSION_ID).map(str::to_owned);
    let session = if let Some(requested_session_id) = requested_session_id.as_deref() {
        state
            .sessions
            .lock()
            .await
            .get(requested_session_id)
            .cloned()
    } else {
        None
    };
    let known_session = session.is_some();
    let session = match session {
        Some(session) => session,
        None => match ServerSession::open_http(
            Arc::clone(&state.port),
            requested_session_id.clone(),
            false,
            None,
        )
        .await
        {
            Ok(session) => Arc::new(session),
            Err(error) => return server_error(error),
        },
    };
    let is_temporary = !known_session;
    let response = match session
        .submit(
            request_headers(&headers),
            line,
            &tokio_util::sync::CancellationToken::new(),
        )
        .await
    {
        Ok(snapshot) => http_operation_response(snapshot),
        Err(error) => server_error(error),
    };
    if is_temporary {
        let wire_id = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|header| header.to_str().ok())
            .map(str::to_owned);
        if let Some(wire_id) = wire_id {
            state
                .sessions
                .lock()
                .await
                .insert(wire_id, Arc::clone(&session));
        } else if let Err(error) = session.close().await {
            tracing::warn!(?error, "[mcp_server] temporary HTTP session cleanup failed");
        }
    }
    response
}

fn http_operation_response(snapshot: tinymcp_bus::ServerOperationSnapshot) -> Response {
    let Some(metadata) = snapshot.http_response else {
        return server_error(ServerError::MissingHttpMetadata);
    };
    let body = match snapshot.state {
        tinymcp_bus::ServerOperationState::Complete { response } => response.unwrap_or_default(),
        _ => String::new(),
    };
    response_from_metadata(metadata, body)
}

async fn handle_get(State(state): State<HttpState>, headers: HeaderMap) -> Response {
    http_session_request(&state, &headers, ServerHttpSessionMethod::Get).await
}

async fn handle_delete(State(state): State<HttpState>, headers: HeaderMap) -> Response {
    http_session_request(&state, &headers, ServerHttpSessionMethod::Delete).await
}

async fn http_session_request(
    state: &HttpState,
    headers: &HeaderMap,
    method: ServerHttpSessionMethod,
) -> Response {
    if let Some(response) = check_auth(state, headers) {
        return response;
    }
    let requested_session_id = header_value(headers, HEADER_SESSION_ID).map(str::to_owned);
    let session = if let Some(requested_session_id) = requested_session_id.as_deref() {
        state
            .sessions
            .lock()
            .await
            .get(requested_session_id)
            .cloned()
    } else {
        None
    };
    let internal_session_id = session.as_ref().map(|session| session.id().to_owned());
    let request = ServerHttpSessionRequest {
        method: method.clone(),
        requested_session_id: requested_session_id.clone(),
        internal_session_id: internal_session_id.clone(),
        headers: request_headers(headers),
    };
    let metadata = match state.port.http_session(request).await {
        Ok(metadata) => metadata,
        Err(error) => return server_error(ServerError::Module(error)),
    };
    let response = response_from_metadata(metadata, String::new());
    if method == ServerHttpSessionMethod::Get && response.status() == StatusCode::OK {
        return Sse::new(stream::pending::<Result<Event, Infallible>>())
            .keep_alive(
                KeepAlive::new()
                    .interval(Duration::from_secs(10))
                    .text("keepalive"),
            )
            .into_response();
    }
    if method == ServerHttpSessionMethod::Delete && response.status() == StatusCode::NO_CONTENT {
        let local_cleanup = match session.as_ref() {
            Some(session) => session.terminate_callbacks().await.err(),
            None => None,
        };
        if let Some(requested_session_id) = requested_session_id {
            state.sessions.lock().await.remove(&requested_session_id);
        }
        if let Some(error) = local_cleanup {
            return server_error(error);
        }
    }
    response
}

async fn close_http_sessions(state: &HttpState) {
    let sessions = std::mem::take(&mut *state.sessions.lock().await);
    for (_, session) in sessions {
        if let Err(error) = session.close().await {
            tracing::warn!(?error, "[mcp_server] HTTP session cleanup failed");
        }
    }
}

fn check_auth(state: &HttpState, headers: &HeaderMap) -> Option<Response> {
    let expected = state.auth_token.as_deref()?;
    let provided = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    (provided != Some(expected)).then(|| {
        (
            StatusCode::UNAUTHORIZED,
            [("content-type", "text/plain")],
            "unauthorized",
        )
            .into_response()
    })
}

fn request_headers(headers: &HeaderMap) -> Map<String, Value> {
    headers_object(headers.iter().filter_map(|(name, value)| {
        // Authentication is checked by the host. Its credentials are never
        // protocol input for the module's ordinary bus calls.
        if matches!(
            name.as_str(),
            "authorization" | "proxy-authorization" | "cookie"
        ) {
            return None;
        }
        value
            .to_str()
            .ok()
            .map(|value| (name.as_str().to_ascii_lowercase(), value.to_owned()))
    }))
}

fn response_from_metadata(
    metadata: tinymcp_bus::ServerHttpResponseMetadata,
    body: String,
) -> Response {
    let status =
        StatusCode::from_u16(metadata.status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = if let Some(error_body) = metadata.error_body {
        (status, error_body).into_response()
    } else if body.is_empty() {
        status.into_response()
    } else {
        (status, [("content-type", "application/json")], body).into_response()
    };
    for (name, value) in metadata.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            response.headers_mut().insert(name, value);
        }
    }
    if let Some(content_type) = metadata.content_type {
        if let Ok(value) = HeaderValue::from_str(&content_type) {
            response
                .headers_mut()
                .insert(axum::http::header::CONTENT_TYPE, value);
        }
    }
    response
}

fn server_error(error: ServerError) -> Response {
    let status = match error {
        ServerError::Module(ModuleCallError::Unavailable) => StatusCode::SERVICE_UNAVAILABLE,
        ServerError::Module(_) => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    text_error(status, "MCP server unavailable")
}

fn text_error(status: StatusCode, body: &'static str) -> Response {
    (
        status,
        [("content-type", "text/plain; charset=utf-8")],
        body,
    )
        .into_response()
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
