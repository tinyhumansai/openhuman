//! Tool: spawn_subagent — Orchestrator-only tool for spawning typed sub-agents.

use super::traits::{PermissionLevel, Tool, ToolResult};
use crate::openhuman::agent::harness::archetypes::AgentArchetype;
use async_trait::async_trait;
use serde_json::json;

/// Spawns a sub-agent of a specified archetype to handle a delegated task.
/// Available only to the Orchestrator archetype.
pub struct SpawnSubagentTool;

impl SpawnSubagentTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Spawn a specialised sub-agent to handle a task. Available archetypes: \
         code_executor (writes/runs code), skills_agent (skill tools like Notion/Gmail), \
         tool_maker (writes polyfills for missing commands), researcher (reads docs/web), \
         critic (reviews code quality). Provide the archetype, a clear task prompt, and \
         optional context from prior results."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["archetype", "prompt"],
            "properties": {
                "archetype": {
                    "type": "string",
                    "enum": ["code_executor", "skills_agent", "tool_maker", "researcher", "critic"],
                    "description": "Which specialised sub-agent to spawn."
                },
                "prompt": {
                    "type": "string",
                    "description": "Clear, specific instruction for the sub-agent."
                },
                "context": {
                    "type": "string",
                    "description": "Optional context from prior task results or workspace state."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let archetype_str = args
            .get("archetype")
            .and_then(|v| v.as_str())
            .unwrap_or("code_executor");

        let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");

        let context = args.get("context").and_then(|v| v.as_str()).unwrap_or("");

        if prompt.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("prompt is required".into()),
            });
        }

        // Parse archetype (validation).
        let _archetype: AgentArchetype = serde_json::from_value(json!(archetype_str))
            .map_err(|_| anyhow::anyhow!("unknown archetype: {archetype_str}"))?;

        // Placeholder: In the full implementation, this will construct a sub-agent
        // via AgentBuilder with the archetype's tool subset, model, and sandbox,
        // then run its tool loop and return the result.
        tracing::info!(
            "[spawn_subagent] would spawn {archetype_str} sub-agent with prompt length={}",
            prompt.len()
        );

        Ok(ToolResult {
            success: true,
            output: format!(
                "[Sub-agent {archetype_str}] Task received. Prompt: {prompt}\n\
                 Context length: {} chars\n\
                 (Full sub-agent execution will be wired in next phase)",
                context.len()
            ),
            error: None,
        })
    }
}
