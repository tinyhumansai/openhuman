//! JSON-RPC / CLI controller surface for security policy introspection.

use serde_json::json;

use crate::openhuman::security::SecurityPolicy;
use crate::rpc::RpcOutcome;

pub fn security_policy_info() -> RpcOutcome<serde_json::Value> {
    let policy = SecurityPolicy::default();
    let payload = json!({
        "autonomy": policy.autonomy,
        "workspace_only": policy.workspace_only,
        "allowed_commands": policy.allowed_commands,
        "max_actions_per_hour": policy.max_actions_per_hour,
        "require_approval_for_medium_risk": policy.require_approval_for_medium_risk,
        "block_high_risk_commands": policy.block_high_risk_commands,
    });
    RpcOutcome::single_log(payload, "security_policy_info computed")
}
