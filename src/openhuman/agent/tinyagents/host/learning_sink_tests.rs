use super::*;
use std::sync::Mutex;
use tokio::sync::mpsc;

/// A hook that forwards each `TurnContext` it receives, so tests can await
/// the spawned fan-out deterministically instead of sleeping.
struct RecordingHook {
    tx: mpsc::UnboundedSender<TurnContext>,
}

#[async_trait]
impl PostTurnHook for RecordingHook {
    fn name(&self) -> &str {
        "recording"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        let _ = self.tx.send(ctx.clone());
        Ok(())
    }
}

/// A hook that always fails, to pin that a hook error never reaches the
/// sink's caller.
struct FailingHook {
    called: Arc<Mutex<bool>>,
}

#[async_trait]
impl PostTurnHook for FailingHook {
    fn name(&self) -> &str {
        "failing"
    }

    async fn on_turn_complete(&self, _ctx: &TurnContext) -> anyhow::Result<()> {
        *self.called.lock().unwrap() = true;
        Err(anyhow::anyhow!("hook blew up"))
    }
}

fn sample() -> TurnSummary {
    TurnSummary::new("thread-7", "orchestrator")
        .with_text("I prefer terse answers.", "Understood.")
        .with_tool("shell")
        .with_tool("read_file")
}

#[test]
fn projection_carries_identity_and_text() {
    let ctx = OpenHumanLearningSink::turn_context_from(&sample());
    assert_eq!(ctx.user_message, "I prefer terse answers.");
    assert_eq!(ctx.assistant_response, "Understood.");
    assert_eq!(ctx.session_id.as_deref(), Some("thread-7"));
    assert_eq!(ctx.agent_id.as_deref(), Some("orchestrator"));
    assert!(ctx.entrypoint.is_none());
    assert_eq!(ctx.iteration_count, 1);
    assert_eq!(ctx.turn_duration_ms, 0);
}

#[test]
fn tool_names_are_never_projected_into_tool_call_records() {
    // The regression this whole adapter is shaped around: a names-only
    // summary must not become fabricated `ToolCallRecord` outcomes, or the
    // tool_effectiveness tallies start recording invented successes.
    let summary = sample();
    assert!(summary.used_tools(), "fixture must carry tool names");
    let ctx = OpenHumanLearningSink::turn_context_from(&summary);
    assert!(
        ctx.tool_calls.is_empty(),
        "no outcome data exists to build a ToolCallRecord from"
    );
}

#[test]
fn blank_identifiers_normalize_to_none() {
    // A blank agent id must not become `Some("")`, which would key hook
    // state on an empty string rather than falling back to a global bucket.
    let summary = TurnSummary::new("   ", "  ").with_text("hi", "hello");
    let ctx = OpenHumanLearningSink::turn_context_from(&summary);
    assert!(ctx.session_id.is_none());
    assert!(ctx.agent_id.is_none());
}

#[tokio::test]
async fn on_turn_complete_dispatches_the_projected_context() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let sink = OpenHumanLearningSink::new(vec![Arc::new(RecordingHook { tx })]);
    assert_eq!(sink.hook_count(), 1);

    sink.on_turn_complete(&sample()).await.expect("advisory Ok");

    let ctx = rx.recv().await.expect("hook received the turn");
    assert_eq!(ctx.user_message, "I prefer terse answers.");
    assert_eq!(ctx.session_id.as_deref(), Some("thread-7"));
    assert!(ctx.tool_calls.is_empty());
}

#[tokio::test]
async fn a_failing_hook_does_not_fail_the_sink() {
    // The turn is already committed when this runs, so the crate contract
    // forbids surfacing a hook failure to the caller.
    let called = Arc::new(Mutex::new(false));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let sink = OpenHumanLearningSink::new(vec![
        Arc::new(FailingHook {
            called: Arc::clone(&called),
        }),
        Arc::new(RecordingHook { tx }),
    ]);

    assert!(sink.on_turn_complete(&sample()).await.is_ok());

    // Awaiting the surviving hook proves the fan-out ran past the failure.
    rx.recv().await.expect("the second hook still fires");
    assert!(*called.lock().unwrap(), "the failing hook was invoked");
}

#[tokio::test]
async fn an_empty_hook_list_is_a_successful_no_op() {
    let sink = OpenHumanLearningSink::new(Vec::new());
    assert_eq!(sink.hook_count(), 0);
    assert!(sink.on_turn_complete(&sample()).await.is_ok());
}

#[tokio::test]
async fn with_hook_appends_and_is_usable_as_a_trait_object() {
    // The runtime stores this capability as `Option<Arc<dyn LearningSink>>`;
    // pin that the adapter is object-safe in that position.
    let (tx, mut rx) = mpsc::unbounded_channel();
    let sink = OpenHumanLearningSink::new(Vec::new())
        .with_hook(Arc::new(RecordingHook { tx }) as Arc<dyn PostTurnHook>);
    assert_eq!(sink.hook_count(), 1);

    let sink: Arc<dyn LearningSink> = Arc::new(sink);
    sink.on_turn_complete(&sample()).await.expect("advisory Ok");
    rx.recv().await.expect("appended hook fires");
}

/// Inert [`Memory`] so `from_learning_config` can be exercised without a
/// store. Signatures mirror the trait impl in `learning/tool_tracker.rs`.
struct InertMemory;

#[async_trait]
impl Memory for InertMemory {
    fn name(&self) -> &str {
        "inert"
    }

    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: crate::openhuman::memory::MemoryCategory,
        _session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: crate::openhuman::memory::RecallOpts<'_>,
    ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(
        &self,
        _namespace: &str,
        _key: &str,
    ) -> anyhow::Result<Option<crate::openhuman::memory::MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&crate::openhuman::memory::MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
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

#[tokio::test]
async fn from_learning_config_installs_the_config_only_hooks() {
    // The point is the composition, not the hooks' own behaviour (which
    // they test themselves against their own mocks).
    let memory: Arc<dyn Memory> = Arc::new(InertMemory);
    let sink = OpenHumanLearningSink::from_learning_config(LearningConfig::default(), memory);
    assert_eq!(sink.hook_count(), 2, "user profile + tool tracker");
    // Learning is off by default, so both hooks self-gate to a no-op — the
    // call must still succeed.
    assert!(sink.on_turn_complete(&sample()).await.is_ok());
}
