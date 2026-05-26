//! JSON-RPC / CLI controller surface for security policy introspection.

use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::security::SecurityPolicy;
use crate::rpc::RpcOutcome;

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

pub fn security_policy_info_for_config(config: &Config) -> RpcOutcome<serde_json::Value> {
    let policy = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);
    let payload = policy_info_payload(policy);
    RpcOutcome::single_log(payload, "security_policy_info computed from active config")
}

pub async fn load_and_get_security_policy_info() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = crate::openhuman::config::ops::load_config_with_timeout().await?;
    Ok(security_policy_info_for_config(&config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_policy_info_returns_all_documented_fields() {
        // Locks in the JSON shape the JSON-RPC clients depend on —
        // any rename / removal of a field would break the UI.
        let outcome = security_policy_info_for_config(&Config::default());
        for key in [
            "autonomy",
            "workspace_only",
            "allowed_commands",
            "max_actions_per_hour",
            "require_approval_for_medium_risk",
            "block_high_risk_commands",
        ] {
            assert!(
                outcome.value.get(key).is_some(),
                "missing `{key}` in security_policy_info payload: {}",
                outcome.value
            );
        }
        assert!(outcome
            .logs
            .iter()
            .any(|l| l.contains("security_policy_info computed")));
    }

    #[test]
    fn security_policy_info_matches_default_config_policy_values() {
        let outcome = security_policy_info_for_config(&Config::default());
        let config = Config::default();
        let default = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);
        assert_eq!(outcome.value["autonomy"], json!(default.autonomy));
        assert_eq!(
            outcome.value["allowed_commands"],
            json!(default.allowed_commands)
        );
        assert_eq!(
            outcome.value["max_actions_per_hour"],
            json!(default.max_actions_per_hour)
        );
        assert_eq!(
            outcome.value["workspace_only"],
            json!(default.workspace_only)
        );
        assert_eq!(
            outcome.value["block_high_risk_commands"],
            json!(default.block_high_risk_commands)
        );
        assert_eq!(
            outcome.value["require_approval_for_medium_risk"],
            json!(default.require_approval_for_medium_risk)
        );
    }

    #[test]
    fn security_policy_info_reflects_configured_action_budget() {
        let mut config = crate::openhuman::config::Config::default();
        config.autonomy.max_actions_per_hour = 77;

        let outcome = security_policy_info_for_config(&config);

        assert_eq!(outcome.value["max_actions_per_hour"], json!(77));
    }
}
