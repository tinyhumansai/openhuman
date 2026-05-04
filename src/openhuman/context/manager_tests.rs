use super::*;
use crate::openhuman::providers::{ChatMessage, ToolCall, ToolResultMessage};
use async_trait::async_trait;
use std::sync::Mutex;

fn user(s: &str) -> ConversationMessage {
    ConversationMessage::Chat(ChatMessage::user(s))
}

fn call(id: &str) -> ConversationMessage {
    ConversationMessage::AssistantToolCalls {
        text: None,
        tool_calls: vec![ToolCall {
            id: id.into(),
            name: "t".into(),
            arguments: "{}".into(),
        }],
    }
}

fn result(id: &str, body: &str) -> ConversationMessage {
    ConversationMessage::ToolResults(vec![ToolResultMessage {
        tool_call_id: id.into(),
        content: body.into(),
    }])
}

/// Mock summarizer that records how many times it was called and
/// can be configured to succeed or fail.
struct MockSummarizer {
    calls: Mutex<usize>,
    should_fail: bool,
}

impl MockSummarizer {
    fn ok() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(0),
            should_fail: false,
        })
    }
    fn failing() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(0),
            should_fail: true,
        })
    }
    fn call_count(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl Summarizer for MockSummarizer {
    async fn summarize(
        &self,
        history: &mut Vec<ConversationMessage>,
        _model: &str,
    ) -> Result<SummaryStats> {
        *self.calls.lock().unwrap() += 1;
        if self.should_fail {
            anyhow::bail!("mock failure");
        }
        // Rewrite the history to a single system summary to
        // simulate a successful reduction.
        let removed = history.len();
        history.clear();
        history.push(ConversationMessage::Chat(ChatMessage::system(
            "mock summary",
        )));
        Ok(SummaryStats {
            messages_removed: removed,
            approx_tokens_freed: 1_000,
            summary_chars: 12,
        })
    }
}

fn manager_with(summarizer: Arc<dyn Summarizer>) -> ContextManager {
    let config = ContextConfig::default();
    ContextManager::new(
        &config,
        summarizer,
        "test-model".into(),
        SystemPromptBuilder::with_defaults(),
    )
}

#[tokio::test]
async fn reduce_returns_noop_when_guard_is_healthy() {
    let summarizer = MockSummarizer::ok();
    let mut manager = manager_with(summarizer.clone());

    // Low utilisation — guard says ok, pipeline is a no-op.
    manager.record_usage(&UsageInfo {
        input_tokens: 5_000,
        output_tokens: 500,
        context_window: 100_000,
        ..Default::default()
    });

    let mut history = vec![user("hi")];
    let outcome = manager.reduce_before_call(&mut history).await.unwrap();

    assert!(matches!(outcome, ReductionOutcome::NoOp));
    assert_eq!(summarizer.call_count(), 0);
}

#[tokio::test]
async fn reduce_surfaces_microcompact_without_calling_summarizer() {
    let summarizer = MockSummarizer::ok();
    let mut manager = manager_with(summarizer.clone());

    // Push utilisation above the 90% soft threshold.
    manager.record_usage(&UsageInfo {
        input_tokens: 92_000,
        output_tokens: 4_000,
        context_window: 100_000,
        ..Default::default()
    });

    // Build a history with several older tool-result envelopes
    // that microcompact can clear — the default keep_recent is
    // DEFAULT_KEEP_RECENT_TOOL_RESULTS (5), so include at least
    // 7 pairs so the older ones are eligible.
    let mut history = vec![
        call("t1"),
        result("t1", &"x".repeat(5_000)),
        call("t2"),
        result("t2", &"x".repeat(5_000)),
        call("t3"),
        result("t3", "a"),
        call("t4"),
        result("t4", "b"),
        call("t5"),
        result("t5", "c"),
        call("t6"),
        result("t6", "d"),
        call("t7"),
        result("t7", "e"),
    ];
    let outcome = manager.reduce_before_call(&mut history).await.unwrap();

    match outcome {
        ReductionOutcome::Microcompacted {
            envelopes_cleared, ..
        } => {
            assert!(envelopes_cleared > 0);
        }
        other => panic!("expected Microcompacted, got {other:?}"),
    }
    assert_eq!(
        summarizer.call_count(),
        0,
        "microcompact must not invoke summarizer"
    );
}

#[tokio::test]
async fn reduce_dispatches_summarizer_and_records_success() {
    let summarizer = MockSummarizer::ok();
    let mut manager = manager_with(summarizer.clone());

    manager.record_usage(&UsageInfo {
        input_tokens: 92_000,
        output_tokens: 4_000,
        context_window: 100_000,
        ..Default::default()
    });

    // History with no old tool-result envelopes — microcompact
    // has nothing to clear, so the pipeline signals
    // AutocompactionRequested and the manager calls the summarizer.
    let mut history = vec![user("one"), user("two"), user("three")];
    let outcome = manager.reduce_before_call(&mut history).await.unwrap();

    match outcome {
        ReductionOutcome::Summarized(stats) => {
            assert_eq!(stats.messages_removed, 3);
        }
        other => panic!("expected Summarized, got {other:?}"),
    }
    assert_eq!(summarizer.call_count(), 1);
    assert_eq!(
        history.len(),
        1,
        "mock replaced history with a single summary msg"
    );
    // Guard breaker should NOT be tripped on success.
    assert!(!manager.pipeline.guard.is_compaction_disabled());
}

#[tokio::test]
async fn summarizer_failure_trips_breaker_after_three_tries() {
    let summarizer = MockSummarizer::failing();
    let mut manager = manager_with(summarizer);

    manager.record_usage(&UsageInfo {
        input_tokens: 92_000,
        output_tokens: 4_000,
        context_window: 100_000,
        ..Default::default()
    });

    // Try three times — each call sends the pipeline into
    // AutocompactionRequested, the mock summarizer fails, and
    // the breaker nudges forward. The fourth call should report
    // Exhausted because the breaker is tripped.
    for _ in 0..3 {
        let mut history = vec![user("a"), user("b"), user("c")];
        let outcome = manager.reduce_before_call(&mut history).await.unwrap();
        match outcome {
            ReductionOutcome::SummarizationFailed { .. } => {}
            other => panic!("expected SummarizationFailed, got {other:?}"),
        }
    }
    assert!(manager.pipeline.guard.is_compaction_disabled());

    // Nudge the guard above the hard limit so the next pipeline
    // pass returns ContextExhausted.
    manager.record_usage(&UsageInfo {
        input_tokens: 96_000,
        output_tokens: 2_000,
        context_window: 100_000,
        ..Default::default()
    });
    let mut history = vec![user("x")];
    let outcome = manager.reduce_before_call(&mut history).await.unwrap();
    assert!(matches!(outcome, ReductionOutcome::Exhausted { .. }));
}

#[tokio::test]
async fn disabled_autocompact_returns_not_attempted() {
    let summarizer = MockSummarizer::ok();
    let mut config = ContextConfig::default();
    // Keep master switch on but disable just the autocompact stage
    // so the pipeline routes through AutocompactionDisabled instead
    // of NoOp.
    config.autocompact_enabled = false;
    let mut manager = ContextManager::new(
        &config,
        summarizer.clone(),
        "test-model".into(),
        SystemPromptBuilder::with_defaults(),
    );

    manager.record_usage(&UsageInfo {
        input_tokens: 92_000,
        output_tokens: 4_000,
        context_window: 100_000,
        ..Default::default()
    });

    // No old tool-result envelopes — microcompact cannot free
    // anything, so the pipeline lands in the autocompact branch.
    let mut history = vec![user("one"), user("two"), user("three")];
    let outcome = manager.reduce_before_call(&mut history).await.unwrap();

    match outcome {
        ReductionOutcome::NotAttempted { utilisation_pct } => {
            assert!(utilisation_pct >= 90);
        }
        other => panic!("expected NotAttempted, got {other:?}"),
    }
    assert_eq!(
        summarizer.call_count(),
        0,
        "summarizer must not run when autocompact is disabled"
    );
}

#[tokio::test]
async fn disabled_manager_returns_noop() {
    let summarizer = MockSummarizer::ok();
    let mut config = ContextConfig::default();
    config.enabled = false;
    let mut manager = ContextManager::new(
        &config,
        summarizer.clone(),
        "test-model".into(),
        SystemPromptBuilder::with_defaults(),
    );

    // High utilisation would normally trigger something.
    manager.record_usage(&UsageInfo {
        input_tokens: 96_000,
        output_tokens: 2_000,
        context_window: 100_000,
        ..Default::default()
    });

    let mut history = vec![user("a"), user("b"), user("c")];
    let outcome = manager.reduce_before_call(&mut history).await.unwrap();
    assert!(matches!(outcome, ReductionOutcome::NoOp));
    assert_eq!(summarizer.call_count(), 0);
}

#[test]
fn stats_reports_snapshot() {
    let summarizer = MockSummarizer::ok();
    let mut manager = manager_with(summarizer);
    manager.record_usage(&UsageInfo {
        input_tokens: 10_000,
        output_tokens: 2_000,
        context_window: 100_000,
        ..Default::default()
    });
    manager.tick_turn();
    manager.record_tool_calls(3);

    let s = manager.stats();
    assert_eq!(s.input_tokens, 10_000);
    assert_eq!(s.output_tokens, 2_000);
    assert_eq!(s.context_window, 100_000);
    assert_eq!(s.utilisation_pct, Some(12));
    assert_eq!(s.session_memory_total_tokens, 12_000);
    assert_eq!(s.session_memory_current_turn, 1);
    assert_eq!(s.session_memory_total_tool_calls, 3);
}
