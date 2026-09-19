use super::*;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use tinyinference_llm::message::{AssistantMessage, MessageDelta};
use tinyinference_llm::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tinyinference_llm::tool::ToolCall;
use tinytools::ToolResult;
use tinytools::{Tool, ToolSpec};

fn native_tool_profile() -> &'static ModelProfile {
    static PROFILE: std::sync::LazyLock<ModelProfile> = std::sync::LazyLock::new(|| ModelProfile {
        provider: Some("subagent-graph-test".to_string()),
        tool_calling: true,
        parallel_tool_calls: true,
        streaming: true,
        ..ModelProfile::default()
    });
    &PROFILE
}

fn tool_response(id: &str, name: &str, arguments: serde_json::Value) -> ModelResponse {
    ModelResponse {
        message: AssistantMessage {
            id: None,
            content: Vec::new(),
            tool_calls: vec![ToolCall::new(id, name, arguments)],
            usage: None,
        },
        usage: None,
        finish_reason: Some("tool_calls".to_string()),
        raw: None,
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
        correlation: None,
        resolved_route: None,
    }
}

struct EchoTool;
#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echo"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let m = args.get("msg").and_then(|v| v.as_str()).unwrap_or("");
        Ok(ToolResult::success(format!("echoed:{m}")))
    }
}

struct TwoStepProvider {
    calls: AtomicUsize,
}
#[async_trait]
impl ChatModel<()> for TwoStepProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(native_tool_profile())
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            Ok(tool_response("1", "echo", serde_json::json!({"msg": "hi"})))
        } else {
            Ok(ModelResponse::assistant("all done"))
        }
    }
}

#[path = "graph_failed_run_tests.rs"]
mod graph_failed_run_tests;

#[test]
fn parallel_worker_without_thread_keeps_parent_transcript_affinity() {
    assert_eq!(
        inherited_thread_id(Some("parent-thread".to_string()), None),
        Some("parent-thread".to_string())
    );
    assert_eq!(
        inherited_thread_id(
            Some("parent-thread".to_string()),
            Some("task-thread".to_string())
        ),
        Some("task-thread".to_string())
    );
}

#[tokio::test]
async fn explicit_worker_thread_replaces_parent_for_model_run_and_transcript() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut parent = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    parent.thread_id = Some("parent-thread".to_owned());
    let mut history = vec![ChatMessage::user("finish this")];

    run_subagent_via_graph(
        crate::agent::tinyagents::TurnModelSource::from_model(Arc::new(TwoStepProvider {
            calls: AtomicUsize::new(1),
        })),
        "mock-model",
        0.0,
        &mut history,
        Arc::new(vec![]),
        vec![],
        vec![],
        HashSet::new(),
        2,
        None,
        None,
        "researcher",
        "worker-task",
        false,
        Some("worker-thread".to_owned()),
        parent,
        None,
        workspace.path().to_path_buf(),
        None,
        1024,
        false,
        "root__worker-thread",
        "test",
        None,
        AgentTokenjuiceCompression::Off,
        None,
    )
    .await
    .expect("worker run");

    let path = tinyagents_session::transcript::resolve_keyed_transcript_path(
        workspace.path(),
        "root__worker-thread",
    )
    .expect("transcript path");
    let persisted =
        tinyagents_session::transcript::read_transcript(&path).expect("worker transcript");
    assert_eq!(persisted.meta.thread_id.as_deref(), Some("worker-thread"));
    assert_ne!(persisted.meta.thread_id.as_deref(), Some("parent-thread"));
}

#[tokio::test]
async fn subagent_runs_through_the_graph_engine_with_real_tools() {
    let provider = Arc::new(TwoStepProvider {
        calls: AtomicUsize::new(0),
    });
    let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(EchoTool)]);
    let mut allowed = HashSet::new();
    allowed.insert("echo".to_string());
    let mut history = vec![ChatMessage::user("please echo hi")];

    let (output, iterations, usage, early_exit, hit_cap, _breaker) = run_subagent_via_graph(
        crate::agent::tinyagents::TurnModelSource::from_model(provider),
        "mock-model",
        0.0,
        &mut history,
        parent_tools,
        vec![],
        vec![],
        allowed,
        10,
        None,
        None,
        "researcher",
        "task-1",
        false,
        None,
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
        None,
        std::env::temp_dir(),
        None,
        1024,
        false,
        "root-session__real_tools",
        "mock-channel",
        None,
        AgentTokenjuiceCompression::Off,
        // No host config in tests: the graph takes byte-cap-only
        // context defaults instead of reading the developer machine's
        // real config.toml, which is what the old in-graph load did.
        None,
    )
    .await
    .expect("graph subagent runs");

    assert_eq!(output, "all done");
    assert_eq!(iterations, 2);
    assert!(early_exit.is_none());
    assert!(!hit_cap, "a clean finish should not report a cap hit");
    let _ = usage;
    // History was written back: user + assistant(tool) + tool result + assistant(final).
    assert!(history.len() >= 4);
    assert!(history.iter().any(|m| m.content.contains("echoed:hi")));
}

