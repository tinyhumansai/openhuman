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
mod structured_error;

pub use dispatch::try_dispatch;
pub use structured_error::{StructuredRpcError, STRUCTURED_RPC_ERROR_SENTINEL};

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_preserves_value_and_logs() {
        let outcome: RpcOutcome<i64> = RpcOutcome::new(7, vec!["a".into(), "b".into()]);
        assert_eq!(outcome.value, 7);
        assert_eq!(outcome.logs, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn single_log_stores_exactly_one_log() {
        let outcome = RpcOutcome::single_log(json!({"ok": true}), "hello");
        assert_eq!(outcome.logs.len(), 1);
        assert_eq!(outcome.logs[0], "hello");
        assert_eq!(outcome.value, json!({"ok": true}));
    }

    #[test]
    fn single_log_accepts_string_and_str_via_into() {
        let a = RpcOutcome::single_log(json!(1), "static str");
        let b = RpcOutcome::single_log(json!(1), String::from("owned string"));
        assert_eq!(a.logs[0], "static str");
        assert_eq!(b.logs[0], "owned string");
    }

    #[test]
    fn into_cli_compatible_json_no_logs_returns_bare_value() {
        let outcome = RpcOutcome::<serde_json::Value>::new(json!({"x": 1}), vec![]);
        let out = outcome.into_cli_compatible_json().unwrap();
        assert_eq!(out, json!({"x": 1}));
        assert!(out.get("logs").is_none());
    }

    #[test]
    fn into_cli_compatible_json_with_logs_wraps_in_envelope() {
        let outcome = RpcOutcome::single_log(json!(42), "did something");
        let out = outcome.into_cli_compatible_json().unwrap();
        assert_eq!(out["result"], json!(42));
        assert_eq!(out["logs"], json!(["did something"]));
        // And only those two keys exist.
        assert_eq!(out.as_object().unwrap().len(), 2);
    }

    #[test]
    fn into_cli_compatible_json_serializes_typed_value() {
        #[derive(serde::Serialize)]
        struct Payload<'a> {
            name: &'a str,
            count: u32,
        }
        let outcome = RpcOutcome::new(
            Payload {
                name: "atlas",
                count: 3,
            },
            vec![],
        );
        let out = outcome.into_cli_compatible_json().unwrap();
        assert_eq!(out, json!({"name": "atlas", "count": 3}));
    }

    #[test]
    fn into_cli_compatible_json_treats_null_value_as_bare_when_no_logs() {
        let outcome: RpcOutcome<Option<i32>> = RpcOutcome::new(None, vec![]);
        let out = outcome.into_cli_compatible_json().unwrap();
        assert!(out.is_null());
    }

    #[test]
    fn into_cli_compatible_json_preserves_log_order() {
        let outcome = RpcOutcome::new(
            json!({"ok": true}),
            vec!["first".into(), "second".into(), "third".into()],
        );
        let out = outcome.into_cli_compatible_json().unwrap();
        assert_eq!(out["logs"], json!(["first", "second", "third"]));
    }

    #[test]
    fn into_cli_compatible_json_empty_string_logs_still_envelope() {
        // An empty log string is still a log — envelope shape must kick in.
        let outcome = RpcOutcome::new(json!("x"), vec!["".into()]);
        let out = outcome.into_cli_compatible_json().unwrap();
        assert!(out.get("result").is_some());
        assert_eq!(out["logs"], json!([""]));
    }
}
