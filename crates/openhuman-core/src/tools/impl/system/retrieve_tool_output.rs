//! Tool: retrieve_tool_output — fetch the original of a compacted tool result.
//!
//! The legacy name for TinyJuice's `juice_retrieve`. Native tool-output
//! compaction (Stage 1a) hands out a `retrieve_tool_output("<hash>")` sentinel,
//! and models keep calling this name. It advertises the same schema as
//! `juice_retrieve` (`token`, optional `range`), so it delegates to that tool
//! and only keeps its own name: the arguments, the `hash` fallback, the range
//! slice and the eviction message all stay in one implementation.
//!
//! Read-only, no side effects, no path/network access.

use std::sync::Arc;

use async_trait::async_trait;
use tinyjuice_host::retrieve_tool::{RetrieveFn, RetrieveToolOutputTool as JuiceRetrieveTool};
use tinytools::{PermissionLevel, Tool, ToolResult};

/// The `retrieve_tool_output` tool, bound to a host-supplied lookup.
pub struct RetrieveToolOutputTool {
    inner: JuiceRetrieveTool,
}

impl RetrieveToolOutputTool {
    /// Build the tool over `retrieve`, which resolves a token (and optional
    /// range) to the original.
    pub fn new(retrieve: RetrieveFn) -> Self {
        Self {
            inner: JuiceRetrieveTool::new(retrieve),
        }
    }
}

#[async_trait]
impl Tool for RetrieveToolOutputTool {
    fn name(&self) -> &str {
        "retrieve_tool_output"
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.inner.parameters_schema()
    }

    fn permission_level(&self) -> PermissionLevel {
        self.inner.permission_level()
    }

    fn is_concurrency_safe(&self, args: &serde_json::Value) -> bool {
        self.inner.is_concurrency_safe(args)
    }

    fn external_effect(&self) -> bool {
        self.inner.external_effect()
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.inner.execute(args).await
    }
}

/// Bind the recovery tool to the module-owned cache.
pub fn retrieve_tool_output_tool() -> RetrieveToolOutputTool {
    RetrieveToolOutputTool::new(Arc::new(|token, range| {
        Box::pin(async move { crate::inference::tokenjuice::retrieve(token, range).await })
    }))
}

#[cfg(test)]
#[path = "retrieve_tool_output_tests.rs"]
mod tests;
