//! `memory_tools_list` — list every stored rule for a given tool.
//!
//! Routed through [`MemoryGuard`](crate::openhuman::memory::guard::MemoryGuard)
//! rather than a raw `ToolMemoryStore`. `MemoryToolMemory::tool_rules` on the
//! embedded driver is literally `tool_memory_store(self.memory()).list_rules(…)`,
//! and the wire type matches by identity, not conversion:
//! `memory::tool_memory::ToolMemoryRule` **is**
//! `crate::openhuman::memory::api::tool_memory::ToolMemoryRule`. So the re-point is exact —
//! same rules, same order, same serialization — with `Capability::ToolMemory`
//! admitted first.

use crate::openhuman::memory::api::provider::MemoryProvider;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::memory::ops::tool_memory::NO_TOOL_MEMORY;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryToolsListTool;

#[derive(Debug, Deserialize)]
struct Args {
    tool_name: String,
}

#[async_trait]
impl Tool for MemoryToolsListTool {
    fn name(&self) -> &str {
        "memory_tools_list"
    }

    fn description(&self) -> &str {
        "List every stored memory rule for the given tool. Rules are durable \
         learnings about how to use the tool — priorities, gotchas, user \
         edicts. Returns the rules ordered by priority (Critical → Low) and \
         updated_at DESC within each priority."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["tool_name"],
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "Exact tool name (e.g. `bash`, `web_search`)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_tools_list: {e}"))?;
        log::debug!("[tool][memory_tools] list tool_name={}", parsed.tool_name);
        let guard = active_memory_guard()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tools_list: {e}"))?;
        let rules = guard
            .as_tool_memory()
            .ok_or_else(|| anyhow::anyhow!("memory_tools_list: {NO_TOOL_MEMORY}"))?
            .tool_rules(&parsed.tool_name)
            .await
            .map_err(|e| anyhow::anyhow!("memory_tools_list: {e}"))?;
        log::debug!(
            "[tool][memory_tools] list via guard tool_name={} rules={}",
            parsed.tool_name,
            rules.len()
        );
        let json = serde_json::to_string(&rules)?;
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
#[path = "list_tests.rs"]
mod tests;
