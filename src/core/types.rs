//! Shared core-level type definitions and response formats.
//!
//! This module contains structs and methods for handling RPC requests and
//! responses, as well as maintaining application state across subsystems.

use serde::{Deserialize, Serialize};
use serde_json::json;

/// Standard response structure for commands that include execution logs.
///
/// This is commonly used in internal APIs and CLI outputs where it's
/// important to see the side-effects or diagnostic information alongside
/// the primary result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse<T> {
    /// The primary data returned by the command.
    pub result: T,
    /// A list of log messages generated during command execution.
    /// These can include warnings, info, or trace messages.
    pub logs: Vec<String>,
}

/// Success payload from a core RPC handler before JSON-RPC wrapping.
///
/// This internal type allows handlers to return a generic JSON value along
/// with optional logs. It is transformed into a [`RpcSuccess`] or a
/// combined object by [`invocation_to_rpc_json`].
#[derive(Debug, Clone)]
pub struct InvocationResult {
    /// The value returned by the RPC function call, serialized to JSON.
    pub value: serde_json::Value,
    /// A list of execution logs.
    pub logs: Vec<String>,
}

impl InvocationResult {
    /// Creates a success result from any serializable value with no logs.
    ///
    /// This is the most common way to return a value from a controller.
    pub fn ok<T: Serialize>(v: T) -> Result<Self, String> {
        Ok(Self {
            value: serde_json::to_value(v).map_err(|e| e.to_string())?,
            logs: vec![],
        })
    }

    /// Creates a success result from a serializable value with accompanying logs.
    ///
    /// Use this when the domain logic has meaningful logs to surface to the caller.
    pub fn with_logs<T: Serialize>(v: T, logs: Vec<String>) -> Result<Self, String> {
        Ok(Self {
            value: serde_json::to_value(v).map_err(|e| e.to_string())?,
            logs,
        })
    }
}

/// Formats an [`InvocationResult`] into its standard JSON-RPC format.
///
/// If there are no logs, returns the value directly. Otherwise, returns an
/// object containing both `result` and `logs` keys.
///
/// # Logic
///
/// - `logs.is_empty()` -> `inv.value`
/// - `!logs.is_empty()` -> `{ "result": inv.value, "logs": inv.logs }`
pub fn invocation_to_rpc_json(inv: InvocationResult) -> serde_json::Value {
    if inv.logs.is_empty() {
        inv.value
    } else {
        json!({ "result": inv.value, "logs": inv.logs })
    }
}

/// Standard JSON-RPC 2.0 request format.
///
/// As defined in the [JSON-RPC 2.0 Specification](https://www.jsonrpc.org/specification).
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    /// The JSON-RPC version. MUST be exactly "2.0".
    #[allow(dead_code)]
    pub jsonrpc: String,
    /// Unique identifier for the request. MUST be a String, Number, or Null.
    /// The server will return this same ID in the response.
    pub id: serde_json::Value,
    /// The name of the method to be invoked (e.g., `openhuman.memory_doc_put`).
    pub method: String,
    /// Parameters for the method call. MUST be a structured value (Object or Array).
    /// Defaults to null if not provided.
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Standard JSON-RPC 2.0 success response format.
#[derive(Debug, Serialize)]
pub struct RpcSuccess {
    /// The JSON-RPC version. ALWAYS "2.0".
    pub jsonrpc: &'static str,
    /// The identifier mirrored from the original request.
    pub id: serde_json::Value,
    /// The result of the successful method invocation.
    pub result: serde_json::Value,
}

/// Standard JSON-RPC 2.0 error response format.
#[derive(Debug, Serialize)]
pub struct RpcFailure {
    /// The JSON-RPC version. ALWAYS "2.0".
    pub jsonrpc: &'static str,
    /// The identifier mirrored from the original request.
    pub id: serde_json::Value,
    /// Information about the error that occurred.
    pub error: RpcError,
}

