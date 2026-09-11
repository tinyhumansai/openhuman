//! Git-worktree isolation for parallel, edit-capable agent workers.
//!
//! Parallel coding workers spawned via [`super::tools::spawn_parallel_agents`]
//! historically shared one workspace (`Config.action_dir`). Two workers editing
//! overlapping files left the parent with stale assumptions and silent
//! clobbers. Each isolated worker instead gets its own `git worktree` checkout
//! of the **user's project repo** (the directory the coding agent edits), so
//! file edits never collide.
//!
//! ## Where the implementation lives
//!
//! The git plumbing itself is **TinyAgents'**
//! ([`tinyagents_harness::workspace::git`]): worktree create/list/status/diff/
//! remove, repo-root validation, run-id sanitizing, and cross-worker overlap
//! detection are host-agnostic and are re-exported here under their historical
//! OpenHuman names so call sites and the RPC surface are unchanged.
//!
//! What stays in OpenHuman is the part that depends on *this* host: the
//! [`OpenHumanWorktreeIsolation`] adapter, which stamps an
//! `openhuman.worktree:{agent}:{run_id}` policy id onto prepared descriptors and
//! announces workspace lifecycle on the OpenHuman event bus so audit and
//! observability subscribers can correlate an isolated run with its allowed
//! root.
//!
//! ## Scope and safety
//!
//! - This targets the **user's project repository** rooted at the agent's
//!   `action_dir` — it never operates on OpenHuman's own source tree.
//! - Every operation validates that `repo_root` is a real git repository
//!   first (via `git rev-parse --is-inside-work-tree`), so a stray path can
//!   never be mutated.
//! - [`remove`] refuses to delete a **dirty** worktree unless `force = true`.
//!   Clean worktrees can be auto-reclaimed; dirty ones require an explicit
//!   user decision (acceptance criterion of #3376).
//!
//! The wrapper shells out to `git` through [`std::process::Command`] with an
//! explicit, validated working directory. It does not inherit ambient git
//! configuration that could redirect operations elsewhere.

use std::path::{Path, PathBuf};

use tinyagents_harness::tool::SandboxMode;
use tinyagents_harness::workspace::{
    GitWorktreeIsolation, WorkspaceDescriptor, WorkspaceIsolation,
};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;

// The git worktree surface is TinyAgents'. These aliases keep the historical
// OpenHuman spellings — `worktree::create`, `worktree::BaseRef`,
// `WorktreeStatus` — so the RPC schemas, tools, and tests that name them do not
// change. `WorktreeStatus` in particular is serialized straight to the desktop
// UI; it is field- and `serde`-identical to the crate type, pinned by
// `worktree_status_serializes_with_stable_camel_case_keys`.
pub use tinyagents_harness::workspace::{
    create_git_worktree as create, detect_worktree_overlaps as detect_overlaps,
    git_worktree_diff_summary as diff_summary, git_worktree_status as status,
    list_git_worktrees as list, remove_git_worktree as remove, GitWorktreeBaseRef as BaseRef,
    GitWorktreeError as WorktreeError, GitWorktreeStatus as WorktreeStatus,
    GIT_WORKTREE_SUBDIR as WORKTREE_SUBDIR,
};

/// OpenHuman's [`WorkspaceIsolation`] adapter over the TinyAgents git-worktree
/// provider.
///
/// The crate's [`GitWorktreeIsolation`] already prepares and cleans up the
/// checkout. This wrapper adds the two things that are OpenHuman's rather than
/// every host's: the `openhuman.worktree:*` policy-id convention, and
/// [`DomainEvent::WorkspacePrepared`] / [`DomainEvent::WorkspaceCleanup`]
/// announcements on the global bus.
#[derive(Debug, Clone)]
pub struct OpenHumanWorktreeIsolation {
    inner: GitWorktreeIsolation,
}

impl OpenHumanWorktreeIsolation {
    /// Create an isolation provider rooted at the user's project repo.
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            inner: GitWorktreeIsolation::new(repo_root),
        }
    }

    /// Select which ref new worktrees branch from.
    pub fn with_base_ref(mut self, base_ref: BaseRef) -> Self {
        self.inner = self.inner.with_base_ref(base_ref);
        self
    }

    /// Advertise the sandbox expectation on prepared descriptors.
    pub fn with_sandbox(mut self, sandbox: SandboxMode) -> Self {
        self.inner = self.inner.with_sandbox(sandbox);
        self
    }

    /// Add an extra root tools may touch alongside the isolated checkout.
    pub fn with_trusted_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.inner = self.inner.with_trusted_root(root);
        self
    }

    /// OpenHuman's policy-id convention for an isolated run.
    fn policy_id(run_id: &str, agent: Option<&str>) -> String {
        match agent {
            Some(agent) if !agent.is_empty() => format!("openhuman.worktree:{agent}:{run_id}"),
            _ => format!("openhuman.worktree:{run_id}"),
        }
    }
}

