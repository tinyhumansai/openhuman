//! LLM-callable wrapper over the `security` domain. Read-only, default-ON.
//!
//! Only the policy-info read is exposed as an agent tool; command/path gating
//! is enforced in-engine, not as an agent-callable surface.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::config::Config;
use tinytools::{Tool, ToolResult};

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
    fn exposure(&self) -> tinytools::ToolExposure {
        tinytools::ToolExposure::Deferred
    }

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

/// Build this domain's tools in their persisted catalogue order.
pub(crate) fn registered_tools(
    config: std::sync::Arc<crate::config::Config>,
) -> Vec<Box<dyn tinytools::Tool>> {
    vec![Box::new(SecurityPolicyInfoTool::new(config.clone()))]
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