/// A provider that streams visible text + reasoning through the request's
/// delta sender, exercising the child-progress bridge end to end.
struct ThinkingStreamProvider;
#[async_trait]
impl ChatModel<()> for ThinkingStreamProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(native_tool_profile())
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        Ok(ModelResponse::assistant("Hello"))
    }

    async fn stream(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelStream> {
        let response = ModelResponse::assistant("Hello");
        Ok(ModelStream::new(Box::pin(futures::stream::iter(vec![
            ModelStreamItem::Started,
            ModelStreamItem::MessageDelta(MessageDelta {
                text: String::new(),
                reasoning: "let me think".to_string(),
                tool_call: None,
            }),
            ModelStreamItem::MessageDelta(MessageDelta::text("Hel")),
            ModelStreamItem::MessageDelta(MessageDelta::text("lo")),
            ModelStreamItem::Completed(response),
        ]))))
    }
}

#[tokio::test]
async fn child_text_and_thinking_deltas_are_scoped_to_the_subagent() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgress>(64);
    let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![]);
    let mut history = vec![ChatMessage::user("hi")];

    let (output, _iters, _usage, _early, _hit_cap, _breaker) = run_subagent_via_graph(
        crate::agent::tinyagents::TurnModelSource::from_model(Arc::new(ThinkingStreamProvider)),
        "mock-model",
        0.0,
        &mut history,
        parent_tools,
        vec![],
        vec![],
        HashSet::new(),
        4,
        None,
        Some(tx),
        "researcher",
        "task-7",
        false,
        None,
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
        None,
        std::env::temp_dir(),
        None,
        1024,
        false,
        "root-session__scoped_deltas",
        "mock-channel",
        None,
        AgentTokenjuiceCompression::Off,
        // No host config in tests: the graph takes byte-cap-only
        // context defaults instead of reading the developer machine's
        // real config.toml, which is what the old in-graph load did.
        None,
    )
    .await
    .expect("child-delta subagent runs");

    assert_eq!(output, "Hello");

    let mut text = String::new();
    let mut thinking = String::new();
    let mut saw_iter = false;
    while let Ok(p) = rx.try_recv() {
        match p {
            AgentProgress::SubagentTextDelta { delta, task_id, .. } => {
                assert_eq!(task_id, "task-7");
                text.push_str(&delta);
            }
            AgentProgress::SubagentThinkingDelta {
                delta, agent_id, ..
            } => {
                assert_eq!(agent_id, "researcher");
                thinking.push_str(&delta);
            }
            AgentProgress::SubagentIterationStarted { task_id, .. } => {
                assert_eq!(task_id, "task-7");
                saw_iter = true;
            }
            // The parent-scoped variants must never appear on a child run.
            AgentProgress::TextDelta { .. }
            | AgentProgress::ThinkingDelta { .. }
            | AgentProgress::IterationStarted { .. } => {
                panic!("child run emitted a parent-scoped progress event");
            }
            _ => {}
        }
    }
    assert!(saw_iter, "a SubagentIterationStarted should be emitted");
    assert!(
        text.contains("Hello"),
        "child text deltas should reassemble, got {text:?}"
    );
    assert!(
        thinking.contains("let me think"),
        "child thinking deltas should be forwarded, got {thinking:?}"
    );
}

/// A tool named like the early-exit tool that echoes its `question` arg.
struct AskTool;
#[async_trait]
impl Tool for AskTool {
    fn name(&self) -> &str {
        "ask_user_clarification"
    }
    fn description(&self) -> &str {
        "ask the user a clarifying question"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {"question": {"type": "string"}}})
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let q = args
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(ToolResult::success(q))
    }
}

