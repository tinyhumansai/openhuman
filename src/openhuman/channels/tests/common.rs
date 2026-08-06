use crate::openhuman::channels::{traits, Channel, SendMessage};
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
use crate::openhuman::tools::{Tool, ToolResult};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tinyagents::harness::message::{AssistantMessage, Message};
use tinyagents::harness::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinyagents::harness::tool::ToolCall;

fn message_role(message: &Message) -> &'static str {
    match message {
        Message::System(_) => "system",
        Message::User(_) => "user",
        Message::Assistant(_) => "assistant",
        Message::Tool(_) => "tool",
    }
}

fn native_tool_profile() -> &'static ModelProfile {
    static PROFILE: std::sync::OnceLock<ModelProfile> = std::sync::OnceLock::new();
    PROFILE.get_or_init(|| {
        let mut profile = ModelProfile::default();
        profile.tool_calling = true;
        profile.parallel_tool_calls = true;
        profile
    })
}

fn tool_call_response(step: Option<usize>) -> ModelResponse {
    let mut arguments = serde_json::json!({"symbol": "BTC"});
    if let Some(step) = step {
        arguments["step"] = serde_json::json!(step);
    }
    ModelResponse {
        message: AssistantMessage {
            id: None,
            content: Vec::new(),
            tool_calls: vec![ToolCall::new("mock-price-call", "mock_price", arguments)],
            usage: None,
        },
        usage: None,
        finish_reason: Some("tool_calls".to_string()),
        raw: None,
        resolved_model: None,
        continue_turn: None,
    }
}

// Note: the shared bus handler lock and the "install the real agent
// handler for this test" helper both live in
// `crate::openhuman::agent::bus` as `BUS_HANDLER_LOCK` (re-exported from
// `crate::core::event_bus::testing`) and `use_real_agent_handler` so any
// test in the workspace can drive the real `agent.run_turn` path without
// depending on channels-specific scaffolding.
//
// For stub installations use `mock_agent_run_turn` (also in
// `crate::openhuman::agent::bus`) or the generic `mock_bus_stub` in
// `crate::core::event_bus::testing` for arbitrary bus methods.
pub(super) use crate::openhuman::agent::bus::use_real_agent_handler;

pub(super) fn make_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    // Create minimal workspace files — only the bundled identity prompts
    // plus a MEMORY.md stand-in for what the archivist would write.
    std::fs::write(tmp.path().join("SOUL.md"), "# Soul\nBe helpful.").unwrap();
    std::fs::write(
        tmp.path().join("IDENTITY.md"),
        "# Identity\nName: OpenHuman",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("PROFILE.md"),
        "# User Profile\nName: Test User",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("HEARTBEAT.md"),
        "# Heartbeat\nCheck status.",
    )
    .unwrap();
    std::fs::write(tmp.path().join("MEMORY.md"), "# Memory\nUser likes Rust.").unwrap();
    tmp
}

pub(super) struct DummyModel;

#[async_trait::async_trait]
impl ChatModel<()> for DummyModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        Ok(ModelResponse::assistant("ok"))
    }
}

#[derive(Default)]
pub(super) struct RecordingChannel {
    pub(super) sent_messages: tokio::sync::Mutex<Vec<String>>,
    pub(super) start_typing_calls: AtomicUsize,
    pub(super) stop_typing_calls: AtomicUsize,
}

#[derive(Default)]
pub(super) struct TelegramRecordingChannel {
    pub(super) sent_messages: tokio::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Channel for TelegramRecordingChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.sent_messages
            .lock()
            .await
            .push(format!("{}:{}", message.recipient, message.content));
        Ok(())
    }

    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn start_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl Channel for RecordingChannel {
    fn name(&self) -> &str {
        "test-channel"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.sent_messages
            .lock()
            .await
            .push(format!("{}:{}", message.recipient, message.content));
        Ok(())
    }

    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn start_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        self.start_typing_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
        self.stop_typing_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

pub(super) struct SlowModel {
    pub(super) delay: Duration,
}

#[async_trait::async_trait]
impl ChatModel<()> for SlowModel {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        tokio::time::sleep(self.delay).await;
        let message = request
            .messages
            .iter()
            .rev()
            .find(|message| matches!(message, Message::User(_)))
            .map(Message::text)
            .unwrap_or_default();
        Ok(ModelResponse::assistant(format!("echo: {message}")))
    }
}

