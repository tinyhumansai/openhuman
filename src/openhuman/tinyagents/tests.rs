//! End-to-end tests for the `tinyagents` harness route: a real openhuman
//! [`Provider`] and [`Tool`] driven through [`run_turn_via_tinyagents`].

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use super::*;
use crate::openhuman::inference::provider::{ChatRequest, ChatResponse, Provider, ToolCall};
use crate::openhuman::tools::{Tool, ToolResult};

/// A real openhuman tool the harness will execute.
struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echoes its msg argument"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "msg": { "type": "string" } },
            "required": ["msg"]
        })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let m = args.get("msg").and_then(|v| v.as_str()).unwrap_or("");
        Ok(ToolResult::success(format!("echoed:{m}")))
    }
}

/// Mock provider: first call requests the echo tool, second call answers.
struct EchoThenDone {
    calls: AtomicUsize,
}

#[async_trait]
impl Provider for EchoThenDone {
    async fn chat_with_system(
        &self,
        _s: Option<&str>,
        _m: &str,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<String> {
        Ok(String::new())
    }
    async fn chat(
        &self,
        _r: ChatRequest<'_>,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<ChatResponse> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            Ok(ChatResponse {
                tool_calls: vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "echo".to_string(),
                    arguments: r#"{"msg":"hi"}"#.to_string(),
                    extra_content: None,
                }],
                ..Default::default()
            })
        } else {
            Ok(ChatResponse {
                text: Some("all done".to_string()),
                ..Default::default()
            })
        }
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn turn_runs_through_the_tinyagents_harness_with_real_tools() {
    let provider = Arc::new(EchoThenDone {
        calls: AtomicUsize::new(0),
    });
    let history = vec![ChatMessage::user("please echo hi")];
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(EchoTool)];

    let outcome = run_turn_via_tinyagents(provider, "mock-model", 0.0, history, tools, 10)
        .await
        .expect("tinyagents harness turn runs");

    assert_eq!(outcome.text, "all done");
    assert!(outcome.model_calls >= 2, "expected >=2 model calls");
    assert!(outcome.tool_calls >= 1, "expected the echo tool to run");
    assert!(
        outcome
            .history
            .iter()
            .any(|m| m.content.contains("echoed:hi")),
        "tool result should be threaded into the transcript: {:?}",
        outcome.history
    );
}

/// A provider that streams visible text in chunks through the request's stream
/// sender, then returns the aggregated reply — exercising `ProviderModel::stream`.
struct StreamingProvider;

