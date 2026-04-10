//! Core agent implementation for the OpenHuman platform.
//!
//! This module provides the [`Agent`] struct, which orchestrates the
//! interaction between the AI provider, available tools, memory
//! systems, and the user. It handles the agent's "turn" logic,
//! including tool execution and history management.
//!
//! # File layout
//!
//! This module used to be a single 2000-line `agent.rs` file. It's now
//! split into focused children so each file has a clear role:
//!
//! | File          | Role                                                             |
//! |---------------|------------------------------------------------------------------|
//! | [`types`]     | `Agent` and `AgentBuilder` struct definitions (no logic).        |
//! | [`builder`]   | `AgentBuilder` fluent API + `Agent::from_config` factory.        |
//! | [`turn`]      | The `turn()` lifecycle, tool dispatch, context-pipeline wiring. |
//! | [`runtime`]   | Public accessors, `run_single` / `run_interactive`, helpers.    |
//! | `tests`       | Integration tests (private).                                    |
//!
//! External callers should import [`Agent`] and [`AgentBuilder`] from
//! this module (or from `crate::openhuman::agent`, which re-exports
//! them). The child files are an implementation detail.

mod builder;
mod runtime;
mod turn;
mod types;

#[cfg(test)]
mod tests;

pub use types::{Agent, AgentBuilder};

use crate::openhuman::config::Config;
use anyhow::Result;

/// Convenience entry point to run an agent with the given configuration and message.
pub async fn run(
    config: Config,
    message: Option<String>,
    model_override: Option<String>,
    temperature: f64,
) -> Result<()> {
    let mut effective_config = config;
    if let Some(m) = model_override {
        effective_config.default_model = Some(m);
    }
    effective_config.default_temperature = temperature;

    let mut agent = Agent::from_config(&effective_config)?;

    if let Some(msg) = message {
        let response = agent.run_single(&msg).await?;
        println!("{response}");
    } else {
        agent.run_interactive().await?;
    }

    Ok(())
}
