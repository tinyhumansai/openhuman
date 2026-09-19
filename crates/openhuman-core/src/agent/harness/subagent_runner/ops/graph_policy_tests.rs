use super::*;
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
    ) -> tinyinference_llm::Result<ModelResponse> {
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
        "task-6157",
        false,
        None,
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
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
    use crate::memory::conversations::{self as store, CreateConversationThread};

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
                tool_calls: vec![crate::inference::provider::ToolCall {
                    id: "call-1".to_string(),
                    name: "list_events".to_string(),
                    arguments: "{}".to_string(),
                    extra_content: None,
                }],
                reasoning_content: None,
                extra_metadata: None,
            },
            ConversationMessage::ToolResults(vec![crate::agent::messages::ToolResultMessage {
                tool_call_id: "call-1".to_string(),
                content: RAW.to_string(),
            }]),
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
