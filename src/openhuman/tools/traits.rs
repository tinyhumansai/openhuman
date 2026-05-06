use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// Re-export the unified ToolResult from the lightweight skills types module so all tools use one type.
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
/// - **Skill tools** are integration-facing tools that talk to external
///   services (for example Composio-backed SaaS actions).
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
    /// Integration-facing tools that reach external services.
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

/// Per-invocation options threaded from the agent loop into a tool's
/// execution. Lets callers (the harness, orchestrator, RPC dispatcher)
/// hint at how the tool should shape its output without polluting the
/// tool's user-facing parameter schema.
///
/// Tools that opt in override [`Tool::execute_with_options`] and check
/// these flags; tools that ignore the struct keep working unchanged
/// because the trait's default implementation forwards to
/// [`Tool::execute`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ToolCallOptions {
    /// When true, the caller (typically the agent loop) prefers a
    /// markdown rendering of the result for direct LLM consumption,
    /// because markdown is materially cheaper than JSON in tokens.
    /// Tools should populate `ToolResult::markdown_formatted` when
    /// this is set; the harness will pick that field up if present.
    pub prefer_markdown: bool,
}

/// Description of a tool for the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Core tool trait — implement for any capability (built-in or integration-based).
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

    /// Execute the tool with caller-provided options.
    ///
    /// Default implementation forwards to [`Self::execute`] — existing
    /// tools keep working without changes. Tools that can produce a
    /// compact markdown rendering (saving tokens in the agent loop)
    /// should override this method, inspect
    /// [`ToolCallOptions::prefer_markdown`], and populate
    /// `ToolResult::markdown_formatted` on the returned result.
    async fn execute_with_options(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        self.execute(args).await
    }

    /// Whether this tool can produce a markdown rendering when
    /// [`ToolCallOptions::prefer_markdown`] is set. Default: `false`.
    /// Tools that override [`Self::execute_with_options`] to honor the
    /// flag should also override this to advertise the capability —
    /// telemetry / agent-loop diagnostics use it to attribute token
    /// savings.
    fn supports_markdown(&self) -> bool {
        false
    }

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
    /// or `Skill` for integration-facing tools.
    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    /// Whether two concurrent invocations of this tool are safe to
    /// run in parallel inside a single LLM iteration.
    ///
    /// Read-only tools that touch no shared mutable state should
    /// return `true` (the agent's tool loop can then `join_all` a
    /// batch of read calls instead of awaiting them serially). Tools
    /// that mutate the workspace, write to disk, or interact with
    /// external services that throttle by caller should leave the
    /// default `false`.
    ///
    /// The argument is provided so a tool can refine the answer per
    /// call (e.g. a generic `bash` tool could allow parallel `ls` /
    /// `cat` invocations and reject parallel `npm install`s) — most
    /// tools will ignore it.
    ///
    /// **Wiring note:** the parallel dispatcher in
    /// `harness::tool_loop` currently runs tool calls serially
    /// regardless of this flag. Annotating tools is still load-
    /// bearing: it lets the dispatch refactor land without
    /// coordinating with every tool author. See the parallel-tool
    /// dispatch follow-up issue.
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        false
    }

    /// Per-tool cap on the character length of the result body sent
    /// back to the model.
    ///
    /// When `Some(cap)` and the tool's `output_for_llm` exceeds it,
    /// the agent's tool loop truncates the body and appends a marker
    /// before threading the value into history — protecting the
    /// context window from one chatty tool. When `None` (the
    /// default), no per-tool cap applies and the global
    /// `PayloadSummarizer` (if any) handles oversize bodies.
    ///
    /// Set this on tools whose output is *bounded but unpredictable*
    /// (`bash`, `web_fetch`, etc.); leave it unset on tools where
    /// callers genuinely want full content (`read_file`, `grep`).
    fn max_result_size_chars(&self) -> Option<usize> {
        None
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

    // ── Default trait-method values ────────────────────────────────

    #[test]
    fn default_permission_level_is_read_only() {
        let tool = DummyTool;
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    }

    #[test]
    fn default_scope_is_all() {
        let tool = DummyTool;
        assert_eq!(tool.scope(), ToolScope::All);
    }

    #[test]
    fn default_category_is_system() {
        let tool = DummyTool;
        assert_eq!(tool.category(), ToolCategory::System);
    }

    #[test]
    fn default_is_concurrency_safe_is_false() {
        let tool = DummyTool;
        assert!(!tool.is_concurrency_safe(&serde_json::Value::Null));
    }

    #[test]
    fn default_max_result_size_chars_is_none() {
        let tool = DummyTool;
        assert!(tool.max_result_size_chars().is_none());
    }

    // ── PermissionLevel ordering ───────────────────────────────────

    #[test]
    fn permission_level_is_totally_ordered_from_none_to_dangerous() {
        // The runtime compares PermissionLevel as `<` to reject tools whose
        // required level exceeds the channel max, so the ordering is a
        // load-bearing invariant.
        assert!(PermissionLevel::None < PermissionLevel::ReadOnly);
        assert!(PermissionLevel::ReadOnly < PermissionLevel::Write);
        assert!(PermissionLevel::Write < PermissionLevel::Execute);
        assert!(PermissionLevel::Execute < PermissionLevel::Dangerous);
    }

    #[test]
    fn permission_level_default_is_read_only() {
        assert_eq!(PermissionLevel::default(), PermissionLevel::ReadOnly);
    }

    #[test]
    fn permission_level_display_matches_variant_name() {
        assert_eq!(PermissionLevel::None.to_string(), "None");
        assert_eq!(PermissionLevel::ReadOnly.to_string(), "ReadOnly");
        assert_eq!(PermissionLevel::Write.to_string(), "Write");
        assert_eq!(PermissionLevel::Execute.to_string(), "Execute");
        assert_eq!(PermissionLevel::Dangerous.to_string(), "Dangerous");
    }

    #[test]
    fn permission_level_round_trips_as_json_number() {
        for level in [
            PermissionLevel::None,
            PermissionLevel::ReadOnly,
            PermissionLevel::Write,
            PermissionLevel::Execute,
            PermissionLevel::Dangerous,
        ] {
            let s = serde_json::to_string(&level).unwrap();
            let back: PermissionLevel = serde_json::from_str(&s).unwrap();
            assert_eq!(back, level);
        }
    }

    // ── ToolCategory ───────────────────────────────────────────────

    #[test]
    fn tool_category_default_is_system() {
        assert_eq!(ToolCategory::default(), ToolCategory::System);
    }

    #[test]
    fn tool_category_display_is_lowercase() {
        assert_eq!(ToolCategory::System.to_string(), "system");
        assert_eq!(ToolCategory::Skill.to_string(), "skill");
    }

    #[test]
    fn tool_category_serde_uses_snake_case() {
        // The runtime relies on snake_case JSON for `category` in agent
        // definitions — catch any rename that would break user-facing
        // definition files.
        let s = serde_json::to_string(&ToolCategory::System).unwrap();
        assert_eq!(s, "\"system\"");
        let s = serde_json::to_string(&ToolCategory::Skill).unwrap();
        assert_eq!(s, "\"skill\"");
        let back: ToolCategory = serde_json::from_str("\"skill\"").unwrap();
        assert_eq!(back, ToolCategory::Skill);
    }

    // ── ToolScope ──────────────────────────────────────────────────

    #[test]
    fn tool_scope_variants_are_distinct() {
        assert_ne!(ToolScope::All, ToolScope::AgentOnly);
        assert_ne!(ToolScope::All, ToolScope::CliRpcOnly);
        assert_ne!(ToolScope::AgentOnly, ToolScope::CliRpcOnly);
    }
}
