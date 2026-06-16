//! Durable dynamic workflow runs (issue #3375).
//!
//! A first-class, repeatable multi-agent orchestration model: a declarative
//! [`WorkflowDefinition`] (phase graph) coordinates many child agents, and each
//! run's durable state lives in `session_db::run_ledger` (the `workflow_runs`
//! table) rather than the main chat context, so runs can be listed, inspected,
//! and — once the engine lands — stopped and resumed.
//!
//! PR1 scope: the declarative definition model, the builtin read-only
//! "parallel research with cross-checking" workflow, structural + agent
//! validation, and the read controllers (`workflow_run_list_definitions`,
//! `workflow_run_list`, `workflow_run_get`).
//!
//! PR2 scope (`engine.rs`): the live execution engine — `start`/`stop`/`resume`
//! controllers, phase scheduling that walks the dependency DAG and fans out each
//! phase's agents through the programmatic `AgentOrchestrationSession` with
//! bounded concurrency and a run-wide `max_children` cap, persisting phase
//! outputs to the run ledger after every phase.
//!
//! Namespace note: this is distinct from the existing `workflows` domain, which
//! handles SKILL.md / WORKFLOW.md bundle discovery.

mod engine;
mod ops;
mod schemas;
pub mod types;

pub use engine::{resume_workflow_run, start_workflow_run, stop_workflow_run};
pub use ops::{
    builtin_definitions, definition_by_id, get_run, list_definitions, list_runs,
    validate_definition, validate_structure, PARALLEL_RESEARCH_ID,
};
pub use schemas::{
    all_controller_schemas as all_workflow_run_controller_schemas,
    all_registered_controllers as all_workflow_run_registered_controllers,
};
pub use types::{
    DefinitionError, WorkflowDefinition, WorkflowDefinitionListResponse, WorkflowPhase,
    WorkflowSafetyTier,
};
