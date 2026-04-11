use crate::openhuman::channels::{traits, Channel, SendMessage};
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
use crate::openhuman::providers::{ChatMessage, Provider};
use crate::openhuman::tools::{Tool, ToolResult};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

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
    std::fs::write(tmp.path().join("USER.md"), "# User\nName: Test User").unwrap();
    std::fs::write(
        tmp.path().join("HEARTBEAT.md"),
        "# Heartbeat\nCheck status.",
    )
    .unwrap();
    std::fs::write(tmp.path().join("MEMORY.md"), "# Memory\nUser likes Rust.").unwrap();
    tmp
}

pub(super) struct DummyProvider;

#[async_trait::async_trait]
impl Provider for DummyProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("ok".to_string())
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

pub(super) struct SlowProvider {
    pub(super) delay: Duration,
}

#[async_trait::async_trait]
impl Provider for SlowProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        tokio::time::sleep(self.delay).await;
        Ok(format!("echo: {message}"))
    }
}

pub(super) struct ToolCallingProvider;

pub(super) fn tool_call_payload() -> String {
    r#"<tool_call>
{"name":"mock_price","arguments":{"symbol":"BTC"}}
</tool_call>"#
        .to_string()
}

pub(super) fn tool_call_payload_with_alias_tag() -> String {
    r#"<toolcall>
{"name":"mock_price","arguments":{"symbol":"BTC"}}
</toolcall>"#
        .to_string()
}

#[async_trait::async_trait]
impl Provider for ToolCallingProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok(tool_call_payload())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        let has_tool_results = messages
            .iter()
            .any(|msg| msg.role == "user" && msg.content.contains("[Tool results]"));
        if has_tool_results {
            Ok("BTC is currently around $65,000 based on latest tool output.".to_string())
        } else {
            Ok(tool_call_payload())
        }
    }
}

pub(super) struct ToolCallingAliasProvider;

#[async_trait::async_trait]
impl Provider for ToolCallingAliasProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok(tool_call_payload_with_alias_tag())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        let has_tool_results = messages
            .iter()
            .any(|msg| msg.role == "user" && msg.content.contains("[Tool results]"));
        if has_tool_results {
            Ok("BTC alias-tag flow resolved to final text output.".to_string())
        } else {
            Ok(tool_call_payload_with_alias_tag())
        }
    }
}

pub(super) struct IterativeToolProvider {
    pub(super) required_tool_iterations: usize,
}

impl IterativeToolProvider {
    pub(super) fn completed_tool_iterations(messages: &[ChatMessage]) -> usize {
        messages
            .iter()
            .filter(|msg| msg.role == "user" && msg.content.contains("[Tool results]"))
            .count()
    }
}

#[async_trait::async_trait]
impl Provider for IterativeToolProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok(tool_call_payload())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        let completed_iterations = Self::completed_tool_iterations(messages);
        if completed_iterations >= self.required_tool_iterations {
            Ok(format!(
                "Completed after {completed_iterations} tool iterations."
            ))
        } else {
            Ok(tool_call_payload())
        }
    }
}

#[derive(Default)]
pub(super) struct HistoryCaptureProvider {
    pub(super) calls: Mutex<Vec<Vec<(String, String)>>>,
}

#[async_trait::async_trait]
impl Provider for HistoryCaptureProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("fallback".to_string())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        let snapshot = messages
            .iter()
            .map(|m| (m.role.clone(), m.content.clone()))
            .collect::<Vec<_>>();
        let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
        calls.push(snapshot);
        Ok(format!("response-{}", calls.len()))
    }
}

pub(super) struct MockPriceTool;

#[derive(Default)]
pub(super) struct ModelCaptureProvider {
    pub(super) call_count: AtomicUsize,
    pub(super) models: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Provider for ModelCaptureProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("fallback".to_string())
    }

    async fn chat_with_history(
        &self,
        _messages: &[ChatMessage],
        model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(model.to_string());
        Ok("ok".to_string())
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
        Ok(Vec::new())
    }

    async fn get(&self, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
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
