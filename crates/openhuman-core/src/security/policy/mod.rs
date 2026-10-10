//! `SecurityPolicy` and the autonomy/risk gate. See [`README.md`](README.md)
//! for the invariants this module enforces (workspace-internal path
//! protection, fail-closed command classification, always-forbidden paths).

mod command_checks;
mod enforcement;
#[cfg(feature = "security-module")]
mod native_paths;
mod path_checks;

mod types;

pub use enforcement::validate_path_within_root;
pub use enforcement::{ensure_openhuman_scratch_dir, openhuman_scratch_dir};
pub use types::{
    tool_result_artifacts_dir, ActionTracker, AutonomyLevel, CommandClass, CommandRiskLevel,
    GateDecision, SecurityPolicy, ToolOperation, TrustedAccess, TrustedRoot, POLICY_BLOCKED_MARKER,
    POLICY_DENIED_MARKER,
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
mod proptest_tests;
