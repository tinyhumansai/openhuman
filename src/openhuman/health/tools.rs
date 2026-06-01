//! LLM-callable wrappers over the `health` domain. Read-only, default-ON.

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::tools::traits::{Tool, ToolResult};

use super::ops;

/// Component health snapshot.
pub struct HealthSnapshotTool;

#[async_trait]
impl Tool for HealthSnapshotTool {
    fn name(&self) -> &str {
        "health_snapshot"
    }

    fn description(&self) -> &str {
        "Return a snapshot of core component health (subsystem readiness flags). \
         Use for a quick liveness/readiness check of the running core."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][health] snapshot invoked");
        let outcome = ops::health_snapshot();
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Version / OS / arch / pid.
pub struct HealthSystemInfoTool;

#[async_trait]
impl Tool for HealthSystemInfoTool {
    fn name(&self) -> &str {
        "health_system_info"
    }

    fn description(&self) -> &str {
        "Return basic system info for the running core: version, OS, arch, and \
         process id."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][health] system_info invoked");
        let outcome = ops::system_info();
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
        assert_eq!(HealthSnapshotTool.name(), "health_snapshot");
        assert_eq!(HealthSystemInfoTool.name(), "health_system_info");
        assert_eq!(
            HealthSnapshotTool.permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(HealthSnapshotTool.scope(), ToolScope::All);
    }

    #[tokio::test]
    async fn system_info_executes() {
        let out = HealthSystemInfoTool
            .execute(json!({}))
            .await
            .expect("system_info");
        assert!(out.output_for_llm(false).contains("os"));
    }
}
