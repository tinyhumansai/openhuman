//! Tool: `delegate` — run a multi-stage, durable sub-agent delegation.
//!
//! Where `spawn_subagent` hands a sub-task to a single sub-agent turn, `delegate`
//! drives the durable plan→execute⇄review→finalize graph
//! ([`agent_orchestration::delegation::run_subagent_delegation`](crate::openhuman::agent_orchestration::delegation::run_subagent_delegation)):
//! the chosen agent first plans, then executes, then reviews its own work and
//! revises up to `max_revisions` times before finalizing. The graph checkpoints
//! its typed state to the session DB, so a crashed or paused run is resumable.
//!
//! Use it for non-trivial sub-tasks that benefit from a self-review/revision
//! loop; use `spawn_subagent` for a single focused hand-off.

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent_orchestration::delegation::run_subagent_delegation;
use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// Default reviewer-requested revision budget when the caller omits it.
const DEFAULT_MAX_REVISIONS: usize = 2;
/// Hard ceiling so a caller can't request an unbounded execute⇄review loop.
const MAX_MAX_REVISIONS: usize = 5;

/// Runs the durable multi-stage delegation graph for a chosen sub-agent.
pub struct DelegateGraphTool;

impl Default for DelegateGraphTool {
    fn default() -> Self {
        Self::new()
    }
}

impl DelegateGraphTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for DelegateGraphTool {
    fn name(&self) -> &str {
        // Distinct from the config-driven `delegate` tool (`DelegateTool`, added
        // when `[agents]` are configured) so the two don't collide in the
        // first-match tool registry — a shared `delegate` name made whichever
        // registered first shadow the other. This is the graph delegation path.
        "delegate_graph"
    }

    fn description(&self) -> &str {
        "Delegate a non-trivial sub-task to a specialised sub-agent that PLANS, \
         EXECUTES, then REVIEWS and revises its own work before returning a final \
         result (a durable plan→execute→review→finalize loop). Prefer this over \
         `spawn_subagent` when the sub-task benefits from a self-review/revision \
         pass. Provide `agent_id` and a complete, self-contained `task` (the \
         sub-agent has no memory of this conversation)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let agent_ids: Vec<String> = AgentDefinitionRegistry::global()
            .map(|reg| reg.list().iter().map(|d| d.id.clone()).collect())
            .unwrap_or_default();

        let agent_id_schema = if agent_ids.is_empty() {
            json!({
                "type": "string",
                "description": "Sub-agent id (e.g. code_executor, researcher, critic)."
            })
        } else {
            json!({
                "type": "string",
                "enum": agent_ids,
                "description": "Sub-agent id from the registry."
            })
        };

        json!({
            "type": "object",
            "required": ["agent_id", "task"],
            "properties": {
                "agent_id": agent_id_schema,
                "task": {
                    "type": "string",
                    "description": "Complete, self-contained description of the task. Include all context the sub-agent needs — it cannot see this conversation."
                },
                "max_revisions": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_MAX_REVISIONS,
                    "description": "Maximum reviewer-requested revisions before finalizing (default 2)."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let agent_id = match args.get("agent_id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => return Ok(ToolResult::error("delegate: `agent_id` is required.")),
        };
        let task = match args.get("task").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.to_string(),
            _ => return Ok(ToolResult::error("delegate: `task` is required.")),
        };
        let max_revisions = args
            .get("max_revisions")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(MAX_MAX_REVISIONS))
            .unwrap_or(DEFAULT_MAX_REVISIONS);

        let registry = match AgentDefinitionRegistry::global() {
            Some(reg) => reg,
            None => {
                return Ok(ToolResult::error(
                    "delegate: agent definition registry not initialized.",
                ))
            }
        };
        let definition = match registry.get(&agent_id) {
            Some(def) => def.clone(),
            None => {
                return Ok(ToolResult::error(format!(
                    "delegate: agent definition '{agent_id}' not found in registry."
                )))
            }
        };

        let config = match Config::load_or_init().await {
            Ok(cfg) => Arc::new(cfg),
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "delegate: failed to load config: {e}"
                )))
            }
        };

        match run_subagent_delegation(config, definition, task, max_revisions).await {
            Ok(state) => {
                let final_output = state
                    .final_output
                    .unwrap_or_else(|| "(delegation produced no final output)".to_string());
                let note = if state.cancelled {
                    " (cancelled)"
                } else if state.revisions > 0 {
                    " (after revision)"
                } else {
                    ""
                };
                Ok(ToolResult::success(format!(
                    "[Delegated to {agent_id}{note}, {} review pass(es)]\n{final_output}",
                    state.reviews.len()
                )))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "delegate failed for '{agent_id}': {e}"
            ))),
        }
    }
}
