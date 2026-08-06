//! Filesystem-offload convention for worker artifacts on long-horizon tasks
//! (#3883).
//!
//! ## The problem
//!
//! For minutes-to-hours runs, keeping compressed results *in context* still
//! accumulates summary text step after step, and it can never restore full
//! fidelity. The summarizer detour
//! ([`crate::openhuman::agent::tinyagents::payload_summarizer`]) shrinks one oversized
//! payload at a time; it does not stop the aggregate from growing.
//!
//! ## The convention
//!
//! Two directories under the agent's existing `action_dir`:
//!
//! | Directory              | Holds                                                    |
//! | ---------------------- | -------------------------------------------------------- |
//! | `action_dir/outputs/`  | Deliverables. Handed between steps **by path**.          |
//! | `action_dir/workspace/`| Scratch. Intermediate files not meant to be handed back. |
//!
//! A worker that produces a large result writes it to `outputs/` and returns
//! the path plus a short abstract. Context stays lean and the full artifact is
//! recoverable with an ordinary `file_read`.
//!
//! Two halves enforce it:
//!
//! * **Prompt** — [`render_artifact_offload_contract`] is appended to every
//!   typed sub-agent's system prompt, so workers offload on purpose.
//! * **Harness** — [`offload_oversized_result`] runs on every sub-agent outcome,
//!   so an oversized result is offloaded even when the worker inlined it anyway.
//!
//! The summarizer detour and the `tool_result_budget_bytes` truncation stay
//! exactly as they are: they are the **fallback** for anything this convention
//! does not catch, and for every failure mode here (a refused path, a full
//! disk) the caller keeps its inline payload and falls through to them.
//!
//! ## Hardening
//!
//! [`resolve_artifact_path`] is fail-closed. Absolute paths, `..` traversal, and
//! anything that escapes its convention root are refused, and when a
//! `SecurityPolicy` is available so is anything reaching the core's internal
//! `workspace_dir` — both the blanket containment check and
//! `is_workspace_internal_path`. Offload targets resolve under `action_dir`,
//! never `workspace_dir`.
//!
//! ## Logging
//!
//! Every write emits `[artifact] wrote worker artifact under action_dir`, and
//! every path a handoff carries to a parent emits `[artifact] handoff carried an
//! artifact path to the parent`, so a run journal shows both ends of a pointer.

mod contract;
mod ops;
mod paths;
mod types;

pub use contract::{
    render_artifact_offload_contract, should_render_offload_contract, ARTIFACT_OFFLOAD_HEADING,
    OFFLOAD_WRITE_TOOL,
};
pub use ops::{
    build_abstract, effective_offload_threshold, extract_artifact_paths, note_artifact_handoff,
    offload_oversized_result, render_artifact_pointer, should_offload, ArtifactOffload,
    HANDOFF_STAGE_CONSUMED, HANDOFF_STAGE_RECORDED,
};
pub use paths::{relative_to_action_dir, resolve_artifact_path, sanitize_component};
pub use types::{
    ArtifactKind, OffloadError, OffloadedArtifact, ABSTRACT_BUDGET_CHARS, ARTIFACT_POINTER_PREFIX,
    DEFAULT_OFFLOAD_THRESHOLD_BYTES, OUTPUTS_DIR, SCRATCH_DIR,
};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
