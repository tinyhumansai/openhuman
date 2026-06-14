mod command_checks;
mod enforcement;
mod path_checks;

#[path = "policy_command.rs"]
mod policy_command;

mod types;

pub use enforcement::validate_path_within_root;
pub use enforcement::{ensure_openhuman_scratch_dir, openhuman_scratch_dir};
pub use types::{
    ActionTracker, AutonomyLevel, CommandClass, CommandRiskLevel, GateDecision, SecurityPolicy,
    ToolOperation, TrustedAccess, TrustedRoot, POLICY_BLOCKED_MARKER, POLICY_DENIED_MARKER,
};

#[cfg(test)]
use std::path::{Path, PathBuf};

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
