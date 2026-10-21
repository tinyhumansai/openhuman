//! Registration of agent dialogue and delegation tools.

use super::orchestration::tools::{
    CloseSubagentTool, ContinueSubagentTool, DelegateGraphTool, ListSubagentsTool,
    SpawnAsyncSubagentTool, SpawnParallelAgentsTool, SpawnSubagentTool, SteerSubagentTool,
    WaitSubagentTool,
};
use super::tools::{PlanExitTool, TodoTool};
use crate::config::Config;
#[cfg(feature = "skills")]
use crate::skills::run_workflow::{AwaitWorkflowTool, RunWorkflowTool};
use std::path::Path;
use tinyagents_harness_tools as harness_tools;
use tinytools::Tool;
use tinytools_std::filesystem::WorkspaceStateTool;

/// Build the dialogue tool family in its persisted catalogue order.
pub(crate) fn dialogue_tools(action_dir: &Path, root_config: &Config) -> Vec<Box<dyn Tool>> {
    vec![
        // Sub-agent dispatch — lets the parent agent delegate focused
        // sub-tasks (research, code execution, API specialists, …) by
        // calling `spawn_subagent { agent_id, prompt, … }`. The runner
        // builds a narrow Agent from an `AgentDefinition` lookup and
        // returns a single text result. See
        // `agent::subagent_host` for the dispatch path.
        Box::new(SpawnSubagentTool::new()),
        Box::new(SpawnAsyncSubagentTool::new()),
        // Interactive clarification early-exit. Sub-agents pause on this tool
        // (checkpoint → `AwaitingUser`, see `subagent_host`) and
        // the orchestrator resumes them via `continue_subagent` (#4291).
        // Several agent scopes (orchestrator, crypto, markets, scheduler,
        // desktop control) name it, so it must exist in the base
        // registry or none of them can actually ask the user anything.
        Box::new(harness_tools::AskClarificationTool::new()),
        // Read-only project overview (git status, recent commits, top-level
        // tree) rooted at the agent action dir. Named by the orchestrator and
        // planner scopes.
        Box::new(WorkspaceStateTool::new(action_dir.to_path_buf())),
        // Steer/list/close reusable async sub-agents and collect results by
        // durable `subagent_session_id` (preferred) or transient `task_id`.
        Box::new(ListSubagentsTool::new()),
        Box::new(SteerSubagentTool::new()),
        Box::new(crate::agent::tools::WaitTool::new()),
        Box::new(crate::agent::tools::WaitLoopTool::new()),
        Box::new(WaitSubagentTool::new()),
        Box::new(CloseSubagentTool::new()),
        Box::new(ContinueSubagentTool::new()),
        Box::new(SpawnParallelAgentsTool::new()),
        // Multi-stage durable delegation (issue #4249, Phase 3): runs the chosen
        // sub-agent through the tinyagents plan→execute→review→finalize graph,
        // checkpointed to the session DB. Heavier than spawn_subagent; for
        // sub-tasks that benefit from a self-review/revision loop.
        Box::new(DelegateGraphTool::new()),
        // The session todo list (Claude/Codex style): one whole-list write per
        // call, scoped to the conversation thread. `plan_exit` is the marker
        // that hands a plan-mode pass off to a build-mode pass.
        Box::new(TodoTool::new(root_config.workspace_dir.clone())),
        // Interactive plan-review gate: parks the live turn on a thread-scoped
        // plan the user must approve before execution (Codex/Claude plan mode).
        Box::new(crate::plan_review::RequestPlanReviewTool::new()),
        Box::new(PlanExitTool::new()),
        // Workflow composition: `run_workflow` runs another workflow as a
        // subagent and (by default) waits on its result like a function call;
        // `await_workflow` re-attaches to a run that outlived its inline wait.
        // Both wrap `skill_runtime::spawn_workflow_run_background` +
        // `await_run_outcome` — the same spawn path `openhuman.skills_run`
        // JSON-RPC uses, so RPC and tool callers stay in sync.
        #[cfg(feature = "skills")]
        Box::new(RunWorkflowTool::new()),
        #[cfg(feature = "skills")]
        Box::new(AwaitWorkflowTool::new()),
    ]
}
