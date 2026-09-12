use super::*;
use crate::openhuman::tools::ToolResult;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use tinyinference::message::{AssistantMessage, MessageDelta};
use tinyinference::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tinyinference::tool::ToolCall;

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
    ) -> tinyinference::Result<ModelResponse> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            Ok(tool_response("1", "echo", serde_json::json!({"msg": "hi"})))
        } else {
            Ok(ModelResponse::assistant("all done"))
        }
    }
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
        crate::openhuman::agent::tinyagents::TurnModelSource::from_model(provider),
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
    ) -> tinyinference::Result<ModelResponse> {
        Ok(ModelResponse::assistant("Hello"))
    }

    async fn stream(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelStream> {
        let response = ModelResponse::assistant("Hello");
        Ok(Box::pin(futures::stream::iter(vec![
            ModelStreamItem::Started,
            ModelStreamItem::MessageDelta(MessageDelta {
                text: String::new(),
                reasoning: "let me think".to_string(),
                tool_call: None,
            }),
            ModelStreamItem::MessageDelta(MessageDelta::text("Hel")),
            ModelStreamItem::MessageDelta(MessageDelta::text("lo")),
            ModelStreamItem::Completed(response),
        ])))
    }
}

#[tokio::test]
async fn child_text_and_thinking_deltas_are_scoped_to_the_subagent() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgress>(64);
    let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![]);
    let mut history = vec![ChatMessage::user("hi")];

    let (output, _iters, _usage, _early, _hit_cap, _breaker) = run_subagent_via_graph(
        crate::openhuman::agent::tinyagents::TurnModelSource::from_model(Arc::new(
            ThinkingStreamProvider,
        )),
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
    ) -> tinyinference::Result<ModelResponse> {
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
        crate::openhuman::agent::tinyagents::TurnModelSource::from_model(provider.clone()),
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
    ) -> tinyinference::Result<ModelResponse> {
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
        crate::openhuman::agent::tinyagents::TurnModelSource::from_model(Arc::new(
            LoopForeverProvider,
        )),
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
#[derive(Clone, Default)]
struct WarnSink(Arc<std::sync::Mutex<Vec<String>>>);

impl WarnSink {
    /// Every captured message containing `marker`, in emission order.
    fn matching(&self, marker: &str) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.contains(marker))
            .cloned()
            .collect()
    }
}

struct WarnLayer(WarnSink);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for WarnLayer {
    /// Record the `message` field of every `WARN` event; ignore other levels.
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if *event.metadata().level() != tracing::Level::WARN {
            return;
        }
        struct Message(Option<String>);
        impl tracing::field::Visit for Message {
            /// Keep the formatted `message` field and discard the rest.
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = Some(format!("{value:?}"));
                }
            }
        }
        let mut message = Message(None);
        event.record(&mut message);
        if let Some(text) = message.0 {
            self.0 .0.lock().unwrap().push(text);
        }
    }
}

/// Stands in for `spawn_subagent`: records whether it was ever dispatched, so a
/// test can prove the tool was not merely absent from the advertisement but
/// genuinely unreachable.
struct SpawnProbeTool(Arc<std::sync::atomic::AtomicBool>);

#[async_trait]
impl Tool for SpawnProbeTool {
    /// The exact name the #4452 strip must match.
    fn name(&self) -> &str {
        "spawn_subagent"
    }
    /// Irrelevant to the strip, which reads only the name.
    fn description(&self) -> &str {
        "spawn a sub-agent"
    }
    /// Irrelevant to the strip, which reads only the name.
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    /// Records the dispatch. Reaching this line means the test has failed.
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.0.store(true, Ordering::SeqCst);
        Ok(ToolResult::success("spawned"))
    }
}

/// Asks for `spawn_subagent` once, then finishes regardless of the answer.
struct SpawnAttemptProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl ChatModel<()> for SpawnAttemptProvider {
    /// Native tool calling, so the harness advertises tools normally.
    fn profile(&self) -> Option<&ModelProfile> {
        Some(native_tool_profile())
    }

    /// Ask for `spawn_subagent` on the first turn, then finish on the second.
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(tool_response("1", "spawn_subagent", serde_json::json!({})))
        } else {
            Ok(ModelResponse::assistant("all done"))
        }
    }
}

const REFUSAL: &str = "refusing to register spawn/delegate tool";

/// Runs one sub-agent turn whose parent surface carries `spawn_subagent`, with
/// `allowed` deciding whether the allowlist readmits it. Returns
/// (spawn tool executed?, refusal warnings emitted).
async fn run_with_spawn_tool_in_parent_surface(allowed: HashSet<String>) -> (bool, Vec<String>) {
    use tracing_subscriber::layer::SubscriberExt;

    let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sink = WarnSink::default();
    let subscriber = tracing_subscriber::registry().with(WarnLayer(sink.clone()));
    let _subscriber_guard = tracing::subscriber::set_default(subscriber);

    let provider = Arc::new(SpawnAttemptProvider {
        calls: AtomicUsize::new(0),
    });
    let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![
        Box::new(EchoTool),
        Box::new(SpawnProbeTool(executed.clone())),
    ]);
    let mut history = vec![ChatMessage::user("spawn a helper")];

    run_subagent_via_graph(
        crate::openhuman::agent::tinyagents::TurnModelSource::from_model(provider),
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
        "task-6157",
        false,
        None,
        std::env::temp_dir(),
        None,
        1024,
        false,
        "root-session__spawn_refusal",
        "mock-channel",
        None,
        AgentTokenjuiceCompression::Off,
        None,
    )
    .await
    .expect("graph subagent runs");

    (executed.load(Ordering::SeqCst), sink.matching(REFUSAL))
}

