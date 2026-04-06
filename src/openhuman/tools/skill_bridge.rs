//! Bridge between the QuickJS skill runtime and the agent's `Tool` trait registry.
//!
//! Each running skill exposes tools via `ToolDefinition`. This module wraps them
//! as `Tool` trait implementations so the agent loop can discover and execute
//! skill tools (Notion, Gmail, etc.) alongside built-in tools.
//!
//! Both built-in tools and skill tools now use the same unified `ToolResult`
//! type (MCP content blocks), so no result conversion is needed.

use async_trait::async_trait;
use std::sync::Arc;

use crate::openhuman::skills::qjs_engine::RuntimeEngine;
use crate::openhuman::skills::types::ToolDefinition;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// A `Tool` implementation that delegates execution to a running QuickJS skill instance.
///
/// The tool name uses the convention `{skill_id}__{tool_name}` so the agent loop
/// can match tool calls from the LLM to the correct skill and tool.
pub struct SkillToolBridge {
    /// Namespaced tool name: `{skill_id}__{tool_name}`.
    namespaced_name: String,
    /// Skill identifier (e.g. "notion", "gmail").
    skill_id: String,
    /// Original tool name within the skill (e.g. "search-blocks", "send-email").
    tool_name: String,
    /// Human-readable description from the skill manifest.
    description: String,
    /// JSON Schema for the tool's input parameters.
    input_schema: serde_json::Value,
    /// Reference to the runtime engine for executing tool calls.
    engine: Arc<RuntimeEngine>,
}

impl SkillToolBridge {
    fn new(skill_id: String, tool_def: &ToolDefinition, engine: Arc<RuntimeEngine>) -> Self {
        let namespaced_name = format!("{}__{}", skill_id, tool_def.name);
        Self {
            namespaced_name,
            skill_id,
            tool_name: tool_def.name.clone(),
            description: tool_def.description.clone(),
            input_schema: tool_def.input_schema.clone(),
            engine,
        }
    }
}

#[async_trait]
impl Tool for SkillToolBridge {
    fn name(&self) -> &str {
        &self.namespaced_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        if self.input_schema.is_null() || self.input_schema == serde_json::json!({}) {
            serde_json::json!({ "type": "object", "properties": {} })
        } else {
            self.input_schema.clone()
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        // Skill tools interact with external services; treat as write-level.
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!(
            "[skill-bridge] Executing {}.{} (namespaced: {})",
            self.skill_id,
            self.tool_name,
            self.namespaced_name,
        );

        // Both the skill runtime and the Tool trait now use the same ToolResult type,
        // so we just forward the result directly — no conversion needed.
        match self
            .engine
            .call_tool(&self.skill_id, &self.tool_name, args)
            .await
        {
            Ok(result) => {
                if result.is_error {
                    log::warn!(
                        "[skill-bridge] {}.{} returned error: {}",
                        self.skill_id,
                        self.tool_name,
                        result.output()
                    );
                } else {
                    log::debug!(
                        "[skill-bridge] {}.{} succeeded ({} bytes)",
                        self.skill_id,
                        self.tool_name,
                        result.output().len()
                    );
                }
                Ok(result)
            }
            Err(err) => {
                log::error!(
                    "[skill-bridge] {}.{} execution failed: {}",
                    self.skill_id,
                    self.tool_name,
                    err
                );
                Ok(ToolResult::error(err))
            }
        }
    }
}

/// Collect all tools from running skills and wrap them as `Box<dyn Tool>`.
///
/// Returns an empty vec if the runtime engine is not initialized (e.g. CLI mode
/// without the desktop app running).
pub fn collect_skill_tools() -> Vec<Box<dyn Tool>> {
    let engine = match crate::openhuman::skills::global_engine() {
        Some(e) => e,
        None => {
            log::debug!("[skill-bridge] No global engine — skipping skill tools");
            return Vec::new();
        }
    };

    let all = engine.all_tools();
    log::info!(
        "[skill-bridge] Discovered {} tool(s) from running skills",
        all.len()
    );

    all.into_iter()
        .map(|(skill_id, tool_def)| {
            log::debug!(
                "[skill-bridge]   + {}__{} — {}",
                skill_id,
                tool_def.name,
                tool_def.description.chars().take(60).collect::<String>()
            );
            Box::new(SkillToolBridge::new(
                skill_id,
                &tool_def,
                Arc::clone(&engine),
            )) as Box<dyn Tool>
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaced_name_format() {
        let skill_id = "notion";
        let tool_name = "search-blocks";
        let expected = format!("{}__{}", skill_id, tool_name);
        assert_eq!(expected, "notion__search-blocks");
    }

    #[test]
    fn collect_returns_empty_without_engine() {
        // Global engine is not set in test context, so should return empty.
        let tools = collect_skill_tools();
        assert!(tools.is_empty());
    }
}
