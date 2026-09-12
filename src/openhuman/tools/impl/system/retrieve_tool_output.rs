//! Tool: retrieve_tool_output — fetch the original of a compacted tool result.
//!
//! Native tool-output compaction (Stage 1a) may replace a large tool result
//! with a compacted view and a `retrieve_tool_output("<hash>")` sentinel,
//! stashing the original in the TokenJuice store. This tool hands the original
//! back on demand, so even lossy compaction stays reversible.
//!
//! Read-only, no side effects, no path/network access.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct RetrieveToolOutputTool;

impl RetrieveToolOutputTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RetrieveToolOutputTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for RetrieveToolOutputTool {
    fn name(&self) -> &str {
        "retrieve_tool_output"
    }

    fn description(&self) -> &str {
        "Retrieve the full, original text of a tool result that was compacted to \
         save context. When a tool output shows a marker like \
         `retrieve_tool_output(\"a1b2c3d4e5f6\")`, call this with that hash to get \
         the complete original back. Use it only when you actually need the dropped \
         detail — the compacted view is usually enough."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "hash": {
                    "type": "string",
                    "description": "The hash from a retrieve_tool_output(\"…\") marker."
                }
            },
            "required": ["hash"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let hash = args
            .get("hash")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(hash) = hash else {
            return Ok(ToolResult::error(
                "retrieve_tool_output: missing required 'hash' argument".to_string(),
            ));
        };

        match crate::openhuman::inference::tokenjuice::retrieve(hash.to_string(), None).await {
            Ok(Some(original)) => {
                log::debug!(
                    "[compaction][ccr] retrieved hash={} bytes={}",
                    hash,
                    original.len()
                );
                Ok(ToolResult::success(original))
            }
            // Deliberately does NOT say "re-run the tool". Re-running
            // regenerates the same oversized result, which is compacted and
            // offloaded again under a new hash that can be evicted just as fast
            // — so a blind re-run turns one cache miss into an unbounded
            // compact→retrieve→re-run loop (observed live: a parent agent
            // re-delegated forever on an evicted subagent result). Tell the
            // model to proceed with the compacted summary it already has.
            Ok(None) => Ok(ToolResult::error(format!(
                "retrieve_tool_output: the full original for hash '{hash}' is no longer \
                 cached (evicted, or from an earlier session). Do NOT re-run the same tool \
                 call to regenerate it — that produces the same oversized result and is \
                 compacted again. Proceed using the compacted summary already shown above; \
                 only if a specific missing detail is essential, retry with narrower \
                 arguments (a tighter query, filter, or smaller limit) so the result is \
                 small enough to keep in full."
            ))),
            Err(error) => Ok(ToolResult::error(error)),
        }
    }
}

#[cfg(test)]
#[path = "retrieve_tool_output_tests.rs"]
mod tests;