/// The real shape: `run_typed_mode` has already stripped `spawn_subagent` from
/// the allowlist, so the registration-time backstop is re-refusing a name that
/// was never admitted. That is the happy path and must not warn (#6157) — the
/// old unconditional warn fired here on every sub-agent run.
#[tokio::test]
async fn a_sub_agent_cannot_reach_a_spawn_tool_and_the_healthy_run_is_quiet() {
    let allowed = HashSet::from(["echo".to_string()]);
    let (executed, warnings) = run_with_spawn_tool_in_parent_surface(allowed).await;

    assert!(
        !executed,
        "a sub-agent must never be able to invoke spawn_subagent (#4452)"
    );
    assert!(
        warnings.is_empty(),
        "an already-excluded spawn tool is the happy path, not a warning: {warnings:?}"
    );
}

/// The misbuilt-allowlist case the backstop exists for: `spawn_subagent` is
/// admitted, so registration must still refuse it *and* say so — this is the one
/// shape in which the warning carries information.
#[tokio::test]
async fn an_allowlist_that_readmits_a_spawn_tool_is_refused_loudly() {
    let allowed = HashSet::from(["echo".to_string(), "spawn_subagent".to_string()]);
    let (executed, warnings) = run_with_spawn_tool_in_parent_surface(allowed).await;

    assert!(
        !executed,
        "the backstop must still refuse the tool, not merely log about it"
    );
    assert_eq!(
        warnings.len(),
        1,
        "the readmitted spawn tool warns exactly once: {warnings:?}"
    );
}

/// #5934 (item 1): a mirrored sub-agent **tool result** must not be painted as
/// something the human typed.
///
/// Both worker-thread mirrors write tool output with `sender: "user"`, and the
/// renderer maps every non-`"agent"` sender to `role: 'user'`
/// (`app/src/providers/assistantUiMessages.ts:536`) while keying visibility
/// only on `extraMetadata.hidden` (`:664`, and the same flag in
/// `ChatThreadView.tsx:312` / `timeline/selectors.ts:50`). Worker threads are
/// openable chats — `create_worker_thread` stamps `labels: ["tasks"]` and
/// `threadFilter.ts` lists that tab — so an unflagged mirror row shows the
/// tool's raw output in a right-aligned user bubble.
///
/// The invariant: every non-`"agent"` row these mirrors write is `hidden`. The
/// record stays in the log for the process rail; it just stops being chat.
#[test]
fn mirrored_tool_results_are_hidden_from_the_worker_thread_chat() {
    use crate::openhuman::memory::conversations::{self as store, CreateConversationThread};

    let dir = std::env::temp_dir().join(format!("wt-5934-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    for id in ["worker-typed", "worker-recovered"] {
        store::ensure_thread(
            dir.clone(),
            CreateConversationThread {
                id: id.to_string(),
                title: "task".to_string(),
                created_at: now.clone(),
                parent_thread_id: Some("parent-1".to_string()),
                labels: Some(vec!["tasks".to_string()]),
                personality_id: None,
            },
        )
        .unwrap();
    }

    const RAW: &str = "{\"events\":[{\"title\":\"raw tool output the human never typed\"}]}";

    // The typed path (`ConversationMessage::ToolResults`), used on a normal run.
    mirror_worker_thread(
        &dir,
        "worker-typed",
        "researcher",
        "task-1",
        &[
            ConversationMessage::AssistantToolCalls {
                text: Some("checking the calendar".to_string()),
                tool_calls: vec![crate::openhuman::inference::provider::ToolCall {
                    id: "call-1".to_string(),
                    name: "list_events".to_string(),
                    arguments: "{}".to_string(),
                    extra_content: None,
                }],
                reasoning_content: None,
                extra_metadata: None,
            },
            ConversationMessage::ToolResults(vec![
                crate::openhuman::agent::messages::ToolResultMessage {
                    tool_call_id: "call-1".to_string(),
                    content: RAW.to_string(),
                },
            ]),
        ],
        Some("Here is your week."),
    );

    // The error-recovery path (`role: "tool"`), used when a run fails mid-turn.
    mirror_worker_thread_from_history(
        &dir,
        "worker-recovered",
        "researcher",
        "task-1",
        &[
            ChatMessage::assistant("checking the calendar"),
            ChatMessage::tool(RAW),
        ],
        Some("[subagent run failed before completion]"),
    );

    // Collected, not asserted per row: both mirrors must be reported, so a
    // failure names every path that is still painting a tool result as chat.
    let mut painted_as_user_chat: Vec<String> = Vec::new();
    for thread_id in ["worker-typed", "worker-recovered"] {
        let rows = store::get_messages(dir.clone(), thread_id).unwrap();
        assert!(
            rows.iter().any(|r| r.content == RAW),
            "{thread_id}: the mirror wrote no tool-result row to assert on"
        );
        for row in rows.iter().filter(|r| r.sender != "agent") {
            if row.extra_metadata.get("hidden").and_then(|v| v.as_bool()) != Some(true) {
                painted_as_user_chat.push(format!(
                    "{thread_id}: sender={:?} not hidden: {}",
                    row.sender, row.content
                ));
            }
        }
    }
    assert!(
        painted_as_user_chat.is_empty(),
        "mirrored tool results render in a user chat bubble:\n{}",
        painted_as_user_chat.join("\n")
    );

    let _ = std::fs::remove_dir_all(&dir);
}
