//! Orchestrator / multi-agent harness configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for the multi-agent orchestrator harness.
///
/// When `enabled` is false (default), the system behaves as a single-agent loop
/// using the existing `Agent` + tool-call path. Backward compatible.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrchestratorConfig {
    /// Enable multi-agent orchestrator mode.
    #[serde(default)]
    pub enabled: bool,

    /// Per-archetype configuration overrides.
    /// Keys are archetype names (e.g. "code_executor", "researcher").
    #[serde(default)]
    pub archetypes: HashMap<String, ArchetypeConfig>,

    /// Maximum concurrent sub-agents across all sessions.
    #[serde(default = "default_max_concurrent_agents")]
    pub max_concurrent_agents: usize,

    /// Enable the Archivist background daemon (post-session nudge loop).
    #[serde(default = "default_true")]
    pub archivist_enabled: bool,

    /// Enable FTS5 episodic recall tables in SQLite memory.
    #[serde(default = "default_true")]
    pub fts5_enabled: bool,

    /// Enable self-healing (ToolMaker auto-polyfill on "command not found").
    #[serde(default = "default_true")]
    pub self_healing_enabled: bool,

    /// Maximum number of task nodes in a single DAG plan.
    #[serde(default = "default_max_dag_tasks")]
    pub max_dag_tasks: usize,

    /// Maximum retry attempts for a failed DAG task node.
    #[serde(default = "default_max_retries")]
    pub max_task_retries: u8,
}

/// Per-archetype configuration override.
///
/// Any field left `None` uses the archetype's built-in default.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArchetypeConfig {
    /// Model name or hint override (e.g. "coding-v1", "local:phi3").
    #[serde(default)]
    pub model: Option<String>,

    /// System prompt override (inline or path).
    #[serde(default)]
    pub system_prompt: Option<String>,

    /// Temperature override.
    #[serde(default)]
    pub temperature: Option<f64>,

    /// Maximum tool iterations override.
    #[serde(default)]
    pub max_tool_iterations: Option<usize>,

    /// Timeout in seconds for this archetype's sub-agent runs.
    #[serde(default)]
    pub timeout_secs: Option<u64>,

    /// Sandbox mode override: "sandboxed", "read_only", or "none".
    #[serde(default)]
    pub sandbox: Option<String>,
}

fn default_max_concurrent_agents() -> usize {
    4
}

fn default_true() -> bool {
    true
}

fn default_max_dag_tasks() -> usize {
    8
}

fn default_max_retries() -> u8 {
    2
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            archetypes: HashMap::new(),
            max_concurrent_agents: default_max_concurrent_agents(),
            archivist_enabled: default_true(),
            fts5_enabled: default_true(),
            self_healing_enabled: default_true(),
            max_dag_tasks: default_max_dag_tasks(),
            max_task_retries: default_max_retries(),
        }
    }
}

impl Default for ArchetypeConfig {
    fn default() -> Self {
        Self {
            model: None,
            system_prompt: None,
            temperature: None,
            max_tool_iterations: None,
            timeout_secs: None,
            sandbox: None,
        }
    }
}
