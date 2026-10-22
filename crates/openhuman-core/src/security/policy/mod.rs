//! `SecurityPolicy` and the autonomy/risk gate. See [`README.md`](README.md)
//! for the invariants this module enforces (workspace-internal path
//! protection, fail-closed command classification, always-forbidden paths).

mod command_checks;
pub mod command_scans;
pub use command_scans::*;
mod enforcement;
mod path_checks;

mod types;

pub use enforcement::validate_path_within_root;
pub use enforcement::{ensure_openhuman_scratch_dir, openhuman_scratch_dir};
#[cfg(any(feature = "modules", test))]
pub(crate) use path_checks::home_credential_stores;
#[cfg(feature = "modules")]
pub(crate) use path_checks::{
    ALWAYS_FORBIDDEN_PREFIXES, SENSITIVE_COMPONENTS, SENSITIVE_FILES, SENSITIVE_PAIRS,
};
pub use types::{
    ActionTracker, AutonomyLevel, CommandRiskLevel, GateDecision, SecurityPolicy, ToolOperation,
    TrustedAccess, TrustedRoot, tool_result_artifacts_dir,
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

#[cfg(test)]
#[path = "policy_media_artifact_tests.rs"]
mod policy_media_artifact_tests;
