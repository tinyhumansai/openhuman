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
use crate::openhuman::config::{AgentConfig, MemoryConfig};
use crate::openhuman::memory::{self, Memory};
use crate::openhuman::providers::{
    ChatMessage, ChatRequest, ChatResponse, ConversationMessage, Provider, ToolCall,
    ToolResultMessage,
};
use crate::openhuman::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

// ═══════════════════════════════════════════════════════════════════════════
// Test Helpers — Mock Provider, Mock Tool, Mock Memory
// ═══════════════════════════════════════════════════════════════════════════

/// A mock LLM provider that returns pre-scripted responses in order.
/// When the queue is exhausted it returns a simple "done" text response.
struct ScriptedProvider {
    responses: Mutex<Vec<ChatResponse>>,
    /// Records every request for assertion.
    requests: Mutex<Vec<Vec<ChatMessage>>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok("fallback".into())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        self.requests
            .lock()
            .unwrap()
            .push(request.messages.to_vec());

        let mut guard = self.responses.lock().unwrap();
        if guard.is_empty() {
            return Ok(ChatResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: None,
            });
        }
        Ok(guard.remove(0))
    }
}

/// A mock provider that always returns an error.
struct FailingProvider;

#[async_trait]
impl Provider for FailingProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        anyhow::bail!("provider error")
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        anyhow::bail!("provider error")
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
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = MemoryConfig {
        backend: "none".into(),
        ..MemoryConfig::default()
    };
    let mem = Arc::from(memory::create_memory(&cfg, tmp.path()).unwrap());
    (mem, tmp)
}

fn make_sqlite_memory() -> (Arc<dyn Memory>, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = MemoryConfig {
        backend: "sqlite".into(),
        ..MemoryConfig::default()
    };
    let mem = Arc::from(memory::create_memory(&cfg, tmp.path()).unwrap());
    (mem, tmp)
}

/// Build an agent with an isolated temp workspace.
/// Returns `(Agent, TempDir)` — hold `_tmp` in the test to keep the dir alive.
fn build_agent_with(
    provider: Box<dyn Provider>,
    tools: Vec<Box<dyn Tool>>,
    dispatcher: Box<dyn ToolDispatcher>,
) -> (Agent, tempfile::TempDir) {
    let (mem, tmp) = make_memory();
    let agent = Agent::builder()
        .provider(provider)
        .tools(tools)
        .memory(mem)
        .tool_dispatcher(dispatcher)
        .workspace_dir(tmp.path().to_path_buf())
        .build()
        .unwrap();
    (agent, tmp)
}

fn build_agent_with_memory(
    provider: Box<dyn Provider>,
    tools: Vec<Box<dyn Tool>>,
    mem: Arc<dyn Memory>,
    auto_save: bool,
) -> (Agent, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let agent = Agent::builder()
        .provider(provider)
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
    provider: Box<dyn Provider>,
    tools: Vec<Box<dyn Tool>>,
    config: AgentConfig,
) -> (Agent, tempfile::TempDir) {
    let (mem, tmp) = make_memory();
    let agent = Agent::builder()
        .provider(provider)
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
    }
}

