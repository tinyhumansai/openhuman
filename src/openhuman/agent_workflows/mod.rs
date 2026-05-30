//! Agent Workflows domain — phase-keyed guidance bound to task lifecycle.
//!
//! Mirrors the `skills` domain layout. A *workflow* is a `WORKFLOW.md` file
//! with YAML frontmatter describing **phases** (lifecycle hooks such as
//! `on_pick_up_task`, `on_close_task`, `on_enter_directory`). Each phase can
//! inject rules, run gated scripts, scope visible tools, and surface
//! working-directory context. Workflows are bound to *tasks*: the harness fires
//! phases off task-board status transitions (pick-up / close) and directory
//! changes, then runs the phase's effects before the agent's next turn.

pub mod discover;
pub mod inject;
pub mod ops;
pub mod parse;
pub mod schemas;
pub mod select;
pub mod types;
pub mod workdir;

pub use types::{
    is_workspace_trusted, ToolScope, Workflow, WorkflowFrontmatter, WorkflowPhase, WorkflowScope,
    WorkflowSummary, KNOWN_PHASES, PHASE_CLOSE_TASK, PHASE_ENTER_DIRECTORY, PHASE_PICK_UP_TASK,
    WORKFLOW_MD,
};

pub use discover::{discover_workflows, load_workflows};
pub use inject::{render_available_workflows, render_workflow_catalog};
pub use ops::{create_workflow, read_workflow, uninstall_workflow};
pub use select::{best_match, effective_tool_scope, phase_guidance};
pub use workdir::working_dir_context;

pub use schemas::{
    all_controller_schemas as all_agent_workflows_controller_schemas,
    all_registered_controllers as all_agent_workflows_registered_controllers,
};