/// A provider whose first turn calls `ask_user_clarification`; a second turn
/// would answer, but the early-exit pause should stop the loop before it.
struct AskThenAnswer {
    calls: AtomicUsize,
}
#[async_trait]
impl ChatModel<()> for AskThenAnswer {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(native_tool_profile())
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            Ok(tool_response(
                "ask-1",
                "ask_user_clarification",
                serde_json::json!({"question": "which file?"}),
            ))
        } else {
            Ok(ModelResponse::assistant("should not be reached"))
        }
    }
}

#[tokio::test]
async fn ask_user_clarification_pauses_and_surfaces_the_question() {
    let provider = Arc::new(AskThenAnswer {
        calls: AtomicUsize::new(0),
    });
    let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(AskTool)]);
    let mut allowed = HashSet::new();
    allowed.insert("ask_user_clarification".to_string());
    let mut history = vec![ChatMessage::user("help me")];

    let (output, iterations, _usage, early_exit, _hit_cap, _breaker) = run_subagent_via_graph(
        crate::agent::tinyagents::TurnModelSource::from_model(provider.clone()),
        "mock-model",
        0.0,
        &mut history,
        parent_tools,
        vec![],
        vec![],
        allowed,
        10,
        None,
        None,
        "researcher",
        "task-9",
        false,
        None,
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
        None,
        std::env::temp_dir(),
        None,
        1024,
        false,
        "root-session__clarification",
        "mock-channel",
        None,
        AgentTokenjuiceCompression::Off,
        // No host config in tests: the graph takes byte-cap-only
        // context defaults instead of reading the developer machine's
        // real config.toml, which is what the old in-graph load did.
        None,
    )
    .await
    .expect("ask-clarification subagent runs");

    // The loop paused after the tool round: the early-exit tool is surfaced
    // and the question is the returned text — the second model turn never ran.
    assert_eq!(early_exit.as_deref(), Some("ask_user_clarification"));
    assert_eq!(output, "which file?");
    assert_eq!(
        iterations, 1,
        "the loop should pause before a second model call"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

/// A tool that always succeeds, so the loop keeps going until the cap.
struct NoopTool;
#[async_trait]
impl Tool for NoopTool {
    fn name(&self) -> &str {
        "noop"
    }
    fn description(&self) -> &str {
        "no-op"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(&self, _a: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
}

/// A provider that never finishes: every tool-enabled turn asks for `noop`.
/// A request with no tools is the cap-hit summary call — it returns prose.
struct LoopForeverProvider;
#[async_trait]
impl ChatModel<()> for LoopForeverProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(native_tool_profile())
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        if !request.tools.is_empty() {
            Ok(tool_response("n", "noop", serde_json::json!({})))
        } else {
            // The summary call (tools=None): return a progress checkpoint.
            Ok(ModelResponse::assistant("progress: explored two leads"))
        }
    }
}

#[tokio::test]
async fn cap_hit_summarizes_a_resumable_checkpoint() {
    let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(NoopTool)]);
    let mut allowed = HashSet::new();
    allowed.insert("noop".to_string());
    let mut history = vec![ChatMessage::user("do a big task")];

    let (output, iterations, _usage, early_exit, hit_cap, _breaker) = run_subagent_via_graph(
        crate::agent::tinyagents::TurnModelSource::from_model(Arc::new(LoopForeverProvider)),
        "mock-model",
        0.0,
        &mut history,
        parent_tools,
        vec![],
        vec![],
        allowed,
        2,
        None,
        None,
        "researcher",
        "task-cap",
        false,
        None,
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
        None,
        std::env::temp_dir(),
        None,
        1024,
        false,
        "root-session__cap_hit",
        "mock-channel",
        None,
        AgentTokenjuiceCompression::Off,
        // No host config in tests: the graph takes byte-cap-only
        // context defaults instead of reading the developer machine's
        // real config.toml, which is what the old in-graph load did.
        None,
    )
    .await
    .expect("cap-hit subagent runs");

    // The loop paused at the 2-call budget and summarized instead of erroring.
    assert!(early_exit.is_none());
    assert!(hit_cap, "reaching the model-call cap should report hit_cap");
    assert_eq!(iterations, 2, "the loop should stop at the model-call cap");
    assert!(
        output.contains("progress: explored two leads"),
        "cap hit should return the summary checkpoint, got {output:?}"
    );
}

// ── Spawn-tool refusal on a sub-agent run (#6157, #4452) ────────────────

/// Collects the messages of `WARN` events whose text contains a marker, so a
/// test can assert on a log line that has no other observable side effect.
#[path = "graph_policy_tests.rs"]
mod graph_policy_tests;
