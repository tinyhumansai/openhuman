//! A real loopback Streamable HTTP MCP server for host integration tests.
//!
//! Keeping this fixture at the protocol boundary exercises TinyMCP's actual
//! client while avoiding stdio sandbox/trust policy in tests of registry
//! installation and host tool projection.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

#[derive(Clone)]
struct EchoState {
    calls: Arc<Mutex<Vec<Value>>>,
}

/// An ephemeral actual MCP HTTP endpoint exposing one `echo` tool.
pub struct LocalMcpHttpServer {
    endpoint: String,
    calls: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}

impl LocalMcpHttpServer {
    /// Start the loopback server and return its Streamable HTTP endpoint.
    pub async fn start_echo() -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/", post(handle_request))
            .with_state(EchoState {
                calls: Arc::clone(&calls),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local MCP fixture");
        let endpoint = format!("http://{}/", listener.local_addr().expect("local address"));
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve local MCP fixture");
        });
        Self {
            endpoint,
            calls,
            task,
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Snapshot the actual `tools/call` requests received by the server.
    pub fn tool_calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Drop for LocalMcpHttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle_request(
    State(state): State<EchoState>,
    Json(request): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        "initialize" => json!({
            "protocolVersion": tinymcp_bus::LATEST_PROTOCOL_VERSION,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "openhuman-test-echo", "version": "1.0.0" }
        }),
        "notifications/initialized" => {
            return (StatusCode::NO_CONTENT, Json(Value::Null));
        }
        "tools/list" => json!({
            "tools": [{
                "name": "echo",
                "description": "Echo a message for the OpenHuman harness test.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "message": { "type": "string" } },
                    "required": ["message"]
                }
            }]
        }),
        "tools/call" => {
            state
                .calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(request.get("params").cloned().unwrap_or(Value::Null));
            let message = request
                .pointer("/params/arguments/message")
                .and_then(Value::as_str)
                .unwrap_or("");
            json!({
                "content": [{ "type": "text", "text": message }],
                "isError": false
            })
        }
        _ => {
            return (
                StatusCode::OK,
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": "method not found" }
                })),
            );
        }
    };
    (
        StatusCode::OK,
        Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })),
    )
}
