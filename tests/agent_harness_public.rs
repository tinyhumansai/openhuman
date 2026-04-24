use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::harness::{
    check_interrupt, current_fork, current_parent, with_fork_context, with_parent_context,
    ForkContext, InterruptFence, ParentExecutionContext,
};
use openhuman_core::openhuman::agent::hooks::{
    fire_hooks, sanitize_tool_output, PostTurnHook, ToolCallRecord, TurnContext,
};
use openhuman_core::openhuman::config::AgentConfig;
use openhuman_core::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
use openhuman_core::openhuman::providers::{ChatMessage, ChatRequest, ChatResponse, Provider};
use parking_lot::Mutex;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::Notify;

struct StubProvider;

#[async_trait]
impl Provider for StubProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok("ok".into())
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        Ok(ChatResponse {
            text: Some("ok".into()),
            tool_calls: Vec::new(),
            usage: None,
        })
    }
}

struct StubMemory;

#[async_trait]
impl Memory for StubMemory {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: openhuman_core::openhuman::memory::RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(
        &self,
    ) -> Result<Vec<openhuman_core::openhuman::memory::NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "stub"
    }
}

fn sample_turn() -> TurnContext {
    TurnContext {
        user_message: "hello".into(),
        assistant_response: "world".into(),
        tool_calls: vec![ToolCallRecord {
            name: "shell".into(),
            arguments: serde_json::json!({}),
            success: true,
            output_summary: "ok".into(),
            duration_ms: 10,
        }],
        turn_duration_ms: 15,
        session_id: Some("s1".into()),
        iteration_count: 1,
    }
}

fn stub_parent_context() -> ParentExecutionContext {
    ParentExecutionContext {
        provider: Arc::new(StubProvider),
        all_tools: Arc::new(vec![]),
        all_tool_specs: Arc::new(vec![]),
        model_name: "stub-model".into(),
        temperature: 0.4,
        workspace_dir: std::path::PathBuf::from("/tmp"),
        memory: Arc::new(StubMemory),
        agent_config: AgentConfig::default(),
        skills: Arc::new(vec![]),
        memory_context: Some("ctx".into()),
        session_id: "test-session".into(),
        channel: "test-channel".into(),
        connected_integrations: vec![],
        composio_client: None,
        tool_call_format: openhuman_core::openhuman::context::prompt::ToolCallFormat::PFormat,
        session_key: "test-session".into(),
        session_parent_prefix: None,
    }
}

struct RecordingHook {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
    notify: Arc<Notify>,
    fail: bool,
}

#[async_trait]
impl PostTurnHook for RecordingHook {
    fn name(&self) -> &str {
        self.name
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> Result<()> {
        self.calls
            .lock()
            .push(format!("{}:{}", self.name, ctx.user_message));
        self.notify.notify_waiters();
        if self.fail {
            anyhow::bail!("hook failed");
        }
        Ok(())
    }
}

#[test]
fn interrupt_fence_shares_and_resets_state() {
    let fence = InterruptFence::default();
    assert!(!fence.is_interrupted());
    assert!(check_interrupt(&fence).is_ok());

    let clone = fence.clone();
    let raw = fence.flag_handle();
    fence.trigger();
    assert!(clone.is_interrupted());
    assert!(raw.load(Ordering::Relaxed));
    assert!(check_interrupt(&fence).is_err());

    raw.store(false, Ordering::Relaxed);
    fence.reset();
    assert!(!fence.is_interrupted());
}

#[tokio::test]
async fn interrupt_signal_handler_is_installable() {
    let fence = InterruptFence::new();
    fence.install_signal_handler();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert!(!fence.is_interrupted());
}

#[tokio::test]
async fn fork_and_parent_contexts_are_visible_only_within_scope() {
    assert!(current_parent().is_none());
    assert!(current_fork().is_none());

    let fork = ForkContext {
        system_prompt: Arc::new("hello".into()),
        tool_specs: Arc::new(vec![]),
        message_prefix: Arc::new(vec![ChatMessage::system("hello")]),
        fork_task_prompt: "do thing".into(),
    };

    with_fork_context(fork, async {
        let inner = current_fork().expect("fork context should be visible");
        assert_eq!(*inner.system_prompt, "hello");
        assert_eq!(inner.fork_task_prompt, "do thing");
        assert_eq!(inner.message_prefix.len(), 1);
    })
    .await;

    let parent = stub_parent_context();
    with_parent_context(parent, async {
        let inner = current_parent().expect("parent context should be visible");
        assert_eq!(inner.model_name, "stub-model");
        assert_eq!(inner.session_id, "test-session");
        assert_eq!(inner.channel, "test-channel");
        assert_eq!(inner.memory_context.as_deref(), Some("ctx"));
    })
    .await;

    assert!(current_parent().is_none());
    assert!(current_fork().is_none());
}

#[test]
fn sanitize_tool_output_classifies_common_errors() {
    assert_eq!(
        sanitize_tool_output("fine", "shell", true),
        "shell: ok (4 chars)"
    );
    assert_eq!(
        sanitize_tool_output("Connection timeout while fetching", "http_request", false),
        "http_request: failed (timeout)"
    );
    assert_eq!(
        sanitize_tool_output("permission denied opening file", "file_read", false),
        "file_read: failed (permission_denied)"
    );
    assert_eq!(
        sanitize_tool_output("unknown tool called", "delegate", false),
        "delegate: failed (unknown_tool)"
    );
    assert_eq!(
        sanitize_tool_output("bad syntax in payload", "json", false),
        "json: failed (parse_error)"
    );
    assert_eq!(
        sanitize_tool_output("no such file or directory", "file_read", false),
        "file_read: failed (not_found)"
    );
    assert_eq!(
        sanitize_tool_output("network connection reset by peer", "http_request", false),
        "http_request: failed (connection_error)"
    );
    assert_eq!(
        sanitize_tool_output("something strange happened", "shell", false),
        "shell: failed (error)"
    );
}

#[tokio::test]
async fn fire_hooks_dispatches_all_hooks_even_when_one_fails() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let notify = Arc::new(Notify::new());
    let hooks: Vec<Arc<dyn PostTurnHook>> = vec![
        Arc::new(RecordingHook {
            name: "ok",
            calls: Arc::clone(&calls),
            notify: Arc::clone(&notify),
            fail: false,
        }),
        Arc::new(RecordingHook {
            name: "fail",
            calls: Arc::clone(&calls),
            notify: Arc::clone(&notify),
            fail: true,
        }),
    ];

    fire_hooks(&hooks, sample_turn());

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if calls.lock().len() == 2 {
                break;
            }
            notify.notified().await;
        }
    })
    .await
    .expect("hooks should complete");

    let calls = calls.lock().clone();
    assert!(calls.contains(&"ok:hello".into()));
    assert!(calls.contains(&"fail:hello".into()));
}

#[test]
fn fire_hooks_accepts_empty_hook_lists() {
    fire_hooks(&[], sample_turn());
}
