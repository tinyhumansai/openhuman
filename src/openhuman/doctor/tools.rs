//! LLM-callable wrappers over the `doctor` diagnostics domain.
//!
//! Read-only health diagnostics; both tools delegate to
//! [`crate::openhuman::doctor::ops`] and ship default-ON.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{Tool, ToolResult};

use super::ops;

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

/// Run the full diagnostic battery.
pub struct DoctorHealthTool {
    config: Arc<Config>,
}

impl DoctorHealthTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for DoctorHealthTool {
    fn name(&self) -> &str {
        "doctor_health"
    }

    fn description(&self) -> &str {
        "Run the full diagnostic battery (config, connectivity, dependencies, \
         runtime) and return a structured report of checks with pass/warn/fail \
         status. Use to diagnose why the assistant or a subsystem isn't working."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][doctor] health invoked");
        emit!(ops::doctor_report(&self.config).await, "doctor_health")
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Probe model availability.
pub struct DoctorModelsTool {
    config: Arc<Config>,
}

impl DoctorModelsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for DoctorModelsTool {
    fn name(&self) -> &str {
        "doctor_models"
    }

    fn description(&self) -> &str {
        "Probe configured AI models for availability and return a per-model \
         report. Set `use_cache` false to force a fresh probe (default true)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "use_cache": { "type": "boolean", "description": "Use cached probe results (default true)." }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][doctor] models invoked");
        let use_cache = args
            .get("use_cache")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        emit!(
            ops::doctor_models(&self.config, use_cache).await,
            "doctor_models"
        )
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};

    fn cfg() -> Arc<Config> {
        Arc::new(Config::default())
    }

    #[test]
    fn metadata() {
        assert_eq!(DoctorHealthTool::new(cfg()).name(), "doctor_health");
        assert_eq!(DoctorModelsTool::new(cfg()).name(), "doctor_models");
        assert_eq!(
            DoctorHealthTool::new(cfg()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(DoctorHealthTool::new(cfg()).scope(), ToolScope::All);
    }
}
