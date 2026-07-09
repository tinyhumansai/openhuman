//! The single orchestration wake graph (stages 4–5).
//!
//! The whole wake path is **one** `tinyagents` [`CompiledGraph`] (mirroring the
//! spec's one `StateGraph`):
//!
//! ```text
//!   normalize ─► frontend ─(instructions)─► execute ─► compress ─► world_diff ─┐
//!                   ▲                                                            │
//!                   └────────────────────────────────────────────────────────── ┘
//!                   │
//!                   └─(channel_response)─► send_dm ─► context_guard ─► done
//! ```
//!
//! One [`OrchestrationState`] flows through conditional edges. `frontend` is the
//! router (command-routing): `channel_response` present → wrap up (`send_dm`),
//! else → `execute` and back. The reasoning core (`execute`) always sets
//! `agent_reply`, so the second `frontend` pass compiles a `channel_response`; a
//! hard `max_supersteps` backstop guarantees termination (spec §5 loop
//! continuity).
//!
//! Every behaviour-bearing operation — the two-pass front end (Quick LLM), the
//! reasoning core + sub-agent spawning, 20:1 compression, the world-diff append,
//! utilization + eviction, and the DM reply — is bundled behind one injected
//! [`OrchestrationRuntime`] so the graph mechanics (routing, termination, node
//! ordering) are unit-testable with a single stub, while production wires the
//! real agents / store / memory in [`super::ops`].

pub mod build;
pub mod compress;
pub mod state;
pub mod world_diff;

pub use build::{
    build_orchestration_graph, orchestration_graph_topology, run_orchestration_graph,
    EvictionOutcome, ExecuteOutcome, OrchestrationRuntime, OrchestrationUpdate,
};
pub use state::{CompressedEntry, OrchestrationState, WorldDiff, WorldDiffEntry};

#[cfg(test)]
mod tests;
