//! Filesystem-offload convention for worker artifacts on long-horizon tasks
//! (#3883).
//!
//! **The mechanics now live in
//! [`tinyagents_harness::artifacts`]** — thresholds, path resolution, pointer
//! rendering, the symlink re-check, and the writer itself. This module is what
//! `plan-agents.md` Phase 5 leaves behind: the wiring, plus the two halves that
//! are genuinely OpenHuman's.
//!
//! ## What stayed, and why
//!
//! * **The tool names.** The prompt contract now lives in the crate too
//!   (`render_artifact_offload_contract`); the write-tool name it may mention,
//!   [`OFFLOAD_WRITE_TOOL`], and the read-tool name for
//!   [`tinyagents_harness::artifacts::render_artifact_pointer`], [`READ_TOOL`], are host vocabulary and are
//!   passed in.
//! * **[`policy`] — the two host policies.** `SecurityPolicy`'s workspace
//!   containment and `sanitize_text`'s credential scrubbing. The crate asks;
//!   these answer.
//!
//! ## The convention
//!
//! Two directories under the agent's existing `action_dir`:
//!
//! | Directory               | Holds                                                    |
//! | ----------------------- | -------------------------------------------------------- |
//! | `action_dir/outputs/`   | Deliverables. Handed between steps **by path**.          |
//! | `action_dir/workspace/` | Scratch. Intermediate files not meant to be handed back. |
//!
//! A worker that produces a large result writes it to `outputs/` and returns
//! the path plus a short abstract. Context stays lean and the full artifact is
//! recoverable with an ordinary `file_read`.
//!
//! Two halves enforce it:
//!
//! * **Prompt** — `render_artifact_offload_contract` is appended to every
//!   typed sub-agent's system prompt, so workers offload on purpose.
//! * **Harness** — [`offload_oversized_result`] runs on every sub-agent
//!   outcome, so an oversized result is offloaded even when the worker inlined
//!   it anyway.
//!
//! The summarizer detour
//! ([`crate::agent::tinyagents::payload_summarizer`]) and the
//! `tool_result_budget_bytes` truncation stay exactly as they are: they are the
//! **fallback** for anything this convention does not catch, and for every
//! failure mode here — a refused path, a full disk — the caller keeps its
//! inline payload and falls through to them.

pub mod policy;

use std::path::Path;
use std::sync::Arc;

use tinyagents_harness::artifacts::{ArtifactKind, ArtifactOffload};
use tinystoragedrivers::BlobStore;

use crate::security::SecurityPolicy;

/// Tool a sub-agent must actually hold before the offload contract is worth
/// rendering. Passed to the crate's
/// [`render_artifact_offload_contract`](tinyagents_harness::artifacts::render_artifact_offload_contract)
/// and `should_render_offload_contract`: a prompt may only name tools its agent
/// can really call, and every skill-filtered specialist has no filesystem tools
/// (see `subagent_host::ops_tests::typed_mode_filters_tools_by_skill_filter`).
pub const OFFLOAD_WRITE_TOOL: &str = "file_write";

pub use policy::{SanitizingRedactor, WorkspaceGuard};
use tinyagents_harness::artifacts::OffloadedArtifact;

/// Tool OpenHuman tells a parent to open an offloaded artifact with.
///
/// Passed into the crate's pointer renderer rather than baked into it: tool
/// names are host vocabulary, and a crate that hard-coded one would name a tool
/// some other host does not have.
///
/// Distinct from [`OFFLOAD_WRITE_TOOL`], which is the tool the *worker* needs in
/// order to be told about the convention at all. A prompt may only name tools
/// its agent actually holds, and the reader and the writer are different agents.
pub const READ_TOOL: &str = "file_read";

/// Build a writer for one sub-agent run over `blobs`, wired to OpenHuman's
/// policies.
///
/// Supplies both host policies every time, so no call site can accidentally
/// construct an unguarded or unredacted writer — the crate permits `None` for
/// each, and OpenHuman never wants either.
pub fn new_artifact_offload(
    blobs: Arc<dyn BlobStore>,
    policy: Option<Arc<SecurityPolicy>>,
    agent_id: impl Into<String>,
    task_id: impl Into<String>,
) -> ArtifactOffload {
    let offload =
        ArtifactOffload::new(blobs, agent_id, task_id).with_redactor(Arc::new(SanitizingRedactor));
    match policy {
        Some(policy) => offload.with_path_policy(Arc::new(WorkspaceGuard::new(policy))),
        // No policy means the workspace checks are skipped; traversal and
        // containment still apply. Matches the previous `Option<&SecurityPolicy>`
        // behaviour rather than silently hardening a path that used to be open.
        None => offload,
    }
}

