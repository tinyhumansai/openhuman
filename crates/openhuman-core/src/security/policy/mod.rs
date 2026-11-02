//! Host context and adapters for TinySecurity policy operations.

mod enforcement;
mod module_gate;
mod native_commands;
mod native_paths;
mod path_checks;

mod types;

pub use module_gate::{host_action_class, PathKind};
#[cfg(test)]
pub(crate) use native_commands::Sealed;
pub use native_commands::{
    CommandFlow, CommandGateway, CommandQuery, CommandStage, CommandVerdict, Refusal,
};

pub use enforcement::validate_path_within_root;
pub use enforcement::{ensure_openhuman_scratch_dir, openhuman_scratch_dir};
pub use types::{
    tool_result_artifacts_dir, AutonomyLevel, CommandClass, GateDecision, SecurityPolicy,
    ToolOperation, TrustedAccess, TrustedRoot,
};

#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;

#[cfg(all(test, feature = "security-module"))]
#[path = "policy_media_artifact_tests.rs"]
mod policy_media_artifact_tests;

#[cfg(all(test, not(feature = "security-module")))]
#[path = "path_checks_no_module_tests.rs"]
mod no_module_tests;
