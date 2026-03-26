//! Core agent traits ported from OpenHuman.
//!
//! Each trait defines an extension point. Noop implementations are provided
//! as test doubles and reference implementations.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

// ═══════════════════════════════════════════════════════════════════
// Provider trait — LLM model interface
// ═══════════════════════════════════════════════════════════════════

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }

    pub fn tool(content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
        }
    }
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// An LLM response that may contain text, tool calls, or both.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

impl ChatResponse {
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    pub fn text_or_empty(&self) -> &str {
        self.text.as_deref().unwrap_or("")
    }
}

/// Request payload for provider chat calls.
#[derive(Debug, Clone, Copy)]
pub struct ChatRequest<'a> {
    pub messages: &'a [ChatMessage],
    pub tools: Option<&'a [ToolSpec]>,
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// One-shot chat with optional system prompt.
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String>;

    /// Multi-turn conversation.
    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let system = messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.as_str());
        let last_user = messages
            .iter()
            .rfind(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");
        self.chat_with_system(system, last_user, model, temperature)
            .await
    }

    /// Warm up the HTTP connection pool.
    async fn warmup(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Noop provider that always returns an empty string.
pub struct NoopProvider;

#[async_trait]
impl Provider for NoopProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok(String::new())
    }
}

// ═══════════════════════════════════════════════════════════════════
// Tool trait — executable capabilities
// ═══════════════════════════════════════════════════════════════════

/// Result of a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

/// Description of a tool for the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Core tool trait — implement for any capability.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult>;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

/// Noop tool that always succeeds with empty output.
pub struct NoopTool;

#[async_trait]
impl Tool for NoopTool {
    fn name(&self) -> &str {
        "noop"
    }
    fn description(&self) -> &str {
        "No-op tool (does nothing)"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            success: true,
            output: String::new(),
            error: None,
        })
    }
}

// ═══════════════════════════════════════════════════════════════════
// Memory trait — persistence backends
// ═══════════════════════════════════════════════════════════════════

/// A single memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub key: String,
    pub content: String,
    pub category: MemoryCategory,
    pub timestamp: String,
    pub session_id: Option<String>,
    pub score: Option<f64>,
}

/// Memory categories for organization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    Core,
    Daily,
    Conversation,
    Custom(String),
}

impl std::fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core => write!(f, "core"),
            Self::Daily => write!(f, "daily"),
            Self::Conversation => write!(f, "conversation"),
            Self::Custom(name) => write!(f, "{name}"),
        }
    }
}

/// Core memory trait — implement for any persistence backend.
#[async_trait]
pub trait Memory: Send + Sync {
    fn name(&self) -> &str;

    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()>;

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>>;

    async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>>;

    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>>;

    async fn forget(&self, key: &str) -> anyhow::Result<bool>;

    async fn count(&self) -> anyhow::Result<usize>;

    async fn health_check(&self) -> bool;
}

/// Noop memory that stores nothing.
pub struct NoopMemory;

#[async_trait]
impl Memory for NoopMemory {
    fn name(&self) -> &str {
        "noop"
    }
    async fn store(
        &self,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }
    async fn get(&self, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }
    async fn list(
        &self,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }
    async fn forget(&self, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }
    async fn health_check(&self) -> bool {
        true
    }
}

// ═══════════════════════════════════════════════════════════════════
// Observer trait — observability backends
// ═══════════════════════════════════════════════════════════════════

/// Events the observer can record.
#[derive(Debug, Clone)]
pub enum ObserverEvent {
    AgentStart {
        provider: String,
        model: String,
    },
    LlmRequest {
        provider: String,
        model: String,
        messages_count: usize,
    },
    LlmResponse {
        provider: String,
        model: String,
        duration: Duration,
        success: bool,
        error_message: Option<String>,
    },
    AgentEnd {
        provider: String,
        model: String,
        duration: Duration,
        tokens_used: Option<u64>,
        cost_usd: Option<f64>,
    },
    ToolCallStart {
        tool: String,
    },
    ToolCall {
        tool: String,
        duration: Duration,
        success: bool,
    },
    TurnComplete,
    ChannelMessage {
        channel: String,
        direction: String,
    },
    HeartbeatTick,
    Error {
        component: String,
        message: String,
    },
}

/// Numeric metrics.
#[derive(Debug, Clone)]
pub enum ObserverMetric {
    RequestLatency(Duration),
    TokensUsed(u64),
    ActiveSessions(u64),
    QueueDepth(u64),
}

/// Core observability trait — implement for any backend.
pub trait Observer: Send + Sync + 'static {
    fn record_event(&self, event: &ObserverEvent);
    fn record_metric(&self, metric: &ObserverMetric);
    fn flush(&self) {}
    fn name(&self) -> &str;
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Noop observer that discards all events.
#[derive(Default)]
pub struct NoopObserver;

