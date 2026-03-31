use serde::Serialize;

use crate::rpc::RpcOutcome;

fn rpc_json<T: Serialize>(outcome: RpcOutcome<T>) -> Result<serde_json::Value, String> {
    outcome.into_cli_compatible_json()
}

pub async fn try_dispatch(
    method: &str,
    _params: serde_json::Value,
) -> Option<Result<serde_json::Value, String>> {
    match method {
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
