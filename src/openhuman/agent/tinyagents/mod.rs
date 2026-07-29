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
pub mod host;
pub(crate) mod journal;
pub(crate) mod middleware;
pub(crate) mod model;
pub(crate) mod observability;
pub(crate) mod orchestration;
pub(crate) mod payload_summarizer;
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

#[cfg(test)]
#[path = "tinyagents_tests.rs"]
mod tests;
include!("mod_part_01.rs");
include!("mod_part_02.rs");
include!("mod_part_03.rs");
include!("mod_part_04.rs");
include!("mod_part_05.rs");
