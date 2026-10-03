//! Agent-owned dialogue and control tools.
//!
//! These tools act on the agent loop or its todo list rather than on files,
//! memory, or the network. Wire names are
//! given in parentheses where they differ from the type name:
//!
//! - `AskClarificationTool` (`ask_user_clarification`, from `tinyagents_harness::tools`) — returns the
//!   question as its output; the turn actually pauses only because callers
//!   list this name in the harness seam's `early_exit_tools`.
//! - [`DelegateTool`] — hands a subtask to a named agent with its own
//!   provider/model configuration.
//! - [`PlanExitTool`] — ends a plan-mode pass by returning the plan plus
//!   [`PLAN_EXIT_MARKER`]. The mode switch itself lives outside the tool;
//!   nothing in this crate consumes the marker yet.
//! - `RunWorkflowTool` / `AwaitWorkflowTool` — spawn a
//!   `crate::skills::runtime` workflow run and wait on its outcome. Compiled
//!   in only with the `skills` feature, so builds without it omit both tools
//!   from the catalog.
//! - [`TodoTool`] — the session's todo list (whole-list write, thread-scoped).
//!
//! `crate::tools` re-exports everything here (`pub use
//! crate::agent::tools::*;` in `tools/mod.rs`); `tools::ops` registers the
//! tools into the catalog.
mod delegate;
mod plan_exit;
// Pure `skill_runtime` client (spawn + await a workflow run) — compiled out
// with the `skills` gate so the tool list OMITS these rather than degrading
// them to a disabled-error.
#[cfg(feature = "skills")]
mod run_workflow;
mod todo;

pub use delegate::DelegateTool;
pub(crate) use delegate::DelegateToolDispatch;
pub use plan_exit::{PlanExitTool, PLAN_EXIT_MARKER};
#[cfg(feature = "skills")]
pub use run_workflow::{
    AwaitWorkflowTool, RunWorkflowTool, AWAIT_WORKFLOW_TOOL_NAME, RUN_WORKFLOW_TOOL_NAME,
};
pub use todo::TodoTool;
pub(crate) use todo::TodoToolDispatch;
