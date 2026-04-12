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
