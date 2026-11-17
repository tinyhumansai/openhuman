//! Workflow discovery: scanning root directories, scope resolution, collision handling,
//! and skill resource reading.
//!
//! Split into submodules by responsibility: [`api`] holds the public
//! discovery entry points, [`scan`] holds the root-directory scan engine,
//! and [`resource`] holds bundled-resource reading.

mod api;
mod resource;
mod scan;

// Re-imported here (rather than only inside the submodules that need them)
// so `use super::*` in the `#[path]`-included test files below still finds
// `Path`, `Workflow`, `WorkflowScope`, and `precedence` exactly as it did
// before this module was split into submodules.
#[cfg(test)]
use std::path::Path;

pub use api::{
    discover_automations, discover_workflows, discovery_home_dir, is_workspace_trusted,
    load_workflow_metadata, load_workflow_metadata_for,
};
pub use resource::{read_workflow_resource, read_workflow_resource_for};

pub(crate) use api::discover_workflows_inner;
pub(crate) use api::invalidate_workflow_metadata_cache;
pub(crate) use api::load_workflow_metadata_for_domains;
pub(crate) use api::load_workflow_metadata_for_with_home;
#[cfg(feature = "flows")]
pub(crate) use api::DiscoveryGate;

#[cfg(test)]
pub(crate) use api::DISCOVERY_CALLS;

#[cfg(test)]
#[path = "ops_discover_include_skills_tests_tests.rs"]
mod include_skills_tests;