/// Helper: create a plain text ChatResponse.
fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.into()),
        tool_calls: vec![],
        usage: None,
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
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Simple text response (no tools)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_returns_text_when_no_tools_called() {
    let provider = Box::new(ScriptedProvider::new(vec![text_response("Hello world")]));
    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("hi").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty text response from provider"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Single tool call → final response
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_executes_single_tool_then_returns() {
    let provider = Box::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "echo".into(),
            arguments: r#"{"message": "hello from tool"}"#.into(),
        }]),
        text_response("I ran the tool"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("run echo").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response after tool execution"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Multi-step tool chain (tool A → tool B → response)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_handles_multi_step_tool_chain() {
    let (counting_tool, count) = CountingTool::new();

    let provider = Box::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "counter".into(),
            arguments: "{}".into(),
        }]),
        tool_response(vec![ToolCall {
            id: "tc2".into(),
            name: "counter".into(),
            arguments: "{}".into(),
        }]),
        tool_response(vec![ToolCall {
            id: "tc3".into(),
            name: "counter".into(),
            arguments: "{}".into(),
        }]),
        text_response("Done after 3 calls"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(counting_tool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("count 3 times").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response after multi-step chain"
    );
    assert_eq!(*count.lock().unwrap(), 3);
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Max-iteration bailout
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_bails_out_at_max_iterations() {
    // Create more tool calls than max_tool_iterations allows.
    let max_iters = 3;
    let mut responses = Vec::new();
    for i in 0..max_iters + 5 {
        responses.push(tool_response(vec![ToolCall {
            id: format!("tc{i}"),
            name: "echo".into(),
            arguments: r#"{"message": "loop"}"#.into(),
        }]));
    }

    let provider = Box::new(ScriptedProvider::new(responses));

    let config = AgentConfig {
        max_tool_iterations: max_iters,
        ..AgentConfig::default()
    };

    let (mut agent, _tmp) = build_agent_with_config(provider, vec![Box::new(EchoTool)], config);

    let result = agent.turn("infinite loop").await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("maximum tool iterations"),
        "Expected max iterations error, got: {err}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Unknown tool name recovery
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_handles_unknown_tool_gracefully() {
    let provider = Box::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "nonexistent_tool".into(),
            arguments: "{}".into(),
        }]),
        text_response("I couldn't find that tool"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("use nonexistent").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response after unknown tool recovery"
    );

    // Verify the tool result mentioned "Unknown tool"
    let has_tool_result = agent.history().iter().any(|msg| match msg {
        ConversationMessage::ToolResults(results) => {
            results.iter().any(|r| r.content.contains("Unknown tool"))
        }
        _ => false,
    });
    assert!(
        has_tool_result,
        "Expected tool result with 'Unknown tool' message"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. Tool execution failure recovery
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_recovers_from_tool_failure() {
    let provider = Box::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "fail".into(),
            arguments: "{}".into(),
        }]),
        text_response("Tool failed but I recovered"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(FailingTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("try failing tool").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response after tool failure recovery"
    );
}

