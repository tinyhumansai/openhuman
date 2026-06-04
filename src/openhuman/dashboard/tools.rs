//! LLM-callable wrapper over the `dashboard` domain. Read-only, default-ON.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{Tool, ToolResult};

use super::ops;

/// Per-model health table.
pub struct DashboardModelHealthTool {
    config: Arc<Config>,
}

impl DashboardModelHealthTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for DashboardModelHealthTool {
    fn name(&self) -> &str {
        "dashboard_model_health"
    }

    fn description(&self) -> &str {
        "Return the model-health table the dashboard shows: per-model status and \
         the active routing configuration. Use to advise on which models are \
         healthy/available."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][dashboard] model_health invoked");
        let outcome = ops::model_health(&self.config)
            .map_err(|e| anyhow::anyhow!("dashboard_model_health: {e}"))?;
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

    #[test]
    fn metadata() {
        let t = DashboardModelHealthTool::new(Arc::new(Config::default()));
        assert_eq!(t.name(), "dashboard_model_health");
        assert_eq!(t.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(t.scope(), ToolScope::All);
    }
}
