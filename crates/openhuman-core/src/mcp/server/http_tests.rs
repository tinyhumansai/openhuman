use super::*;
use async_trait::async_trait;
use axum::body::to_bytes;
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    future::pending,
    sync::{Arc, Mutex},
};
use tinymcp_bus::{
    ServerCallback, ServerHostCall, ServerHostReply, ServerHttpResponseMetadata,
    ServerHttpSessionConfig, ServerHttpSessionRequest, ServerInput, ServerOperationRef,
    ServerOperationSnapshot, ServerOperationState, ServerSessionConfig, HEADER_SESSION_ID,
};

#[derive(Default)]
struct HttpFixtureState {
    input: Option<ServerInput>,
    callback_mode: bool,
}

struct HttpFixturePort(Mutex<HttpFixtureState>);

#[async_trait]
impl ServerPort for HttpFixturePort {
    async fn open(&self, config: ServerSessionConfig) -> Result<String, ModuleCallError> {
        Ok(config.session_id)
    }

    async fn open_http(&self, config: ServerHttpSessionConfig) -> Result<String, ModuleCallError> {
        self.open(config.server).await
    }

    async fn submit(
        &self,
        _session: &str,
        _headers: Map<String, Value>,
        input: ServerInput,
    ) -> Result<(), ModuleCallError> {
        self.0.lock().unwrap().input = Some(input);
        Ok(())
    }

    async fn poll(&self, _session: &str) -> Result<ServerOperationSnapshot, ModuleCallError> {
        let state = self.0.lock().unwrap();
        let input = state.input.clone().expect("request was submitted");
        let operation_state = if state.callback_mode {
            ServerOperationState::Callback {
                callback: ServerCallback {
                    id: "blocked-callback".to_owned(),
                    call: ServerHostCall::ReadResource {
                        source_type: "mcp:test".to_owned(),
                        headers: Map::new(),
                        uri: "openhuman://prompts/identity".to_owned(),
                    },
                },
            }
        } else {
            ServerOperationState::Complete {
                response: Some(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#.to_owned()),
            }
        };
        Ok(ServerOperationSnapshot {
            operation_id: input.operation_id,
            state: operation_state,
            http_response: Some(ServerHttpResponseMetadata {
                status_code: 200,
                headers: BTreeMap::from([
                    (HEADER_SESSION_ID.to_owned(), "wire-session".to_owned()),
                    ("mcp-protocol-version".to_owned(), "2025-11-25".to_owned()),
                ]),
                content_type: Some("application/json".to_owned()),
                error_body: None,
            }),
        })
    }

    async fn complete(
        &self,
        _session: &str,
        _callback: &str,
        _reply: ServerHostReply,
    ) -> Result<(), ModuleCallError> {
        Ok(())
    }

    async fn cancel(&self, _operation: ServerOperationRef) -> Result<(), ModuleCallError> {
        Ok(())
    }

    async fn close(&self, _session: &str) -> Result<(), ModuleCallError> {
        Ok(())
    }

    async fn http_session(
        &self,
        request: ServerHttpSessionRequest,
    ) -> Result<ServerHttpResponseMetadata, ModuleCallError> {
        let version = request
            .headers
            .get("mcp-protocol-version")
            .and_then(Value::as_str);
        let accepted = version == Some("2025-11-25");
        Ok(ServerHttpResponseMetadata {
            status_code: if accepted { 204 } else { 400 },
            headers: BTreeMap::new(),
            content_type: Some("text/plain".to_owned()),
            error_body: (!accepted).then(|| "invalid protocol version".to_owned()),
        })
    }
}

fn http_test_headers(session_id: &str, version: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HEADER_SESSION_ID,
        HeaderValue::from_str(session_id).unwrap(),
    );
    headers.insert(
        "mcp-protocol-version",
        HeaderValue::from_str(version).unwrap(),
    );
    headers.insert("accept", HeaderValue::from_static("application/json"));
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers
}

