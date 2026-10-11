//! Where the session host's tool-result artifact store comes from (#6408).
//!
//! Extracted from `runtime_session.rs` rather than inlined there: the choice of
//! root is the whole correctness question for this feature, and it needs more
//! prose than a call site should carry.

use std::path::{Path, PathBuf};

use crate::agent::harness::tool_result_artifacts::new_tool_result_store;
use tinyagents_harness::artifacts::tool_results::ToolResultArtifactStore;

/// How long another session's tool-result artifacts survive before a later
/// session sweeps them.
///
/// **A conservative default chosen for safety, not a retention policy.** The
/// artifact store had no bound at all — nothing anywhere deletes these files —
/// and shipping unbounded growth to fix a token-burn bug would trade one
/// problem for another. This is the smallest thing that cannot grow without
/// limit; the feature's owner should confirm or replace it.
///
/// Alternatives considered: a count cap (needs a policy for which artifacts are
/// worth keeping, which this code has no basis to decide), and a sweep at
/// session end (there is no single point where a session host ends, and a crash
/// would skip it entirely). Age is the only one of the three that is correct
/// without knowing the workload.
///
/// 24 hours because an artifact is only useful while the run that produced it
/// can still read it back, which is bounded by a turn — a day is already far
/// more generous than that window, and leaves a working day of artifacts
/// available for debugging a session after the fact.
const ARTIFACT_RETENTION: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Where artifacts are written: `<workspace_dir>/artifacts/tool-results`, the
/// core's own state, never the agent's working directory.
///
/// The store used to be rooted at `action_dir` (or the per-turn workspace
/// descriptor's root) and hand the model a path relative to it. That kept the
/// pointer readable, but the action directory is very often a project the
/// agent is editing: every oversized tool output became a stray
/// `artifacts/tool-results/…` file in it, swept into `git add -A` and shipped
/// in the diff. In a coding benchmark one task's whole patch was twelve of
/// these files and a scratch test.
///
/// The store is now detached. It writes under the workspace and hands out the
/// **absolute** path, which resolves the same way whatever the turn's action
/// directory or workspace descriptor is, so the read path no longer has to be
/// matched against the write path (the #6483 trap: `FsGate::scoped_to_workspace`
/// moving relative resolution to the descriptor root). Reading it back is
/// policy: `SecurityPolicy::from_config` grants this directory as a read-only
/// trusted root, and `is_workspace_internal_path` already exempts it from the
/// internal-state boundary.
fn artifact_root(workspace_dir: &Path) -> PathBuf {
    // `ToolResultArtifactStore::detached` adds its own `tool-results` child
    // namespace so pruning stays within the store-owned directory. Pass the
    // workspace's `artifacts` directory here; the resulting store root is the
    // policy-granted `<workspace>/artifacts/tool-results` directory.
    workspace_dir.join("artifacts")
}

/// Build the store the turn path hands to `TurnContextMiddleware`, sweeping
/// other sessions' stale artifacts on the way.
///
/// The sweep is best-effort by design: a failed prune must never fail a turn,
/// because the worst it costs is disk, while failing the turn costs the user
/// their message.
pub(super) fn build_artifact_store(
    workspace_dir: &Path,
    session_key: &str,
) -> ToolResultArtifactStore {
    let store = new_tool_result_store(artifact_root(workspace_dir), session_key);
    prune(&store, "workspace");
    store
}

fn prune(store: &ToolResultArtifactStore, which: &str) {
    match store.prune_stale_sessions(ARTIFACT_RETENTION) {
        Ok(0) => {}
        Ok(removed) => log::debug!(
            "[agent][tool-result-artifacts] pruned {removed} stale {which} artifact session dir(s) under {}",
            store.root().display()
        ),
        Err(error) => log::warn!(
            "[agent][tool-result-artifacts] {which} artifact prune failed (continuing): {error}"
        ),
    }
}

#[cfg(test)]
#[path = "artifact_wiring_tests.rs"]
mod tests;
