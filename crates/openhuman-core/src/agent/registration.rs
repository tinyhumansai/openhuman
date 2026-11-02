//! Registration of agent dialogue and delegation tools.

use super::orchestration::tools::SpawnSubagentTool;
use super::tools::TodoTool;
use crate::config::Config;
#[cfg(feature = "skills")]
use crate::skills::run_workflow::{AwaitWorkflowTool, RunWorkflowTool};
use std::path::Path;
use tinyagents_harness_tools as harness_tools;
use tinyagents_harness_tools::fs::filesystem::WorkspaceStateTool;
use tinytools::Tool;

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
        // Interactive clarification early-exit. A paused child is resumed by
        // spawning again with the same task_key.
        // Several agent scopes (orchestrator, crypto, markets, scheduler,
        // desktop control) name it, so it must exist in the base
        // registry or none of them can actually ask the user anything.
        Box::new(harness_tools::AskClarificationTool::new()),
        // Read-only project overview (git status, recent commits, top-level
        // tree) rooted at the agent action dir. Named by the orchestrator and
        // planner scopes.
        Box::new(WorkspaceStateTool::new(action_dir.to_path_buf())),
        // The session todo list is scoped to the conversation thread.
        // `plan_exit` hands a completed plan-mode pass to the build-mode pass.
        Box::new(TodoTool::new(root_config.workspace_dir.clone())),
        // Interactive plan-review gate: parks the live turn on a thread-scoped
        // plan the user must approve before execution (Codex/Claude plan mode).
        Box::new(crate::plan_review::host::request_plan_review_tool()),
        Box::new(super::tools::plan_exit::plan_exit_tool()),
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
