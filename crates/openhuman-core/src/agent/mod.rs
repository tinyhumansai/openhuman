//! Agent Domain — multi-agent orchestration, tool execution, and session management.
//!
//! This domain owns the core "brain" of OpenHuman. It coordinates how LLMs
//! interact with the system via tools, manages conversation history, and
//! handles autonomous behaviors like trigger triage and episodic memory indexing.
//!
//! ## Key Components
//!
//! - **[`session_host::OpenHumanSessionHost`]**: The primary entry point for running a
//!   conversation. It manages the loop of sending prompts to a provider and
//!   executing the resulting tool calls.
//! - **[`crate::agent_registry::agents`]**: Definitions for built-in
//!   specialized agents (Orchestrator, Code Executor, Researcher, etc.).
//! - **[`triage`]**: A high-performance pipeline for classifying and responding
//!   to external triggers (webhooks, cron jobs) using small local models.
//! - **`tinytools_agent::dialect`**: Canonical strategies for formatting and
//!   parsing tool calls (XML, JSON, P-Format).
//! - **[`subagent_host`]**: OpenHuman's planner, executor and persistence
//!   adapters for sub-agents driven by the neutral TinyAgents lifecycle.
//!   within a parent agent's tool loop, enabling hierarchical delegation.
pub use crate::attachments::HostAttachment;
pub mod bus;
pub mod context;
pub(crate) mod cost;
pub mod goals;
pub mod harness;
pub mod hooks;
/// Host-registered agents the core's own drivers (cron, workflow nodes) can
/// resolve by id, with their definition, host tools and context.
pub mod host_agents;
pub mod host_overrides;
pub(crate) mod message_convert;
pub mod messages;
pub mod multimodal;
pub mod orchestration;
/// Cross-platform shell selection shared by [`crate::modules::tinybox::host_runtime::NativeRuntime`]
/// and [`crate::sandbox::ops`] so all three shell-spawning sites
/// agree on `cmd.exe` (Windows) vs `bash`/`sh` (Unix). Fixes #4705 where
/// the sandbox paths hardcoded `sh` and failed at spawn on Windows.
/// Structured tracing export off the [`progress`] channel: turns the
/// real-time [`progress::AgentProgress`] stream into OpenTelemetry/
/// Langfuse-style spans (turn → iteration → tool / subagent) correlated by
/// session id with user attribution (issue #3886).
/// Prompt plumbing — types, section builders, and
/// [`SystemPromptBuilder`](prompts::SystemPromptBuilder).
pub mod prompts;
/// Host-owned payload carried through TinyAgents run queues.
pub mod queued_turn;
pub(crate) mod registration;
/// OpenHuman's composition and policy layer around the neutral TinyAgents
/// session runtime. Generic history, transcript delta, prefix and tool
/// snapshots are owned by `tinyagents-runtime`.
pub mod session_host;
/// Shared keyed leases preventing distinct host runners from checking out the
/// same durable conversation concurrently.
pub(crate) mod session_lease;
/// Product-specific adapters around `tinyagents_orchestration::subagent`.
/// Generic lifecycle ordering and task-key coalescing live in TinyAgents;
/// definitions, prompts, tools, policy, checkpoints and progress remain here.
pub mod subagent_host;
#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
pub mod tinyagents;
pub mod tools;
/// Explicit per-turn host environment handed to the session an entry point runs.
pub mod turn_env;
/// Trusted host execution shared by web, native channels and other inbound
/// surfaces. Presentation and delivery remain adapters at each surface.
pub(crate) mod turn_host;
/// Shared idle and backstop policy for top-level agent turns.
pub(crate) mod turn_policy;
/// Canonical mechanical projection from agent progress to channel output
/// events, shared by web and native channel adapters.
pub(crate) mod turn_progress;
#[allow(unused_imports)]
pub use session_host::{
    CompactOptions, CompactOutcome, HostTools, HostTurnTools, OpenHumanSessionHost,
    SessionHostBuilder, TurnContext, TurnOverrides,
};
