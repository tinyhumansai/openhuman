//! The **sub-agent turn graph** (issue #4249).
//!
//! Per the per-folder `graph.rs` convention, this module owns the sub-agent
//! folder's graph definition, its available tools, and its summarization step —
//! all thin over the shared tinyagents seam
//! ([`run_turn_via_tinyagents_shared`]).
//!
//! **Graph.** A single agent-loop turn driven by the tinyagents harness: the
//! model is called, requested tools run, and the loop repeats until the model
//! returns without further tool calls or the iteration budget is exhausted. The
//! canonical sub-agent turn path (the legacy `run_inner_loop` / `run_turn_engine`
//! are removed); `run_typed_mode` calls it unconditionally.
//!
//! **Available tools.** The sub-agent reuses the parent's harness tools plus the
//! per-spawn dynamic tools, advertised via the canonical shared-tool adapter over the shared
//! `Arc<Vec<Box<dyn Tool>>>` tool sets (`[dynamic_tools, parent_tools]` — dynamic
//! first so a shadowing dynamic tool executes, matching advertisement), filtered
//! by `allowed_names`. `ask_user_clarification` is the early-exit tool.
//!
//! **Summarization.** When the sub-agent model's effective context window is
//! known, the shared seam installs the context-window summarization step
//! (`tinyagents::summarize`) ahead of the deterministic front-trim — see
//! [`run_subagent_via_graph`], which resolves the window before dispatch.
//!
//! It mirrors the original seams: child progress deltas (`Subagent*` events incl.
//! thinking), mid-flight steering, the `ask_user_clarification` early-exit pause,
//! and a graceful model-call-cap checkpoint summary
//! (`SubagentCheckpoint::summarize_cap_hit`).

// Split by responsibility rather than by line count:
//
// - [`dispatch`] — drives one sub-agent turn through the shared tinyagents
//   seam, builds its context middleware, and folds a cap-hit checkpoint
//   summary back into the result.
// - [`transcript`] — persists a sub-agent turn's (or a failed run's) raw
//   transcript to `session_raw`.
// - [`worker_mirror`] — mirrors a sub-agent turn's conversation onto its
//   spawn's worker thread.
mod dispatch;
mod transcript;
mod worker_mirror;

// Re-exported under the original flat `graph::` path so external callers
// (`ops/mod.rs`, `ops/runner.rs`, `graph_tests.rs`'s `use super::*`) are
// unaffected by the responsibility split.
#[cfg(test)]
pub(super) use dispatch::inherited_thread_id;
pub(crate) use dispatch::run_agent_turn_request_via_default_graph;
pub(super) use dispatch::{AggregatedUsage, run_subagent_via_graph};

// Only what `graph_tests.rs`'s `use super::*` still needs directly (the rest
// of the original flat imports now live with the code that uses them, in
// `dispatch.rs` / `transcript.rs` / `worker_mirror.rs`).
#[cfg(test)]
use crate::agent::messages::{ChatMessage, ConversationMessage};
#[cfg(test)]
use crate::agent::progress::AgentProgress;
#[cfg(test)]
use crate::inference::tokenjuice::AgentTokenjuiceCompression;
#[cfg(test)]
use std::collections::HashSet;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use tinytools::Tool;
#[cfg(test)]
use worker_mirror::{mirror_worker_thread, mirror_worker_thread_from_history};

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
