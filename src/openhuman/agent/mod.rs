//! Agent Domain — multi-agent orchestration, tool execution, and session management.
//!
//! This domain owns the core "brain" of OpenHuman. It coordinates how LLMs
//! interact with the system via tools, manages conversation history, and
//! handles autonomous behaviors like trigger triage and episodic memory indexing.
//!
//! ## Key Components
//!
//! - **[`harness::session::Agent`]**: The primary entry point for running a
//!   conversation. It manages the loop of sending prompts to a provider and
//!   executing the resulting tool calls.
//! - **[`crate::openhuman::agent_registry::agents`]**: Definitions for built-in
//!   specialized agents (Orchestrator, Code Executor, Researcher, etc.).
//! - **[`triage`]**: A high-performance pipeline for classifying and responding
//!   to external triggers (webhooks, cron jobs) using small local models.
//! - **[`dispatcher`]**: Pluggable strategies for how tool calls are formatted
//!   in prompts and parsed from responses (XML, JSON, P-Format).
//! - **[`harness::subagent_runner`]**: Logic for spawning "sub-agents" from
//!   within a parent agent's tool loop, enabling hierarchical delegation.

pub mod bus;
pub mod cost;
pub mod debug;
pub mod dispatcher;
pub mod error;
pub mod harness;
pub mod hooks;
pub mod host_runtime;
pub mod library;
pub mod memory_loader;
pub mod multimodal;
pub mod pformat;
pub mod progress;
/// Structured tracing export off the [`progress`] channel: turns the
/// real-time [`progress::AgentProgress`] stream into OpenTelemetry/
/// Langfuse-style spans (turn → iteration → tool / subagent) correlated by
/// session id with user attribution (issue #3886).
pub mod progress_tracing;
/// Prompt plumbing — types, section builders, and
/// [`SystemPromptBuilder`](prompts::SystemPromptBuilder). Moved from
/// `openhuman::context::prompt` so prompt rendering lives next to the
/// agents that consume it. `openhuman::context::prompt` is retained as
/// a thin re-export shim for now.
pub mod prompts;
mod schemas;
pub mod stop_hooks;
pub mod task_board;
pub mod task_dispatcher;
pub(crate) mod task_session;
pub mod tool_policy;
pub mod tools;
pub mod tree_loader;
pub mod triage;
/// Turn-origin task-local — explicit trust/routing label scoped by every
/// entry point that invokes the agent (web chat, channel runtime,
/// subconscious, cron, CLI). Read by the approval gate to make
/// origin-aware decisions rather than inferring trust from the absence of
/// `APPROVAL_CHAT_CONTEXT`.
pub mod turn_origin;
pub use schemas::{
    all_controller_schemas as all_agent_controller_schemas,
    all_registered_controllers as all_agent_registered_controllers,
};

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use harness::session::{Agent, AgentBuilder};
