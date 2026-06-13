//! Configuration for the Claude Agent SDK subprocess provider.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaudeAgentSdkConfig {
    /// Whether the Claude Agent SDK provider is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Path to the `claude` CLI binary. Defaults to `"claude"` (PATH lookup).
    #[serde(default = "default_claude_binary")]
    pub binary: String,

    /// Default model passed via `--model` when the provider string is just
    /// `"claude_agent_sdk"` with no model suffix.
    #[serde(default = "default_claude_model")]
    pub default_model: String,

    /// Maximum spend in USD before aborting a request (`--max-turns` controls
    /// loop depth; `--budget` controls cost). `None` means no cap.
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
}

fn default_claude_binary() -> String {
    "claude".to_string()
}

fn default_claude_model() -> String {
    "claude-sonnet-4-6".to_string()
}

impl Default for ClaudeAgentSdkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            binary: default_claude_binary(),
            default_model: default_claude_model(),
            max_budget_usd: None,
        }
    }
}
