//! Task-local carrier for a **per-worker `action_dir` override**.
//!
//! Sibling of [`super::sandbox_context`] and [`super::fork_context`]. When an
//! edit-capable worker opts into git-worktree isolation
//! (`isolation = "worktree"`), the subagent runner installs the worktree's
//! checkout path here for the duration of that worker's run. Acting tools
//! (shell, git) read it via [`current_action_dir_override`] and, when present,
//! redirect their working directory to the isolated worktree instead of the
//! shared `Config.action_dir`.
//!
//! Why a task-local instead of rebuilding every tool with a new `action_dir`:
//! the subagent runner reuses the parent's already-constructed tool instances
//! (`parent.all_tools`), each of which captured `security.action_dir` at build
//! time. Threading a fresh `action_dir` into those instances would mean
//! reconstructing the entire tool set per worker. A task-local keeps the change
//! additive and scoped to exactly the worker turn that needs it — and the scope
//! does not leak into detached tasks (standard [`tokio::task_local!`]
//! semantics). When unset, tools fall through to `security.action_dir`, so the
//! non-isolated path is byte-for-byte unchanged.

use std::path::PathBuf;

tokio::task_local! {
    /// Absolute path to the isolated worktree checkout for the currently
    /// running worker. `None`-equivalent: the scope is simply not active.
    pub static CURRENT_ACTION_DIR_OVERRIDE: PathBuf;
}

/// Returns the active per-worker `action_dir` override, if one is installed.
///
/// Returns `None` when called outside [`with_action_dir_override`] — e.g. the
/// non-isolated parallel path, the main agent turn, CLI / JSON-RPC tool
/// dispatch, or unit tests that invoke a [`crate::openhuman::tools::Tool`]
/// directly.
pub fn current_action_dir_override() -> Option<PathBuf> {
    CURRENT_ACTION_DIR_OVERRIDE.try_with(|p| p.clone()).ok()
}

/// Run `future` with `action_dir` installed as the worker's action-dir
/// override. Intended call site is the subagent runner, wrapping the inner
/// tool-call loop for a worktree-isolated worker.
pub async fn with_action_dir_override<F, R>(action_dir: PathBuf, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    CURRENT_ACTION_DIR_OVERRIDE.scope(action_dir, future).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn override_absent_outside_scope() {
        assert_eq!(current_action_dir_override(), None);
    }

    #[tokio::test]
    async fn override_visible_inside_scope() {
        let dir = PathBuf::from("/tmp/worker-xyz");
        let seen =
            with_action_dir_override(dir.clone(), async { current_action_dir_override() }).await;
        assert_eq!(seen, Some(dir));
    }

    #[tokio::test]
    async fn override_does_not_leak() {
        with_action_dir_override(PathBuf::from("/tmp/a"), async {
            assert_eq!(current_action_dir_override(), Some(PathBuf::from("/tmp/a")));
        })
        .await;
        assert_eq!(current_action_dir_override(), None);
    }
}
