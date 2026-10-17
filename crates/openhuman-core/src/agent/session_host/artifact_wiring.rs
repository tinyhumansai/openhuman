//! Where the session host's tool-result artifact store comes from (#6408).
//!
//! Extracted from `runtime_session.rs` rather than inlined there: the choice of
//! storage is the whole correctness question for this feature, and it needs more
//! prose than a call site should carry.
//!
//! The bodies live on the acting tenant's scoped blob store (the same wiring the
//! sub-agent artifact offload uses, [`artifact_blobs_for`]), so they are
//! tenant-scoped and survive a restart. `file_read` opens the filesystem, so
//! [`HostArtifactStore`] materialises a body into
//! `<workspace>/artifacts/tool-results` on read; that directory is a cache the
//! sweep below keeps bounded.

use std::path::Path;
use std::sync::Arc;

use crate::agent::harness::tool_result_artifacts::{new_tool_result_store, HostArtifactStore};
use crate::agent::session_store::artifact_blobs_for;
use crate::security::policy::tool_result_artifacts_dir;
use tinystoragedrivers::{MemoryStorage, Scope, StorageBackend};

/// How long a tool-result artifact survives: the blob lifetime handed to
/// drivers with blob TTL, and the age past which another session's
/// materialised copies are swept.
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

/// Build the store the turn path hands to `TurnContextMiddleware`, sweeping
/// other sessions' stale materialised copies on the way.
///
/// With no blob store for the tenant (a SaaS task with no scope, a refused
/// backend) the artifacts live in a private in-memory store for this process,
/// so a large result is still stored and readable within the run.
///
/// The sweep is best-effort by design: a failed prune must never fail a turn,
/// because the worst it costs is disk, while failing the turn costs the user
/// their message.
pub(super) fn build_artifact_store(workspace_dir: &Path, session_key: &str) -> HostArtifactStore {
    let blobs = artifact_blobs_for(workspace_dir).unwrap_or_else(|| {
        tracing::warn!(
            "[agent][tool-result-artifacts] no blob store for the acting tenant — artifacts stay in memory for this process"
        );
        let scope = Scope::new("local").expect("static scope");
        Arc::clone(
            MemoryStorage::new()
                .for_scope(&scope)
                .expect("memory scope")
                .blobs(),
        )
    });
    let root = tool_result_artifacts_dir(workspace_dir);
    prune_materialized(&root, ARTIFACT_RETENTION);
    new_tool_result_store(blobs, root, session_key).with_retention(ARTIFACT_RETENTION)
}

/// The newest modification time anywhere under `path`, itself included.
fn newest_modified(path: &Path) -> Option<std::time::SystemTime> {
    let mut newest = std::fs::symlink_metadata(path).ok()?.modified().ok()?;
    if path.is_dir() {
        for entry in std::fs::read_dir(path).ok()?.flatten() {
            if let Some(modified) = newest_modified(&entry.path()) {
                newest = newest.max(modified);
            }
        }
    }
    Some(newest)
}

/// Remove materialised session directories under `root` untouched for `ttl`.
/// Returns how many went. The blobs themselves are the store's lifecycle.
fn prune_materialized(root: &Path, ttl: std::time::Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let Some(cutoff) = std::time::SystemTime::now().checked_sub(ttl) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || path.is_symlink() {
            continue;
        }
        if newest_modified(&path).is_some_and(|newest| newest < cutoff) {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => removed += 1,
                Err(error) => tracing::warn!(
                    "[agent][tool-result-artifacts] materialised prune failed (continuing): {error}"
                ),
            }
        }
    }
    if removed > 0 {
        tracing::debug!(
            "[agent][tool-result-artifacts] pruned {removed} stale materialised session dir(s) under {}",
            root.display()
        );
    }
    removed
}

#[cfg(test)]
#[path = "artifact_wiring_tests.rs"]
mod tests;
