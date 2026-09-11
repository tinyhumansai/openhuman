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
//! * **[`contract`] — the prompt half.** It is OpenHuman prompt text naming
//!   OpenHuman tools, which `plan-agents.md` §6 lists as not publishable. The
//!   crate's [`render_artifact_pointer`] takes the read-tool name as a
//!   parameter for the same reason, and [`READ_TOOL`] is what OpenHuman passes.
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
//! * **Prompt** — [`render_artifact_offload_contract`] is appended to every
//!   typed sub-agent's system prompt, so workers offload on purpose.
//! * **Harness** — [`offload_oversized_result`] runs on every sub-agent
//!   outcome, so an oversized result is offloaded even when the worker inlined
//!   it anyway.
//!
//! The summarizer detour
//! ([`crate::openhuman::agent::tinyagents::payload_summarizer`]) and the
//! `tool_result_budget_bytes` truncation stay exactly as they are: they are the
//! **fallback** for anything this convention does not catch, and for every
//! failure mode here — a refused path, a full disk — the caller keeps its
//! inline payload and falls through to them.

mod contract;
pub mod policy;

use std::path::PathBuf;
use std::sync::Arc;

use tinyagents_harness::artifacts::ArtifactOffload;

use crate::openhuman::security::SecurityPolicy;

pub use contract::{
    render_artifact_offload_contract, should_render_offload_contract, ARTIFACT_OFFLOAD_HEADING,
    OFFLOAD_WRITE_TOOL,
};
pub use policy::{SanitizingRedactor, WorkspaceGuard};
pub use tinyagents_harness::artifacts::{
    build_abstract, effective_offload_threshold, extract_artifact_paths, note_artifact_handoff,
    relative_to_root as relative_to_action_dir, render_artifact_pointer, resolve_artifact_path,
    sanitize_component, should_offload, ArtifactKind, ArtifactOffload as Offload, OffloadError,
    OffloadedArtifact, ABSTRACT_BUDGET_CHARS, ARTIFACT_POINTER_PREFIX,
    DEFAULT_OFFLOAD_THRESHOLD_BYTES, HANDOFF_STAGE_CONSUMED, HANDOFF_STAGE_RECORDED, OUTPUTS_DIR,
    SCRATCH_DIR,
};

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

/// Build a writer for one sub-agent run, wired to OpenHuman's policies.
///
/// Supplies both host policies every time, so no call site can accidentally
/// construct an unguarded or unredacted writer — the crate permits `None` for
/// each, and OpenHuman never wants either.
pub fn new_artifact_offload(
    action_dir: PathBuf,
    policy: Option<Arc<SecurityPolicy>>,
    agent_id: impl Into<String>,
    task_id: impl Into<String>,
) -> ArtifactOffload {
    let offload = ArtifactOffload::new(action_dir, agent_id, task_id)
        .with_redactor(Arc::new(SanitizingRedactor));
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
/// Thin wrapper over the crate's function that pins OpenHuman's [`READ_TOOL`].
/// Every failure mode is soft: the caller gets the original payload back and
/// the summarizer detour and tool-result budget stay in charge as the fallback.
pub async fn offload_oversized_result(
    output: String,
    offload: &ArtifactOffload,
    threshold_bytes: usize,
) -> (String, Option<OffloadedArtifact>) {
    tinyagents_harness::artifacts::offload_oversized_result(
        output,
        offload,
        threshold_bytes,
        READ_TOOL,
    )
    .await
}

#[cfg(test)]
#[path = "artifact_offload_tests.rs"]
mod tests;
