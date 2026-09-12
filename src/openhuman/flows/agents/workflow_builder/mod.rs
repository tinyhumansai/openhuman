//! The `workflow_builder` built-in agent: this host's wiring of the tinyflows
//! authoring copilot.
//!
//! The copilot itself — the standing archetype that teaches the graph DSL and
//! the propose-only contract, and the turn brief that opens an authoring turn —
//! lives in `tinyflows-copilot` and names no harness. What is here is the
//! wiring: [`prompt`] assembles that archetype with this host's runtime
//! sections (user files, the agent's tool list, the workspace footer), and
//! `agent.toml` registers the agent with this host's registry.
//!
//! [`builder_prompt`] is the historical path the brief was reached by; it is a
//! re-export now, so `agents::workflow_builder::builder_prompt::BuilderRequest`
//! keeps resolving.

pub mod prompt;

#[cfg(test)]
#[path = "tool_wording_tests.rs"]
mod tool_wording_tests;

/// The harness-independent turn brief, from `tinyflows-copilot`.
pub use tinyflows_copilot::builder as builder_prompt;
