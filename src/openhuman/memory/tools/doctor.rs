//! Agent tool: diagnose the memory pipeline (#002 FR-009).
//!
//! Thin wrapper over [`health::run_doctor`] so the agent can self-diagnose an
//! empty / stalled wiki and tell the user the single first blocking cause +
//! how to fix it — the same report the `memory_tree_doctor` RPC and CLI
//! return. Read-only: takes no arguments and mutates nothing, so it carries no
//! security-gate (matching the read-only memory tools).

use crate::openhuman::config::Config;
use crate::openhuman::memory_tree::health::async_run_doctor;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// Let the agent run the one-shot memory-pipeline diagnostic.
pub struct MemoryDoctorTool {
    config: Arc<Config>,
}

impl MemoryDoctorTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for MemoryDoctorTool {
    fn name(&self) -> &str {
        "memory_doctor"
    }

    fn description(&self) -> &str {
        "Diagnose why the memory tree / wiki is empty or stalled. Returns per-stage health \
         (embeddings config, scheduler gate, job queue, extraction/recall degradation, \
         summary-tree precondition), the single first blocking cause with a fix, and current \
         counters. Read-only — takes no arguments."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {}, "required": [] })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let report = async_run_doctor(&self.config).await;
        // Serialize the structured report so the model gets the typed stages +
        // first_blocking_cause + counters verbatim (it can summarize for the
        // user from there). serde of a plain struct can't fail here.
        let payload = serde_json::to_string_pretty(&report)
            .unwrap_or_else(|e| format!("{{\"error\":\"serialize doctor report: {e}\"}}"));
        Ok(ToolResult::success(payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Arc<Config>) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        (tmp, Arc::new(cfg))
    }

    #[test]
    fn name_and_schema() {
        let (_tmp, cfg) = test_config();
        let tool = MemoryDoctorTool::new(cfg);
        assert_eq!(tool.name(), "memory_doctor");
        // No required args.
        assert_eq!(tool.parameters_schema()["required"], json!([]));
    }

    #[tokio::test]
    async fn execute_returns_a_report_for_a_misconfigured_workspace() {
        let _g = crate::openhuman::memory_tree::health::test_guard();
        let (_tmp, cfg) = test_config();
        // No embeddings provider, local AI off → unhealthy with a typed cause.
        let tool = MemoryDoctorTool::new(cfg);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.is_error);
        let out = result.output();
        assert!(
            out.contains("\"healthy\""),
            "report should serialize: {out}"
        );
        assert!(
            out.contains("embeddings_unconfigured") || out.contains("\"healthy\": false"),
            "misconfigured workspace should surface a blocking cause: {out}"
        );
    }
}
