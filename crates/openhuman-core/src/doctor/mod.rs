//! Native diagnostic checks for configuration, workspace, and runtime health.

pub(crate) mod checks;
mod contract;
mod core;
pub mod ops;
pub mod profile;
pub mod registry;
mod runner;
mod schemas;
pub mod tools;

pub use contract::{
    ComponentId, DoctorCheck, DoctorConfigSource, DoctorCtx, DoctorOptions, DoctorReport,
    DoctorSummary, DoctorTier, Finding, FindingSeverity, ValueSource,
};
pub use core::{ModelProbeEntry, ModelProbeOutcome, ModelProbeReport, ModelProbeSummary};
pub use ops::*;
pub use registry::{
    register_check_source, register_check_source_in, registered_checks_for, DoctorCheckSource,
};
pub(crate) use schemas::all_controller_entries as all_doctor_controller_entries;
#[cfg(test)]
pub(crate) use schemas::doctor_parameter_contracts;
pub use schemas::{
    all_controller_schemas as all_doctor_controller_schemas,
    all_registered_controllers as all_doctor_registered_controllers,
};

#[cfg(test)]
#[path = "config_file_tests.rs"]
mod config_file_red_tests;

#[cfg(test)]
#[path = "config_provenance_tests.rs"]
mod config_provenance_red_tests;

#[cfg(test)]
#[path = "config_semantics_tests.rs"]
mod config_semantics_red_tests;

#[cfg(test)]
#[path = "inference_config_tests.rs"]
mod inference_config_red_tests;

#[cfg(test)]
#[path = "embeddings_config_tests.rs"]
mod embeddings_config_red_tests;

#[cfg(test)]
#[path = "checks/config_file_tests.rs"]
mod config_file_tests;

#[cfg(test)]
#[path = "runner_tests.rs"]
mod runner_tests;

#[cfg(test)]
#[path = "connectivity_tests.rs"]
mod connectivity_tests;

#[cfg(test)]
#[path = "checks/config_provenance_tests.rs"]
mod config_provenance_tests;

#[cfg(test)]
#[path = "checks/inference_tests.rs"]
mod inference_config_tests;

#[cfg(test)]
#[path = "inference_catalog_tests.rs"]
mod inference_catalog_tests;

#[cfg(test)]
#[path = "inference_auth_error_tests.rs"]
mod inference_auth_error_tests;

#[cfg(test)]
#[path = "inference_model_catalog_tests.rs"]
mod inference_model_catalog_tests;

#[cfg(test)]
#[path = "inference_catalog_test_support.rs"]
mod inference_catalog_test_support;

#[cfg(test)]
pub(crate) mod test_support;
