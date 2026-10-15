//! JSON-RPC / CLI controller surface for security policy introspection.

use crate::core::runtime::DispatchCtx;
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
        // Additive fields: the keys above are the original wire shape.
        "enabled": policy.enabled,
        "privacy_mode": policy.privacy_mode,
        "forbidden_paths": policy.forbidden_paths,
        "trusted_root_count": policy.trusted_roots.len(),
        "auto_approve": policy.auto_approve,
        "auto_approve_all": policy.auto_approve_all,
        "allow_tool_install": policy.allow_tool_install,
    })
}

pub fn security_policy_info_for_config(config: &Config) -> Outcome<serde_json::Value> {
    let policy = SecurityPolicy::from_full_config(config);
    let payload = policy_info_payload(policy);
    Outcome::single_log(payload, "security_policy_info computed from active config")
}

/// The module's complete effective policy as seen by the authenticated UI.
/// Host paths are left out; the module reports them to the host only.
#[cfg(feature = "security-module")]
async fn module_section(config: &Config) -> serde_json::Value {
    match crate::modules::security::policy_info(config).await {
        Ok(info) => json!({
            "status": "active",
            "contract_version": info.contract_version,
            "generation": info.generation,
            "enabled": info.policy.enabled,
            "allowed_commands": info.policy.allowed_commands,
            "forbidden_paths": info.policy.forbidden_paths,
            "judge": info.judge,
            "callbacks": info.callbacks,
            "scoped_policies": info.scoped_policies,
            "capabilities": info.capabilities,
        }),
        Err(error) => {
            log::warn!("[security][rpc] module policy unavailable: {error}");
            json!({ "status": "unavailable" })
        }
    }
}

#[cfg(not(feature = "security-module"))]
async fn module_section(_config: &Config) -> serde_json::Value {
    json!({ "status": "not_built" })
}

pub async fn load_and_get_security_policy_info(
    ctx: &DispatchCtx,
) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
    let mut outcome = security_policy_info_for_config(&config);
    if let Some(object) = outcome.value.as_object_mut() {
        object.insert("module".to_owned(), module_section(&config).await);
    }
    Ok(outcome)
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
