//! LLM-callable wrappers over the `cost` domain. Read-only, default-ON.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::config::Config;
use tinytools::{Tool, ToolResult};

use super::rpc;

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

/// 7-day cost dashboard.
pub struct CostDashboardTool {
    config: Arc<Config>,
}

impl CostDashboardTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for CostDashboardTool {
    fn exposure(&self) -> tinytools::ToolExposure {
        tinytools::ToolExposure::Deferred
    }

    fn name(&self) -> &str {
        "cost_get_dashboard"
    }

    fn description(&self) -> &str {
        "Return the cost dashboard: recent AI spend broken down for an at-a-glance \
         view. Use when the user asks how much they've spent recently."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][cost] dashboard invoked");
        emit!(rpc::dashboard(&self.config), "cost_get_dashboard")
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Per-day cost history.
pub struct CostDailyHistoryTool {
    config: Arc<Config>,
}

impl CostDailyHistoryTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for CostDailyHistoryTool {
    fn exposure(&self) -> tinytools::ToolExposure {
        tinytools::ToolExposure::Deferred
    }

    fn name(&self) -> &str {
        "cost_get_daily_history"
    }

    fn description(&self) -> &str {
        "Return per-day AI cost history for the last `days` days (default 30). \
         Use for trend analysis of spend over time."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "days": { "type": "integer", "minimum": 1, "description": "How many days back (default 30)." }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][cost] daily_history invoked");
        let days = args
            .get("days")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as u32)
            .unwrap_or(30);
        emit!(
            rpc::daily_history(&self.config, days),
            "cost_get_daily_history"
        )
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Session/daily/monthly cost summary.
pub struct CostSummaryTool {
    config: Arc<Config>,
}

impl CostSummaryTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for CostSummaryTool {
    fn exposure(&self) -> tinytools::ToolExposure {
        tinytools::ToolExposure::Deferred
    }

    fn name(&self) -> &str {
        "cost_get_summary"
    }

    fn description(&self) -> &str {
        "Return aggregate cost totals (session / daily / monthly). Use for a \
         quick spend summary."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][cost] summary invoked");
        emit!(rpc::summary(&self.config), "cost_get_summary")
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Build this domain's tools in their persisted catalogue order.
pub(crate) fn registered_tools(
    config: std::sync::Arc<crate::config::Config>,
) -> Vec<Box<dyn tinytools::Tool>> {
    vec![
        Box::new(CostDashboardTool::new(config.clone())),
        Box::new(CostDailyHistoryTool::new(config.clone())),
        Box::new(CostSummaryTool::new(config.clone())),
    ]
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