async fn http_test_session(
    port: Arc<dyn ServerPort>,
    state: &HttpState,
    dispatch: impl Fn(
            ServerHostCall,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = ServerHostReply> + Send + 'static>>
        + Send
        + Sync
        + 'static,
) -> Arc<ServerSession> {
    let session = Arc::new(
        ServerSession::open_with_port_and_dispatch(port, dispatch)
            .await
            .unwrap(),
    );
    state
        .sessions
        .lock()
        .await
        .insert("wire-session".to_owned(), Arc::clone(&session));
    session
}

#[tokio::test]
async fn rejected_http_delete_keeps_session_usable() {
    let port = Arc::new(HttpFixturePort(Mutex::new(HttpFixtureState::default())));
    let state = HttpState {
        port: port.clone(),
        auth_token: None,
        sessions: Arc::default(),
    };
    http_test_session(port, &state, |call| {
        Box::pin(async move {
            let _ = call;
            ServerHostReply::Internal {
                message: "unused fixture callback".to_owned(),
            }
        })
    })
    .await;

    let rejected = http_session_request(
        &state,
        &http_test_headers("wire-session", "unsupported-version"),
        ServerHttpSessionMethod::Delete,
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    assert!(state.sessions.lock().await.contains_key("wire-session"));

    let usable = handle_post(
        State(state.clone()),
        http_test_headers("wire-session", "2025-11-25"),
        Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#),
    )
    .await;
    assert_eq!(usable.status(), StatusCode::OK);
    assert!(state.sessions.lock().await.contains_key("wire-session"));
}

struct CallbackDrop(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for CallbackDrop {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[tokio::test]
async fn successful_http_delete_joins_blocked_callback_before_removing_session() {
    let port = Arc::new(HttpFixturePort(Mutex::new(HttpFixtureState {
        callback_mode: true,
        ..HttpFixtureState::default()
    })));
    let state = HttpState {
        port: port.clone(),
        auth_token: None,
        sessions: Arc::default(),
    };
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let dropped_tx = Arc::new(Mutex::new(Some(dropped_tx)));
    http_test_session(port, &state, move |_call| {
        let started_tx = Arc::clone(&started_tx);
        let dropped_tx = Arc::clone(&dropped_tx);
        Box::pin(async move {
            let _drop_guard = CallbackDrop(dropped_tx.lock().unwrap().take());
            if let Some(sender) = started_tx.lock().unwrap().take() {
                let _ = sender.send(());
            }
            pending::<ServerHostReply>().await
        })
    })
    .await;

    let post_state = State(state.clone());
    let post = tokio::spawn(async move {
        handle_post(
            post_state,
            http_test_headers("wire-session", "2025-11-25"),
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#),
        )
        .await
    });
    started_rx.await.expect("callback started");

    let deleted = handle_delete(
        State(state.clone()),
        http_test_headers("wire-session", "2025-11-25"),
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    dropped_rx.await.expect("DELETE joined callback cleanup");
    assert!(!state.sessions.lock().await.contains_key("wire-session"));
    post.await.expect("POST finished after terminal close");
}

#[tokio::test]
async fn module_http_metadata_controls_status_headers_and_response_body() {
    let response = response_from_metadata(
        tinymcp_bus::ServerHttpResponseMetadata {
            status_code: 202,
            headers: BTreeMap::from([
                ("mcp-session-id".to_owned(), "session-7".to_owned()),
                ("mcp-protocol-version".to_owned(), "2025-11-25".to_owned()),
            ]),
            content_type: Some("application/json".to_owned()),
            error_body: None,
        },
        r#"{"jsonrpc":"2.0","id":4,"result":{}}"#.to_owned(),
    );

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(response.headers()[HEADER_SESSION_ID], "session-7");
    assert_eq!(response.headers()["mcp-protocol-version"], "2025-11-25");
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        r#"{"jsonrpc":"2.0","id":4,"result":{}}"#
    );
}

#[tokio::test]
async fn module_http_error_metadata_preserves_plain_text_status_and_body() {
    let response = response_from_metadata(
        tinymcp_bus::ServerHttpResponseMetadata {
            status_code: 400,
            headers: BTreeMap::new(),
            content_type: Some("text/plain; charset=utf-8".to_owned()),
            error_body: Some("missing or invalid Mcp-Session-Id header".to_owned()),
        },
        String::new(),
    );

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        "missing or invalid Mcp-Session-Id header"
    );
}

#[tokio::test]
async fn invalid_module_http_status_fails_closed_as_internal_server_error() {
    let response = response_from_metadata(
        tinymcp_bus::ServerHttpResponseMetadata {
            status_code: 99,
            headers: BTreeMap::new(),
            content_type: None,
            error_body: Some("bad status".to_owned()),
        },
        String::new(),
    );

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        "bad status"
    );
}

#[test]
fn module_protocol_headers_exclude_host_authentication_secrets() {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer private-host-token"),
    );
    headers.insert(
        "proxy-authorization",
        HeaderValue::from_static("Basic private-proxy-token"),
    );
    headers.insert("cookie", HeaderValue::from_static("session=private-cookie"));
    headers.insert(HEADER_SESSION_ID, HeaderValue::from_static("session-7"));
    headers.insert(
        "mcp-protocol-version",
        HeaderValue::from_static("2025-11-25"),
    );
    headers.insert("accept", HeaderValue::from_static("application/json"));
    let forwarded = request_headers(&headers);
    assert!(!forwarded.contains_key("authorization"));
    assert!(!forwarded.contains_key("proxy-authorization"));
    assert!(!forwarded.contains_key("cookie"));
    assert_eq!(
        forwarded[&HEADER_SESSION_ID.to_ascii_lowercase()],
        "session-7"
    );
    assert_eq!(forwarded["mcp-protocol-version"], "2025-11-25");
    assert_eq!(forwarded["accept"], "application/json");
}
