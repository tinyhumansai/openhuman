//! `tinyagents` integration — drive an openhuman agent turn on the published
//! [`tinyagents`](https://crates.io/crates/tinyagents) orchestration framework
//! (issue #4249).
//!
//! openhuman's agent execution runs on the `tinyagents` crate
//! (LangGraph/LangChain-style durable graphs + an agent-loop harness with model/
//! tool registries, middleware, retry/fallback, and limits). This module is the
//! **adapter seam**: it bridges openhuman's `Provider`, `Tool`, and `ChatMessage`
//! types onto the crate's `ChatModel`, `Tool`, and `Message` traits, then drives
//! a turn through [`AgentHarness::invoke`]. The chat / channel / sub-agent
//! routes call [`run_turn_via_tinyagents_shared`] (default ON in production).
//!
//! The chat route is at functional parity with the legacy `run_turn_engine`:
//! the [`OpenhumanEventBridge`] mirrors the harness event stream onto
//! `AgentProgress` (live tool timeline, incremental text deltas, cost footer),
//! [`native model streaming`] forwards true token streaming, multimodal markers
//! are expanded, and history is trimmed to the context window. Mid-flight
//! steering, sub-agent child-progress deltas (incl. thinking), and the
//! `ask_user_clarification` early-exit pause are all re-wired onto the
//! tinyagents harness.

pub(crate) mod abort_guard;
pub mod config;
pub(crate) mod convert;
pub(crate) mod delegation;
mod embeddings;
mod harness_assembly;
mod harness_context_ladder;
mod harness_tool_registration;
pub mod host;
pub(crate) mod journal;
pub(crate) mod middleware;
pub(crate) mod model;
pub(crate) mod model_helpers;
pub(crate) mod observability;
pub(crate) mod orchestration;
// `pub` since issue #6014, and the inconsistency it removes is the point:
// `AgentBuilder::payload_summarizer` is a **public** setter taking
// `Arc<dyn PayloadSummarizer>`, so the seam was already advertised to embedders
// — while the trait itself, its outcome type and its reason enum were all
// `pub(crate)`, which made the setter uncallable from outside this crate. An
// embedder could therefore see the extension point, and could not use it.
//
// The default implementation dispatches a sub-agent, which is exactly what an
// embedder may be unable to do (OpenCompany withholds spawn tools under
// multi-tenancy), so "bring your own summarizer" is the case this seam exists
// for rather than an exotic one.
pub mod payload_summarizer;
mod policy_denial;
pub(crate) mod reaper;
pub(crate) mod replay;
pub mod resolved_route;
pub(crate) mod retriever;
mod routes;
pub(crate) mod run_cancellation_context;
mod steering_forwarder;
pub(crate) mod stop_hooks;
mod summarize;
pub mod thread_context;
pub mod todos;
pub(crate) mod tools;
mod topology;
mod turn_models;
mod turn_outcome;
mod turn_policy;
mod turn_run_error;
mod turn_run_finalize;
mod turn_runner;

pub(crate) use crate::agent::message_convert::chat_message_to_message;
#[cfg(feature = "flows")]
pub(crate) use crate::agent::message_convert::{reasoning_from_content, ta_call_to_oh_call};

#[allow(unused_imports)] // Wired into the recall/retrieval facade in workstream 09.2.
pub(crate) use embeddings::ProviderEmbeddingModel;
pub(crate) use middleware::{
    render_unanswered_steps, HandoffConfig, TranscriptSnapshot, TranscriptSnapshotSink,
    TurnContextMiddleware,
};
pub(crate) use observability::SubagentScope;
pub use resolved_route::{
    current_resolved_provider_route, current_route_slot, record_resolved_provider_route,
    with_resolved_provider_route_scope, with_route_slot, ResolvedProviderRoute, RouteSlot,
};
pub(crate) use run_cancellation_context::current_run_cancellation;
pub(crate) use topology::all_graph_topologies;
pub use turn_models::TurnModelSource;
pub(crate) use turn_models::TurnModels;
pub(crate) use turn_outcome::{
    HaltSummarySlot, TinyagentsTurnOutcome, ToolCallOutcome, ToolOutcomeSink,
};
pub(crate) use turn_policy::{agent_turn_wall_clock_ms, ToolPolicyEnforcement};
pub(crate) use turn_runner::run_turn_via_tinyagents_shared;

// Test-only glue so `tinyagents_tests.rs`'s `use super::*;` sees the
// lower-level policy helpers it exercises directly (they otherwise stay
// private to `turn_policy`, which is the correct production visibility).
#[cfg(test)]
use tinyagents_harness::runtime::InvalidArgsPolicy;
#[cfg(test)]
use turn_policy::{
    model_call_wall_clock_ms, parse_agent_turn_wall_clock_ms, parse_model_call_wall_clock_ms,
    run_policy_for, DEFAULT_AGENT_TURN_TIMEOUT_SECS, DEFAULT_MODEL_CALL_TIMEOUT_SECS,
};

#[cfg(test)]
#[path = "tinyagents_tests.rs"]
mod tests;
