//! Tool: `spawn_parallel_agents` — fan out independent sub-agent tasks.

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
#[cfg(test)]
use crate::openhuman::agent_orchestration::spawn_parallel_graph::with_ownership_boundary;
#[cfg(test)]
use crate::openhuman::agent_orchestration::spawn_parallel_graph::ParallelAgentLineage;
#[cfg(test)]
use crate::openhuman::agent_orchestration::spawn_parallel_graph::ParallelAgentResult;
#[cfg(test)]
use crate::openhuman::agent_orchestration::spawn_parallel_graph::ParallelAgentTask;
use crate::openhuman::agent_orchestration::spawn_parallel_graph::{
    format_spawn_parallel_success, run_spawn_parallel_graph_with_cancellation_and_workspace,
    SpawnParallelGraphOutcome, SpawnParallelTaskValidationError,
};
use crate::openhuman::tinyagents::current_run_cancellation;
use crate::openhuman::tools::traits::{
    PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolTimeout,
};
use async_trait::async_trait;
use serde_json::json;
use tinyagents::harness::tool::ToolExecutionContext;

pub struct SpawnParallelAgentsTool;

impl SpawnParallelAgentsTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SpawnParallelAgentsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SpawnParallelAgentsTool {
    fn name(&self) -> &str {
        "spawn_parallel_agents"
    }

    fn description(&self) -> &str {
        "Run two or more independent sub-agent tasks concurrently and collect their results. \
         Read-only and worktree-isolated workers run in parallel; shared-workspace workers with \
         write-capable tools require disjoint `files:` ownership and run through a serial fallback. \
         Each task has `{agent_id, prompt, context?, toolkit?, ownership?, isolation?, base_ref?}`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let agent_ids: Vec<String> = AgentDefinitionRegistry::global()
            .map(|reg| reg.list().iter().map(|d| d.id.clone()).collect())
            .unwrap_or_default();
        let agent_id_schema = if agent_ids.is_empty() {
            json!({ "type": "string" })
        } else {
            json!({ "type": "string", "enum": agent_ids })
        };
        json!({
            "type": "object",
            "required": ["tasks"],
            "properties": {
                "tasks": {
                    "type": "array",
                    "minItems": 2,
                    "items": {
                        "type": "object",
                        "required": ["agent_id", "prompt"],
                        "properties": {
                            "agent_id": agent_id_schema,
                            "prompt": { "type": "string" },
                            "context": { "type": "string" },
                            "toolkit": { "type": "string" },
                            "ownership": {
                                "type": "string",
                                "description": "Disjoint file/module/responsibility boundary for this worker."
                            },
                            "isolation": {
                                "type": "string",
                                "enum": ["none", "worktree"],
                                "description": "File-isolation strategy. `none` (default) shares the workspace; write-capable shared workers need disjoint `files:` ownership and are serialized. `worktree` gives an edit-capable worker its own git worktree checkout so parallel edits never collide."
                            },
                            "base_ref": {
                                "type": "string",
                                "enum": ["head", "fresh"],
                                "description": "For `isolation = worktree`: branch the worktree from current HEAD (`head`, default) or the repo's default branch (`fresh`)."
                            }
                        }
                    }
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    /// Run **without** the global per-tool wall-clock deadline. This is a
    /// fan-out primitive: it spawns N independent sub-agents and awaits them
    /// concurrently via `spawn_parallel_graph`'s bounded `map_reduce`. Under the
    /// default `Inherit` policy the whole fan-out is hard-killed at the
    /// single-tool timeout (120s by default) — so a group of long workers is
    /// truncated at one worker's budget instead of running to ~the slowest, and
    /// each worker's research is cut short. The fan-out is already bounded
    /// internally — by `max_concurrency`, the run cancellation token, and each
    /// sub-agent's own iteration/turn caps — so it governs its own lifetime,
    /// like the long-running scripting tools (`shell`, `node_exec`).
    fn timeout_policy(&self, _args: &serde_json::Value) -> ToolTimeout {
        ToolTimeout::Unbounded
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        tool_context: Option<&ToolExecutionContext>,
    ) -> anyhow::Result<ToolResult> {
        tracing::debug!("[spawn_parallel_agents] execute entry");
        let workspace_descriptor = tool_context.and_then(|ctx| ctx.workspace.clone());
        let cancellation = current_run_cancellation().unwrap_or_else(|| {
            tracing::debug!(
                "[spawn_parallel_agents] no active tinyagents run cancellation token; using local token"
            );
            tinyagents::CancellationToken::new()
        });
        let outcome = run_spawn_parallel_graph_with_cancellation_and_workspace(
            args,
            cancellation,
            workspace_descriptor,
        )
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
        match outcome {
            SpawnParallelGraphOutcome::Collected(collected) => Ok(ToolResult::success(
                format_spawn_parallel_success(&collected),
            )),
            SpawnParallelGraphOutcome::InvalidRequest(
                SpawnParallelTaskValidationError::MissingTasks(message),
            ) => {
                tracing::debug!("[spawn_parallel_agents] missing_tasks_parameter");
                Err(anyhow::anyhow!(message))
            }
            SpawnParallelGraphOutcome::InvalidRequest(
                SpawnParallelTaskValidationError::InvalidTasks(message),
            ) => {
                tracing::debug!(error = %message, "[spawn_parallel_agents] invalid_tasks_array");
                Err(anyhow::anyhow!(message))
            }
            SpawnParallelGraphOutcome::InvalidRequest(
                SpawnParallelTaskValidationError::Rejected(message),
            ) => {
                tracing::debug!("[spawn_parallel_agents] rejected_too_few_tasks");
                Ok(ToolResult::error(message))
            }
            SpawnParallelGraphOutcome::Rejected(message) => Ok(ToolResult::error(message)),
            SpawnParallelGraphOutcome::Cancelled(message) => Ok(ToolResult::error(message)),
        }
    }
}

#[cfg(test)]
#[path = "spawn_parallel_agents_tests.rs"]
mod tests;