#[async_trait::async_trait]
impl WorkspaceIsolation for OpenHumanWorktreeIsolation {
    async fn prepare(
        &self,
        run_id: &str,
        agent: Option<&str>,
    ) -> tinyagents_harness::Result<WorkspaceDescriptor> {
        tracing::debug!(
            run_id,
            agent = agent.unwrap_or(""),
            "[worktree] workspace_prepare_start"
        );

        let descriptor = self
            .inner
            .prepare(run_id, agent)
            .await?
            .with_policy_id(Self::policy_id(run_id, agent));

        tracing::debug!(
            root = %descriptor.root.display(),
            policy_id = %descriptor.policy_id,
            "[worktree] workspace_prepare_done"
        );
        // Announce the prepared workspace so audit/observability subscribers can
        // correlate the isolated run with its allowed root.
        tracing::debug!(
            root = %descriptor.root.display(),
            policy_id = %descriptor.policy_id,
            "[workspace] workspace_prepared_emit"
        );
        let _ = BUS.publish(DomainEvent::WorkspacePrepared {
            policy_id: descriptor.policy_id.clone(),
            root: descriptor.root.display().to_string(),
        });
        Ok(descriptor)
    }

    async fn cleanup(&self, descriptor: &WorkspaceDescriptor) -> tinyagents_harness::Result<()> {
        tracing::debug!(
            root = %descriptor.root.display(),
            policy_id = %descriptor.policy_id,
            "[worktree] workspace_cleanup_start"
        );

        match self.inner.cleanup(descriptor).await {
            Ok(()) => {
                tracing::debug!(
                    root = %descriptor.root.display(),
                    policy_id = %descriptor.policy_id,
                    "[worktree] workspace_cleanup_done"
                );
                tracing::debug!(
                    policy_id = %descriptor.policy_id,
                    "[workspace] workspace_cleanup_emit_ok"
                );
                let _ = BUS.publish(DomainEvent::WorkspaceCleanup {
                    policy_id: descriptor.policy_id.clone(),
                    error: None,
                });
                Ok(())
            }
            Err(err) => {
                let message = err.to_string();
                tracing::warn!(
                    root = %descriptor.root.display(),
                    policy_id = %descriptor.policy_id,
                    error = %message,
                    "[worktree] workspace_cleanup_failed"
                );
                tracing::debug!(
                    policy_id = %descriptor.policy_id,
                    error = %message,
                    "[workspace] workspace_cleanup_emit_err"
                );
                let _ = BUS.publish(DomainEvent::WorkspaceCleanup {
                    policy_id: descriptor.policy_id.clone(),
                    error: Some(message),
                });
                Err(err)
            }
        }
    }
}

/// Fail-closed workspace path gate that mirrors
/// [`WorkspaceDescriptor::enforce`] but routes the violation onto OpenHuman's
/// global event bus instead of the SDK [`EventSink`], so audit/observability
/// subscribers see out-of-root rejections.
///
/// This is a **carrier-side check only** — it publishes a
/// [`DomainEvent::WorkspaceViolation`] and returns an error when `path` escapes
/// the descriptor's allowed roots. It does **not** replace the authoritative
/// enforcement done by `SecurityPolicy`/landlock; it is an additional
/// observability + fail-closed signal keyed on the descriptor the isolated run
/// carries.
///
/// [`EventSink`]: tinyagents_harness::events::EventSink
pub fn enforce_workspace_path(
    descriptor: &WorkspaceDescriptor,
    path: &Path,
) -> std::result::Result<(), WorkspacePathError> {
    if descriptor.allows(path) {
        return Ok(());
    }
    let rendered = path.display().to_string();
    tracing::warn!(
        path = %rendered,
        policy_id = %descriptor.policy_id,
        "[workspace] workspace_violation"
    );
    BUS.publish(DomainEvent::WorkspaceViolation { path: rendered });
    Err(WorkspacePathError::OutsideWorkspace(path.to_path_buf()))
}

/// Rejection from [`enforce_workspace_path`].
///
/// Separate from [`WorktreeError`] (which is TinyAgents' git-plumbing error)
/// because this gate is about OpenHuman's descriptor policy, not about git.
#[derive(Debug, thiserror::Error)]
pub enum WorkspacePathError {
    /// The path escaped every root the descriptor allows.
    #[error("path is outside the allowed workspace roots: {0}")]
    OutsideWorkspace(PathBuf),
}

#[cfg(test)]
#[path = "worktree_tests.rs"]
mod tests;
