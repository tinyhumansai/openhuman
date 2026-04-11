//! Orchestrator / multi-agent harness configuration.
//!
//! The fields here gate the orthogonal sub-agent features that run
//! alongside the main agent's tool loop. There is no "DAG planner"
//! flow any more — delegation is always done through the
//! `spawn_subagent` tool, which hands off to an
//! [`crate::openhuman::agent::harness::definition::AgentDefinition`]
//! looked up in the global registry.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for the multi-agent orchestrator harness.
///
/// None of these fields enable or disable the multi-agent harness as a
/// whole — sub-agent delegation through `spawn_subagent` is always
/// available to the main agent. They only toggle the orthogonal
/// features listed below.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrchestratorConfig {
    /// Enable the Archivist background daemon (post-session nudge loop).
    #[serde(default = "default_true")]
    pub archivist_enabled: bool,

    /// Enable FTS5 episodic recall tables in SQLite memory.
    #[serde(default = "default_true")]
    pub fts5_enabled: bool,

    /// Enable self-healing (ToolMaker auto-polyfill on "command not found").
    #[serde(default = "default_true")]
    pub self_healing_enabled: bool,

    /// Allow `spawn_subagent { mode: "fork", … }` calls. Fork mode replays
    /// the parent's exact rendered prompt + tool schemas + message prefix
    /// so the inference backend's automatic prefix caching kicks in.
    /// Defaults to true; flip to false to force every sub-agent into
    /// typed mode (e.g. on backends that don't benefit from prefix
    /// caching, or while debugging).
    #[serde(default = "default_true")]
    pub fork_mode_enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            archivist_enabled: default_true(),
            fts5_enabled: default_true(),
            self_healing_enabled: default_true(),
            fork_mode_enabled: default_true(),
        }
    }
}
