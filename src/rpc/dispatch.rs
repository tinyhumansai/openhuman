//! Legacy compatibility shim for domain-specific RPC dispatch.
//!
//! Domain routing now lives in the controller registry (`src/core/all.rs`).
//! This module is intentionally minimal so callers can fall through to
//! unknown-method handling while older call sites remain compile-compatible.

/// Dispatches an RPC method to legacy handlers.
///
/// Returns `None` for all methods; controller-registry dispatch is authoritative.
pub async fn try_dispatch(
    _method: &str,
    _params: serde_json::Value,
) -> Option<Result<serde_json::Value, String>> {
    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::try_dispatch;

    #[tokio::test]
    async fn dispatch_returns_none_for_unknown_method() {
        let result = try_dispatch("nonexistent.method", json!({})).await;
        assert!(result.is_none(), "unknown methods should return None");
    }

    #[tokio::test]
    async fn dispatch_security_method_now_falls_through() {
        let result = try_dispatch("openhuman.security_policy_info", json!({})).await;
        assert!(
            result.is_none(),
            "security method should be handled by registry path"
        );
    }
}
