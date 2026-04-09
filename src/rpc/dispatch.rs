//! Main dispatcher for domain-specific RPC methods.
//!
//! This module routes RPC calls to their respective domain handlers (e.g.,
//! security, memory, local AI). It serves as an extension point for
//! domain-level functionality in the OpenHuman platform.

use serde::Serialize;

use crate::rpc::RpcOutcome;

/// Helper to convert an [`RpcOutcome`] into a JSON value compatible with the CLI.
fn rpc_json<T: Serialize>(outcome: RpcOutcome<T>) -> Result<serde_json::Value, String> {
    outcome.into_cli_compatible_json()
}

/// Dispatches an RPC method to its domain-specific handler.
///
/// If the method is recognized, it executes the handler and returns the
/// result wrapped in a `Some(Result)`. If not recognized, it returns `None`.
///
/// # Arguments
///
/// * `method` - The name of the RPC method to invoke.
/// * `params` - The parameters for the call as a JSON value.
///
/// # Returns
///
/// `Some(Ok(Value))` if successful, `Some(Err(String))` if the handler failed,
/// or `None` if the method was not found in this dispatcher.
pub async fn try_dispatch(
    method: &str,
    _params: serde_json::Value,
) -> Option<Result<serde_json::Value, String>> {
    match method {
        // Core security policy information.
        "openhuman.security_policy_info" => Some(rpc_json(
            crate::openhuman::security::rpc::security_policy_info(),
        )),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::try_dispatch;

    /// Unknown methods must return `None` so callers can fall through.
    #[tokio::test]
    async fn dispatch_returns_none_for_unknown_method() {
        let result = try_dispatch("nonexistent.method", json!({})).await;
        assert!(result.is_none(), "unknown methods should return None");
    }
}
