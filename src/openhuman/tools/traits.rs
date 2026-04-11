use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// Re-export the unified ToolResult from the skills module so all tools use one type.
pub use crate::openhuman::skills::types::{ToolContent, ToolResult};

/// Controls where a tool is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolScope {
    /// Available in agent loop, CLI, and RPC.
    All,
    /// Only available in the autonomous agent loop.
    #[allow(dead_code)]
    AgentOnly,
    /// Only available via explicit CLI/RPC invocation (not autonomous agent).
    CliRpcOnly,
}

/// Category of a tool — used by the sub-agent runner to scope which
/// tools a given sub-agent is allowed to see.
///
/// The distinction matters because:
///
/// - **System tools** are built-in Rust implementations (shell, file_read,
///   file_write, cron_*, memory_*, …) that run inside the core process
///   with direct host access.
/// - **Skill tools** are QuickJS skill exports bridged through
///   [`crate::openhuman::tools::SkillToolBridge`]. They
///   talk to external services (Notion, Gmail, Telegram, …) via
///   user-installed skill packages.
///
/// The orchestrator uses this category to spawn dedicated tool-execution
/// sub-agents: one scoped to `Skill` for service integrations (running
/// with the backend's `agentic` model hint), and others scoped to
/// `System` for code/file/host work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    /// Built-in Rust tools with direct host access.
    #[default]
    System,
    /// QuickJS skill tools bridged from the runtime engine.
    Skill,
}

impl std::fmt::Display for ToolCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::System => write!(f, "system"),
            Self::Skill => write!(f, "skill"),
        }
    }
}

/// Permission level required to execute a tool.
///
/// Channels can set a maximum permission level to restrict which tools
/// are available. Tools requiring a level above the channel's maximum
/// are rejected before execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum PermissionLevel {
    /// No permission needed (metadata-only operations).
    None = 0,
    /// Read-only operations (file reads, memory recall, listing).
    #[default]
    ReadOnly = 1,
    /// Write operations (file writes, memory store).
    Write = 2,
    /// Command execution (shell, scripts).
    Execute = 3,
    /// Dangerous/destructive operations (hardware, system-level).
    Dangerous = 4,
}

impl std::fmt::Display for PermissionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::ReadOnly => write!(f, "ReadOnly"),
            Self::Write => write!(f, "Write"),
            Self::Execute => write!(f, "Execute"),
            Self::Dangerous => write!(f, "Dangerous"),
        }
    }
}

/// Description of a tool for the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Core tool trait — implement for any capability (built-in or skill-based).
#[async_trait]
pub trait Tool: Send + Sync {
    /// Tool name (used in LLM function calling)
    fn name(&self) -> &str;

    /// Human-readable description
    fn description(&self) -> &str;

    /// JSON schema for parameters
    fn parameters_schema(&self) -> serde_json::Value;

    /// Execute the tool with given arguments.
    /// Returns a unified `ToolResult` (MCP content blocks + error flag).
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult>;

    /// Permission level required to execute this tool.
    /// Channels with a lower maximum permission level will reject this tool.
    /// Default: `ReadOnly`. Override for write/execute/dangerous tools.
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    /// Where this tool may be executed. Default: `All`.
    /// Override to restrict (e.g. `CliRpcOnly` for phone calls).
    fn scope(&self) -> ToolScope {
        ToolScope::All
    }

    /// Category of this tool — `System` for built-in Rust tools (default)
    /// or `Skill` for tools bridged from the QuickJS skill runtime.
    ///
    /// The sub-agent runner uses this to filter the parent's tool
    /// registry when a sub-agent definition sets `category_filter`.
    /// Skill-bridged tools override this to return
    /// [`ToolCategory::Skill`].
    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    /// Get the full spec for LLM registration
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyTool;

    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "dummy_tool"
        }

        fn description(&self) -> &str {
            "A deterministic test tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                }
            })
        }

        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
            let text = args
                .get("value")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            Ok(ToolResult::success(text))
        }
    }

    #[test]
    fn spec_uses_tool_metadata_and_schema() {
        let tool = DummyTool;
        let spec = tool.spec();

        assert_eq!(spec.name, "dummy_tool");
        assert_eq!(spec.description, "A deterministic test tool");
        assert_eq!(spec.parameters["type"], "object");
        assert_eq!(spec.parameters["properties"]["value"]["type"], "string");
    }

    #[tokio::test]
    async fn execute_returns_expected_output() {
        let tool = DummyTool;
        let result = tool
            .execute(serde_json::json!({ "value": "hello-tool" }))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.output(), "hello-tool");
    }

    #[test]
    fn tool_result_serialization_roundtrip() {
        let result = ToolResult::error("boom");

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ToolResult = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_error);
        assert_eq!(parsed.output(), "boom");
    }
}
