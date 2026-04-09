//! Shared core-level type definitions and response formats.
//!
//! This module contains structs and methods for handling RPC requests and
//! responses, as well as maintaining application state across subsystems.

use serde::{Deserialize, Serialize};
use serde_json::json;

/// Standard response structure for commands that include execution logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse<T> {
    /// The primary data returned by the command.
    pub result: T,
    /// A list of log messages generated during command execution.
    pub logs: Vec<String>,
}

/// Success payload from a core RPC handler before JSON-RPC wrapping.
///
/// This internal type allows handlers to return a generic JSON value along
/// with optional logs.
#[derive(Debug, Clone)]
pub struct InvocationResult {
    /// The value returned by the RPC function call.
    pub value: serde_json::Value,
    /// A list of execution logs.
    pub logs: Vec<String>,
}

impl InvocationResult {
    /// Creates a success result from any serializable value with no logs.
    pub fn ok<T: Serialize>(v: T) -> Result<Self, String> {
        Ok(Self {
            value: serde_json::to_value(v).map_err(|e| e.to_string())?,
            logs: vec![],
        })
    }

    /// Creates a success result from a serializable value with accompanying logs.
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
pub fn invocation_to_rpc_json(inv: InvocationResult) -> serde_json::Value {
    if inv.logs.is_empty() {
        inv.value
    } else {
        json!({ "result": inv.value, "logs": inv.logs })
    }
}

/// Standard JSON-RPC request format.
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    /// The JSON-RPC version (e.g., `2.0`).
    #[allow(dead_code)]
    pub jsonrpc: String,
    /// Unique identifier for the request, to be mirrored in the response.
    pub id: serde_json::Value,
    /// The name of the method to be invoked.
    pub method: String,
    /// Parameters for the method call. Defaults to null if not provided.
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Standard JSON-RPC success response format.
#[derive(Debug, Serialize)]
pub struct RpcSuccess {
    /// The JSON-RPC version (always `2.0`).
    pub jsonrpc: &'static str,
    /// The identifier mirrored from the original request.
    pub id: serde_json::Value,
    /// The result of the successful method invocation.
    pub result: serde_json::Value,
}

/// Standard JSON-RPC error response format.
#[derive(Debug, Serialize)]
pub struct RpcFailure {
    /// The JSON-RPC version (always `2.0`).
    pub jsonrpc: &'static str,
    /// The identifier mirrored from the original request.
    pub id: serde_json::Value,
    /// Information about the error that occurred.
    pub error: RpcError,
}

/// Detail about an RPC invocation error.
#[derive(Debug, Serialize)]
pub struct RpcError {
    /// Standardized error code (e.g., -32601 for Method not found).
    pub code: i64,
    /// A short, human-readable error message.
    pub message: String,
    /// Optional additional diagnostic data.
    pub data: Option<serde_json::Value>,
}

/// Global core-level application state.
#[derive(Clone)]
pub struct AppState {
    /// The current version of the OpenHuman core binary.
    pub core_version: String,
}