#[async_trait]
impl Provider for StreamingProvider {
    async fn chat_with_system(
        &self,
        _s: Option<&str>,
        _m: &str,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<String> {
        Ok(String::new())
    }
    async fn chat(
        &self,
        r: ChatRequest<'_>,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<ChatResponse> {
        use crate::openhuman::inference::provider::{ProviderDelta, UsageInfo};
        if let Some(tx) = r.stream {
            for chunk in ["Hel", "lo ", "world"] {
                let _ = tx
                    .send(ProviderDelta::TextDelta {
                        delta: chunk.to_string(),
                    })
                    .await;
            }
        }
        Ok(ChatResponse {
            text: Some("Hello world".to_string()),
            usage: Some(UsageInfo {
                input_tokens: 12,
                output_tokens: 4,
                ..Default::default()
            }),
            ..Default::default()
        })
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn streaming_path_forwards_text_deltas_and_cost() {
    use crate::openhuman::agent::progress::AgentProgress;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgress>(64);
    let registry: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![]);
    let history = vec![ChatMessage::user("hi")];

    let outcome = run_turn_via_tinyagents_shared(
        Arc::new(StreamingProvider),
        "mock-model",
        0.0,
        history,
        vec![registry],
        std::collections::HashSet::new(),
        4,
        Some(tx),
        None,
        None,
        None,
        &[],
        false,
        None,
        TurnContextMiddleware::defaults(),
        None,
    )
    .await
    .expect("streaming turn runs");

    assert_eq!(outcome.text, "Hello world");
    assert_eq!((outcome.input_tokens, outcome.output_tokens), (12, 4));

    // Collect the mirrored progress: incremental text deltas + a cost update.
    let mut text = String::new();
    let mut saw_cost = false;
    while let Ok(p) = rx.try_recv() {
        match p {
            AgentProgress::TextDelta { delta, .. } => text.push_str(&delta),
            AgentProgress::TurnCostUpdated { input_tokens, .. } => {
                assert_eq!(input_tokens, 12);
                saw_cost = true;
            }
            _ => {}
        }
    }
    assert!(
        text.contains("Hello world"),
        "incremental text deltas should reassemble the reply, got {text:?}"
    );
    assert!(saw_cost, "a TurnCostUpdated should be emitted");
}

/// A provider that records the messages of every request it receives.
struct CapturingProvider {
    captured: std::sync::Mutex<Vec<Vec<ChatMessage>>>,
}

#[async_trait]
impl Provider for CapturingProvider {
    async fn chat_with_system(
        &self,
        _s: Option<&str>,
        _m: &str,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<String> {
        Ok(String::new())
    }
    async fn chat(
        &self,
        r: ChatRequest<'_>,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.captured.lock().unwrap().push(r.messages.to_vec());
        Ok(ChatResponse {
            text: Some("acknowledged".to_string()),
            ..Default::default()
        })
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn pre_queued_steer_message_is_injected_into_the_request() {
    use crate::openhuman::agent::harness::run_queue::{QueueMode, QueuedMessage, RunQueue};

    let provider = Arc::new(CapturingProvider {
        captured: std::sync::Mutex::new(Vec::new()),
    });
    let run_queue = RunQueue::new();
    run_queue
        .push(QueuedMessage {
            text: "switch focus to memory safety".into(),
            mode: QueueMode::Steer,
            client_id: "steer".into(),
            thread_id: "t1".into(),
            queued_at_ms: 0,
            model_override: None,
            temperature: None,
            profile_id: None,
            locale: None,
        })
        .await;

    let registry: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![]);
    let outcome = run_turn_via_tinyagents_shared(
        provider.clone(),
        "mock-model",
        0.0,
        vec![ChatMessage::user("investigate the bug")],
        vec![registry],
        std::collections::HashSet::new(),
        4,
        None,
        None,
        None,
        Some(run_queue),
        &[],
        false,
        None,
        TurnContextMiddleware::defaults(),
        None,
    )
    .await
    .expect("steered turn runs");

    assert_eq!(outcome.text, "acknowledged");
    let captured = provider.captured.lock().unwrap();
    let steered = captured
        .iter()
        .flatten()
        .any(|m| m.role == "user" && m.content.contains("switch focus to memory safety"));
    assert!(
        steered,
        "the queued steer should be injected as a user turn, got: {:?}",
        captured
            .iter()
            .flatten()
            .map(|m| (&m.role, &m.content))
            .collect::<Vec<_>>()
    );
}

/// A provider that pops distinct scripted texts from a shared FIFO, recording
/// the order of consumption — models the global mock the parallel children share.
struct FifoProvider {
    responses: std::sync::Mutex<std::collections::VecDeque<String>>,
    calls: AtomicUsize,
}

#[async_trait]
impl Provider for FifoProvider {
    async fn chat_with_system(
        &self,
        _s: Option<&str>,
        _m: &str,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<String> {
        Ok(String::new())
    }
    async fn chat(
        &self,
        _r: ChatRequest<'_>,
        _model: &str,
        _t: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // Yield once so two concurrent turns on the same task actually interleave.
        tokio::task::yield_now().await;
        let text = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default();
        Ok(ChatResponse {
            text: Some(text),
            ..Default::default()
        })
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
}

/// Two sub-agent-style turns (`pause_at_cap = true`) running concurrently on the
/// *same task* (as `spawn_parallel_agents` does via `join_all`) must each get a
/// distinct FIFO response and not deadlock — the `parallel_subagent_fanout`
/// regression in miniature.
#[tokio::test]
async fn concurrent_shared_turns_each_get_a_distinct_result() {
    let provider = Arc::new(FifoProvider {
        responses: std::sync::Mutex::new(
            ["AAA_CANARY".to_string(), "BBB_CANARY".to_string()].into(),
        ),
        calls: AtomicUsize::new(0),
    });
    let registry: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![]);

    let one = run_turn_via_tinyagents_shared(
        provider.clone(),
        "mock-model",
        0.0,
        vec![ChatMessage::user("task one")],
        vec![registry.clone()],
        std::collections::HashSet::new(),
        4,
        None,
        None,
        None,
        None,
        &[],
        true,
        None,
        TurnContextMiddleware::defaults(),
        None,
    );
    let two = run_turn_via_tinyagents_shared(
        provider.clone(),
        "mock-model",
        0.0,
        vec![ChatMessage::user("task two")],
        vec![registry],
        std::collections::HashSet::new(),
        4,
        None,
        None,
        None,
        None,
        &[],
        true,
        None,
        TurnContextMiddleware::defaults(),
        None,
    );

    let (a, b) = tokio::join!(one, two);
    let a = a.expect("turn one runs");
    let b = b.expect("turn two runs");

    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        2,
        "exactly one model call per turn"
    );
    let mut got = [a.text.as_str(), b.text.as_str()];
    got.sort_unstable();
    assert_eq!(
        got,
        ["AAA_CANARY", "BBB_CANARY"],
        "each concurrent turn must receive a distinct FIFO response; got {got:?}"
    );
}
