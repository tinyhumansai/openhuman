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
//! - **[`agents`]**: Definitions for built-in specialized agents (Orchestrator,
//!   Code Executor, Researcher, etc.).
//! - **[`triage`]**: A high-performance pipeline for classifying and responding
//!   to external triggers (webhooks, cron jobs) using small local models.
//! - **[`dispatcher`]**: Pluggable strategies for how tool calls are formatted
//!   in prompts and parsed from responses (XML, JSON, P-Format).
//! - **[`harness::subagent_runner`]**: Logic for spawning "sub-agents" from
//!   within a parent agent's tool loop, enabling hierarchical delegation.

pub mod agents;
pub mod bus;
pub mod dispatcher;
pub mod error;
pub mod harness;
pub mod hooks;
pub mod host_runtime;
pub mod memory_loader;
pub mod multimodal;
pub mod pformat;
pub mod progress;
mod schemas;
pub mod triage;
pub mod welcome_proactive;
pub use schemas::{
    all_controller_schemas as all_agent_controller_schemas,
    all_registered_controllers as all_agent_registered_controllers,
};

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use harness::session::{Agent, AgentBuilder};