/// Offload `output` when it exceeds `threshold_bytes`, returning the text the
/// parent should receive plus the artifact when one was written.
///
/// The artifact lives on the blob store, but the parent opens it with
/// [`READ_TOOL`], which reads the filesystem. So the stored body is also
/// materialised at `materialize_root/<key>` (the parent's action root), which
/// is the path the pointer quotes. Every failure mode is soft: the caller gets
/// the original payload back and the summarizer detour and tool-result budget
/// stay in charge as the fallback. A pointer is never returned for a file that
/// could not be written.
pub async fn offload_oversized_result(
    output: String,
    offload: &ArtifactOffload,
    threshold_bytes: usize,
    materialize_root: &Path,
    policy: Option<&SecurityPolicy>,
) -> (String, Option<OffloadedArtifact>) {
    // The blob key is relative, so the crate's path policy cannot see where the
    // reader's copy will land. Refuse here, before anything is stored, a target
    // that would reach the core's internal state.
    if let Some(policy) = policy {
        if let Ok(key) = offload.resolve(ArtifactKind::Output, &offload.default_result_name()) {
            let target = materialize_root.join(&key);
            if policy.is_workspace_internal_path(&target)
                || target.starts_with(&policy.workspace_dir)
            {
                tracing::warn!(
                    path = %key,
                    "[artifact] materialized target reaches workspace state; keeping the inline result"
                );
                return (output, None);
            }
        }
    }
    let (pointer, artifact) = tinyagents_harness::artifacts::offload_oversized_result(
        output.clone(),
        offload,
        threshold_bytes,
        READ_TOOL,
    )
    .await;
    let Some(artifact) = artifact else {
        return (output, None);
    };
    match materialize(offload, &artifact, materialize_root).await {
        Ok(()) => (pointer, Some(artifact)),
        Err(error) => {
            tracing::warn!(
                path = %artifact.relative_path,
                error = %error,
                "[artifact] could not materialize the stored artifact; keeping the inline result"
            );
            (output, None)
        }
    }
}

/// Offload one finished sub-agent run's `output` for the tenant in scope: the
/// artifact goes to its blob store and is materialised under the policy's
/// action root (`action_dir` without a policy), the parent's root.
///
/// With no blob store for the tenant (a SaaS task with no scope, a refused
/// backend) nothing is offloaded and `output` comes back untouched, like every
/// other soft failure.
pub async fn offload_run_result(
    output: String,
    policy: Option<Arc<SecurityPolicy>>,
    action_dir: std::path::PathBuf,
    agent_id: String,
    task_id: &str,
    threshold_bytes: usize,
) -> (String, Option<OffloadedArtifact>) {
    let materialize_root = policy
        .as_ref()
        .map(|p| p.action_dir.clone())
        .unwrap_or_else(|| action_dir.clone());
    let workspace_dir = policy
        .as_ref()
        .map(|p| p.workspace_dir.clone())
        .unwrap_or(action_dir);
    let Some(blobs) = crate::agent::session_store::artifact_blobs_for(&workspace_dir) else {
        tracing::debug!(
            task_id = %task_id,
            agent_id = %agent_id,
            "[artifact] no blob store for the acting tenant — skipping offload (summarizer/truncation backstop applies)"
        );
        return (output, None);
    };
    let offload = new_artifact_offload(blobs, policy.clone(), agent_id, task_id);
    offload_oversized_result(
        output,
        &offload,
        threshold_bytes,
        &materialize_root,
        policy.as_deref(),
    )
    .await
}

/// Write the stored (redacted) body of `artifact` to `root/<key>`.
async fn materialize(
    offload: &ArtifactOffload,
    artifact: &OffloadedArtifact,
    root: &Path,
) -> Result<(), String> {
    let body = offload
        .read(&artifact.relative_path)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the artifact vanished from the blob store".to_string())?;
    // `read` re-ran the fail-closed key checks (no absolute names, `..`, or
    // control characters), so the join cannot leave `root`.
    let target = root.join(&artifact.relative_path);
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| error.to_string())?;
    }
    tokio::fs::write(&target, body)
        .await
        .map_err(|error| error.to_string())?;
    tracing::debug!(path = %artifact.relative_path, "[artifact] materialized for file_read");
    Ok(())
}

#[cfg(test)]
#[path = "artifact_offload_tests.rs"]
mod tests;
