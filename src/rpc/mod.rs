//! Shared types for JSON-RPC / CLI controller surfaces.
//!
//! This module provides the foundational types and utilities for handling
//! RPC outcomes across different domain modules. It ensures a consistent
//! response format for both internal consumption and external presentation.
//!
//! Domain `rpc` modules should use [`RpcOutcome`] to wrap their results,
//! which facilitates consistent logging and error handling.

use serde::Serialize;
use serde_json::json;

mod dispatch;

pub use dispatch::try_dispatch;

/// Successful RPC handler result: serialized JSON value plus optional log lines.
///
/// This type represents the result of a domain-specific RPC call, including
/// any log messages generated during execution.
#[derive(Debug)]
pub struct RpcOutcome<T> {
    /// The actual data returned by the RPC call.
    pub value: T,
    /// A collection of log messages for auditing or debugging.
    pub logs: Vec<String>,
}

impl<T> RpcOutcome<T> {
    /// Creates a new `RpcOutcome` with a value and a list of logs.
    pub fn new(value: T, logs: Vec<String>) -> Self {
        Self { value, logs }
    }
}

impl<T: Serialize> RpcOutcome<T> {
    /// Creates a new `RpcOutcome` with a value and a single log message.
    pub fn single_log(value: T, log: impl Into<String>) -> Self {
        Self {
            value,
            logs: vec![log.into()],
        }
    }

    /// Converts the outcome into a CLI-compatible JSON value.
    ///
    /// The resulting JSON shape matches the core CLI expectations:
    /// - If no logs are present, the value is returned directly.
    /// - If logs are present, an object with `result` and `logs` keys is returned.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization to JSON fails.
    pub fn into_cli_compatible_json(self) -> Result<serde_json::Value, String> {
        let RpcOutcome { value, logs } = self;
        let value = serde_json::to_value(value).map_err(|e| e.to_string())?;
        if logs.is_empty() {
            Ok(value)
        } else {
            Ok(json!({ "result": value, "logs": logs }))
        }
    }
}
