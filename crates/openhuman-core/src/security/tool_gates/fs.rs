//! `SecurityPolicy` as the filesystem tools' [`FsGate`].
//!
//! A mechanical mapping: every method is one call the tools used to make on
//! the policy directly, with the same argument and the same answer. No policy
//! is decided here; the adapter only names the questions.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness_tools::fs::filesystem::FsGate;

use crate::security::policy::scoped_to_workspace;
use crate::security::{GateDecision, SecurityPolicy};

#[async_trait]
impl FsGate for SecurityPolicy {
    fn can_act(&self) -> bool {
        SecurityPolicy::can_act(self)
    }

    fn is_read_only(&self) -> bool {
        // Deliberately the raw tier, not `can_act()`: `git_operations` asks
        // this in addition to `can_act()` and always has.
        self.gate_decision(crate::security::policy::CommandClass::Write)
            == crate::security::GateDecision::Block
    }

    fn is_rate_limited(&self) -> bool {
        SecurityPolicy::is_rate_limited(self)
    }

    fn record_action(&self) -> bool {
        SecurityPolicy::record_action(self)
    }

    fn write_needs_approval(&self) -> bool {
        self.gate_decision(crate::security::policy::CommandClass::Write) == GateDecision::Prompt
    }

    fn action_dir(&self) -> &Path {
        &self.action_dir
    }

    fn is_path_string_allowed(&self, path: &str) -> bool {
        SecurityPolicy::is_path_string_allowed(self, path)
    }

    async fn validate_path(&self, path: &str) -> Result<PathBuf, String> {
        SecurityPolicy::validate_path(self, path).await
    }

    async fn validate_parent_path(&self, path: &str) -> Result<PathBuf, String> {
        SecurityPolicy::validate_parent_path(self, path).await
    }

    fn scoped_to_workspace(&self, root: &Path) -> Arc<dyn FsGate> {
        Arc::new(scoped_to_workspace(self, root))
    }
}

#[cfg(test)]
#[path = "fs_tests.rs"]
mod tests;
