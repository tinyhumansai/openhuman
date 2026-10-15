//! `SecurityPolicy` as the filesystem tools' [`FsGate`].
//!
//! A mechanical mapping: every method is one call the tools used to make on
//! the policy directly, with the same argument and the same answer. No policy
//! is decided here; the adapter only names the questions.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tinytools_std::filesystem::FsGate;

use crate::security::policy::{TrustedAccess, TrustedRoot};
use crate::security::{AutonomyLevel, GateDecision, SecurityPolicy};
use tinybox_bus::CommandClass;

/// Clone `security` and scope it to `root`, a run's workspace descriptor root.
///
/// The root becomes both the relative-path resolution root (`action_dir`)
/// **and** a `ReadWrite` trusted root. The grant is the load-bearing half:
/// `action_dir` only decides where a relative path lands, while the allow/deny
/// decision reads `workspace_dir` + `trusted_roots`
/// (`SecurityPolicy::is_resolved_path_allowed_for`). Without the grant a
/// descriptor rooted outside `workspace_dir` moved the cwd but refused every
/// read and write in it.
///
/// The grant is *additive and per-call*: it is pushed onto a clone, so nothing
/// process-global is mutated and concurrent turns cannot race each other. It
/// also cannot widen the hard invariants: `is_always_forbidden` (credential
/// stores, core OS dirs) and `is_workspace_internal_path` (core-managed state
/// under `workspace_dir`) are both evaluated *before* any trusted-root
/// shortcut, so a granted root can never expose them.
///
/// The root always originates from trusted in-process code (the session
/// builder, the sub-agent runner, or the `cwd` RPC parameter), never from
/// model-supplied text.
pub(super) fn security_scoped_to_root(security: &SecurityPolicy, root: &Path) -> SecurityPolicy {
    let mut scoped = security.clone();
    scoped.action_dir = root.to_path_buf();
    scoped.trusted_roots.push(TrustedRoot {
        path: root.to_string_lossy().to_string(),
        access: TrustedAccess::ReadWrite,
    });
    scoped
}

#[async_trait]
impl FsGate for SecurityPolicy {
    fn can_act(&self) -> bool {
        SecurityPolicy::can_act(self)
    }

    fn is_read_only(&self) -> bool {
        // Deliberately the raw tier, not `can_act()`: `git_operations` asks
        // this in addition to `can_act()` and always has.
        self.autonomy == AutonomyLevel::ReadOnly
    }

    fn is_rate_limited(&self) -> bool {
        SecurityPolicy::is_rate_limited(self)
    }

    fn record_action(&self) -> bool {
        SecurityPolicy::record_action(self)
    }

    fn write_needs_approval(&self) -> bool {
        // The module's gate for write-class actions. An unanswered question is a
        // prompt, never a silent allow.
        match self.command_class_gate_blocking(CommandClass::Write) {
            Ok(gate) => gate == GateDecision::Prompt,
            Err(refusal) => {
                log::warn!("[fs][gate] tier gate unavailable; asking for approval: {refusal:?}");
                true
            }
        }
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
        Arc::new(security_scoped_to_root(self, root))
    }
}
