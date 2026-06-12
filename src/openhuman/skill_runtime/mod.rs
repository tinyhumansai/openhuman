//! Skill runtime: execution, cancellation, and run-log polling for installed
//! SKILL.md workflows.
//!
//! `workflows` owns discovery and installed skill metadata. `skill_registry`
//! owns remote catalogs and install sources. This module owns actually running
//! a skill, regardless of whether the skill's instructions call Python, Node,
//! shell tools, or another OpenHuman agent tool.

pub mod agent;
pub mod ops;
mod run_machinery;
pub mod schemas;
pub mod tools;

pub use run_machinery::{await_run_outcome, spawn_workflow_run_background, WorkflowRunStarted};
pub use schemas::{
    all_skill_runtime_controller_schemas, all_skill_runtime_registered_controllers,
    skill_runtime_schemas,
};
