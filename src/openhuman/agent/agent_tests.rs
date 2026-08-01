//! Comprehensive agent-loop test suite.
//!
//! Tests exercise the full `Agent.turn()` cycle with mock providers and tools,
//! covering every edge case an agentic tool loop must handle:
//!
//!   1. Simple text response (no tools)
//!   2. Single tool call → final response
//!   3. Multi-step tool chain (tool A → tool B → response)
//!   4. Max-iteration bailout
//!   5. Unknown tool name recovery
//!   6. Tool execution failure recovery
//!   7. Parallel tool dispatch
//!   8. History trimming during long conversations
//!   9. Memory auto-save round-trip
//!  10. Native vs XML dispatcher integration
//!  11. Empty / whitespace-only LLM responses
//!  12. Mixed text + tool call responses
//!  13. Multi-tool batch in a single response
//!  14. System prompt generation & tool instructions
//!  15. Context enrichment from memory loader
//!  16. ConversationMessage serialization round-trip
//!  17. Tool call with stringified JSON arguments
//!  18. Conversation history fidelity (tool call → tool result → assistant)
//!  19. Builder validation (missing required fields)
//!  20. Idempotent system prompt insertion

use crate::openhuman::agent::dispatcher::{
    NativeToolDispatcher, ToolDispatcher, ToolExecutionResult, XmlToolDispatcher,
};
use crate::openhuman::agent::harness::session::Agent;
use crate::openhuman::agent::messages::{ChatMessage, ConversationMessage, ToolResultMessage};
use crate::openhuman::config::{AgentConfig, MemoryConfig};
use crate::openhuman::inference::provider::{ChatResponse, ToolCall};
use crate::openhuman::memory::Memory;
use crate::openhuman::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tinyinference::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinymemory_core::store as memory_store;

// ═══════════════════════════════════════════════════════════════════════════
// Test Helpers — Mock Provider, Mock Tool, Mock Memory
// ═══════════════════════════════════════════════════════════════════════════

/// A mock LLM provider that returns pre-scripted responses in order.
/// When the queue is exhausted it returns a simple "done" text response.
struct ScriptedProvider {
    responses: Mutex<Vec<ChatResponse>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }
}

#[async_trait]
impl ChatModel<()> for ScriptedProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::LazyLock<ModelProfile> =
            std::sync::LazyLock::new(|| ModelProfile {
                provider: Some("agent-test".to_string()),
                tool_calling: true,
                parallel_tool_calls: true,
                ..ModelProfile::default()
            });
        Some(&PROFILE)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        let mut guard = self.responses.lock().unwrap();
        let response = if guard.is_empty() {
            ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }
        } else {
            guard.remove(0)
        };
        Ok(
            crate::openhuman::agent::tinyagents::model::native_model_response_for_request(
                &response, &request,
            ),
        )
    }
}

/// A mock provider that always returns an error.
struct FailingProvider;

#[async_trait]
impl ChatModel<()> for FailingProvider {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        Err(tinyinference::Error::Model("provider error".to_string()))
    }
}

/// A simple echo tool that returns its arguments as output.
struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Echoes the input"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": {"type": "string"}
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let msg = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("(empty)")
            .to_string();
        Ok(ToolResult::success(msg))
    }
}

/// A tool that always fails execution.
struct FailingTool;

#[async_trait]
impl Tool for FailingTool {
    fn name(&self) -> &str {
        "fail"
    }

    fn description(&self) -> &str {
        "Always fails"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::error("intentional failure"))
    }
}

/// A tool that panics (tests error propagation).
struct PanickingTool;

#[async_trait]
impl Tool for PanickingTool {
    fn name(&self) -> &str {
        "panicker"
    }

    fn description(&self) -> &str {
        "Panics on execution"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        anyhow::bail!("catastrophic tool failure")
    }
}

/// A tool that tracks how many times it was called.
struct CountingTool {
    count: Arc<Mutex<usize>>,
}

