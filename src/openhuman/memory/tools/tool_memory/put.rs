//! `memory_tools_put` — upsert a tool-scoped memory rule.
//!
//! Routed through [`MemoryGuard`](crate::openhuman::memory::guard::MemoryGuard).
//! `MemoryToolMemory::put_tool_rule` delegates to the same
//! `ToolMemoryStore::put_rule` this tool used to build by hand, with one
//! asymmetry: the contract method returns unit while the store returns the
//! *stored* rule (trim/lower-cased `tool_name`, `created_at` preserved on
//! upsert, `updated_at` refreshed) — which is what this tool answers with. The
//! asymmetry is recovered exactly by reading the rule back:
//! `ToolMemoryRule::new` always generates the id before the write, so there is
//! no server-assigned identity to lose, and `tool_memory_namespace` applies the
//! same `trim().to_lowercase()` the write normalised into, so reading back with
//! the caller's raw `tool_name` hits the same namespace.
//!
//! A concurrent delete between the write and the read-back yields no rule. That
//! answers with an error, never a fabricated rule — absence, not a lie.
//!
//! **Behaviour change, deliberate:** the write now takes
//! `SecurityPolicy::enforce_write_tier`, so the tool is refused under the
//! `readonly` autonomy tier with `"memory guard: "`-prefixed text, and
//! store-level validation errors arrive as `MemoryError::Invalid` rather than as
//! a raw string.

use crate::openhuman::memory::api::provider::MemoryProvider;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::openhuman::memory::api::tool_memory::{
    ToolMemoryPriority, ToolMemoryRule, ToolMemorySource,
};
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::memory::ops::tool_memory::NO_TOOL_MEMORY;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryToolsPutTool;

#[derive(Debug, Deserialize)]
struct Args {
    tool_name: String,
    rule: String,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

fn parse_priority(s: Option<&str>) -> ToolMemoryPriority {
    match s.map(|x| x.to_ascii_lowercase()) {
        Some(ref v) if v == "critical" => ToolMemoryPriority::Critical,
        Some(ref v) if v == "high" => ToolMemoryPriority::High,
        _ => ToolMemoryPriority::Normal,
    }
}

#[async_trait]
impl Tool for MemoryToolsPutTool {
    fn name(&self) -> &str {
        "memory_tools_put"
    }

    fn description(&self) -> &str {
        "Record a durable rule / learning for the given tool. Use when the \
         user gives a directive that should survive future sessions, or \
         when a tool failure pattern is worth pinning. Returns the stored \
         rule with its assigned id and timestamps."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["tool_name", "rule"],
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "Exact tool name the rule applies to."
                },
                "rule": {
                    "type": "string",
                    "description": "Free-text rule, edict, or learning to pin."
                },
                "priority": {
                    "type": "string",
                    "enum": ["critical", "high", "normal"],
                    "description": "How aggressively to surface the rule. Default: normal."
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional free-form tags (e.g. `safety`, `permission`)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_tools_put: {e}"))?;
        log::debug!(
            "[tool][memory_tools] put tool_name={} priority={:?} tags={}",
            parsed.tool_name,
            parsed.priority,
            parsed.tags.len()
        );
        let guard = active_memory_guard()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tools_put: {e}"))?;
        let family = guard
            .as_tool_memory()
            .ok_or_else(|| anyhow::anyhow!("memory_tools_put: {NO_TOOL_MEMORY}"))?;
        let mut rule = ToolMemoryRule::new(
            &parsed.tool_name,
            &parsed.rule,
            parse_priority(parsed.priority.as_deref()),
            ToolMemorySource::UserExplicit,
        );
        rule.tags = parsed.tags;
        let rule_id = rule.id.clone();
        let tool_name = rule.tool_name.clone();
        family
            .put_tool_rule(rule)
            .await
            .map_err(|e| anyhow::anyhow!("memory_tools_put: {e}"))?;
        // `put_tool_rule` answers with unit; the tool's contract is the stored
        // rule (normalised tool_name, preserved created_at, refreshed
        // updated_at), so read it back by the id generated above.
        let stored = family
            .tool_rules(&tool_name)
            .await
            .map_err(|e| anyhow::anyhow!("memory_tools_put: {e}"))?
            .into_iter()
            .find(|r| r.id == rule_id)
            .ok_or_else(|| {
                anyhow::anyhow!("memory_tools_put: stored rule {rule_id} not found on read-back")
            })?;
        log::debug!(
            "[tool][memory_tools] put via guard tool_name={} id={} read_back=ok",
            stored.tool_name,
            stored.id
        );
        let json = serde_json::to_string(&stored)?;
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
#[path = "put_tests.rs"]
mod tests;
