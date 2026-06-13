//! LLM-callable wrapper over the `security` domain. Read-only, default-ON.
//!
//! Only the policy-info read is exposed as an agent tool; command/path gating
//! is enforced in-engine, not as an agent-callable surface.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{Tool, ToolResult};

use super::ops;

/// Report the current security/autonomy policy.
pub struct SecurityPolicyInfoTool {
    config: Arc<Config>,
}

impl SecurityPolicyInfoTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for SecurityPolicyInfoTool {
    fn name(&self) -> &str {
        "security_policy_info"
    }

    fn description(&self) -> &str {
        "Report the effective security/autonomy policy: access level, \
         workspace-only flag, allowed commands, rate limits, and approval \
         requirements. Use to explain what the agent is/aren't permitted to do."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][security] policy_info invoked");
        let outcome = ops::security_policy_info_for_config(&self.config);
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};

    #[tokio::test]
    async fn metadata_and_execute() {
        let t = SecurityPolicyInfoTool::new(Arc::new(Config::default()));
        assert_eq!(t.name(), "security_policy_info");
        assert_eq!(t.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(t.scope(), ToolScope::All);
        let out = t.execute(json!({})).await.expect("policy_info");
        assert!(!out.output_for_llm(false).is_empty());
    }
}