pub(super) struct ToolCallingModel;

#[async_trait::async_trait]
impl ChatModel<()> for ToolCallingModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(native_tool_profile())
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        let has_tool_results = request.messages.iter().any(|message| {
            matches!(message, Message::Tool(_)) || message.text().contains("[Tool results]")
        });
        if has_tool_results {
            Ok(ModelResponse::assistant(
                "BTC is currently around $65,000 based on latest tool output.",
            ))
        } else {
            Ok(tool_call_response(None))
        }
    }
}

pub(super) struct IterativeToolModel {
    pub(super) required_tool_iterations: usize,
}

impl IterativeToolModel {
    pub(super) fn completed_tool_iterations(messages: &[Message]) -> usize {
        messages
            .iter()
            .filter(|message| {
                matches!(message, Message::Tool(_)) || message.text().contains("[Tool results]")
            })
            .count()
    }
}

#[async_trait::async_trait]
impl ChatModel<()> for IterativeToolModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(native_tool_profile())
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        let completed_iterations = Self::completed_tool_iterations(&request.messages);
        if completed_iterations >= self.required_tool_iterations {
            Ok(ModelResponse::assistant(format!(
                "Completed after {completed_iterations} tool iterations."
            )))
        } else {
            // Prefix a per-iteration progress note so each turn's assistant
            // output is distinct. A healthy multi-step agent varies its
            // narration as it advances; only byte-identical repeats (the
            // degeneration signature) should trip the harness repeat guard.
            Ok(tool_call_response(Some(completed_iterations)))
        }
    }
}

#[derive(Default)]
pub(super) struct HistoryCaptureModel {
    pub(super) calls: Mutex<Vec<Vec<(String, String)>>>,
}

#[async_trait::async_trait]
impl ChatModel<()> for HistoryCaptureModel {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        let snapshot = request
            .messages
            .iter()
            .map(|message| (message_role(message).to_string(), message.text()))
            .collect::<Vec<_>>();
        let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
        calls.push(snapshot);
        Ok(ModelResponse::assistant(format!(
            "response-{}",
            calls.len()
        )))
    }
}

pub(super) struct MockPriceTool;

#[derive(Default)]
pub(super) struct ModelCaptureModel {
    pub(super) call_count: AtomicUsize,
    pub(super) models: Mutex<Vec<String>>,
    label: String,
}

impl ModelCaptureModel {
    pub(super) fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }
}

#[async_trait::async_trait]
impl ChatModel<()> for ModelCaptureModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(self.label.clone());
        Ok(ModelResponse::assistant("ok"))
    }
}

#[async_trait::async_trait]
impl Tool for MockPriceTool {
    fn name(&self) -> &str {
        "mock_price"
    }

    fn description(&self) -> &str {
        "Return a mocked BTC price"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "symbol": { "type": "string" }
            },
            "required": ["symbol"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let symbol = args.get("symbol").and_then(serde_json::Value::as_str);
        if symbol != Some("BTC") {
            return Ok(ToolResult::error("unexpected symbol"));
        }

        Ok(ToolResult::success("BTC is $65,000"))
    }
}

pub(super) struct NoopMemory;

#[async_trait::async_trait]
impl Memory for NoopMemory {
    fn name(&self) -> &str {
        "noop"
    }

    async fn store(
        &self,
        _namespace: &str,
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
        _opts: crate::openhuman::memory::RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(
        &self,
    ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

pub(super) struct AlwaysFailChannel {
    pub(super) name: &'static str,
    pub(super) calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Channel for AlwaysFailChannel {
    fn name(&self) -> &str {
        self.name
    }

    async fn send(&self, _message: &SendMessage) -> anyhow::Result<()> {
        Ok(())
    }

    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
    ) -> anyhow::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("listen boom")
    }
}
