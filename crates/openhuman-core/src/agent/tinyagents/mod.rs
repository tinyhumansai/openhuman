//! `tinyagents` integration — drive an openhuman agent turn on the published
//! [`tinyagents`](https://crates.io/crates/tinyagents) orchestration framework
//! (issue #4249).
//!
//! openhuman's agent execution runs on the `tinyagents` crate
//! (LangGraph/LangChain-style durable graphs + an agent-loop harness with model/
//! tool registries, middleware, retry/fallback, and limits). This module is the
//! **adapter seam**: it bridges openhuman's `Provider`, `Tool`, and `TranscriptMessage`
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

pub mod config;
pub(crate) mod context_wiring;
mod deadline_wind_down;
pub mod discovery;
pub mod host;
pub(crate) mod hosted_error;
pub(crate) mod journal;
// The tool harness behind a live voice session (`voice::live`).
#[cfg(feature = "voice")]
pub(crate) mod live_harness;
mod memory_summarizer;
pub(crate) mod middleware;
pub(crate) mod observability;
// `pub` since issue #6014, and the inconsistency it removes is the point:
// `AgentBuilder::payload_summarizer` is a **public** setter taking
// `Arc<DynPayloadSummarizer>`, so the seam was already advertised to embedders
// — while the trait itself, its outcome type and its reason enum were all
// `pub(crate)`, which made the setter uncallable from outside this crate. An
// embedder could therefore see the extension point, and could not use it.
//
// The default implementation dispatches a sub-agent, which is exactly what an
// embedder may be unable to do (OpenCompany withholds spawn tools under
// multi-tenancy), so "bring your own summarizer" is the case this seam exists
// for rather than an exotic one.
pub mod budget;
pub mod payload_summarizer;
mod policy_denial;
pub(crate) mod reaper;
pub(crate) mod state_block;
mod steering_forwarder;
mod supplemental_trace;
pub(crate) mod thread_reasoning;
pub(crate) mod tool_registration;
mod turn_exec;
mod turn_exec_native;
mod turn_middleware;
mod turn_outcome;
mod turn_plan;
pub(crate) mod turn_policy;
mod turn_run_error;
mod turn_run_finalize;
mod verify_before_finish;

pub(crate) use crate::agent::message_convert::chat_message_to_message;
#[cfg(feature = "flows")]
pub(crate) use crate::agent::message_convert::ta_call_to_oh_call;

pub use crate::inference::turn_models::TurnModelSource;
pub(crate) use observability::SubagentScope;
pub(crate) use supplemental_trace::SupplementalTrace;
pub(crate) use thread_reasoning::apply_requested_effort;
pub(crate) use turn_exec::TurnSpec;
pub(crate) use turn_exec_native::NativeSubagentRunContext;
#[cfg(test)]
pub(crate) use turn_outcome::DeferredTurnTerminal;
pub(crate) use turn_outcome::TinyagentsTurnOutcome;
#[cfg(test)]
pub(crate) use turn_policy::is_subagent_spawn_tool;
pub(crate) use turn_policy::{
    agent_turn_wall_clock_ms_for, chat_provider_is_local, local_web_turn_backstop_secs,
    ToolPolicyEnforcement,
};

// Test-only glue so `tinyagents_tests.rs`'s `use super::*;` sees the
// lower-level policy helpers it exercises directly (they otherwise stay
// private to `turn_policy`, which is the correct production visibility).
#[cfg(test)]
use tinyagents_harness::runtime::InvalidArgsPolicy;
#[cfg(test)]
use turn_policy::run_policy_for;

#[cfg(test)]
#[path = "nested_calls_tests.rs"]
mod nested_calls_tests;
#[cfg(test)]
#[path = "prompt_cache_golden_tests.rs"]
mod prompt_cache_golden_tests;
#[cfg(test)]
#[path = "tinyagents_tests.rs"]
mod tests;