#[tokio::test]
async fn turn_recovers_from_tool_error() {
    let provider = Box::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "panicker".into(),
            arguments: "{}".into(),
        }]),
        text_response("I recovered from the error"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(PanickingTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("try panicking").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response after tool error recovery"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. Provider error propagation
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_propagates_provider_error() {
    let (mut agent, _tmp) = build_agent_with(
        Box::new(FailingProvider),
        vec![],
        Box::new(NativeToolDispatcher),
    );

    let result = agent.turn("hello").await;
    assert!(result.is_err(), "Expected provider error to propagate");
}

// ═══════════════════════════════════════════════════════════════════════════
// 8. History trimming during long conversations
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn history_trims_after_max_messages() {
    let max_history = 6;
    let mut responses = vec![];
    for _ in 0..max_history + 5 {
        responses.push(text_response("ok"));
    }

    let provider = Box::new(ScriptedProvider::new(responses));
    let config = AgentConfig {
        max_history_messages: max_history,
        ..AgentConfig::default()
    };

    let (mut agent, _tmp) = build_agent_with_config(provider, vec![], config);

    for i in 0..max_history + 5 {
        let _ = agent.turn(&format!("msg {i}")).await.unwrap();
    }

    // System prompt (1) + trimmed messages
    // Should not exceed max_history + 1 (system prompt)
    assert!(
        agent.history().len() <= max_history + 1,
        "History length {} exceeds max {} + 1 (system)",
        agent.history().len(),
        max_history,
    );

    // System prompt should always be preserved
    let first = &agent.history()[0];
    assert!(matches!(first, ConversationMessage::Chat(c) if c.role == "system"));
}

// ═══════════════════════════════════════════════════════════════════════════
// 9. Memory auto-save round-trip
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn auto_save_stores_messages_in_memory() {
    let (mem, _tmp) = make_sqlite_memory();
    let provider = Box::new(ScriptedProvider::new(vec![text_response(
        "I remember everything",
    )]));

    let (mut agent, _tmp2) = build_agent_with_memory(
        provider,
        vec![],
        mem.clone(),
        true, // auto_save enabled
    );

    let _ = agent.turn("Remember this fact").await.unwrap();

    // Both user message and assistant response should be saved
    let count = mem.count().await.unwrap();
    assert!(
        count >= 2,
        "Expected at least 2 memory entries, got {count}"
    );
}

#[tokio::test]
async fn auto_save_disabled_does_not_store() {
    let (mem, _tmp) = make_sqlite_memory();
    let provider = Box::new(ScriptedProvider::new(vec![text_response("hello")]));

    let (mut agent, _tmp2) = build_agent_with_memory(
        provider,
        vec![],
        mem.clone(),
        false, // auto_save disabled
    );

    let _ = agent.turn("test message").await.unwrap();

    let count = mem.count().await.unwrap();
    assert_eq!(count, 0, "Expected 0 memory entries with auto_save off");
}

// ═══════════════════════════════════════════════════════════════════════════
// 10. Native vs XML dispatcher integration
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn xml_dispatcher_parses_and_loops() {
    let provider = Box::new(ScriptedProvider::new(vec![
        xml_tool_response("echo", r#"{"message": "xml-test"}"#),
        text_response("XML tool completed"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(XmlToolDispatcher),
    );

    let response = agent.turn("test xml").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response from XML dispatcher"
    );
}

#[tokio::test]
async fn native_dispatcher_sends_tool_specs() {
    let provider = Box::new(ScriptedProvider::new(vec![text_response("ok")]));
    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let _ = agent.turn("hi").await.unwrap();

    // NativeToolDispatcher.should_send_tool_specs() returns true
    let dispatcher = NativeToolDispatcher;
    assert!(dispatcher.should_send_tool_specs());
}

#[tokio::test]
async fn xml_dispatcher_does_not_send_tool_specs() {
    let dispatcher = XmlToolDispatcher;
    assert!(!dispatcher.should_send_tool_specs());
}

// ═══════════════════════════════════════════════════════════════════════════
// 11. Empty / whitespace-only LLM responses
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_handles_empty_text_response() {
    let provider = Box::new(ScriptedProvider::new(vec![ChatResponse {
        text: Some(String::new()),
        tool_calls: vec![],
        usage: None,
    }]));

    let (mut agent, _tmp) = build_agent_with(provider, vec![], Box::new(NativeToolDispatcher));

    let response = agent.turn("hi").await.unwrap();
    assert!(response.is_empty());
}

#[tokio::test]
async fn turn_handles_none_text_response() {
    let provider = Box::new(ScriptedProvider::new(vec![ChatResponse {
        text: None,
        tool_calls: vec![],
        usage: None,
    }]));

    let (mut agent, _tmp) = build_agent_with(provider, vec![], Box::new(NativeToolDispatcher));

    // Should not panic — falls back to empty string
    let response = agent.turn("hi").await.unwrap();
    assert!(response.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════════
// 12. Mixed text + tool call responses
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_preserves_text_alongside_tool_calls() {
    let provider = Box::new(ScriptedProvider::new(vec![
        ChatResponse {
            text: Some("Let me check...".into()),
            tool_calls: vec![ToolCall {
                id: "tc1".into(),
                name: "echo".into(),
                arguments: r#"{"message": "hi"}"#.into(),
            }],
            usage: None,
        },
        text_response("Here are the results"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("check something").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty final response after mixed text+tool"
    );

    // The intermediate text should be in history
    let has_intermediate = agent.history().iter().any(|msg| match msg {
        ConversationMessage::Chat(c) => c.role == "assistant" && c.content.contains("Let me check"),
        _ => false,
    });
    assert!(has_intermediate, "Intermediate text should be in history");
}

// ═══════════════════════════════════════════════════════════════════════════
// 13. Multi-tool batch in a single response
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn turn_handles_multiple_tools_in_one_response() {
    let (counting_tool, count) = CountingTool::new();

    let provider = Box::new(ScriptedProvider::new(vec![
        tool_response(vec![
            ToolCall {
                id: "tc1".into(),
                name: "counter".into(),
                arguments: "{}".into(),
            },
            ToolCall {
                id: "tc2".into(),
                name: "counter".into(),
                arguments: "{}".into(),
            },
            ToolCall {
                id: "tc3".into(),
                name: "counter".into(),
                arguments: "{}".into(),
            },
        ]),
        text_response("All 3 done"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(counting_tool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("batch").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response after multi-tool batch"
    );
    assert_eq!(
        *count.lock().unwrap(),
        3,
        "All 3 tools should have been called"
    );
}

#[tokio::test]
async fn e2e_native_loop_executes_text_fallback_tool_calls_and_persists_history() {
    let provider = Box::new(ScriptedProvider::new(vec![
        ChatResponse {
            text: Some(
                "I'll inspect now.\n<invoke>{\"name\":\"echo\",\"arguments\":{\"message\":\"from-fallback\"}}</invoke>"
                    .into(),
            ),
            tool_calls: vec![],
            usage: None,
        },
        text_response("Completed via tool"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let response = agent.turn("please use a tool").await.unwrap();
    assert_eq!(response, "Completed via tool");

    let mut assistant_tool_calls: Option<Vec<ToolCall>> = None;
    let mut tool_results: Option<Vec<ToolResultMessage>> = None;

    for msg in agent.history() {
        match msg {
            ConversationMessage::AssistantToolCalls { tool_calls, .. } => {
                assistant_tool_calls = Some(tool_calls.clone());
            }
            ConversationMessage::ToolResults(results) => {
                tool_results = Some(results.clone());
            }
            _ => {}
        }
    }

    let calls = assistant_tool_calls.expect("assistant tool calls should be persisted");
    let results = tool_results.expect("tool results should be persisted");
    assert_eq!(calls.len(), 1, "expected one parsed/persisted tool call");
    assert_eq!(results.len(), 1, "expected one tool result");
    assert_eq!(calls[0].name, "echo");
    assert!(
        calls[0].arguments.contains("from-fallback"),
        "persisted tool-call arguments should include fallback payload"
    );
    assert_eq!(
        calls[0].id, results[0].tool_call_id,
        "tool result must map to persisted assistant tool-call id"
    );
    assert_eq!(results[0].content, "from-fallback");
}

// ═══════════════════════════════════════════════════════════════════════════
// 14. System prompt generation & tool instructions
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn system_prompt_injected_on_first_turn() {
    let provider = Box::new(ScriptedProvider::new(vec![text_response("ok")]));
    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    assert!(agent.history().is_empty(), "History should start empty");

    let _ = agent.turn("hi").await.unwrap();

    // First message should be the system prompt
    let first = &agent.history()[0];
    assert!(
        matches!(first, ConversationMessage::Chat(c) if c.role == "system"),
        "First history entry should be system prompt"
    );
}

#[tokio::test]
async fn system_prompt_not_duplicated_on_second_turn() {
    let provider = Box::new(ScriptedProvider::new(vec![
        text_response("first"),
        text_response("second"),
    ]));
    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let _ = agent.turn("hi").await.unwrap();
    let _ = agent.turn("hello again").await.unwrap();

    let system_count = agent
        .history()
        .iter()
        .filter(|msg| matches!(msg, ConversationMessage::Chat(c) if c.role == "system"))
        .count();
    assert_eq!(system_count, 1, "System prompt should appear exactly once");
}

// ═══════════════════════════════════════════════════════════════════════════
// 15. Conversation history fidelity
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn history_contains_all_expected_entries_after_tool_loop() {
    let provider = Box::new(ScriptedProvider::new(vec![
        tool_response(vec![ToolCall {
            id: "tc1".into(),
            name: "echo".into(),
            arguments: r#"{"message": "tool-out"}"#.into(),
        }]),
        text_response("final answer"),
    ]));

    let (mut agent, _tmp) = build_agent_with(
        provider,
        vec![Box::new(EchoTool)],
        Box::new(NativeToolDispatcher),
    );

    let _ = agent.turn("test").await.unwrap();

    // Expected history entries:
    //   0: system prompt
    //   1: user message "test"
    //   2: AssistantToolCalls
    //   3: ToolResults
    //   4: assistant "final answer"
    let history = agent.history();
    assert!(
        history.len() >= 5,
        "Expected at least 5 history entries, got {}",
        history.len()
    );

    assert!(matches!(&history[0], ConversationMessage::Chat(c) if c.role == "system"));
    assert!(matches!(&history[1], ConversationMessage::Chat(c) if c.role == "user"));
    assert!(matches!(
        &history[2],
        ConversationMessage::AssistantToolCalls { .. }
    ));
    assert!(matches!(&history[3], ConversationMessage::ToolResults(_)));
    assert!(
        matches!(&history[4], ConversationMessage::Chat(c) if c.role == "assistant" && c.content == "final answer")
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 16. Builder validation
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn builder_fails_without_provider() {
    let (mem, _tmp) = make_memory();
    let result = Agent::builder()
        .tools(vec![])
        .memory(mem)
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .workspace_dir(_tmp.path().to_path_buf())
        .build();

    assert!(result.is_err(), "Building without provider should fail");
}

// ═══════════════════════════════════════════════════════════════════════════
// 17. Multi-turn conversation maintains context
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn multi_turn_maintains_growing_history() {
    let provider = Box::new(ScriptedProvider::new(vec![
        text_response("response 1"),
        text_response("response 2"),
        text_response("response 3"),
    ]));

    let (mut agent, _tmp) = build_agent_with(provider, vec![], Box::new(NativeToolDispatcher));

    let r1 = agent.turn("msg 1").await.unwrap();
    let len_after_1 = agent.history().len();

    let r2 = agent.turn("msg 2").await.unwrap();
    let len_after_2 = agent.history().len();

    let r3 = agent.turn("msg 3").await.unwrap();
    let len_after_3 = agent.history().len();

    assert_eq!(r1, "response 1");
    assert_eq!(r2, "response 2");
    assert_eq!(r3, "response 3");

    // History should grow with each turn (user + assistant per turn)
    assert!(
        len_after_2 > len_after_1,
        "History should grow after turn 2"
    );
    assert!(
        len_after_3 > len_after_2,
        "History should grow after turn 3"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 18. Tool call with stringified JSON arguments (common LLM pattern)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn native_dispatcher_handles_stringified_arguments() {
    let dispatcher = NativeToolDispatcher;
    let response = ChatResponse {
        text: Some(String::new()),
        tool_calls: vec![ToolCall {
            id: "tc1".into(),
            name: "echo".into(),
            arguments: r#"{"message": "hello"}"#.into(),
        }],
        usage: None,
    };

    let (_, calls) = dispatcher.parse_response(&response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "echo");
    assert_eq!(
        calls[0].arguments.get("message").unwrap().as_str().unwrap(),
        "hello"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 19. XML dispatcher edge cases
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn xml_dispatcher_handles_nested_json() {
    let response = ChatResponse {
        text: Some(
            r#"<tool_call>
{"name": "file_write", "arguments": {"path": "test.json", "content": "{\"key\": \"value\"}"}}
</tool_call>"#
                .into(),
        ),
        tool_calls: vec![],
        usage: None,
    };

    let dispatcher = XmlToolDispatcher;
    let (_, calls) = dispatcher.parse_response(&response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "file_write");
    assert_eq!(
        calls[0].arguments.get("path").unwrap().as_str().unwrap(),
        "test.json"
    );
}

#[test]
fn xml_dispatcher_handles_empty_tool_call_tag() {
    let response = ChatResponse {
        text: Some("<tool_call>\n</tool_call>\nSome text".into()),
        tool_calls: vec![],
        usage: None,
    };

    let dispatcher = XmlToolDispatcher;
    let (text, calls) = dispatcher.parse_response(&response);
    assert!(calls.is_empty());
    assert!(text.contains("Some text"));
}

#[test]
fn xml_dispatcher_handles_unclosed_tool_call() {
    let response = ChatResponse {
        text: Some("Before\n<tool_call>\n{\"name\": \"shell\"}".into()),
        tool_calls: vec![],
        usage: None,
    };

    let dispatcher = XmlToolDispatcher;
    let (text, calls) = dispatcher.parse_response(&response);
    // Should not panic; robust parser recovers the JSON tool call.
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert!(text.contains("Before"));
}

// ═══════════════════════════════════════════════════════════════════════════
// 20. ConversationMessage serialization round-trip
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn conversation_message_serialization_roundtrip() {
    let messages = vec![
        ConversationMessage::Chat(ChatMessage::system("system")),
        ConversationMessage::Chat(ChatMessage::user("hello")),
        ConversationMessage::AssistantToolCalls {
            text: Some("checking".into()),
            tool_calls: vec![ToolCall {
                id: "tc1".into(),
                name: "shell".into(),
                arguments: "{}".into(),
            }],
        },
        ConversationMessage::ToolResults(vec![ToolResultMessage {
            tool_call_id: "tc1".into(),
            content: "ok".into(),
        }]),
        ConversationMessage::Chat(ChatMessage::assistant("done")),
    ];

    for msg in &messages {
        let json = serde_json::to_string(msg).unwrap();
        let parsed: ConversationMessage = serde_json::from_str(&json).unwrap();

        // Verify the variant type matches
        match (msg, &parsed) {
            (ConversationMessage::Chat(a), ConversationMessage::Chat(b)) => {
                assert_eq!(a.role, b.role);
                assert_eq!(a.content, b.content);
            }
            (
                ConversationMessage::AssistantToolCalls {
                    text: a_text,
                    tool_calls: a_calls,
                },
                ConversationMessage::AssistantToolCalls {
                    text: b_text,
                    tool_calls: b_calls,
                },
            ) => {
                assert_eq!(a_text, b_text);
                assert_eq!(a_calls.len(), b_calls.len());
            }
            (ConversationMessage::ToolResults(a), ConversationMessage::ToolResults(b)) => {
                assert_eq!(a.len(), b.len());
            }
            _ => panic!("Variant mismatch after serialization"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 21. Tool dispatcher format_results
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn xml_format_results_includes_status_and_output() {
    let dispatcher = XmlToolDispatcher;
    let results = vec![
        ToolExecutionResult {
            name: "shell".into(),
            output: "file1.txt\nfile2.txt".into(),
            success: true,
            tool_call_id: None,
        },
        ToolExecutionResult {
            name: "file_read".into(),
            output: "Error: file not found".into(),
            success: false,
            tool_call_id: None,
        },
    ];

    let msg = dispatcher.format_results(&results);
    let content = match msg {
        ConversationMessage::Chat(c) => c.content,
        _ => panic!("Expected Chat variant"),
    };

    assert!(content.contains("shell"));
    assert!(content.contains("file1.txt"));
    assert!(content.contains("ok"));
    assert!(content.contains("file_read"));
    assert!(content.contains("error"));
}

#[test]
fn native_format_results_maps_tool_call_ids() {
    let dispatcher = NativeToolDispatcher;
    let results = vec![
        ToolExecutionResult {
            name: "a".into(),
            output: "out1".into(),
            success: true,
            tool_call_id: Some("tc-001".into()),
        },
        ToolExecutionResult {
            name: "b".into(),
            output: "out2".into(),
            success: true,
            tool_call_id: Some("tc-002".into()),
        },
    ];

    let msg = dispatcher.format_results(&results);
    match msg {
        ConversationMessage::ToolResults(r) => {
            assert_eq!(r.len(), 2);
            assert_eq!(r[0].tool_call_id, "tc-001");
            assert_eq!(r[0].content, "out1");
            assert_eq!(r[1].tool_call_id, "tc-002");
            assert_eq!(r[1].content, "out2");
        }
        _ => panic!("Expected ToolResults"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 22. to_provider_messages conversion
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn xml_dispatcher_converts_history_to_provider_messages() {
    let dispatcher = XmlToolDispatcher;
    let history = vec![
        ConversationMessage::Chat(ChatMessage::system("sys")),
        ConversationMessage::Chat(ChatMessage::user("hi")),
        ConversationMessage::AssistantToolCalls {
            text: Some("checking".into()),
            tool_calls: vec![ToolCall {
                id: "tc1".into(),
                name: "shell".into(),
                arguments: "{}".into(),
            }],
        },
        ConversationMessage::ToolResults(vec![ToolResultMessage {
            tool_call_id: "tc1".into(),
            content: "ok".into(),
        }]),
        ConversationMessage::Chat(ChatMessage::assistant("done")),
    ];

    let messages = dispatcher.to_provider_messages(&history);

    // Should have: system, user, assistant (from tool calls), user (tool results), assistant
    assert!(messages.len() >= 4);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].role, "user");
}

#[test]
fn native_dispatcher_converts_tool_results_to_tool_messages() {
    let dispatcher = NativeToolDispatcher;
    let history = vec![ConversationMessage::ToolResults(vec![
        ToolResultMessage {
            tool_call_id: "tc1".into(),
            content: "output1".into(),
        },
        ToolResultMessage {
            tool_call_id: "tc2".into(),
            content: "output2".into(),
        },
    ])];

    let messages = dispatcher.to_provider_messages(&history);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "tool");
    assert_eq!(messages[1].role, "tool");
}

// ═══════════════════════════════════════════════════════════════════════════
// 23. XML tool instructions generation
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn xml_dispatcher_generates_tool_instructions() {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let dispatcher = XmlToolDispatcher;
    let instructions = dispatcher.prompt_instructions(&tools);

    assert!(instructions.contains("## Tool Use Protocol"));
    assert!(instructions.contains("<tool_call>"));
    assert!(instructions.contains("echo"));
    assert!(instructions.contains("Echoes the input"));
}

#[test]
fn native_dispatcher_prompt_instructions_are_protocol_only_not_tool_catalog() {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let dispatcher = NativeToolDispatcher;
    let instructions = dispatcher.prompt_instructions(&tools);
    assert!(instructions.contains("## Tool Use Protocol"));
    assert!(instructions.contains("native tool-calling"));
    assert!(
        !instructions.contains("echo"),
        "native dispatcher does not inline tool names/schemas; provider sends tool specs when should_send_tool_specs is true"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 24. Clear history
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn clear_history_resets_conversation() {
    let provider = Box::new(ScriptedProvider::new(vec![
        text_response("first"),
        text_response("second"),
    ]));

    let (mut agent, _tmp) = build_agent_with(provider, vec![], Box::new(NativeToolDispatcher));

    let _ = agent.turn("hi").await.unwrap();
    assert!(!agent.history().is_empty());

    agent.clear_history();
    assert!(agent.history().is_empty());

    // Next turn should re-inject system prompt
    let _ = agent.turn("hello again").await.unwrap();
    assert!(matches!(
        &agent.history()[0],
        ConversationMessage::Chat(c) if c.role == "system"
    ));
}

// ═══════════════════════════════════════════════════════════════════════════
// 25. run_single delegates to turn
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn run_single_delegates_to_turn() {
    let provider = Box::new(ScriptedProvider::new(vec![text_response("via run_single")]));
    let (mut agent, _tmp) = build_agent_with(provider, vec![], Box::new(NativeToolDispatcher));

    let response = agent.run_single("test").await.unwrap();
    assert!(
        !response.is_empty(),
        "Expected non-empty response from run_single"
    );
}
