//! Tool: `spawn_parallel_agents` — fan out independent sub-agent tasks.

use crate::agent::harness::definition::AgentDefinitionRegistry;
#[cfg(test)]
use crate::agent::orchestration::spawn_parallel_graph::with_ownership_boundary;
#[cfg(test)]
use crate::agent::orchestration::spawn_parallel_graph::ParallelAgentLineage;
#[cfg(test)]
use crate::agent::orchestration::spawn_parallel_graph::ParallelAgentResult;
use crate::agent::orchestration::spawn_parallel_graph::ParallelAgentTask;
use crate::agent::orchestration::spawn_parallel_graph::{
    format_spawn_parallel_success, run_spawn_parallel_tasks_with_cancellation_and_workspace,
    SpawnParallelGraphOutcome,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::tool::ToolDispatch;
use tinytools::ToolRunContext;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolTimeout};

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

/// Typed-parent dispatch for `spawn_parallel_agents`.
///
/// The canonical `Tool` declaration remains usable for schema, policy, and
/// capability registration, while this dispatch is the only harness execution
/// path. It receives the live parent run rather than recovering cancellation
/// from a task-local, so nested graph fan-out observes the exact parent token
/// and workspace grant.
pub(crate) struct SpawnParallelAgentsDispatch {
    tool: Arc<dyn Tool>,
}

impl SpawnParallelAgentsDispatch {
    pub(crate) fn new(tool: Arc<dyn Tool>) -> Self {
        Self { tool }
    }
}

#[async_trait]
impl ToolDispatch<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for SpawnParallelAgentsDispatch
{
    fn tool(&self) -> Arc<dyn Tool> {
        self.tool.clone()
    }

    async fn execute(
        &self,
        _state: &(),
        call_id: tinyagents_harness::CallId,
        arguments: serde_json::Value,
        _options: ToolCallOptions,
        parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        execute_spawn_parallel_agents(
            arguments,
            parent.cancellation.clone(),
            parent.workspace.clone(),
            parent.data.child(),
            Some(parent),
            Some(call_id.as_str().to_string()),
        )
        .await
    }
}

/// Executes the graph from explicitly supplied parent-run values.
///
/// This is shared by typed harness callers and requires their live parent;
/// standalone raw-tool execution fails closed.
pub(crate) async fn execute_spawn_parallel_agents(
    args: serde_json::Value,
    cancellation: tinyagents_harness::CancellationToken,
    workspace_descriptor: Option<tinytools::WorkspaceDescriptor>,
    run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    live_parent: Option<&RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>>,
    parent_call_id: Option<String>,
) -> anyhow::Result<ToolResult> {
    tracing::debug!("[spawn_parallel_agents] execute entry");
    let tasks = match parse_parallel_agent_tasks(&args) {
        Ok(tasks) => tasks,
        Err(ParallelAgentTaskRequestError::MissingTasks(message))
        | Err(ParallelAgentTaskRequestError::InvalidTasks(message)) => {
            return Err(anyhow::anyhow!(message));
        }
        Err(ParallelAgentTaskRequestError::Rejected(message)) => {
            return Ok(ToolResult::error(message));
        }
    };
    let Some(live_parent) = live_parent else {
        return Ok(ToolResult::error(
            "spawn_parallel_agents called outside of an agent turn",
        ));
    };
    let outcome = run_spawn_parallel_tasks_with_cancellation_and_workspace(
        tasks,
        cancellation,
        workspace_descriptor,
        run_context,
        live_parent,
        parent_call_id,
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?;
    match outcome {
        SpawnParallelGraphOutcome::Collected(collected) => Ok(ToolResult::success(
            format_spawn_parallel_success(&collected),
        )),
        SpawnParallelGraphOutcome::Rejected(message) => Ok(ToolResult::error(message)),
        SpawnParallelGraphOutcome::Cancelled(message) => Ok(ToolResult::error(message)),
    }
}

/// Decode the tool's JSON request before it reaches host execution policy.
///
/// This remains beside the tool rather than becoming a TinyAgents API: the
/// parameter shape includes OpenHuman-specific ownership and worktree-policy
/// fields.
fn parse_parallel_agent_tasks(
    args: &serde_json::Value,
) -> Result<Vec<ParallelAgentTask>, ParallelAgentTaskRequestError> {
    let tasks_value = args.get("tasks").cloned().ok_or_else(|| {
        ParallelAgentTaskRequestError::MissingTasks("Missing 'tasks' parameter".into())
    })?;
    let tasks: Vec<ParallelAgentTask> = serde_json::from_value(tasks_value).map_err(|err| {
        ParallelAgentTaskRequestError::InvalidTasks(format!("Invalid tasks array: {err}"))
    })?;
    if tasks.len() < 2 {
        return Err(ParallelAgentTaskRequestError::Rejected(
            "spawn_parallel_agents requires at least two tasks".into(),
        ));
    }
    Ok(tasks)
}

enum ParallelAgentTaskRequestError {
    MissingTasks(String),
    InvalidTasks(String),
    Rejected(String),
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
         Each task has `{agent_id, prompt, context?, ownership?, isolation?, base_ref?}`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let agent_ids: Vec<String> = AgentDefinitionRegistry::current()
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
    /// like the long-running shell tool.
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
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let parent_call_id = crate::tools::host_extensions::tool_call_id(tool_context);
        if let Some(live_parent) = super::ambient_parent_run_context("direct-spawn-parallel") {
            return execute_spawn_parallel_agents(
                args,
                live_parent.cancellation.clone(),
                live_parent.workspace.clone(),
                live_parent.data.child(),
                Some(&live_parent),
                parent_call_id,
            )
            .await;
        }
        let workspace_descriptor = tool_context.and_then(|ctx| ctx.workspace().cloned());
        execute_spawn_parallel_agents(
            args,
            tinyagents_harness::CancellationToken::new(),
            workspace_descriptor,
            crate::agent::tinyagents::host::OpenHumanRunContext::new(),
            None,
            parent_call_id,
        )
        .await
    }
}

#[cfg(test)]
#[path = "spawn_parallel_agents_tests.rs"]
mod tests;
