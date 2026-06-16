//! Integration tests for the turn engine's autocompaction wiring.
//!
//! Layer 1 (`context::summarizer::tests`) proves `summarize_chat_history`
//! summarizes correctly when called directly. These tests prove the *glue*:
//! that `run_turn_engine` actually invokes it on its own when the context
//! guard reports the window is filling — and, just as importantly, that it
//! does NOT when a caller opts out (`autocompact = None`, the main-agent /
//! channel path). Without these, the feature could silently regress (e.g. a
//! refactor passing `None`, or the `CompactionNeeded` arm never reaching the
//! hook) while every unit test stayed green.
//!
//! The whole flow is driven deterministically with no network:
//!  * a scripted provider returns canned responses and reports usage that
//!    pushes the guard past its 0.90 trigger (95k / 100k tokens);
//!  * the provider pins `effective_context_window` to `None`, so the
//!    pre-dispatch token-budget trims stay disabled — autocompaction is the
//!    only thing that can mutate `history`;
//!  * the first response carries a tool call so the loop runs a second
//!    iteration, where `guard.check()` finally sees the recorded high usage.

use super::*;
use crate::openhuman::agent::harness::engine::progress::NullProgress;
use crate::openhuman::agent::harness::engine::{
    DefaultParser, ErrorCheckpoint, NullObserver, ToolRunResult, ToolSource,
};
use crate::openhuman::agent::harness::parse::ParsedToolCall;
use crate::openhuman::config::{MultimodalConfig, MultimodalFileConfig};
use crate::openhuman::context::EngineAutocompact;
use crate::openhuman::inference::provider::{ChatResponse, ToolCall, UsageInfo};
use async_trait::async_trait;
use std::sync::Mutex;

/// Provider that replays a queue of `chat()` responses and records every
/// `chat_with_history()` call — that method is ONLY reached via the
/// autocompaction summary, so its call count is a clean "compaction fired"
/// signal independent of inspecting `history`.
struct CompactionProvider {
    responses: Mutex<Vec<ChatResponse>>,
    summarize_calls: Mutex<usize>,
}

impl CompactionProvider {
    fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses),
            summarize_calls: Mutex::new(0),
        })
    }
    fn summarize_call_count(&self) -> usize {
        *self.summarize_calls.lock().unwrap()
    }
}

#[async_trait]
impl Provider for CompactionProvider {
    async fn chat_with_system(
        &self,
        _system: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("noop".into())
    }

