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

// -- #3104 / Codex #3779 Finding B: batch-break leaves no orphaned tool-call id -

/// Provider that emits a single assistant response carrying MULTIPLE native tool
/// calls in one message (the native-mode batch the reviewer flagged), then a
/// plain final text on any later call so the loop can terminate.
struct BatchToolCallProvider {
    served: Mutex<bool>,
    calls: Vec<ToolCall>,
}

impl BatchToolCallProvider {
    fn new(calls: Vec<ToolCall>) -> Arc<Self> {
        Arc::new(Self {
            served: Mutex::new(false),
            calls,
        })
    }
}

#[async_trait]
impl Provider for BatchToolCallProvider {
    async fn chat_with_system(
        &self,
        _system: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("noop".into())
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let mut served = self.served.lock().unwrap();
        if *served {
            // Loop should have already halted on the first batch; this is only a
            // safety net so the engine can never block waiting for more input.
            return Ok(ChatResponse {
                text: Some("FINAL".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            });
        }
        *served = true;
        Ok(ChatResponse {
            text: Some(String::new()),
            tool_calls: self.calls.clone(),
            usage: None,
            reasoning_content: None,
        })
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    async fn effective_context_window(&self, _model: &str) -> Option<u64> {
        None
    }
}

/// Tool source whose FIRST executed call returns a terminal (budget-exhausted)
/// delegated-inference failure carrying the sub-agent dispatch wrapper, so the
/// shared `RepeatFailureGuard` halts on the first occurrence and the batch
/// breaks before the remaining call(s) run. Any later call would succeed — but
/// must never be reached.
struct FailFirstToolSource {
    specs: Vec<crate::openhuman::tools::ToolSpec>,
    executed: Vec<String>,
}

#[async_trait]
impl ToolSource for FailFirstToolSource {
    fn request_specs(&self) -> &[crate::openhuman::tools::ToolSpec] {
        &self.specs
    }

    async fn execute_call(
        &mut self,
        call: &ParsedToolCall,
        _iteration: usize,
        _progress: &dyn super::ProgressReporter,
        _progress_call_id: &str,
    ) -> ToolRunResult {
        self.executed.push(call.name.clone());
        if self.executed.len() == 1 {
            // Sub-agent dispatch wrapper (`failed and did not complete`) + a
            // budget body → terminal inference failure → halt on first failure.
            ToolRunResult {
                text: "run_code failed and did not complete — no work was performed. \
                       Error: {\"error\":\"insufficient balance — add credits\"}"
                    .into(),
                success: false,
            }
        } else {
            ToolRunResult {
                text: "ok".into(),
                success: true,
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_with_source(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tool_source: &mut dyn ToolSource,
) -> TurnEngineOutcome {
    let progress = NullProgress;
    let mut observer = NullObserver;
    let checkpoint = ErrorCheckpoint;
    let parser = DefaultParser;
    let multimodal = MultimodalConfig::default();
    let multimodal_files = MultimodalFileConfig::default();

    run_turn_engine(
        provider,
        history,
        tool_source,
        &progress,
        &mut observer,
        &checkpoint,
        &parser,
        "test-provider",
        "ctx-test-model-xyz",
        0.0,
        true,
        &multimodal,
        &multimodal_files,
        8,
        None,
        &[],
        None,
        None,
    )
    .await
    .expect("turn engine should complete")
}

/// Collect the `tool_call` ids referenced by an assistant message's native-mode
/// JSON content (`{"content":…,"tool_calls":[{"id":…}, …]}`). Returns empty for
/// non-native / non-tool-call assistant messages.
fn assistant_tool_call_ids(content: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|v| {
            v.get("tool_calls").and_then(|tc| tc.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|c| c.get("id").and_then(|i| i.as_str()).map(str::to_string))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// Collect the `tool_call_id`s of every `role: tool` result message in history.
fn tool_result_ids(history: &[ChatMessage]) -> Vec<String> {
    history
        .iter()
        .filter(|m| m.role == "tool")
        .filter_map(|m| {
            serde_json::from_str::<serde_json::Value>(&m.content)
                .ok()
                .and_then(|v| {
                    v.get("tool_call_id")
                        .and_then(|i| i.as_str())
                        .map(str::to_string)
                })
        })
        .collect()
}

#[tokio::test]
async fn batch_break_trims_assistant_tool_calls_to_executed_no_orphan_id() {
    // The model emits THREE native tool calls in one message. The first returns a
    // terminal budget failure → the shared guard halts on the first occurrence
    // and the batch breaks, so calls #2 and #3 never run. The persisted assistant
    // message must reference ONLY the executed call id, and there must be exactly
    // one matching `role: tool` result — no orphaned tool-call id that an
    // OpenAI-compatible provider would reject on the next request.
    let provider = BatchToolCallProvider::new(vec![
        ToolCall {
            id: "call-A".into(),
            name: "run_code".into(),
            arguments: "{\"prompt\":\"a\"}".into(),
            extra_content: None,
        },
        ToolCall {
            id: "call-B".into(),
            name: "run_code".into(),
            arguments: "{\"prompt\":\"b\"}".into(),
            extra_content: None,
        },
        ToolCall {
            id: "call-C".into(),
            name: "run_code".into(),
            arguments: "{\"prompt\":\"c\"}".into(),
            extra_content: None,
        },
    ]);
    let mut tool_source = FailFirstToolSource {
        specs: Vec::new(),
        executed: Vec::new(),
    };
    let mut history = vec![ChatMessage::system("SYSTEM"), ChatMessage::user("TASK")];

    let outcome = run_with_source(provider.as_ref(), &mut history, &mut tool_source).await;

    // Only the first call ran; the batch broke before #2/#3.
    assert_eq!(
        tool_source.executed.as_slice(),
        ["run_code"],
        "only the first tool call should have executed before the terminal halt"
    );
    // The turn returned the root-cause halt summary (budget), not a generic stop.
    assert!(
        outcome.text.contains("out of inference budget"),
        "expected the budget root-cause halt summary, got: {}",
        outcome.text
    );

    // The persisted assistant message references ONLY the executed call id.
    let assistant = history
        .iter()
        .find(|m| m.role == "assistant" && m.content.contains("tool_calls"))
        .expect("an assistant message carrying tool_calls must be in history");
    let asst_ids = assistant_tool_call_ids(&assistant.content);
    assert_eq!(
        asst_ids,
        vec!["call-A".to_string()],
        "assistant tool-call list must be trimmed to the executed prefix \
         (no orphaned call-B/call-C): {asst_ids:?}"
    );

    // Exactly one tool result, matching that id — perfect id ↔ result pairing.
    let result_ids = tool_result_ids(&history);
    assert_eq!(
        result_ids,
        vec!["call-A".to_string()],
        "exactly one tool-result, matching the single executed tool-call id: {result_ids:?}"
    );

    // The invariant the provider enforces: every persisted assistant tool-call id
    // has a corresponding tool-result, and vice-versa (no orphans either way).
    assert_eq!(
        asst_ids, result_ids,
        "tool-call ids and tool-result ids must be in lockstep after a batch break"
    );
}

#[tokio::test]
async fn single_failing_call_pairs_id_with_result_boundary() {
    // Boundary: a SINGLE native tool call that fails terminally. There is no
    // un-executed suffix to trim, but the executed-prefix path must still produce
    // exactly one assistant tool-call id paired with one tool-result (the
    // degenerate case must not regress to zero results or an orphan).
    let provider = BatchToolCallProvider::new(vec![ToolCall {
        id: "only-1".into(),
        name: "run_code".into(),
        arguments: "{}".into(),
        extra_content: None,
    }]);
    let mut tool_source = FailFirstToolSource {
        specs: Vec::new(),
        executed: Vec::new(),
    };
    let mut history = vec![ChatMessage::system("SYSTEM"), ChatMessage::user("TASK")];

    run_with_source(provider.as_ref(), &mut history, &mut tool_source).await;

    let assistant = history
        .iter()
        .find(|m| m.role == "assistant" && m.content.contains("tool_calls"))
        .expect("assistant message with tool_calls");
    assert_eq!(assistant_tool_call_ids(&assistant.content), vec!["only-1"]);
    assert_eq!(tool_result_ids(&history), vec!["only-1"]);
}

#[tokio::test]
async fn full_batch_success_keeps_all_ids_paired() {
    // Success path: when NO break happens (all calls run), every emitted tool-call
    // id must be preserved and paired 1:1 with its result — proving the trim is
    // additive (only fires on truncation) and never drops calls on the happy path.
    struct AllOkToolSource {
        specs: Vec<crate::openhuman::tools::ToolSpec>,
    }
    #[async_trait]
    impl ToolSource for AllOkToolSource {
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

    let provider = BatchToolCallProvider::new(vec![
        ToolCall {
            id: "x1".into(),
            name: "noop".into(),
            arguments: "{}".into(),
            extra_content: None,
        },
        ToolCall {
            id: "x2".into(),
            name: "noop".into(),
            arguments: "{}".into(),
            extra_content: None,
        },
    ]);
    let mut tool_source = AllOkToolSource { specs: Vec::new() };
    let mut history = vec![ChatMessage::system("SYSTEM"), ChatMessage::user("TASK")];

    run_with_source(provider.as_ref(), &mut history, &mut tool_source).await;

    let assistant = history
        .iter()
        .find(|m| m.role == "assistant" && m.content.contains("tool_calls"))
        .expect("assistant message with tool_calls");
    let asst_ids = assistant_tool_call_ids(&assistant.content);
    let result_ids = tool_result_ids(&history);
    assert_eq!(
        asst_ids,
        vec!["x1".to_string(), "x2".to_string()],
        "both emitted tool-call ids must be preserved on the all-success path"
    );
    assert_eq!(
        result_ids,
        vec!["x1".to_string(), "x2".to_string()],
        "both tool results must be present and ordered on the all-success path"
    );
    assert_eq!(asst_ids, result_ids);
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
