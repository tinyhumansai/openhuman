//! Workflow metadata helpers.

pub mod bus;
pub mod ops;
pub mod ops_create;
pub mod ops_discover;
pub mod ops_install;
pub mod ops_parse;
pub mod ops_types;
pub mod preflight;
pub mod registry;
pub mod run_log;
pub mod schemas;
pub mod tools;
pub mod types;

#[cfg(test)]
#[path = "e2e_plumbing_tests.rs"]
mod e2e_plumbing_tests;

#[cfg(test)]
#[path = "e2e_run_tests.rs"]
mod e2e_run_tests;

pub use ops::*;
pub use schemas::{
    all_workflows_controller_schemas, all_workflows_registered_controllers, workflows_schemas,
};
