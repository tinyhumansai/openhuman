//! OpenHuman's implementations of the two artifact-offload host policies.
//!
//! `tinyagents_harness::artifacts` owns the mechanics — thresholds, path
//! resolution, pointer rendering, the symlink re-check. It deliberately knows
//! nothing about `SecurityPolicy` or about which byte patterns are credentials,
//! because neither has any meaning in a redistributed crate. Those two
//! decisions are enforced *here*, on the way into the write.
//!
//! An adapter that widened either one to make a signature fit would silently
//! disable a guarantee the rest of the system assumes, and no crate-side test
//! could catch it.

use std::path::Path;
use std::sync::Arc;

use tinyagents_harness::artifacts::{ArtifactPathPolicy, ArtifactRedactor, Redacted};

use crate::openhuman::memory::safety::sanitize_text;
use crate::openhuman::security::SecurityPolicy;

/// Refuses artifact writes that reach the core's internal `workspace_dir`.
///
/// The two methods answer different questions and produce different errors, so
/// both are wired rather than collapsing them into one check:
///
/// * [`is_internal_state`](ArtifactPathPolicy::is_internal_state) →
///   `SecurityPolicy::is_workspace_internal_path`, the specific state locations,
///   which are refused wherever they sit.
/// * [`internal_root`](ArtifactPathPolicy::internal_root) → `workspace_dir`
///   itself, refused wholesale.
///
/// Offload targets resolve under `action_dir`, never `workspace_dir` — the
/// invariant `is_workspace_internal_path` enforces fail-closed regardless of
/// autonomy tier or `trusted_roots`.
#[derive(Debug)]
pub struct WorkspaceGuard {
    policy: Arc<SecurityPolicy>,
}

impl WorkspaceGuard {
    /// Guard backed by `policy`.
    pub fn new(policy: Arc<SecurityPolicy>) -> Self {
        Self { policy }
    }
}

impl ArtifactPathPolicy for WorkspaceGuard {
    fn is_internal_state(&self, path: &Path) -> bool {
        self.policy.is_workspace_internal_path(path)
    }

    fn internal_root(&self) -> Option<&Path> {
        Some(self.policy.workspace_dir.as_path())
    }
}

/// Scrubs credentials and PII out of an artifact body before it is stored,
/// using the same `sanitize_text` pass that persists oversized tool results.
///
/// Sharing that function is the point: an artifact on disk is exactly as
/// readable as a persisted tool result, so a second, weaker redaction path here
/// would be a hole in the same wall.
#[derive(Debug)]
pub struct SanitizingRedactor;

impl ArtifactRedactor for SanitizingRedactor {
    fn redact(&self, content: &str) -> Redacted {
        let sanitized = sanitize_text(content);
        if sanitized.report.changed() {
            Redacted::rewritten(sanitized.value)
        } else {
            Redacted::unchanged(sanitized.value)
        }
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
