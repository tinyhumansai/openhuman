//! `SecurityPolicy` and the autonomy/risk gate. See [`README.md`](README.md)
//! for the invariants this module enforces (workspace-internal path
//! protection, fail-closed command classification, always-forbidden paths).

mod command_analysis;
mod command_checks;
mod enforcement;
mod module_gate;
pub(crate) mod native_commands;
#[cfg(test)]
pub(crate) mod native_commands_stub;
#[cfg(feature = "security-module")]
mod native_paths;
mod path_checks;
#[cfg(any(feature = "modules", test))]
pub(crate) use path_checks::home_credential_stores;

mod types;

pub(crate) use command_analysis::CommandAnalysis;
pub use enforcement::validate_path_within_root;
pub use enforcement::{ensure_openhuman_scratch_dir, openhuman_scratch_dir};
pub use module_gate::{host_action_class, Budget, PathKind};
pub use native_commands::{
    CommandFlow, CommandGateway, CommandQuery, CommandStage, CommandVerdict, Refusal,
    SharedCommandGateway,
};
pub use types::{
    tool_result_artifacts_dir, ActionTracker, AutonomyLevel, CommandClass, CommandRiskLevel,
    GateDecision, SecurityPolicy, ToolOperation, TrustedAccess, TrustedRoot,
};

#[cfg(test)]
use std::path::{Path, PathBuf};

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "policy_disabled_tests.rs"]
mod policy_disabled_tests;

#[cfg(test)]
#[path = "policy_characterization_tests.rs"]
mod policy_characterization_tests;

#[cfg(test)]
#[path = "policy_media_artifact_tests.rs"]
mod policy_media_artifact_tests;