    async fn chat_with_history(
        &self,
        _messages: &[ChatMessage],
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        *self.summarize_calls.lock().unwrap() += 1;
        Ok("COMPACTED-SUMMARY-BODY".into())
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let mut q = self.responses.lock().unwrap();
        Ok(if q.is_empty() {
            ChatResponse {
                text: Some("FINAL".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }
        } else {
            q.remove(0)
        })
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    /// Pin the effective context window to `None` so the pre-dispatch
    /// token-budget trims stay disabled deterministically — autocompaction is
    /// then the only thing that can mutate `history`. Don't rely on the
    /// unknown-model fallback (it would silently re-enable trimming if the
    /// static table ever learned the test's model id).
    async fn effective_context_window(&self, _model: &str) -> Option<u64> {
        None
    }
}

/// Minimal tool source: advertises nothing and reports success for any call,
/// so the engine's tool-execution seam is satisfied without real tools.
struct NoopToolSource {
    specs: Vec<crate::openhuman::tools::ToolSpec>,
}

#[async_trait]
impl ToolSource for NoopToolSource {
    fn request_specs(&self) -> &[crate::openhuman::tools::ToolSpec] {
        &self.specs
    }

    async fn execute_call(
        &mut self,
        _call: &ParsedToolCall,
        _iteration: usize,
        _progress: &dyn super::ProgressReporter,
        _progress_call_id: &str,
    ) -> ToolRunResult {
        ToolRunResult {
            text: "ok".into(),
            success: true,
        }
    }
}

/// First response: a tool call (so the loop runs a 2nd iteration) plus usage at
/// 95% of a 100k window (so the guard trips on that 2nd iteration). Second
/// response: plain final text, no tools, ending the loop.
fn scripted_responses() -> Vec<ChatResponse> {
    vec![
        ChatResponse {
            text: Some(String::new()),
            tool_calls: vec![ToolCall {
                id: "call-1".into(),
                name: "noop".into(),
                arguments: "{}".into(),
                extra_content: None,
            }],
            usage: Some(UsageInfo {
                input_tokens: 95_000,
                output_tokens: 0,
                context_window: 100_000,
                cached_input_tokens: 0,
                charged_amount_usd: 0.0,
            }),
            reasoning_content: None,
        },
        ChatResponse {
            text: Some("FINAL".into()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        },
    ]
}

/// Seed history: a leading system prompt that must survive compaction, plus
/// distinctly-labelled middle messages that must be summarized away.
fn seed_history() -> Vec<ChatMessage> {
    vec![
        ChatMessage::system("SYSTEM"),
        ChatMessage::user("TASK"),
        ChatMessage::assistant("MID-1"),
        ChatMessage::user("MID-2"),
        ChatMessage::assistant("MID-3"),
        ChatMessage::user("MID-4"),
        ChatMessage::user("TAIL-1"),
        ChatMessage::assistant("TAIL-2"),
    ]
}

#[allow(clippy::too_many_arguments)]
async fn run(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    autocompact: Option<&EngineAutocompact>,
) -> TurnEngineOutcome {
    let mut tool_source = NoopToolSource { specs: Vec::new() };
    let progress = NullProgress;
    let mut observer = NullObserver;
    let checkpoint = ErrorCheckpoint;
    let parser = DefaultParser;
    let multimodal = MultimodalConfig::default();
    let multimodal_files = MultimodalFileConfig::default();

    run_turn_engine(
        provider,
        history,
        &mut tool_source,
        &progress,
        &mut observer,
        &checkpoint,
        &parser,
        "test-provider",
        // The provider pins `effective_context_window` to `None`, so the
        // token-budget trims stay disabled, isolating autocompaction as the
        // only mutator. The model id is otherwise irrelevant here.
        "ctx-test-model-xyz",
        0.0,
        true,
        &multimodal,
        &multimodal_files,
        8,
        None,
        &[],
        None,
        autocompact,
    )
    .await
    .expect("turn engine should complete")
}

#[tokio::test]
async fn engine_autocompacts_history_when_guard_trips() {
    let provider = CompactionProvider::new(scripted_responses());
    let mut history = seed_history();

    let autocompact = EngineAutocompact {
        keep_recent: 2,
        temperature: 0.2,
        summarizer_model: None,
    };

    let outcome = run(provider.as_ref(), &mut history, Some(&autocompact)).await;
    assert_eq!(outcome.text, "FINAL");

    // The summary round-trip fired exactly once (only reachable via autocompact).
    assert_eq!(
        provider.summarize_call_count(),
        1,
        "guard should have triggered exactly one autocompaction summary call"
    );

    // Leading system prompt survived verbatim at the head.
    assert_eq!(history[0].role, "system");
    assert_eq!(history[0].content, "SYSTEM");

    // The reference-only summary (carrying the stub body) is now in history.
    assert!(
        history.iter().any(|m| {
            m.role == "system"
                && m.content.contains("COMPACTED-SUMMARY-BODY")
                && m.content.contains("REFERENCE ONLY")
                && m.content.contains("END OF CONTEXT SUMMARY")
        }),
        "expected a reference-only summary message in history: {history:?}"
    );

    // Middle messages were collapsed into the summary, not left verbatim.
    assert!(
        !history.iter().any(|m| m.content == "MID-1"),
        "old middle messages should have been summarized away: {history:?}"
    );
}

#[tokio::test]
async fn engine_does_not_autocompact_when_opted_out() {
    // Same guard-tripping scenario, but `autocompact = None` (the main-agent /
    // channel path). The engine must NOT summarize — proving the behavior is
    // gated on the opt-in, not on the guard alone.
    let provider = CompactionProvider::new(scripted_responses());
    let mut history = seed_history();

    let outcome = run(provider.as_ref(), &mut history, None).await;
    assert_eq!(outcome.text, "FINAL");

    assert_eq!(
        provider.summarize_call_count(),
        0,
        "no autocompaction summary call should happen when opted out"
    );
    // Original middle messages remain untouched.
    assert!(history.iter().any(|m| m.content == "MID-1"));
    assert!(
        !history
            .iter()
            .any(|m| m.content.contains("END OF CONTEXT SUMMARY")),
        "no summary message should be inserted when opted out"
    );
}
