//! JSON-RPC / CLI controller surface for security policy introspection.

use serde_json::json;

use crate::config::Config;
use crate::core::Outcome;
use crate::security::SecurityPolicy;

fn policy_info_payload(policy: SecurityPolicy) -> serde_json::Value {
    json!({
        "autonomy": policy.autonomy,
        "workspace_only": policy.workspace_only,
        "allowed_commands": policy.allowed_commands,
        "max_actions_per_hour": policy.max_actions_per_hour,
        "require_approval_for_medium_risk": policy.require_approval_for_medium_risk,
        "block_high_risk_commands": policy.block_high_risk_commands,
    })
}

pub fn security_policy_info_for_config(config: &Config) -> Outcome<serde_json::Value> {
    let policy =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir)
            .with_account_dir(config.config_path.parent());
    let payload = policy_info_payload(policy);
    Outcome::single_log(payload, "security_policy_info computed from active config")
}

pub async fn load_and_get_security_policy_info() -> Result<Outcome<serde_json::Value>, String> {
    let config = crate::config::ops::load_config_with_timeout().await?;
    Ok(security_policy_info_for_config(&config))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
