//! Compat shim — prompt plumbing has moved to [`crate::openhuman::agent::prompts`].
//!
//! This file used to hold the full prompt rendering pipeline (type
//! definitions, section builders, `SystemPromptBuilder`,
//! `render_subagent_system_prompt`). All of that now lives under
//! `agent::prompts` so prompt logic sits next to the agents that
//! consume it. This module stays around as a stable import path for
//! the rest of the tree — `use crate::openhuman::agent::context::prompt::...`
//! keeps working unchanged.

pub use crate::openhuman::agent::prompts::*;
