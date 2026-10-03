//! The doctor report: config, workspace, daemon, environment, memory, and
//! agent-SDK diagnostic checks, plus the model-probe report.
//!
//! Split by responsibility rather than by line count — see `core/*.rs`. This
//! file only wires the submodules together and re-exports the public API.

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;

mod config_checks;
mod daemon_env_checks;
mod memory_agent_checks;
mod run;
mod types;
mod workspace_checks;

pub use run::{run, run_models, MemoryEngineCheck};
pub use types::{
    DiagnosticItem, DoctorReport, DoctorSummary, ModelProbeEntry, ModelProbeOutcome,
    ModelProbeReport, ModelProbeSummary, Severity,
};

#[cfg(test)]
use config_checks::{check_config_semantics, embedding_provider_validation_error};
#[cfg(test)]
use daemon_env_checks::truncate_for_display;
#[cfg(test)]
use memory_agent_checks::model_matches;