impl Observer for NoopObserver {
    fn record_event(&self, _event: &ObserverEvent) {}
    fn record_metric(&self, _metric: &ObserverMetric) {}
    fn name(&self) -> &str {
        "noop"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ═══════════════════════════════════════════════════════════════════
// RuntimeAdapter trait — platform abstractions
// ═══════════════════════════════════════════════════════════════════

/// Runtime adapter — abstracts platform differences.
pub trait RuntimeAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn has_shell_access(&self) -> bool;
    fn has_filesystem_access(&self) -> bool;
    fn storage_path(&self) -> PathBuf;
    fn supports_long_running(&self) -> bool;
    fn memory_budget(&self) -> u64 {
        0
    }
    fn build_shell_command(
        &self,
        command: &str,
        workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command>;
}

/// Noop runtime adapter that reports no capabilities.
pub struct NoopRuntimeAdapter;

impl RuntimeAdapter for NoopRuntimeAdapter {
    fn name(&self) -> &str {
        "noop"
    }
    fn has_shell_access(&self) -> bool {
        false
    }
    fn has_filesystem_access(&self) -> bool {
        false
    }
    fn storage_path(&self) -> PathBuf {
        PathBuf::from("/dev/null")
    }
    fn supports_long_running(&self) -> bool {
        false
    }
    fn build_shell_command(
        &self,
        _command: &str,
        _workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command> {
        anyhow::bail!("NoopRuntimeAdapter does not support shell commands")
    }
}

// ═══════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_constructors() {
        let sys = ChatMessage::system("Be helpful");
        assert_eq!(sys.role, "system");
        assert_eq!(sys.content, "Be helpful");

        let user = ChatMessage::user("Hello");
        assert_eq!(user.role, "user");

        let asst = ChatMessage::assistant("Hi there");
        assert_eq!(asst.role, "assistant");

        let tool = ChatMessage::tool("{}");
        assert_eq!(tool.role, "tool");
    }

    #[test]
    fn chat_response_helpers() {
        let empty = ChatResponse {
            text: None,
            tool_calls: vec![],
        };
        assert!(!empty.has_tool_calls());
        assert_eq!(empty.text_or_empty(), "");

        let with_tools = ChatResponse {
            text: Some("Let me check".into()),
            tool_calls: vec![ToolCall {
                id: "1".into(),
                name: "shell".into(),
                arguments: "{}".into(),
            }],
        };
        assert!(with_tools.has_tool_calls());
        assert_eq!(with_tools.text_or_empty(), "Let me check");
    }

    #[test]
    fn tool_spec_from_noop_tool() {
        let tool = NoopTool;
        let spec = tool.spec();
        assert_eq!(spec.name, "noop");
        assert_eq!(spec.parameters["type"], "object");
    }

    #[tokio::test]
    async fn noop_tool_execute() {
        let tool = NoopTool;
        let result = tool.execute(serde_json::json!({})).await.unwrap();
        assert!(result.success);
        assert!(result.output.is_empty());
    }

    #[tokio::test]
    async fn noop_provider_returns_empty() {
        let provider = NoopProvider;
        let result = provider
            .chat_with_system(None, "hello", "model", 0.7)
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn noop_memory_operations() {
        let mem = NoopMemory;
        assert_eq!(mem.name(), "noop");
        assert!(mem.health_check().await);
        assert_eq!(mem.count().await.unwrap(), 0);
        assert!(mem.get("key").await.unwrap().is_none());
        assert!(mem.recall("query", 10, None).await.unwrap().is_empty());
        assert!(!mem.forget("key").await.unwrap());
    }

    #[test]
    fn noop_observer_accepts_events() {
        let observer = NoopObserver;
        observer.record_event(&ObserverEvent::HeartbeatTick);
        observer.record_metric(&ObserverMetric::TokensUsed(42));
        observer.flush();
        assert_eq!(observer.name(), "noop");
    }

    #[test]
    fn noop_runtime_adapter_reports_no_capabilities() {
        let runtime = NoopRuntimeAdapter;
        assert_eq!(runtime.name(), "noop");
        assert!(!runtime.has_shell_access());
        assert!(!runtime.has_filesystem_access());
        assert!(!runtime.supports_long_running());
        assert_eq!(runtime.memory_budget(), 0);
    }

    #[test]
    fn memory_category_display() {
        assert_eq!(MemoryCategory::Core.to_string(), "core");
        assert_eq!(MemoryCategory::Daily.to_string(), "daily");
        assert_eq!(MemoryCategory::Conversation.to_string(), "conversation");
        assert_eq!(
            MemoryCategory::Custom("notes".into()).to_string(),
            "notes"
        );
    }

    #[test]
    fn memory_category_serde() {
        let core = serde_json::to_string(&MemoryCategory::Core).unwrap();
        assert_eq!(core, "\"core\"");
        let parsed: MemoryCategory = serde_json::from_str("\"daily\"").unwrap();
        assert_eq!(parsed, MemoryCategory::Daily);
    }

    #[test]
    fn tool_result_serialization_roundtrip() {
        let result = ToolResult {
            success: false,
            output: String::new(),
            error: Some("boom".into()),
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: ToolResult = serde_json::from_str(&json).unwrap();
        assert!(!parsed.success);
        assert_eq!(parsed.error.as_deref(), Some("boom"));
    }
}