impl CountingTool {
    fn new() -> (Self, Arc<Mutex<usize>>) {
        let count = Arc::new(Mutex::new(0));
        (
            Self {
                count: count.clone(),
            },
            count,
        )
    }
}

#[async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        "counter"
    }

    fn description(&self) -> &str {
        "Counts calls"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        let mut c = self.count.lock().unwrap();
        *c += 1;
        Ok(ToolResult::success(format!("call #{}", *c)))
    }
}

/// Create an isolated memory instance with its own temp directory.
/// The returned `TempDir` must be held alive for the duration of the test
/// to prevent the directory (and its SQLite database) from being deleted.
fn make_memory() -> (Arc<dyn Memory>, tempfile::TempDir) {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    crate::openhuman::memory::host_impls::install_for_tests();
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = MemoryConfig {
        backend: "none".into(),
        ..MemoryConfig::default()
    };
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    crate::openhuman::memory::host_impls::install_for_tests();
    let mem = Arc::from(memory_store::create_memory(&cfg, tmp.path()).unwrap());
    (mem, tmp)
}

fn make_sqlite_memory() -> (Arc<dyn Memory>, tempfile::TempDir) {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    crate::openhuman::memory::host_impls::install_for_tests();
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = MemoryConfig {
        backend: "sqlite".into(),
        ..MemoryConfig::default()
    };
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    crate::openhuman::memory::host_impls::install_for_tests();
    let mem = Arc::from(memory_store::create_memory(&cfg, tmp.path()).unwrap());
    (mem, tmp)
}

/// Build an agent with an isolated temp workspace.
/// Returns `(Agent, TempDir)` — hold `_tmp` in the test to keep the dir alive.
fn build_agent_with(
    provider: Arc<dyn ChatModel<()>>,
    tools: Vec<Box<dyn Tool>>,
    dispatcher: Box<dyn ToolDispatcher>,
) -> (Agent, tempfile::TempDir) {
    let (mem, tmp) = make_memory();
    let agent = Agent::builder()
        .chat_model(provider)
        .tools(tools)
        .memory(mem)
        .tool_dispatcher(dispatcher)
        .workspace_dir(tmp.path().to_path_buf())
        .build()
        .unwrap();
    (agent, tmp)
}

fn build_agent_with_memory(
    provider: Arc<dyn ChatModel<()>>,
    tools: Vec<Box<dyn Tool>>,
    mem: Arc<dyn Memory>,
    auto_save: bool,
) -> (Agent, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let agent = Agent::builder()
        .chat_model(provider)
        .tools(tools)
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(tmp.path().to_path_buf())
        .auto_save(auto_save)
        .build()
        .unwrap();
    (agent, tmp)
}

fn build_agent_with_config(
    provider: Arc<dyn ChatModel<()>>,
    tools: Vec<Box<dyn Tool>>,
    config: AgentConfig,
) -> (Agent, tempfile::TempDir) {
    let (mem, tmp) = make_memory();
    let agent = Agent::builder()
        .chat_model(provider)
        .tools(tools)
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(tmp.path().to_path_buf())
        .config(config)
        .build()
        .unwrap();
    (agent, tmp)
}

/// Helper: create a ChatResponse with tool calls (native format).
fn tool_response(calls: Vec<ToolCall>) -> ChatResponse {
    ChatResponse {
        text: Some(String::new()),
        tool_calls: calls,
        usage: None,
        reasoning_content: None,
    }
}

/// Helper: create a plain text ChatResponse.
fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.into()),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    }
}

/// Helper: create an XML-style tool call response.
fn xml_tool_response(name: &str, args: &str) -> ChatResponse {
    ChatResponse {
        text: Some(format!(
            "<tool_call>\n{{\"name\": \"{name}\", \"arguments\": {args}}}\n</tool_call>"
        )),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    }
}

#[path = "agent_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "agent_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "agent_tests_part_03_tests.rs"]
mod part_03_tests;
