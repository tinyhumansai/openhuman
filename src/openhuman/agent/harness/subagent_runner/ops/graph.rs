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
//! per-spawn dynamic tools, advertised via [`SharedToolAdapter`] over the shared
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

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
include!("graph_part_01.rs");
include!("graph_part_02.rs");