/// Detail about an RPC invocation error.
///
/// Contains a code, a message, and optional extra data for debugging.
#[derive(Debug, Serialize)]
pub struct RpcError {
    /// Standardized error code.
    /// - -32700: Parse error
    /// - -32600: Invalid Request
    /// - -32601: Method not found
    /// - -32602: Invalid params
    /// - -32603: Internal error
    /// - -32000 to -32099: Reserved for implementation-defined server-errors.
    pub code: i64,
    /// A short, human-readable error message.
    pub message: String,
    /// Optional additional diagnostic data, which can be any JSON value.
    pub data: Option<serde_json::Value>,
}

/// Global core-level application state.
///
/// Currently holds shared metadata like the core version.
#[derive(Clone)]
pub struct AppState {
    /// The current version of the OpenHuman core binary, usually from `CARGO_PKG_VERSION`.
    pub core_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn invocation_result_ok_serializes_value() {
        let result = InvocationResult::ok(json!({"key": "value"})).unwrap();
        assert_eq!(result.value, json!({"key": "value"}));
        assert!(result.logs.is_empty());
    }

    #[test]
    fn invocation_result_with_logs() {
        let result =
            InvocationResult::with_logs(json!(42), vec!["log1".into(), "log2".into()]).unwrap();
        assert_eq!(result.value, json!(42));
        assert_eq!(result.logs.len(), 2);
    }

    #[test]
    fn invocation_to_rpc_json_no_logs_returns_value_directly() {
        let inv = InvocationResult {
            value: json!({"data": true}),
            logs: vec![],
        };
        let json = invocation_to_rpc_json(inv);
        assert_eq!(json, json!({"data": true}));
    }

    #[test]
    fn invocation_to_rpc_json_with_logs_wraps_in_envelope() {
        let inv = InvocationResult {
            value: json!({"data": true}),
            logs: vec!["info".into()],
        };
        let json = invocation_to_rpc_json(inv);
        assert!(json.get("result").is_some());
        assert!(json.get("logs").is_some());
        assert_eq!(json["result"], json!({"data": true}));
        assert_eq!(json["logs"][0], "info");
    }

    #[test]
    fn command_response_serde_roundtrip() {
        let resp = CommandResponse {
            result: "ok".to_string(),
            logs: vec!["log1".into()],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: CommandResponse<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.result, "ok");
        assert_eq!(back.logs.len(), 1);
    }

    #[test]
    fn rpc_request_deserializes() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"test","params":{}}"#;
        let req: RpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "test");
        assert_eq!(req.id, json!(1));
    }

    #[test]
    fn rpc_request_params_default_to_null() {
        let json = r#"{"jsonrpc":"2.0","id":"abc","method":"foo"}"#;
        let req: RpcRequest = serde_json::from_str(json).unwrap();
        assert!(req.params.is_null());
    }

    #[test]
    fn rpc_success_serializes() {
        let resp = RpcSuccess {
            jsonrpc: "2.0",
            id: json!(42),
            result: json!({"ok": true}),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"id\":42"));
    }

    #[test]
    fn rpc_failure_serializes() {
        let resp = RpcFailure {
            jsonrpc: "2.0",
            id: json!("req-1"),
            error: RpcError {
                code: -32601,
                message: "Method not found".into(),
                data: Some(json!({"detail": "unknown"})),
            },
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("-32601"));
        assert!(json.contains("Method not found"));
    }

    #[test]
    fn rpc_failure_serializes_without_data() {
        let resp = RpcFailure {
            jsonrpc: "2.0",
            id: json!(null),
            error: RpcError {
                code: -32700,
                message: "Parse error".into(),
                data: None,
            },
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("-32700"));
    }

    #[test]
    fn app_state_clone() {
        let state = AppState {
            core_version: "0.1.0".into(),
        };
        let cloned = state.clone();
        assert_eq!(cloned.core_version, "0.1.0");
    }
}
