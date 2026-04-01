//! Agent and delegate agent configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a delegate sub-agent used by the `delegate` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DelegateAgentConfig {
    /// Model name (inference uses the OpenHuman backend from main config).
    pub model: String,
    /// Optional system prompt for the sub-agent
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Optional API key override
    #[serde(default)]
    pub api_key: Option<String>,
    /// Temperature override
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Max recursion depth for nested delegation
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
}

fn default_max_depth() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentConfig {
    /// When true: bootstrap_max_chars=6000, rag_chunk_limit=2. Use for 13B or smaller models.
    #[serde(default)]
    pub compact_context: bool,
    #[serde(default = "default_agent_max_tool_iterations")]
    pub max_tool_iterations: usize,
    #[serde(default = "default_agent_max_history_messages")]
    pub max_history_messages: usize,
    #[serde(default)]
    pub parallel_tools: bool,
    /// Maximum number of tool calls to execute concurrently when `parallel_tools` is true.
    #[serde(default = "default_max_parallel_tools")]
    pub max_parallel_tools: usize,
    #[serde(default = "default_agent_tool_dispatcher")]
    pub tool_dispatcher: String,
    /// Maximum characters of memory context to inject per turn.
    /// Higher values provide richer context but consume more of the context window.
    #[serde(default = "default_max_memory_context_chars")]
    pub max_memory_context_chars: usize,
    /// Per-channel maximum permission level for tool execution.
    /// Keys are channel names (e.g., "telegram", "discord", "web", "cli").
    /// Values are permission levels: "none", "readonly", "write", "execute", "dangerous".
    /// Channels not listed default to "readonly".
    #[serde(default)]
    pub channel_permissions: std::collections::HashMap<String, String>,
}

fn default_agent_max_tool_iterations() -> usize {
    10
}

fn default_agent_max_history_messages() -> usize {
    50
}

fn default_max_parallel_tools() -> usize {
    4
}

fn default_agent_tool_dispatcher() -> String {
    "auto".into()
}

fn default_max_memory_context_chars() -> usize {
    2000
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            compact_context: false,
            max_tool_iterations: default_agent_max_tool_iterations(),
            max_history_messages: default_agent_max_history_messages(),
            parallel_tools: false,
            max_parallel_tools: default_max_parallel_tools(),
            tool_dispatcher: default_agent_tool_dispatcher(),
            max_memory_context_chars: default_max_memory_context_chars(),
            channel_permissions: std::collections::HashMap::new(),
        }
    }
}
