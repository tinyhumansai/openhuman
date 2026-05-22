//! MCP JSON-RPC protocol framing: request serialisation, response correlation,
//! and higher-level method helpers (`initialize`, `tools/list`, `tools/call`).
//!
//! All methods are async and use a 30-second timeout by default.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};

use super::transport::{PendingMap, MCP_PROTOCOL_VERSION};
use crate::openhuman::mcp_clients::types::McpTool;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Trait abstracting the MCP stdio transport so tests can inject fakes.
#[async_trait]
pub trait McpTransport: Send + Sync + 'static {
    /// Send an MCP `initialize` request and return the server's result.
    async fn initialize(&self) -> Result<Value, String>;

    /// Send a `tools/list` request and return the parsed tool list.
    async fn list_tools(&self) -> Result<Vec<McpTool>, String>;

    /// Send a `tools/call` request.
    async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value, String>;

    /// Gracefully shut down (send `notifications/cancelled` or just close).
    async fn shutdown(&self);
}

// ── Shared request-counter helper ────────────────────────────────────────────

pub struct RequestIdCounter(Arc<Mutex<u64>>);

impl RequestIdCounter {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(0)))
    }

    pub async fn next(&self) -> u64 {
        let mut guard = self.0.lock().await;
        *guard += 1;
        *guard
    }
}

// ── Dispatch helper used by the real transport ────────────────────────────────

/// Send one JSON-RPC request message and await the response with timeout.
///
/// The caller owns the `pending` map and the writer lock; this function
/// inserts a oneshot sender into `pending`, writes the message, then
/// awaits the receiver with `REQUEST_TIMEOUT`.
pub async fn send_request_and_wait(
    id: u64,
    msg: Value,
    pending: &PendingMap,
    write_fn: impl std::future::Future<Output = anyhow::Result<()>>,
) -> Result<Value, String> {
    let (tx, rx) = oneshot::channel::<Result<Value, String>>();
    {
        let mut map = pending.lock().await;
        map.insert(id, tx);
    }

    if let Err(e) = write_fn.await {
        let mut map = pending.lock().await;
        map.remove(&id);
        return Err(format!("[mcp-client] write failed id={id}: {e}"));
    }

    match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(format!("[mcp-client] channel dropped for id={id}")),
        Err(_) => {
            let mut map = pending.lock().await;
            map.remove(&id);
            Err(format!("[mcp-client] timeout waiting for id={id}"))
        }
    }
}

// ── Parse tools/list response ─────────────────────────────────────────────────

pub fn parse_tools_list(result: Value) -> Result<Vec<McpTool>, String> {
    let tools_arr = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| "tools/list response missing 'tools' array".to_string())?;

    let mut tools = Vec::new();
    for t in tools_arr {
        let name = t
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "tool entry missing 'name' field".to_string())?
            .to_string();
        let description = t
            .get("description")
            .and_then(Value::as_str)
            .map(String::from);
        let input_schema = t.get("inputSchema").cloned().unwrap_or_else(
            || json!({ "type": "object", "properties": {}, "additionalProperties": true }),
        );
        tools.push(McpTool {
            name,
            description,
            input_schema,
        });
    }
    Ok(tools)
}

/// Build an MCP JSON-RPC request object.
pub fn build_request(id: u64, method: &str, params: Option<Value>) -> Value {
    let mut obj = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
    });
    if let Some(p) = params {
        obj["params"] = p;
    }
    obj
}

/// Build an MCP `initialize` params payload.
pub fn build_initialize_params() -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "clientInfo": {
            "name": "openhuman",
            "version": env!("CARGO_PKG_VERSION")
        },
        "capabilities": {}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_request_includes_method_and_id() {
        let req = build_request(7, "tools/list", None);
        assert_eq!(req["jsonrpc"], "2.0");
        assert_eq!(req["id"], 7);
        assert_eq!(req["method"], "tools/list");
        assert!(req.get("params").is_none() || req["params"] == Value::Null);
    }

    #[test]
    fn build_request_with_params() {
        let params = json!({ "name": "my_tool", "arguments": {} });
        let req = build_request(3, "tools/call", Some(params.clone()));
        assert_eq!(req["params"], params);
    }

    #[test]
    fn parse_tools_list_happy_path() {
        let result = json!({
            "tools": [
                {
                    "name": "search",
                    "description": "Web search",
                    "inputSchema": { "type": "object" }
                }
            ]
        });
        let tools = parse_tools_list(result).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "search");
        assert_eq!(tools[0].description.as_deref(), Some("Web search"));
    }

    #[test]
    fn parse_tools_list_missing_tools_key_errors() {
        let result = json!({ "something_else": [] });
        let err = parse_tools_list(result).unwrap_err();
        assert!(err.contains("'tools'"));
    }

    #[test]
    fn parse_tools_list_tool_missing_name_errors() {
        let result = json!({ "tools": [{ "description": "no name" }] });
        let err = parse_tools_list(result).unwrap_err();
        assert!(err.contains("'name'"));
    }

    #[test]
    fn parse_tools_list_no_input_schema_gets_default() {
        let result = json!({ "tools": [{ "name": "tool_no_schema" }] });
        let tools = parse_tools_list(result).unwrap();
        assert_eq!(tools[0].input_schema["type"], "object");
    }

    #[test]
    fn build_initialize_params_has_protocol_version() {
        let params = build_initialize_params();
        assert_eq!(params["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert!(params["clientInfo"]["name"].as_str().is_some());
    }

    #[tokio::test]
    async fn request_id_counter_increments() {
        let counter = RequestIdCounter::new();
        assert_eq!(counter.next().await, 1);
        assert_eq!(counter.next().await, 2);
        assert_eq!(counter.next().await, 3);
    }
}
