//! Agent tool: diagnose the memory pipeline (#002 FR-009).
//!
//! Thin wrapper over
//! [`health::report::run_doctor`](crate::openhuman::memory::tree::health::report::run_doctor)
//! so the agent can self-diagnose an empty / stalled wiki and tell the user the
//! single first blocking cause + how to fix it — the same report the
//! `memory_tree_doctor` RPC and CLI return. Read-only: takes no arguments and
//! mutates nothing, so it carries no security-gate (matching the read-only
//! memory tools).
//!
//! The pass itself is the bound driver's since #5560
//! (`MemoryMaintenance::diagnose`): the counters and the degradation flags only
//! exist in the process that ran the pipeline, and that is the module.

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::health::report::run_doctor;
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
        let report = run_doctor(self.config.as_ref()).await;
        // Serialize the structured report so the model gets the typed stages +
        // first_blocking_cause + counters verbatim (it can summarize for the
        // user from there). serde of a plain struct can't fail here.
        let payload = serde_json::to_string_pretty(&report)
            .unwrap_or_else(|e| format!("{{\"error\":\"serialize doctor report: {e}\"}}"));
        Ok(ToolResult::success(payload))
    }
}

#[cfg(test)]
#[path = "doctor_tests.rs"]
mod tests;
